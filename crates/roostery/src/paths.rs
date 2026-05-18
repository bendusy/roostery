//! Filesystem path resolution for Roostery state (`~/.roostery/`).
//!
//! Replaces the legacy Python `FEISHU_HUB_HOME` / `~/.feishu_hub/` convention
//! with `ROOSTERY_HOME` / `~/.roostery/` (vendor-neutral). The legacy env var
//! is intentionally not consulted — Rust port treats this as a clean break.

use std::path::PathBuf;

const ENV_HOME: &str = "ROOSTERY_HOME";
const DIR_NAME: &str = ".roostery";

pub fn roostery_home() -> PathBuf {
    if let Some(raw) = std::env::var_os(ENV_HOME)
        && !raw.is_empty()
    {
        return PathBuf::from(raw);
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(DIR_NAME);
    }
    PathBuf::from(DIR_NAME)
}

pub fn journal_dir() -> PathBuf {
    roostery_home().join("journal")
}

pub fn state_dir() -> PathBuf {
    roostery_home().join("state")
}

pub fn smoke_state_path() -> PathBuf {
    state_dir().join("smoke.json")
}

pub fn config_path() -> PathBuf {
    roostery_home().join("config.yaml")
}

pub fn scripts_dir() -> PathBuf {
    roostery_home().join("scripts")
}

pub fn env_file() -> PathBuf {
    roostery_home().join("env")
}

pub fn budget_state_path() -> PathBuf {
    state_dir().join("budget.json")
}

pub fn rules_path() -> PathBuf {
    roostery_home().join("rules.yaml")
}

/// 跨模块共享的测试 env 串行化锁。**所有**改 `ROOSTERY_*` / `HOSTNAME` /
/// `FEISHU_*` 等进程级 env 的 `#[test]` / `#[tokio::test]` 都必须先 `lock()`
/// 这个 Mutex。
///
/// 历史教训（bot-stop-hook feature S10.5 修复）：之前每个模块在自己 `mod tests`
/// 里各自声明 `static ENV_LOCK: Mutex<()>`，多模块同时跑触碰同 env var 时
/// race（一个 mod 的 lock 不能阻挡另一个 mod 的 set_var）。任意 test 因 race
/// 失败 panic 会 poison 该 mod 的 lock，连锁让同 mod 后续 env 测试全挂。
///
/// 不上 `#[cfg(test)]` 是有意——使 `crates/roostery/tests/*.rs` 的集成测试
/// 同样能引用。运行期开销是一个 zero-sized Mutex<()>，可以忽略。
pub static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::TEST_ENV_LOCK as ENV_LOCK;
    use super::*;

    #[test]
    fn env_override_wins() {
        let _g = ENV_LOCK.lock().unwrap();
        // Safety: tests touching env are serialized via ENV_LOCK.
        unsafe { std::env::set_var(ENV_HOME, "/tmp/roostery-test-override") };
        assert_eq!(
            roostery_home(),
            PathBuf::from("/tmp/roostery-test-override")
        );
        assert_eq!(
            journal_dir(),
            PathBuf::from("/tmp/roostery-test-override/journal")
        );
        assert_eq!(
            state_dir(),
            PathBuf::from("/tmp/roostery-test-override/state")
        );
        assert_eq!(
            smoke_state_path(),
            PathBuf::from("/tmp/roostery-test-override/state/smoke.json")
        );
        assert_eq!(
            config_path(),
            PathBuf::from("/tmp/roostery-test-override/config.yaml")
        );
        unsafe { std::env::remove_var(ENV_HOME) };
    }

    #[test]
    fn defaults_to_home_dot_roostery() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var(ENV_HOME) };
        let home = dirs::home_dir().expect("test host has HOME");
        assert_eq!(roostery_home(), home.join(DIR_NAME));
    }

    #[test]
    fn ignores_legacy_feishu_hub_home() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var(ENV_HOME) };
        unsafe { std::env::set_var("FEISHU_HUB_HOME", "/tmp/legacy-feishu-hub") };
        let resolved = roostery_home();
        assert!(
            !resolved.starts_with("/tmp/legacy-feishu-hub"),
            "legacy FEISHU_HUB_HOME must not influence resolution, got {resolved:?}"
        );
        unsafe { std::env::remove_var("FEISHU_HUB_HOME") };
    }

    #[test]
    fn empty_env_falls_through_to_default() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var(ENV_HOME, "") };
        let home = dirs::home_dir().expect("test host has HOME");
        assert_eq!(roostery_home(), home.join(DIR_NAME));
        unsafe { std::env::remove_var(ENV_HOME) };
    }
}
