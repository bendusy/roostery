//! `bot_bridge::relay_task` — chat_id → TaskRef 缓存 + step 文案。
//!
//! 见 `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md`
//! §2.1 / §2.2 / §2.3（record_start / record_end / record_adjust / EndOutcome /
//! BOT_CHAT_CACHE_SCHEMA_VERSION / `~/.roostery/state/bot_chats/{app}/{safe_chat}.json`）。
//!
//! step 5 实装：
//! - 每个 BotRole 独立 chat→TaskRef 缓存目录（与 `bot_task_writer::session_tasks/` 平级）
//! - cache 原子写：`.tmp.<pid>.<nanos>` + fs::rename（POSIX 单 inode 原子）
//! - chat_id safe_filename 在文件名层做（防路径跳出 `..` / `/`）
//! - 三态 step 文案：🚀 已收到 / 🔁 用户调整 / ✅ / ❌ / ⚠️ / ⏱️ —— 与 design §3 验收契约对齐
//! - idempotency_key 模板 = `relay:{kind}:{message_id}:{bot_app_id}`（design §2.2 流程级约束）
//!
//! 测试约定参 `bot_task_writer` 模块——`isolate_home` 走 `ROOSTERY_HOME` env，
//! 测试共享 `paths::TEST_ENV_LOCK` 串行化。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::bot_task_writer::{
    AppendStepsOptions, CreateTaskOptions, TaskGuid, TaskRef, TaskWriterError, append_steps,
    create_task,
};
use crate::lark_cli::LarkRunner;

use crate::bot_bridge::event::ImEvent;
use crate::bot_bridge::role::BotRole;

/// cache schema 公开承诺；design §2.1 + 验收契约。
pub const BOT_CHAT_CACHE_SCHEMA_VERSION: u32 = 1;

/// runner 终态——relay_task 的 step 文案 + `runner.rs` 的内部判定共享此 enum。
///
/// design §2.1：四态 Success / Failed / Aborted / Timeout。
/// 每态对应一条 step 文案，前缀 emoji 与 design §3 验收契约 N1/B5/B6 对齐。
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EndOutcome {
    Success { adjust_attempts: u32 },
    Failed { exit_code: i32 },
    Aborted { reason: String },
    Timeout,
}

/// relay_task 错误。design §2.1 两类基本，本实装把 cache 分 load/save 两支
/// 便于诊断（同 TaskWriterError 风格）。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RelayTaskError {
    #[error("task writer failed: {0}")]
    TaskWriter(#[from] TaskWriterError),
    #[error("cache load failed at {path}: {source}")]
    CacheLoad {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cache save failed at {path}: {source}")]
    CacheSave {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

// --- cache schema -------------------------------------------------------

/// 单条 (bot_app_id, chat_id) 的缓存条目。schema_version=1 公开承诺；
/// `#[serde(default)]` 让旧版（缺 schema_version 字段）反序列化时视为 0，
/// `load_cache` 内对 0 当作 1 兼容——只要其他必填字段在，就放行。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct BotChatCacheEntry {
    #[serde(default)]
    schema_version: u32,
    task_guid: String,
    task_url: String,
    chat_id: String,
    bot_app_id: String,
    created_at: String,
    #[serde(default)]
    adjust_count: u32,
    #[serde(default)]
    end_outcome: Option<EndOutcomeRecord>,
}

/// `EndOutcome` 的可序列化镜像（不直接 derive Serialize 在 pub enum 上保留
/// `#[non_exhaustive]`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum EndOutcomeRecord {
    Success { adjust_attempts: u32 },
    Failed { exit_code: i32 },
    Aborted { reason: String },
    Timeout,
}

impl From<&EndOutcome> for EndOutcomeRecord {
    fn from(o: &EndOutcome) -> Self {
        match o {
            EndOutcome::Success { adjust_attempts } => Self::Success {
                adjust_attempts: *adjust_attempts,
            },
            EndOutcome::Failed { exit_code } => Self::Failed {
                exit_code: *exit_code,
            },
            EndOutcome::Aborted { reason } => Self::Aborted {
                reason: reason.clone(),
            },
            EndOutcome::Timeout => Self::Timeout,
        }
    }
}

// --- path helpers ------------------------------------------------------

/// 把 chat_id 安全拼成单文件名。与 `bot_task_writer::safe_filename` 同语义：
/// 非 `[A-Za-z0-9._-]` 替 `_`；连续 `..` 替 `__`；末尾加 `.json`。
/// bot_app_id 层的清洗在 `paths::bot_chat_cache_dir` 内已做，本函数只管文件名层。
fn safe_chat_filename(chat_id: &str) -> String {
    let mut cleaned: String = chat_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", "__");
    }
    if cleaned.is_empty() {
        cleaned.push('_');
    }
    format!("{cleaned}.json")
}

fn cache_path_for(bot_app_id: &str, chat_id: &str) -> PathBuf {
    crate::paths::bot_chat_cache_dir(bot_app_id).join(safe_chat_filename(chat_id))
}

// --- cache I/O ---------------------------------------------------------

/// 读 cache。文件不存在返 Ok(None)；解析失败返 Ok(None) 让 caller 走 create
/// 自然修复（与 `bot_task_writer::load_cache` 同策略，design §3.7 C7.3 借用）。
fn load_cache(path: &Path) -> Result<Option<BotChatCacheEntry>, RelayTaskError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(RelayTaskError::CacheLoad {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    match serde_json::from_slice::<BotChatCacheEntry>(&bytes) {
        Ok(mut entry) => {
            // 旧版缺 schema_version → serde default = 0；视为 1（向后兼容）。
            if entry.schema_version == 0 {
                entry.schema_version = BOT_CHAT_CACHE_SCHEMA_VERSION;
            }
            Ok(Some(entry))
        }
        Err(_) => Ok(None),
    }
}

/// 原子写：tmp 文件 = `<stem>.tmp.<pid>.<nanos>` → write → rename。
/// 复用 `bot_task_writer::save_cache` 的同款抗并发 race pattern（codex audit round-3
/// finding，记 `bot_task_writer` 模块注释）；rename 在 POSIX 同 fs 内原子。
fn save_cache(path: &Path, entry: &BotChatCacheEntry) -> Result<(), RelayTaskError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| RelayTaskError::CacheSave {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let body = serde_json::to_vec_pretty(entry).expect("BotChatCacheEntry serializes");
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!(
        "{stem}.tmp.{pid}.{nanos}",
        stem = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("bot_chat"),
    );
    let tmp = path.with_file_name(tmp_name);
    std::fs::write(&tmp, &body).map_err(|source| RelayTaskError::CacheSave {
        path: path.to_path_buf(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(|source| RelayTaskError::CacheSave {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

// --- idempotency key ---------------------------------------------------

fn idem_key(kind: &str, message_id: &str, bot_app_id: &str) -> String {
    format!("relay:{kind}:{message_id}:{bot_app_id}")
}

// --- step 文案 ----------------------------------------------------------

fn step_text_start(brief: &str) -> String {
    format!("🚀 已收到：{brief}")
}

fn step_text_adjust(attempt: u32, body: &str) -> String {
    format!("🔁 用户调整 (attempt {attempt}): {body}")
}

fn step_text_end(outcome: &EndOutcome, result: &str) -> String {
    match outcome {
        EndOutcome::Success { adjust_attempts } => {
            format!("✅ 完成（adjust={adjust_attempts}）: {result}")
        }
        EndOutcome::Failed { exit_code } => {
            format!("❌ 失败 (exit={exit_code}): {result}")
        }
        EndOutcome::Aborted { reason } => {
            format!("⚠️ 用户请求中止: {reason}")
        }
        EndOutcome::Timeout => "⏱️ 超时".to_string(),
    }
}

// --- public API --------------------------------------------------------

/// 记录 runner 启动：cache hit 复用 TaskRef；cache miss 调 `create_task` 建
/// 新飞书 task + 写 cache + append "🚀 已收到 ..." step。
///
/// 返回 `None` 仅在异常路径已被吸收（当前实现总是返 `Some` 或 Err）。
/// 保留 `Option<TaskRef>` 签名是因为 runner.rs 已按此 shape 调用，便于未来
/// 把"create_task 失败但不阻塞 runner 主路径"（design §3 E2）退化为 `None`。
pub async fn record_start(
    lark: &dyn LarkRunner,
    bot: &BotRole,
    event: &ImEvent,
    message_brief: &str,
) -> Result<Option<TaskRef>, RelayTaskError> {
    let path = cache_path_for(&bot.app_id, &event.chat_id);

    let start_key = idem_key("step-start", &event.message_id, &bot.app_id);

    // cache hit → 复用 TaskGuid，不调 lark create_task；但仍 append "🚀 ..." step
    // 以让接力 task 的连续 @ 都留下时间线（N2 验收）。
    if let Some(entry) = load_cache(&path)? {
        let task_ref = TaskRef {
            guid: TaskGuid::from_existing(entry.task_guid.clone()),
            url: entry.task_url.clone(),
        };
        let step = step_text_start(message_brief);
        let opts = AppendStepsOptions::new()
            .with_idempotency_key(&start_key)
            .with_profile(&bot.app_id);
        append_steps(lark, &task_ref.guid, &[step.as_str()], opts).await?;
        return Ok(Some(task_ref));
    }

    // cache miss → create_task + 写 cache
    let cwd_str = bot.default_cwd.to_string_lossy().to_string();
    let create_key = idem_key("create", &event.message_id, &bot.app_id);
    let create_opts = CreateTaskOptions::new()
        .with_idempotency_key(&create_key)
        .with_profile(&bot.app_id);
    // design §3 E2：create_task 失败不阻塞 runner 主路径——吸收 TaskWriter 错误，
    // 返 Ok(None) 让 runner 走 placeholder guid 路径，reply 不含 task URL。
    let task_ref = match create_task(lark, &bot.app_id, &cwd_str, message_brief, create_opts).await
    {
        Ok(tr) => tr,
        Err(_) => return Ok(None),
    };

    let entry = BotChatCacheEntry {
        schema_version: BOT_CHAT_CACHE_SCHEMA_VERSION,
        task_guid: task_ref.guid.as_str().to_string(),
        task_url: task_ref.url.clone(),
        chat_id: event.chat_id.clone(),
        bot_app_id: bot.app_id.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        adjust_count: 0,
        end_outcome: None,
    };
    save_cache(&path, &entry)?;

    let step = step_text_start(message_brief);
    let opts = AppendStepsOptions::new()
        .with_idempotency_key(&start_key)
        .with_profile(&bot.app_id);
    append_steps(lark, &task_ref.guid, &[step.as_str()], opts).await?;

    Ok(Some(task_ref))
}

/// 记录 /adjust 重启：append step "🔁 用户调整 (attempt N): ..." + cache.adjust_count += 1。
///
/// `chat_id` 是 cache 索引键——record_start / record_end 已按 chat_id 落 cache，
/// 本 fn 需读回同一条 entry 累加 adjust_count，因此 caller 必须把 chat_id 一起传。
pub async fn record_adjust(
    lark: &dyn LarkRunner,
    bot: &BotRole,
    chat_id: &str,
    task_ref: &TaskRef,
    adjust_text: &str,
    attempt: u32,
) -> Result<(), RelayTaskError> {
    let path = cache_path_for(&bot.app_id, chat_id);
    if let Ok(Some(mut entry)) = load_cache(&path) {
        entry.adjust_count = entry.adjust_count.saturating_add(1);
        save_cache(&path, &entry)?;
    }

    let step = step_text_adjust(attempt, adjust_text);
    let adjust_key = idem_key(
        &format!("adjust-{attempt}"),
        task_ref.guid.as_str(),
        &bot.app_id,
    );
    let opts = AppendStepsOptions::new()
        .with_idempotency_key(&adjust_key)
        .with_profile(&bot.app_id);
    append_steps(lark, &task_ref.guid, &[step.as_str()], opts).await?;
    Ok(())
}

/// 记录 runner 终态：append step 按 outcome 走对应文案 + cache.end_outcome 落盘。
///
/// `chat_id` + `source_message_id` 是 runner.rs 已有的上下文，本 fn 仅消费。
pub async fn record_end(
    lark: &dyn LarkRunner,
    bot: &BotRole,
    chat_id: &str,
    source_message_id: &str,
    outcome: &EndOutcome,
    result_text: &str,
) -> Result<Option<TaskRef>, RelayTaskError> {
    let path = cache_path_for(&bot.app_id, chat_id);
    let Some(mut entry) = load_cache(&path)? else {
        // cache 没建（record_start 失败 / 丢失）——append step 无可附，直接返 None。
        // runner.rs 把这视作"无 task URL 可写"的 E2 退化路径。
        return Ok(None);
    };
    entry.end_outcome = Some(EndOutcomeRecord::from(outcome));
    save_cache(&path, &entry)?;

    let task_ref = TaskRef {
        guid: TaskGuid::from_existing(entry.task_guid.clone()),
        url: entry.task_url.clone(),
    };
    let step = step_text_end(outcome, result_text);
    let end_key = idem_key("step-end", source_message_id, &bot.app_id);
    let opts = AppendStepsOptions::new()
        .with_idempotency_key(&end_key)
        .with_profile(&bot.app_id);
    append_steps(lark, &task_ref.guid, &[step.as_str()], opts).await?;
    Ok(Some(task_ref))
}

// =====================================================================
// tests
// =====================================================================

#[cfg(test)]
#[allow(clippy::await_holding_lock)] // ENV_LOCK serializes ROOSTERY_HOME (paths::TEST_ENV_LOCK pattern)
mod tests {
    use super::*;
    use crate::lark_cli::mock::MockLarkRunner;
    use crate::paths::TEST_ENV_LOCK as ENV_LOCK;
    use serde_json::json;
    use std::path::PathBuf;

    fn mk_bot(app_id: &str) -> BotRole {
        BotRole {
            app_id: app_id.into(),
            role: "scout".into(),
            mention_alias: "tl".into(),
            runner: "test".into(),
            default_cwd: PathBuf::from("/tmp"),
            prompt_template: "{message}".into(),
            reply_template: "{result}".into(),
            chat_whitelist: vec![],
            next_bot_mention: String::new(),
        }
    }

    fn mk_event(chat: &str, msg: &str) -> ImEvent {
        ImEvent {
            message_id: msg.into(),
            chat_id: chat.into(),
            chat_type: "group".into(),
            message_type: "text".into(),
            sender_id: "u1".into(),
            content: "@tl do it".into(),
        }
    }

    fn task_create_response() -> serde_json::Value {
        json!({
            "ok": true,
            "data": {
                "guid": "task_guid_xyz",
                "url": "https://feishu.cn/task/xyz"
            }
        })
    }

    fn auth_status_response() -> serde_json::Value {
        json!({
            "userOpenId": "ou_user_test",
            "userName": "TestUser",
            "appId": "cli_bot",
            "tokenStatus": "valid",
        })
    }

    fn profile_list_response() -> serde_json::Value {
        json!([{"name": "default", "active": true}])
    }

    /// 灌一组 record_start cache-miss 路径所需 mock 响应：
    /// auth_status + profile_list + create_task + append_start_step。
    fn enqueue_record_start_miss(mock: &MockLarkRunner) {
        mock.enqueue_ok(auth_status_response());
        mock.enqueue_ok(profile_list_response());
        mock.enqueue_ok(task_create_response());
        mock.enqueue_ok(json!({"ok": true}));
    }

    fn isolate_home(tmp: &tempfile::TempDir) {
        unsafe { std::env::set_var("ROOSTERY_HOME", tmp.path()) };
        unsafe { std::env::set_var("ROOSTERY_HOST", "m4") };
    }
    fn restore_home() {
        unsafe { std::env::remove_var("ROOSTERY_HOME") };
        unsafe { std::env::remove_var("ROOSTERY_HOST") };
    }

    // --- T1: cache hit returns same TaskGuid -----------------------------

    #[tokio::test]
    async fn record_start_cache_hit_reuses_task_guid() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);

        let mock = MockLarkRunner::new();
        // 第一次：auth + profile + create_task + append step = 4 calls
        enqueue_record_start_miss(&mock);
        // 第二次：cache hit，仅 append step → 1 call
        mock.enqueue_ok(json!({"ok": true}));

        let bot = mk_bot("cli_bot_a");
        let ev1 = mk_event("oc_relay", "om_1");
        let ev2 = mk_event("oc_relay", "om_2");

        let r1 = record_start(&mock, &bot, &ev1, "brief 1").await.unwrap();
        let r2 = record_start(&mock, &bot, &ev2, "brief 2").await.unwrap();
        let g1 = r1.unwrap().guid;
        let g2 = r2.unwrap().guid;
        assert_eq!(g1, g2, "cache hit must reuse same TaskGuid");

        // 总调用次数：4 (first miss) + 1 (second hit append) = 5
        assert_eq!(mock.calls().len(), 5);

        restore_home();
    }

    // --- T2: schema_version 缺失向后兼容 ---------------------------------

    #[test]
    fn load_cache_missing_schema_version_compat() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);

        let path = cache_path_for("cli_bot_b", "oc_legacy");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // 手写缺 schema_version 的 entry
        let legacy = r#"{
            "task_guid": "g_legacy",
            "task_url": "https://feishu.cn/task/legacy",
            "chat_id": "oc_legacy",
            "bot_app_id": "cli_bot_b",
            "created_at": "2026-05-19T00:00:00Z"
        }"#;
        std::fs::write(&path, legacy).unwrap();

        let entry = load_cache(&path).unwrap().expect("legacy cache loads");
        assert_eq!(
            entry.schema_version, BOT_CHAT_CACHE_SCHEMA_VERSION,
            "missing schema_version should normalize to current"
        );
        assert_eq!(entry.task_guid, "g_legacy");

        restore_home();
    }

    // --- T3: safe_filename 路径跳出防御 ----------------------------------

    #[test]
    fn safe_chat_filename_neutralizes_path_traversal() {
        // 各种 path-traversal payload
        let r1 = safe_chat_filename("../../etc/passwd");
        assert!(!r1.contains(".."), "got {r1}");
        assert!(!r1.contains('/'), "got {r1}");
        assert!(r1.ends_with(".json"));

        let r2 = safe_chat_filename("/absolute/path");
        assert!(!r2.contains('/'), "got {r2}");
        assert!(r2.ends_with(".json"));

        let r3 = safe_chat_filename("..");
        assert!(!r3.contains(".."), "got {r3}");
        assert!(r3.ends_with(".json"));

        // 空串退化
        let r4 = safe_chat_filename("");
        assert_eq!(r4, "_.json");
    }

    #[test]
    fn cache_path_stays_under_bot_chat_dir_with_evil_chat_id() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);

        let p = cache_path_for("cli_bot_c", "../../../etc/passwd");
        let p_str = p.to_string_lossy().to_string();
        assert!(!p_str.contains(".."), "must not contain '..', got {p_str}");
        // 必须落在 state/bot_chats/cli_bot_c/ 下
        let expected_prefix = tmp.path().join("state/bot_chats/cli_bot_c/");
        assert!(
            p.starts_with(&expected_prefix),
            "must stay under {expected_prefix:?}, got {p:?}"
        );

        restore_home();
    }

    // --- T4: EndOutcome 四态 step 文案匹配 -------------------------------

    #[test]
    fn step_text_end_success_contains_emoji_and_count() {
        let s = step_text_end(&EndOutcome::Success { adjust_attempts: 2 }, "all done");
        assert!(s.contains("✅"), "got: {s}");
        assert!(s.contains("adjust=2"), "got: {s}");
        assert!(s.contains("all done"), "got: {s}");
    }

    #[test]
    fn step_text_end_failed_contains_emoji_and_exit_code() {
        let s = step_text_end(&EndOutcome::Failed { exit_code: 42 }, "boom");
        assert!(s.contains("❌"), "got: {s}");
        assert!(s.contains("exit=42"), "got: {s}");
        assert!(s.contains("boom"), "got: {s}");
    }

    #[test]
    fn step_text_end_aborted_contains_warning_emoji_and_reason() {
        let s = step_text_end(
            &EndOutcome::Aborted {
                reason: "/stop".into(),
            },
            "ignored",
        );
        assert!(s.contains("⚠️"), "got: {s}");
        assert!(s.contains("中止"), "got: {s}");
        assert!(s.contains("/stop"), "got: {s}");
    }

    #[test]
    fn step_text_end_timeout_contains_clock_emoji() {
        let s = step_text_end(&EndOutcome::Timeout, "ignored");
        assert!(s.contains("⏱️"), "got: {s}");
        assert!(s.contains("超时"), "got: {s}");
    }

    // --- T5: 与 session_tasks 平级不互相干扰 -----------------------------

    #[test]
    fn bot_chats_dir_is_sibling_of_session_tasks() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);

        let bot_chats_parent = crate::paths::bot_chat_cache_dir("any_bot");
        let session_tasks_dir = crate::paths::state_dir().join("session_tasks");
        // 兄弟目录：都在 state_dir 下，但分支不同
        assert_eq!(
            bot_chats_parent.parent().unwrap().file_name().unwrap(),
            "bot_chats"
        );
        assert_eq!(
            bot_chats_parent.parent().unwrap().parent().unwrap(),
            session_tasks_dir.parent().unwrap()
        );
        assert_ne!(
            bot_chats_parent.parent().unwrap().file_name(),
            session_tasks_dir.file_name()
        );

        restore_home();
    }

    // --- T6: record_end 追加正确 outcome step ---------------------------

    #[tokio::test]
    async fn record_adjust_increments_cache_count() {
        // 验证 chat_id 入参后 cache.adjust_count 真实递增（之前 task_ref.guid
        // 兜底派生路径 cache miss → 计数器丢失）。
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);

        let mock = MockLarkRunner::new();
        enqueue_record_start_miss(&mock);
        // 两次 adjust 各 1 次 append_steps 调用
        mock.enqueue_ok(json!({"ok": true}));
        mock.enqueue_ok(json!({"ok": true}));

        let bot = mk_bot("cli_bot_adj");
        let ev = mk_event("oc_adj_chat", "om_init");
        let task_ref = record_start(&mock, &bot, &ev, "init brief")
            .await
            .unwrap()
            .unwrap();

        record_adjust(&mock, &bot, &ev.chat_id, &task_ref, "first adjust", 1)
            .await
            .unwrap();
        record_adjust(&mock, &bot, &ev.chat_id, &task_ref, "second adjust", 2)
            .await
            .unwrap();

        let path = cache_path_for(&bot.app_id, &ev.chat_id);
        let entry = load_cache(&path).unwrap().expect("cache entry exists");
        assert_eq!(
            entry.adjust_count, 2,
            "two adjusts must increment cache counter to 2"
        );

        restore_home();
    }

    #[tokio::test]
    async fn record_end_appends_outcome_step_and_persists() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);

        let mock = MockLarkRunner::new();
        enqueue_record_start_miss(&mock); // auth + profile + create + start step
        mock.enqueue_ok(json!({"ok": true})); // end step

        let bot = mk_bot("cli_bot_end");
        let ev = mk_event("oc_end", "om_e1");

        record_start(&mock, &bot, &ev, "brief").await.unwrap();
        let outcome = EndOutcome::Success { adjust_attempts: 0 };
        let r = record_end(
            &mock,
            &bot,
            &ev.chat_id,
            &ev.message_id,
            &outcome,
            "result text",
        )
        .await
        .unwrap();
        assert!(r.is_some(), "record_end should return TaskRef on cache hit");

        // 验证 cache 端 outcome 被落
        let path = cache_path_for(&bot.app_id, &ev.chat_id);
        let entry = load_cache(&path).unwrap().unwrap();
        assert!(entry.end_outcome.is_some());

        // 验证最后一 call 的 step 文案
        let calls = mock.calls();
        let last = &calls[calls.len() - 1];
        let data_idx = last.iter().position(|s| s == "--data").unwrap();
        let payload = &last[data_idx + 1];
        assert!(payload.contains("✅"), "payload: {payload}");
        assert!(payload.contains("result text"), "payload: {payload}");

        restore_home();
    }

    #[tokio::test]
    async fn record_end_without_cache_returns_none() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);

        let mock = MockLarkRunner::new();
        let bot = mk_bot("cli_bot_no_cache");
        let outcome = EndOutcome::Timeout;
        let r = record_end(&mock, &bot, "oc_orphan", "om_orphan", &outcome, "x")
            .await
            .unwrap();
        assert!(r.is_none(), "no cache → None (E2 退化路径)");
        assert_eq!(mock.calls().len(), 0, "no lark call when cache miss");

        restore_home();
    }

    // --- T7: idempotency_key 模板 ---------------------------------------

    #[test]
    fn idempotency_key_template() {
        assert_eq!(
            idem_key("create", "om_1", "cli_bot"),
            "relay:create:om_1:cli_bot"
        );
        assert_eq!(
            idem_key("step-end", "om_2", "cli_bot"),
            "relay:step-end:om_2:cli_bot"
        );
    }

    // --- T8: 原子写不留 tmp 残留 ----------------------------------------

    #[test]
    fn save_cache_atomic_no_tmp_residue() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);

        let path = cache_path_for("cli_bot_atom", "oc_atom");
        let entry = BotChatCacheEntry {
            schema_version: BOT_CHAT_CACHE_SCHEMA_VERSION,
            task_guid: "g".into(),
            task_url: "u".into(),
            chat_id: "oc_atom".into(),
            bot_app_id: "cli_bot_atom".into(),
            created_at: "2026-05-19T00:00:00Z".into(),
            adjust_count: 0,
            end_outcome: None,
        };
        save_cache(&path, &entry).unwrap();
        let parent = path.parent().unwrap();
        let files: Vec<String> = std::fs::read_dir(parent)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(
            files
                .iter()
                .any(|f| f.ends_with(".json") && !f.contains(".tmp"))
        );
        assert!(!files.iter().any(|f| f.contains(".tmp")));

        restore_home();
    }
}
