//! 计算层纯函数 + receive_id 三层链解析。
//!
//! 拆自原 `bot_stop_hook.rs` line 150-336（refactor `2026-05-19-bot-stop-hook-split`）。

use crate::lark_cli::LarkRunner;
use std::path::Path;

/// UTF-8 安全截断到 `max_bytes` 字节内。不切坏多字节字符（floor 到最近 char
/// boundary）。Python `head -c 200` 在中文 / emoji 上会切坏 UTF-8——Rust 红利之一
/// 是这种安全可以编译期约束在类型层。
pub(crate) fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// 从 cwd 路径里抽最后一段（basename）。空路径或全 `/` → `"."`。
pub(crate) fn cwd_basename(cwd: &Path) -> String {
    cwd.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ".".to_string())
}

/// 跨进程**稳态**的 idempotency key 短哈希。
///
/// 用 blake3 而非 [`std::hash::DefaultHasher`]——后者 SipHash 启动种子随机化，
/// 同输入两次进程拿到不同 key，在 lark-cli `--idempotency-key` 链路里幂等失效。
///
/// 长度：8 字符（hex 4 字节，冲突空间 ~4G）足够 session-级幂等。
pub(crate) fn stable_idem_key(parts: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    for p in parts {
        hasher.update(p.as_bytes());
        hasher.update(&[0]); // null 分隔防 ("ab","c") 与 ("a","bc") 碰撞
    }
    let hex = hasher.finalize().to_hex();
    hex.as_str()[..8].to_string()
}

/// IM 兜底 / task assignee 共用的"通知谁"解析。三层 fallback 链：
///
/// 1. `explicit` (caller 显式 override) → 直接用
/// 2. env `ROOSTERY_NOTIFY_TO` → 直接用（不调 identity）
/// 3. `identity::current(runner).user_open_id` → 调 lark-cli profile
/// 4. `config::load().identity.user_id` (非空) → 装机持久态
/// 5. 全空 → `None`（caller 见 None 走 Skipped）
///
/// 任一层失败/缺失**不当 fatal**，自动走下一层；identity 调失败也只是
/// `tracing::warn!` 记一笔继续向 config 兜底。
pub(crate) async fn resolve_receive_id(
    runner: &dyn LarkRunner,
    explicit: Option<&str>,
) -> Option<String> {
    // 1. explicit override
    if let Some(s) = explicit
        && !s.is_empty()
    {
        return Some(s.to_string());
    }
    // 2. env
    if let Ok(s) = std::env::var("ROOSTERY_NOTIFY_TO")
        && !s.is_empty()
    {
        return Some(s);
    }
    // 3. identity (lark-cli profile)
    match crate::identity::current(runner).await {
        Ok(ident) => {
            if let Some(oid) = ident.user_open_id()
                && !oid.is_empty()
            {
                return Some(oid.to_string());
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "identity::current failed; falling back to config");
        }
    }
    // 4. config persisted
    match crate::config::load() {
        Ok(cfg) => {
            if !cfg.identity.user_id.is_empty() {
                return Some(cfg.identity.user_id);
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "config::load failed; treating as no recipient");
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::await_holding_lock)] // ENV_LOCK serializes env mutation (attention.md pattern)
mod tests {
    use super::*;
    use crate::bot_stop_hook::test_helpers::install_tempdir_as_home;
    use crate::bot_stop_hook::test_helpers::write_config_with_user_id;
    use crate::lark_cli::mock::MockLarkRunner;
    use crate::paths::TEST_ENV_LOCK as ENV_LOCK;
    use serde_json::json;

    #[test]
    fn truncate_utf8_ascii_under_cap_unchanged() {
        assert_eq!(truncate_utf8("hello", 200), "hello");
    }

    #[test]
    fn truncate_utf8_emoji_boundary_safe() {
        // "ab😀😀cd" 字节序列：a b (1+1) 😀(4) 😀(4) c d → 共 12 字节
        // 切到 max=7 应落在第一个😀末尾后（a b 😀 = 6 字节），不会切坏第 2 个😀
        let s = "ab😀😀cd";
        let out = truncate_utf8(s, 7);
        assert!(out.is_char_boundary(out.len()));
        assert!(out.starts_with("ab"));
        assert!(out.len() <= 7);
        assert_eq!(out, "ab😀");
    }

    #[test]
    fn cwd_basename_extracts_last_segment() {
        assert_eq!(
            cwd_basename(Path::new("/Users/ben/Projects/roostery")),
            "roostery"
        );
        assert_eq!(
            cwd_basename(Path::new("/Users/ben/Projects/roostery/")),
            "roostery"
        );
        assert_eq!(cwd_basename(Path::new("relative/dir")), "dir");
        assert_eq!(cwd_basename(Path::new("")), ".");
        assert_eq!(cwd_basename(Path::new("/")), ".");
    }

    #[test]
    fn stable_idem_key_deterministic_across_calls() {
        // 关键性质：同输入两次进程拿到同 key（修 std::hash 启动种子随机化的 bug）
        let k1 = stable_idem_key(&["cc", "session-1", "summary X"]);
        let k2 = stable_idem_key(&["cc", "session-1", "summary X"]);
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 8);
        let k3 = stable_idem_key(&["cc", "session-2", "summary X"]);
        assert_ne!(k1, k3);
        // null 分隔防互换：("ab","c") != ("a","bc")
        let k_ab_c = stable_idem_key(&["ab", "c"]);
        let k_a_bc = stable_idem_key(&["a", "bc"]);
        assert_ne!(k_ab_c, k_a_bc);
    }

    #[tokio::test]
    async fn resolve_receive_id_explicit_short_circuits() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        let mock = MockLarkRunner::new();
        let out = resolve_receive_id(&mock, Some("ou_explicit")).await;
        assert_eq!(out.as_deref(), Some("ou_explicit"));
        assert!(mock.calls().is_empty(), "explicit short-circuits lark-cli");
    }

    #[tokio::test]
    async fn resolve_receive_id_env_overrides_identity() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::set_var("ROOSTERY_NOTIFY_TO", "ou_from_env") };
        let mock = MockLarkRunner::new();
        let out = resolve_receive_id(&mock, None).await;
        assert_eq!(out.as_deref(), Some("ou_from_env"));
        assert!(mock.calls().is_empty(), "env hit short-circuits identity");
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
    }

    #[tokio::test]
    async fn resolve_receive_id_falls_back_to_identity() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!({"userOpenId": "ou_from_identity"}))
            .enqueue_ok(json!([{"name": "default", "active": true}]));
        let out = resolve_receive_id(&mock, None).await;
        assert_eq!(out.as_deref(), Some("ou_from_identity"));
        assert_eq!(mock.calls().len(), 2);
    }

    #[tokio::test]
    async fn resolve_receive_id_falls_back_to_config_when_identity_blank() {
        let _g = ENV_LOCK.lock().unwrap();
        let home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        write_config_with_user_id(home.path(), "ou_from_config");
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!({"userName": "Test"}))
            .enqueue_ok(json!([{"name": "default", "active": true}]));
        let out = resolve_receive_id(&mock, None).await;
        assert_eq!(out.as_deref(), Some("ou_from_config"));
    }

    #[tokio::test]
    async fn resolve_receive_id_all_three_empty_returns_none() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!({}))
            .enqueue_ok(json!([{"name": "default", "active": true}]));
        let out = resolve_receive_id(&mock, None).await;
        assert!(out.is_none());
    }
}
