//! # 飞书 syscall 唯一通道
//!
//! 本模块是 Roostery 与飞书通信的**唯一 sanctioned 通道**——所有走向飞书的
//! 调用必须通过 [`LarkRunner`] trait。这是架构红线（见
//! `.codestable/architecture/ARCHITECTURE.md` §6 第 1 条 + `.codestable/attention.md`
//! "命令与脚本陷阱" 节）。
//!
//! ## 绕过本模块的反例（code review 拒收）
//!
//! ```ignore
//! // ❌ 直接 spawn lark-cli
//! tokio::process::Command::new("lark-cli").args(...).output().await;
//!
//! // ❌ 直接 HTTP 调飞书 API
//! reqwest::Client::new().post("https://open.feishu.cn/...").send().await;
//!
//! // ❌ 引 Feishu SDK
//! use lark_sdk::*;
//! ```
//!
//! ## 正确用法
//!
//! ```ignore
//! use roostery::lark_cli::{LarkRunner, LarkCli, Journaled};
//! use roostery::journal::Journal;
//!
//! let runner = Journaled::new(LarkCli::new(), Journal::default(), "shim");
//! let value = runner.run(&["im", "+messages-send", "--user-id", "ou_x", "--text", "hi"]).await?;
//! ```
//!
//! ## 模块组织（compound convention 档 2）
//!
//! - [`runner`]: `LarkRunner` trait + `RunOptions`
//! - [`error`]: `LarkError` rich enum + `retriable()` method
//! - [`subprocess`]: `LarkCli` 默认 subprocess 实现
//! - [`mock`]: `MockLarkRunner` 测试替身（test utility，production 不应依赖）
//! - [`journaled`]: `Journaled<R>` 装饰器，写 journal 前过 `redact::scrub_argv`

pub mod error;
pub mod journaled;
pub mod mock;
pub mod runner;
pub mod subprocess;

pub use error::{LarkError, MAX_FIELD_LEN_IN_ERR};
pub use runner::{LarkRunner, RunOptions};

// --- Compile-time evidence (doctests) ---------------------------------------
//
// `LarkError` and `RunOptions` are `#[non_exhaustive]`. Outside the defining
// crate (i.e. doctests' synthetic crates), exhaustive match without `_`
// fails E0004 and struct literals without `..Default::default()` fail E0639.
//
// Type isolation is also enforced: the four variants and the inner data
// shapes cannot be confused with each other at the type level.

/// Doctest: `match` on `LarkError` without `_` is rejected outside the crate.
///
/// ```compile_fail,E0004
/// use roostery::lark_cli::LarkError;
/// fn label(e: &LarkError) -> &'static str {
///     match e {
///         LarkError::Spawn { .. } => "spawn",
///         LarkError::NonZeroExit { .. } => "exit",
///         LarkError::OutputParse { .. } => "parse",
///         LarkError::Timeout { .. } => "timeout",
///         // missing `_ =>`; #[non_exhaustive] requires it externally
///     }
/// }
/// ```
///
/// With `_ =>` it compiles:
///
/// ```
/// use roostery::lark_cli::LarkError;
/// fn label(e: &LarkError) -> &'static str {
///     match e {
///         LarkError::Spawn { .. } => "spawn",
///         LarkError::NonZeroExit { .. } => "exit",
///         LarkError::OutputParse { .. } => "parse",
///         LarkError::Timeout { .. } => "timeout",
///         _ => "other",
///     }
/// }
/// ```
#[allow(dead_code)]
fn _doctest_anchor_lark_error_non_exhaustive() {}

/// Doctest: `RunOptions { ... }` struct literal is rejected outside the
/// crate (E0639) — even `..Default::default()` does NOT bypass
/// `#[non_exhaustive]`. External callers must use the builder API.
///
/// ```compile_fail,E0639
/// use roostery::lark_cli::RunOptions;
/// use std::time::Duration;
/// let _ = RunOptions {
///     timeout: Some(Duration::from_secs(1)),
///     ..Default::default()
/// };
/// ```
///
/// Use the builder instead:
///
/// ```
/// use roostery::lark_cli::RunOptions;
/// use std::time::Duration;
/// let _ = RunOptions::new().with_timeout(Duration::from_secs(1));
/// ```
#[allow(dead_code)]
fn _doctest_anchor_run_options_non_exhaustive() {}
