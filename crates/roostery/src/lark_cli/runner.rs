//! `LarkRunner` trait + `RunOptions`. See module-level docs.

use crate::lark_cli::error::LarkError;
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

/// 调用单次 lark-cli 的辅助选项。
///
/// **构造方式**：用 `RunOptions::new()` + builder method 链。`#[non_exhaustive]`
/// 锁定外部不能用 struct literal（包括 `..Default::default()` 也不行——见
/// rustc E0639），必须走 builder。这样未来加 `env` / `cwd` / `kill_on_drop`
/// 等字段时 caller `RunOptions::new().with_timeout(d)` 不受影响。
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct RunOptions {
    /// `None` = 用实现的默认值（`LarkCli::default_timeout`，30s）
    pub timeout: Option<Duration>,
    /// `None` = subprocess 不接 stdin
    pub stdin: Option<String>,
    /// `lark-cli --profile <X>` global flag；多 bot 协作场景下指定 profile
    pub profile: Option<String>,
}

impl RunOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn with_stdin(mut self, stdin: impl Into<String>) -> Self {
        self.stdin = Some(stdin.into());
        self
    }

    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = Some(profile.into());
        self
    }
}

/// 飞书 syscall 唯一抽象（roadmap §4.1）。
///
/// 默认 method `run` 委托 `run_with_options(args, default())`，让 caller 在
/// 不需要高级配置时调最简形态。实现者通常只实现 `run_with_options`。
#[async_trait]
pub trait LarkRunner: Send + Sync {
    /// 最简调用形态。`args[0]` 是 lark-cli 子命令（如 `"im"`）。
    async fn run(&self, args: &[&str]) -> Result<Value, LarkError> {
        self.run_with_options(args, RunOptions::default()).await
    }

    /// 高级场景：自定义 timeout / stdin / profile。
    async fn run_with_options(&self, args: &[&str], opts: RunOptions) -> Result<Value, LarkError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_options_default_all_none() {
        let o = RunOptions::default();
        assert!(o.timeout.is_none());
        assert!(o.stdin.is_none());
        assert!(o.profile.is_none());
    }

    #[test]
    fn run_options_builder_chain() {
        let o = RunOptions::new()
            .with_timeout(Duration::from_secs(5))
            .with_stdin("payload")
            .with_profile("bot2");
        assert_eq!(o.timeout, Some(Duration::from_secs(5)));
        assert_eq!(o.stdin.as_deref(), Some("payload"));
        assert_eq!(o.profile.as_deref(), Some("bot2"));
    }
}
