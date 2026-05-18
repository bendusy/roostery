//! `bot_bridge::daemon` — 占位，将在 step 7 实装。
//!
//! 见 `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md`
//! §2.1 / §2.2（run_bridge / BridgeOptions / BridgeReport / mpsc 主循环 +
//! tokio::signal::ctrl_c graceful shutdown）。
//!
//! 本 step 仅落主入口签名与零值返回，串通 CLI → run_bridge → BridgeReport 的形状，
//! 不实装任何编排逻辑。

use std::path::Path;
use std::time::Duration;

/// daemon 启动参数集合；见 design §2.1 `BridgeOptions`。
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct BridgeOptions {
    /// per-bot handle_event 并发上限。0 = 实装期默认值。
    pub max_concurrency: usize,
    /// 处理 N 条 event 后正常退出。0 = unlimited（在本 step 表现为立即返回零值）。
    pub max_events: usize,
    /// 单 event 处理总超时。None = 不限制。
    pub timeout: Option<Duration>,
    /// `--profile` 过滤；空 = 全部 BotRole。
    pub profile_filter: Vec<String>,
}

/// daemon 退出聚合报告；见 design §2.1 `BridgeReport`。
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct BridgeReport {
    pub events_processed: u64,
    pub actions_emitted: u64,
    pub aborts_handled: u64,
    pub adjusts_handled: u64,
    pub errors: u64,
}

/// daemon 启动错误；占位，等待 step 7 补全变体。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BridgeError {
    /// 占位变体——step 7 将替换为真实变体（load_bots / mpsc / lark-cli subscribe 等）。
    #[error("bot_bridge daemon not implemented yet")]
    NotImplemented,
}

/// daemon 主入口；step 1 空实现。
///
/// - `--max-events = 0` 在 step 1 视为"立即返回"信号（无 event 源可消费），
///   方便端到端走通 CLI dispatch + exit 0。
/// - 真正的"0 = unlimited"语义在 step 7 实装。
pub async fn run_bridge(
    _bots_path: &Path,
    opts: BridgeOptions,
) -> Result<BridgeReport, BridgeError> {
    // step 1：无论 opts 怎么传，都直接返回零值报告。
    let _ = opts;
    Ok(BridgeReport::default())
}
