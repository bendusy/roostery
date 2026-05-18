//! 装机相关 API 类型集合：shell 检测 / 跳过原因 / init 选项 / 报告。
//!
//! 拆自原 `onboarding.rs` line 107-195（refactor `2026-05-19-onboarding-split`）。

use super::OnboardingError;
use crate::hooks_merge::AgentKind;
use crate::identity::{Identity, IdentityError};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShellKind {
    Zsh,
    Bash,
}

impl ShellKind {
    pub fn rc_path(self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        Some(match self {
            ShellKind::Zsh => home.join(".zshrc"),
            ShellKind::Bash => home.join(".bashrc"),
        })
    }

    /// Detect from `$SHELL`. Returns `Err(UnsupportedShell { detected })` if
    /// not zsh/bash; the `detected` field is the raw `$SHELL` value (or
    /// `None` if unset) for diagnostic clarity.
    pub fn detect_from_env() -> Result<Self, OnboardingError> {
        let raw = std::env::var("SHELL").ok();
        match raw.as_deref() {
            Some(s) if s.ends_with("/zsh") || s == "zsh" => Ok(ShellKind::Zsh),
            Some(s) if s.ends_with("/bash") || s == "bash" => Ok(ShellKind::Bash),
            _ => Err(OnboardingError::UnsupportedShell { detected: raw }),
        }
    }
}

/// Per-agent skip reason for [`InitReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkipReason {
    NotInstalled,
    UserSkipped,
    MergeFailed(String),
}

#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    pub dry_run: bool,
    pub skip_agents: Vec<AgentKind>,
    /// 显式指定真 lark-cli 路径，跳过 PATH 搜索；优先级最高。
    /// `None` → 读 `ROOSTERY_LARK_CLI_BIN` env → PATH 搜索。
    pub real_lark_cli_override: Option<PathBuf>,
}

/// `InitReport.real_lark_cli` 的来源——让 `format_report` 显式输出走的哪条 path。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RealLarkCliSource {
    /// 来自 `InitOptions.real_lark_cli_override`（一般经 `--real-lark-cli` flag）
    Flag,
    /// 来自 `ROOSTERY_LARK_CLI_BIN` env
    Env,
    /// 来自 PATH 搜索（`which::which_all` 排 shim_target 后第一个候选）
    PathDetected,
}

impl std::fmt::Display for RealLarkCliSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RealLarkCliSource::Flag => "flag",
            RealLarkCliSource::Env => "env",
            RealLarkCliSource::PathDetected => "path",
        })
    }
}

#[derive(Debug)]
pub struct InitReport {
    pub identity: Option<Identity>,
    pub identity_error: Option<IdentityError>,
    pub agents_installed: Vec<AgentKind>,
    pub agents_skipped: Vec<(AgentKind, SkipReason)>,
    pub shim_path: PathBuf,
    pub shell_rc_patched: Option<PathBuf>,
    pub real_lark_cli: PathBuf,
    pub real_lark_cli_source: RealLarkCliSource,
    pub dry_run: bool,
}

impl InitReport {
    pub fn had_errors(&self) -> bool {
        self.agents_skipped
            .iter()
            .any(|(_, r)| matches!(r, SkipReason::MergeFailed(_)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::TEST_ENV_LOCK as ENV_LOCK;

    #[test]
    fn real_lark_cli_source_display_is_lowercase() {
        assert_eq!(RealLarkCliSource::Flag.to_string(), "flag");
        assert_eq!(RealLarkCliSource::Env.to_string(), "env");
        assert_eq!(RealLarkCliSource::PathDetected.to_string(), "path");
    }

    #[test]
    fn init_options_default_real_lark_cli_override_is_none() {
        let opts = InitOptions::default();
        assert!(opts.real_lark_cli_override.is_none());
        assert!(!opts.dry_run);
        assert!(opts.skip_agents.is_empty());
    }

    #[test]
    fn init_options_default() {
        let o = InitOptions::default();
        assert!(!o.dry_run);
        assert!(o.skip_agents.is_empty());
    }

    #[test]
    fn skip_reason_variants_distinguishable() {
        let a = SkipReason::NotInstalled;
        let b = SkipReason::UserSkipped;
        let c = SkipReason::MergeFailed("x".to_string());
        assert_ne!(a, b);
        assert_ne!(b, c);
    }

    #[test]
    fn shell_kind_detect_zsh() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("SHELL").ok();
        unsafe { std::env::set_var("SHELL", "/bin/zsh") };
        assert_eq!(ShellKind::detect_from_env().unwrap(), ShellKind::Zsh);
        unsafe {
            match prev {
                Some(v) => std::env::set_var("SHELL", v),
                None => std::env::remove_var("SHELL"),
            }
        }
    }

    #[test]
    fn shell_kind_detect_bash() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("SHELL").ok();
        unsafe { std::env::set_var("SHELL", "/usr/local/bin/bash") };
        assert_eq!(ShellKind::detect_from_env().unwrap(), ShellKind::Bash);
        unsafe {
            match prev {
                Some(v) => std::env::set_var("SHELL", v),
                None => std::env::remove_var("SHELL"),
            }
        }
    }

    #[test]
    fn shell_kind_detect_fish_errors() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("SHELL").ok();
        unsafe { std::env::set_var("SHELL", "/usr/bin/fish") };
        match ShellKind::detect_from_env() {
            Err(OnboardingError::UnsupportedShell { detected }) => {
                assert_eq!(detected.as_deref(), Some("/usr/bin/fish"));
            }
            other => panic!("expected UnsupportedShell, got {other:?}"),
        }
        unsafe {
            match prev {
                Some(v) => std::env::set_var("SHELL", v),
                None => std::env::remove_var("SHELL"),
            }
        }
    }

    #[test]
    fn shell_kind_detect_unset_errors() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("SHELL").ok();
        unsafe { std::env::remove_var("SHELL") };
        match ShellKind::detect_from_env() {
            Err(OnboardingError::UnsupportedShell { detected: None }) => {}
            other => panic!("expected UnsupportedShell(None), got {other:?}"),
        }
        unsafe {
            if let Some(v) = prev {
                std::env::set_var("SHELL", v);
            }
        }
    }
}
