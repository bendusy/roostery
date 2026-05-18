//! Bot identity task writer — Module F 第 1 子 feature。
//!
//! 三 pub fn 纯库 API：
//! - [`create_task`]：bot 身份创建飞书 task；assignee None 走 `identity::current` 解析当前 user
//! - [`append_steps`]：bot 身份追加 task step stream；空 steps 短路；**始终带 `--yes`**
//! - [`get_or_create_for_session`]：`(agent, session)` 维度幂等，session_cache 持久化在
//!   `~/.roostery/state/session_tasks/{safe}.json`
//!
//! **架构红线显式破例**：`append_steps` 始终带 `--yes`。lark-shared SKILL 红线
//! "未经用户同意不加 `--yes`" 的允许例外——bot 写自己创建的 task 等价 agent
//! 内部行为（append-only step stream，对用户资源无破坏性影响）。详见 design
//! §1.2 D4。
//!
//! See `.codestable/features/2026-05-18-bot-task-writer/bot-task-writer-design.md`
//! §2.1.1。

use crate::lark_cli::{LarkError, LarkRunner};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

pub const SESSION_CACHE_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_HOST_FALLBACK: &str = "unknown";

/// 飞书 Task 引用——`guid` 用 newtype 隔离防与 url / event_id 等其他 id-like 串
/// 混；`url` 是浏览器可点的飞书任务页 URL，业务上扔 IM 消息 / docs 链接用。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TaskRef {
    pub guid: TaskGuid,
    pub url: String,
}

/// 飞书 task guid newtype。与 `business-identifier-newtype` decision 一致：
/// 从飞书侧拿到的、有明确业务语义的标识符隔离类型，避免与 `task_url` /
/// `event_id` / `trace_id` 等 id-like 字符串互换。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct TaskGuid(String);

impl TaskGuid {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_existing(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl std::fmt::Display for TaskGuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TaskWriterError {
    #[error("lark-cli call failed: {source}")]
    LarkCallFailed {
        #[source]
        source: LarkError,
    },
    #[error("lark-cli response shape unexpected (expected {expected}); raw_head={raw_head:?}")]
    ResponseShapeUnexpected {
        expected: &'static str,
        raw_head: String,
    },
    #[error("session cache load failed at {path}: {source}")]
    CacheLoadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("session cache save failed at {path}: {source}")]
    CacheSaveFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("identity resolve failed: {0}")]
    IdentityResolveFailed(#[source] crate::identity::IdentityError),
}

/// 可选参数集合。`#[non_exhaustive]` 锁定外部不能用 struct literal（包括
/// `..Default::default()` 也不行——见 rustc E0639），必须走 builder。这样
/// 未来加字段时 caller 链不受影响（attention.md 已记 RunOptions 同模式）。
#[derive(Default)]
#[non_exhaustive]
pub struct CreateTaskOptions<'a> {
    pub description: Option<&'a str>,
    /// `None` 走 `identity::current` 解析当前 user open_id 作 assignee
    pub assignee_open_id: Option<&'a str>,
    pub idempotency_key: Option<&'a str>,
    /// `None` 走 host_default 三 fallback 链
    pub host: Option<&'a str>,
    pub profile: Option<&'a str>,
}

impl<'a> CreateTaskOptions<'a> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_description(mut self, v: &'a str) -> Self {
        self.description = Some(v);
        self
    }
    pub fn with_assignee_open_id(mut self, v: &'a str) -> Self {
        self.assignee_open_id = Some(v);
        self
    }
    pub fn with_idempotency_key(mut self, v: &'a str) -> Self {
        self.idempotency_key = Some(v);
        self
    }
    pub fn with_host(mut self, v: &'a str) -> Self {
        self.host = Some(v);
        self
    }
    pub fn with_profile(mut self, v: &'a str) -> Self {
        self.profile = Some(v);
        self
    }
}

#[derive(Default)]
#[non_exhaustive]
pub struct AppendStepsOptions<'a> {
    pub idempotency_key: Option<&'a str>,
    pub profile: Option<&'a str>,
}

impl<'a> AppendStepsOptions<'a> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_idempotency_key(mut self, v: &'a str) -> Self {
        self.idempotency_key = Some(v);
        self
    }
    pub fn with_profile(mut self, v: &'a str) -> Self {
        self.profile = Some(v);
        self
    }
}

/// 决定 host 后缀来源：`ROOSTERY_HOST` env > hostname 首段（去 `.local` 等
/// trailing domain） > [`DEFAULT_HOST_FALLBACK`]。
fn host_default() -> String {
    if let Ok(explicit) = std::env::var("ROOSTERY_HOST")
        && !explicit.is_empty()
    {
        return explicit;
    }
    if let Ok(hn) = hostname_first_segment()
        && !hn.is_empty()
    {
        return hn;
    }
    DEFAULT_HOST_FALLBACK.to_string()
}

fn hostname_first_segment() -> std::io::Result<String> {
    // std 没暴露 hostname，走 libc gethostname 等价的 `uname -n` 输出
    // 不引外部 crate（attention.md 提示无新 dep）：用 `HOSTNAME` env 兜底（多数
    // shell 启动时已 export），失败再走空字符串让 caller 走 DEFAULT_HOST_FALLBACK
    // fallback。
    if let Ok(hn) = std::env::var("HOSTNAME")
        && !hn.is_empty()
    {
        return Ok(first_segment(&hn));
    }
    // 终极 fallback：std::env::var 找不到 → 返回空让上层兜底
    Ok(String::new())
}

fn first_segment(host: &str) -> String {
    match host.split_once('.') {
        Some((head, _)) => head.to_string(),
        None => host.to_string(),
    }
}

// --- session cache layer -------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct SessionCacheEntry {
    #[serde(default)]
    schema_version: u32,
    task_guid: String,
    task_url: String,
    created_at: String,
    summary: String,
}

/// `~/.roostery/state/session_tasks/` 目录路径（缺则自建）。
fn session_cache_dir() -> PathBuf {
    crate::paths::state_dir().join("session_tasks")
}

/// `None` = 文件不存在或解析失败；`Some(TaskRef)` = 缓存命中。
/// 解析失败也返 None 不返 Err——cache 损坏 caller 走 create 路径自然修复，
/// 同 design §3.7 C7.3。schema_version 缺失走 default(0) 也 OK——读旧版兼容。
fn load_cache(path: &std::path::Path) -> Result<Option<TaskRef>, TaskWriterError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(TaskWriterError::CacheLoadFailed {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let entry: SessionCacheEntry = match serde_json::from_slice(&bytes) {
        Ok(e) => e,
        Err(_) => return Ok(None), // 损坏→走 create
    };
    Ok(Some(TaskRef {
        guid: TaskGuid::from_existing(entry.task_guid),
        url: entry.task_url,
    }))
}

fn save_cache(
    path: &std::path::Path,
    task: &TaskRef,
    summary: &str,
) -> Result<(), TaskWriterError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| TaskWriterError::CacheSaveFailed {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let entry = SessionCacheEntry {
        schema_version: SESSION_CACHE_SCHEMA_VERSION,
        task_guid: task.guid.as_str().to_string(),
        task_url: task.url.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        summary: summary.to_string(),
    };
    let body = serde_json::to_vec_pretty(&entry).expect("SessionCacheEntry serializes");
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &body).map_err(|source| TaskWriterError::CacheSaveFailed {
        path: path.to_path_buf(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(|source| TaskWriterError::CacheSaveFailed {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// 把 (agent, session) 安全拼成单一文件名：非白名单 `[A-Za-z0-9._-]` 字符
/// 替换 `_`；连续 `..` 替换 `__` 防路径跳出；末尾加 `.json`。
fn safe_filename(agent: &str, session: &str) -> String {
    let raw = format!("{agent}-{session}");
    let mut cleaned: String = raw
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
    format!("{cleaned}.json")
}

/// 把 host suffix 应用到 summary——幂等：若 summary 已含 `· {host}` 不重复加。
fn apply_host_suffix(summary: &str, host: &str) -> String {
    let marker = format!("· {host}");
    if summary.contains(&marker) {
        return summary.to_string();
    }
    format!("{summary} {marker}")
}

/// bot 身份建任务。
///
/// `opts.assignee_open_id = None` 时走 [`identity::current`] 解析当前 user
/// open_id 作 assignee——让 task 进入用户"我的待办"视图。identity 失败返
/// `Err(IdentityResolveFailed)`，**不** silently 不带 assignee（没 assignee 的
/// task 不进 inbox，与 req 核心 UX 冲突；见 design §2.2.3 不变量 10）。
pub async fn create_task(
    runner: &dyn LarkRunner,
    _agent: &str,
    _cwd: &str,
    summary: &str,
    opts: CreateTaskOptions<'_>,
) -> Result<TaskRef, TaskWriterError> {
    // assignee 解析：opts 优先，None 走 identity::current
    let resolved_assignee: Option<String> = match opts.assignee_open_id {
        Some(s) => Some(s.to_string()),
        None => {
            let ident = crate::identity::current(runner)
                .await
                .map_err(TaskWriterError::IdentityResolveFailed)?;
            ident.user_open_id().map(|s| s.to_string())
        }
    };

    let host = match opts.host {
        Some(h) => h.to_string(),
        None => host_default(),
    };
    let final_summary = apply_host_suffix(summary, &host);

    let mut argv: Vec<String> = vec![
        "task".into(),
        "+create".into(),
        "--as".into(),
        "bot".into(),
        "--summary".into(),
        final_summary,
    ];
    if let Some(desc) = opts.description {
        argv.push("--description".into());
        argv.push(desc.to_string());
    }
    if let Some(assignee) = resolved_assignee.as_deref()
        && !assignee.is_empty()
    {
        argv.push("--assignee".into());
        argv.push(assignee.to_string());
    }
    if let Some(key) = opts.idempotency_key {
        argv.push("--idempotency-key".into());
        argv.push(key.to_string());
    }

    let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
    let opts_run = match opts.profile {
        Some(p) => crate::lark_cli::RunOptions::new().with_profile(p.to_string()),
        None => crate::lark_cli::RunOptions::new(),
    };
    let response = runner
        .run_with_options(&argv_refs, opts_run)
        .await
        .map_err(|source| TaskWriterError::LarkCallFailed { source })?;

    parse_task_response(&response)
}

/// 从 `lark-cli task +create` stdout JSON 解 `data.guid` / `data.url` 拼 `TaskRef`。
/// lark-cli shortcut 返 `{ok, data: {guid, url}}` 形态；也兼容裸 `{guid, url}`。
fn parse_task_response(response: &serde_json::Value) -> Result<TaskRef, TaskWriterError> {
    let raw_head = truncate_for_error(&response.to_string());
    let data = response.get("data").unwrap_or(response);
    let guid = data.get("guid").and_then(|v| v.as_str()).ok_or(
        TaskWriterError::ResponseShapeUnexpected {
            expected: "data.guid",
            raw_head: raw_head.clone(),
        },
    )?;
    let url = data.get("url").and_then(|v| v.as_str()).ok_or(
        TaskWriterError::ResponseShapeUnexpected {
            expected: "data.url",
            raw_head,
        },
    )?;
    Ok(TaskRef {
        guid: TaskGuid::from_existing(guid.to_string()),
        url: url.to_string(),
    })
}

fn truncate_for_error(s: &str) -> String {
    const HEAD_CAP: usize = 256;
    if s.len() <= HEAD_CAP {
        return s.to_string();
    }
    let mut end = HEAD_CAP;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

/// bot 身份追加步骤流。
///
/// 空 `steps` 立即 `Ok(())` 不调 lark-cli。`--yes` 已内置（见模块顶部
/// "架构红线显式破例"）。
pub async fn append_steps(
    runner: &dyn LarkRunner,
    task_guid: &TaskGuid,
    steps: &[&str],
    opts: AppendStepsOptions<'_>,
) -> Result<(), TaskWriterError> {
    if steps.is_empty() {
        return Ok(());
    }

    let task_steps_json: Vec<serde_json::Value> = steps
        .iter()
        .map(|s| serde_json::json!({ "content": s }))
        .collect();
    let mut body = serde_json::json!({
        "task_guid": task_guid.as_str(),
        "task_steps": task_steps_json,
    });
    if let Some(key) = opts.idempotency_key
        && let serde_json::Value::Object(ref mut map) = body
    {
        map.insert(
            "idempotent_key".to_string(),
            serde_json::Value::String(key.to_string()),
        );
    }
    let data_str = serde_json::to_string(&body).expect("body serializes");

    let argv = vec![
        "task",
        "agent_task_step_info",
        "append_task_steps",
        "--as",
        "bot",
        "--data",
        &data_str,
        // 架构红线显式破例：bot 写自己创建的 task 等价 agent 内部行为
        // （append-only step stream，对用户资源无破坏性影响）。详见模块顶部 doc
        // 和 design §1.2 D4。
        "--yes",
    ];
    let opts_run = match opts.profile {
        Some(p) => crate::lark_cli::RunOptions::new().with_profile(p.to_string()),
        None => crate::lark_cli::RunOptions::new(),
    };
    runner
        .run_with_options(&argv, opts_run)
        .await
        .map_err(|source| TaskWriterError::LarkCallFailed { source })?;
    Ok(())
}

/// `(agent, session)` 维度幂等：首次 call → create_task + 写
/// `~/.roostery/state/session_tasks/{safe}.json`；后续 call → 读 cache 返
/// 已有 `TaskRef`。
pub async fn get_or_create_for_session(
    runner: &dyn LarkRunner,
    agent: &str,
    session: &str,
    cwd: &str,
    summary: &str,
    opts: CreateTaskOptions<'_>,
) -> Result<TaskRef, TaskWriterError> {
    let cache_path = session_cache_dir().join(safe_filename(agent, session));
    if let Some(cached) = load_cache(&cache_path)? {
        return Ok(cached);
    }

    // 默认 idempotency_key = `{agent}-session-{session}`（Python parity），让
    // lark-cli 跨次调用同 session 不重复创建。
    let default_key = format!("{agent}-session-{session}");
    let opts_with_key = CreateTaskOptions {
        idempotency_key: opts.idempotency_key.or(Some(&default_key)),
        ..opts
    };

    let task = create_task(runner, agent, cwd, summary, opts_with_key).await?;
    save_cache(&cache_path, &task, summary)?;
    Ok(task)
}

#[cfg(test)]
#[allow(clippy::await_holding_lock)] // test ENV_LOCK serializes ROOSTERY_HOME/HOST mutation (attention.md pattern)
mod tests {
    use super::*;

    // --- S1 type tests ----------------------------------------------------

    #[test]
    fn constants_exposed() {
        assert_eq!(SESSION_CACHE_SCHEMA_VERSION, 1);
        assert_eq!(DEFAULT_HOST_FALLBACK, "unknown");
    }

    #[test]
    fn task_guid_serde_transparent() {
        let g = TaskGuid::from_existing("abc123");
        let s = serde_json::to_string(&g).unwrap();
        assert_eq!(s, "\"abc123\"");
        let back: TaskGuid = serde_json::from_str("\"abc123\"").unwrap();
        assert_eq!(back, g);
        assert_eq!(g.as_str(), "abc123");
        assert_eq!(g.to_string(), "abc123");
    }

    // --- S2 host suffix tests --------------------------------------------

    use crate::paths::TEST_ENV_LOCK as ENV_LOCK;

    #[test]
    fn host_default_uses_roostery_host_env() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("ROOSTERY_HOST", "my-laptop") };
        assert_eq!(host_default(), "my-laptop");
        unsafe { std::env::remove_var("ROOSTERY_HOST") };
    }

    #[test]
    fn host_default_falls_back_to_hostname() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var("ROOSTERY_HOST") };
        unsafe { std::env::set_var("HOSTNAME", "m4.local") };
        assert_eq!(host_default(), "m4");
        unsafe { std::env::remove_var("HOSTNAME") };
    }

    #[test]
    fn host_default_unknown_when_all_missing() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var("ROOSTERY_HOST") };
        unsafe { std::env::remove_var("HOSTNAME") };
        assert_eq!(host_default(), DEFAULT_HOST_FALLBACK);
    }

    #[test]
    fn apply_host_suffix_appends_marker() {
        let r = apply_host_suffix("Refactor module", "m4");
        assert_eq!(r, "Refactor module · m4");
    }

    #[test]
    fn apply_host_suffix_is_idempotent() {
        let already = "Refactor module · m4";
        let r = apply_host_suffix(already, "m4");
        assert_eq!(r, already);
    }

    // --- S5 create_task tests --------------------------------------------

    use crate::lark_cli::{LarkError, mock::MockLarkRunner};
    use serde_json::json;

    fn auth_status_response() -> serde_json::Value {
        json!({
            "userOpenId": "ou_user_123",
            "userName": "TestUser",
            "appId": "cli_bot_456",
            "tokenStatus": "valid",
        })
    }

    fn profile_list_response() -> serde_json::Value {
        json!([{"name": "default", "active": true}])
    }

    fn task_create_response() -> serde_json::Value {
        json!({
            "ok": true,
            "data": {
                "guid": "task_guid_abc",
                "url": "https://feishu.cn/task/abc"
            }
        })
    }

    #[tokio::test]
    async fn create_task_happy_with_explicit_assignee() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("ROOSTERY_HOST", "m4") };
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(task_create_response());
        let r = create_task(
            &mock,
            "cc",
            "/tmp/wd",
            "Test task",
            CreateTaskOptions {
                assignee_open_id: Some("ou_explicit"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(r.guid.as_str(), "task_guid_abc");
        assert_eq!(r.url, "https://feishu.cn/task/abc");
        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains(&"--summary".to_string()));
        assert!(calls[0].contains(&"Test task · m4".to_string()));
        assert!(calls[0].contains(&"--assignee".to_string()));
        assert!(calls[0].contains(&"ou_explicit".to_string()));
        unsafe { std::env::remove_var("ROOSTERY_HOST") };
    }

    #[tokio::test]
    async fn create_task_falls_back_to_identity_for_assignee() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("ROOSTERY_HOST", "m4") };
        let mock = MockLarkRunner::new();
        // assignee None → identity::current 触发 auth status + profile list 两调
        mock.enqueue_ok(auth_status_response());
        mock.enqueue_ok(profile_list_response());
        mock.enqueue_ok(task_create_response());
        let r = create_task(
            &mock,
            "cc",
            "/tmp",
            "Auto assignee",
            CreateTaskOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(r.guid.as_str(), "task_guid_abc");
        let calls = mock.calls();
        assert_eq!(calls.len(), 3);
        // 第 3 call 是 task +create，应含 identity 解出的 user_open_id
        assert!(calls[2].contains(&"ou_user_123".to_string()));
        unsafe { std::env::remove_var("ROOSTERY_HOST") };
    }

    #[tokio::test]
    async fn create_task_propagates_lark_error() {
        let _g = ENV_LOCK.lock().unwrap();
        let mock = MockLarkRunner::new();
        mock.enqueue_err(LarkError::Timeout { timeout_ms: 5000 });
        let result = create_task(
            &mock,
            "cc",
            "/tmp",
            "x",
            CreateTaskOptions {
                assignee_open_id: Some("ou_x"),
                ..Default::default()
            },
        )
        .await;
        match result {
            Err(TaskWriterError::LarkCallFailed { .. }) => {}
            other => panic!("expected LarkCallFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_task_missing_guid_returns_shape_error() {
        let _g = ENV_LOCK.lock().unwrap();
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!({"ok": true, "data": {"url": "u"}})); // 缺 guid
        let result = create_task(
            &mock,
            "cc",
            "/tmp",
            "x",
            CreateTaskOptions {
                assignee_open_id: Some("ou_x"),
                ..Default::default()
            },
        )
        .await;
        match result {
            Err(TaskWriterError::ResponseShapeUnexpected { expected, .. }) => {
                assert_eq!(expected, "data.guid");
            }
            other => panic!("expected ResponseShapeUnexpected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_task_identity_error_surfaces() {
        let _g = ENV_LOCK.lock().unwrap();
        let mock = MockLarkRunner::new();
        // auth status fail → identity::current 返 Err
        mock.enqueue_err(LarkError::Timeout { timeout_ms: 1000 });
        let result = create_task(&mock, "cc", "/tmp", "x", CreateTaskOptions::default()).await;
        match result {
            Err(TaskWriterError::IdentityResolveFailed(_)) => {}
            other => panic!("expected IdentityResolveFailed, got {other:?}"),
        }
    }

    // --- S7 get_or_create_for_session tests -----------------------------

    fn isolate_home(tmp: &tempfile::TempDir) {
        unsafe { std::env::set_var("ROOSTERY_HOME", tmp.path()) };
    }
    fn restore_home() {
        unsafe { std::env::remove_var("ROOSTERY_HOME") };
    }

    #[tokio::test]
    async fn get_or_create_first_call_creates_and_caches() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        isolate_home(&tmp);
        unsafe { std::env::set_var("ROOSTERY_HOST", "m4") };
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(task_create_response());
        let r = get_or_create_for_session(
            &mock,
            "cc",
            "sess_42",
            "/tmp/wd",
            "Compile",
            CreateTaskOptions {
                assignee_open_id: Some("ou_x"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(r.guid.as_str(), "task_guid_abc");
        // 验证 cache 文件已写
        let cache_path = session_cache_dir().join("cc-sess_42.json");
        assert!(cache_path.exists());
        unsafe { std::env::remove_var("ROOSTERY_HOST") };
        restore_home();
    }

    #[tokio::test]
    async fn get_or_create_second_call_hits_cache() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        isolate_home(&tmp);
        unsafe { std::env::set_var("ROOSTERY_HOST", "m4") };
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(task_create_response()); // 只 enqueue 一次——第二次应走 cache 不调
        let opts1 = CreateTaskOptions {
            assignee_open_id: Some("ou_x"),
            ..Default::default()
        };
        let r1 = get_or_create_for_session(&mock, "cc", "sess_xy", "/tmp", "S", opts1)
            .await
            .unwrap();
        let opts2 = CreateTaskOptions {
            assignee_open_id: Some("ou_x"),
            ..Default::default()
        };
        let r2 = get_or_create_for_session(&mock, "cc", "sess_xy", "/tmp", "S", opts2)
            .await
            .unwrap();
        assert_eq!(r1, r2);
        // MockLarkRunner 应仅被调一次（第二次走 cache）
        assert_eq!(mock.calls().len(), 1);
        unsafe { std::env::remove_var("ROOSTERY_HOST") };
        restore_home();
    }

    #[tokio::test]
    async fn get_or_create_corrupt_cache_falls_through_to_create() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        isolate_home(&tmp);
        unsafe { std::env::set_var("ROOSTERY_HOST", "m4") };
        // 手工写损坏 cache
        let dir = session_cache_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let cache_path = dir.join("cc-sess_bad.json");
        std::fs::write(&cache_path, "{{ not valid json").unwrap();

        let mock = MockLarkRunner::new();
        mock.enqueue_ok(task_create_response());
        let r = get_or_create_for_session(
            &mock,
            "cc",
            "sess_bad",
            "/tmp",
            "x",
            CreateTaskOptions {
                assignee_open_id: Some("ou_x"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(r.guid.as_str(), "task_guid_abc");
        unsafe { std::env::remove_var("ROOSTERY_HOST") };
        restore_home();
    }

    // --- S6 append_steps tests -------------------------------------------

    #[tokio::test]
    async fn append_steps_empty_does_not_call_lark() {
        let mock = MockLarkRunner::new();
        let guid = TaskGuid::from_existing("g1");
        let r = append_steps(&mock, &guid, &[], AppendStepsOptions::default()).await;
        assert!(r.is_ok());
        assert_eq!(mock.calls().len(), 0);
    }

    #[tokio::test]
    async fn append_steps_includes_yes_flag_and_data() {
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!({"ok": true}));
        let guid = TaskGuid::from_existing("g_xy");
        let steps = ["step one", "step two", "step three"];
        append_steps(&mock, &guid, &steps, AppendStepsOptions::default())
            .await
            .unwrap();
        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains(&"--yes".to_string()));
        assert!(calls[0].contains(&"--data".to_string()));
        // 找 --data 后面的 JSON 串，validate task_guid + 3 steps
        let data_idx = calls[0].iter().position(|s| s == "--data").unwrap();
        let payload = &calls[0][data_idx + 1];
        assert!(payload.contains("g_xy"));
        assert!(payload.contains("step one"));
        assert!(payload.contains("step two"));
        assert!(payload.contains("step three"));
    }

    #[tokio::test]
    async fn append_steps_propagates_lark_error() {
        let mock = MockLarkRunner::new();
        mock.enqueue_err(LarkError::Timeout { timeout_ms: 1000 });
        let guid = TaskGuid::from_existing("g1");
        let result = append_steps(&mock, &guid, &["s"], AppendStepsOptions::default()).await;
        match result {
            Err(TaskWriterError::LarkCallFailed { .. }) => {}
            other => panic!("expected LarkCallFailed, got {other:?}"),
        }
    }

    // --- S4 session cache tests ------------------------------------------

    use tempfile::tempdir;

    fn ref_fixture() -> TaskRef {
        TaskRef {
            guid: TaskGuid::from_existing("guid-xyz"),
            url: "https://feishu.cn/task/xyz".to_string(),
        }
    }

    #[test]
    fn load_cache_missing_file_returns_none() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("nonexistent.json");
        assert!(load_cache(&path).unwrap().is_none());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("entry.json");
        let r = ref_fixture();
        save_cache(&path, &r, "test summary").unwrap();
        let loaded = load_cache(&path).unwrap().unwrap();
        assert_eq!(loaded, r);
    }

    #[test]
    fn save_leaves_no_tmp_artifact() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("entry.json");
        save_cache(&path, &ref_fixture(), "x").unwrap();
        let files: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(files.iter().any(|f| f == "entry.json"));
        assert!(!files.iter().any(|f| f.ends_with(".tmp")));
    }

    #[test]
    fn load_cache_missing_schema_version_compat() {
        // 旧版 cache 没 schema_version 字段——load 应仍 Ok（serde default = 0）
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("old.json");
        let old_body = r#"{
            "task_guid": "g1",
            "task_url": "u1",
            "created_at": "2026-05-18T00:00:00Z",
            "summary": "s"
        }"#;
        std::fs::write(&path, old_body).unwrap();
        let r = load_cache(&path).unwrap().unwrap();
        assert_eq!(r.guid.as_str(), "g1");
        assert_eq!(r.url, "u1");
    }

    #[test]
    fn load_cache_corrupt_returns_none() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("corrupt.json");
        std::fs::write(&path, "not valid json {{{").unwrap();
        // 损坏 → None（让 caller 走 create 自然修复）
        assert!(load_cache(&path).unwrap().is_none());
    }

    // --- S3 safe_filename tests ------------------------------------------

    #[test]
    fn safe_filename_normal_input() {
        assert_eq!(safe_filename("cc", "abc123"), "cc-abc123.json");
        assert_eq!(safe_filename("codex", "sess_42"), "codex-sess_42.json");
    }

    #[test]
    fn safe_filename_replaces_specials() {
        assert_eq!(
            safe_filename("cc/agent", "sess id"),
            "cc_agent-sess_id.json"
        );
        assert_eq!(safe_filename("a@b", "x\\y"), "a_b-x_y.json");
    }

    #[test]
    fn safe_filename_neutralizes_path_traversal() {
        // 用户字段塞 ".." 试图跳出目录
        let result = safe_filename("..", "..");
        assert!(!result.contains(".."));
        assert!(result.ends_with(".json"));
        // 多层 ..
        let result2 = safe_filename("...", "..sess");
        assert!(!result2.contains(".."));
    }

    #[test]
    fn task_writer_error_display_includes_context() {
        let err = TaskWriterError::ResponseShapeUnexpected {
            expected: "data.guid",
            raw_head: "<empty>".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("data.guid"));
        assert!(msg.contains("<empty>"));
    }
}
