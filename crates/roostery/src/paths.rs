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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
