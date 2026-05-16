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
