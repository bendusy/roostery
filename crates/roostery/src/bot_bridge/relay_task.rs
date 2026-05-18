//! `bot_bridge::relay_task` — chat_id → TaskRef 缓存 + step 文案。
//!
//! 见 `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md`
//! §2.1（record_start / record_end / record_adjust / EndOutcome /
//! BOT_CHAT_CACHE_SCHEMA_VERSION）。
//!
//! **本 step（step 4）只落最小占位**：
//! - `EndOutcome` 完整四态——`runner.rs` 立即依赖
//! - `record_start` / `record_end` / `record_adjust` 占位函数返 Ok（含明确 TODO 指向 step 5）
//! - `BOT_CHAT_CACHE_SCHEMA_VERSION` 常量先占坑
//!
//! step 5 会替换占位实现，加入 cache 读写 + step 文案 + idempotency_key。
//! 占位与 runner.rs 的调用点位签名一致，step 5 不需要改 runner.rs。

use crate::bot_task_writer::{TaskRef, TaskWriterError};
use crate::lark_cli::LarkRunner;

use crate::bot_bridge::event::ImEvent;
use crate::bot_bridge::role::BotRole;

/// cache schema 公开承诺；design §2.1 + 检查项。
pub const BOT_CHAT_CACHE_SCHEMA_VERSION: u32 = 1;

/// runner 终态——relay_task 的 step 文案 + `runner.rs` 的内部判定共享此 enum。
///
/// design §2.1：四态 Success / Failed / Aborted / Timeout。
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EndOutcome {
    Success { adjust_attempts: u32 },
    Failed { exit_code: i32 },
    Aborted { reason: String },
    Timeout,
}

/// relay_task 错误（design §2.1 两类，step 5 视实装实际需要扩展）。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RelayTaskError {
    #[error("task writer failed: {0}")]
    TaskWriter(#[from] TaskWriterError),
    #[error("cache load/save failed: {0}")]
    Cache(#[source] std::io::Error),
}

/// 记录 runner 启动；step 4 占位返 None（无 task 关联）。
///
/// step 5 将实装：cache lookup → 命中复用 / 未命中 create_task → append_step "🚀 ..."。
#[allow(clippy::unused_async)]
pub async fn record_start(
    _lark: &dyn LarkRunner,
    _bot: &BotRole,
    _event: &ImEvent,
    _message_brief: &str,
) -> Result<Option<TaskRef>, RelayTaskError> {
    // TODO(step 5): cache lookup / create_task / append step "🚀 已收到 ..."。
    Ok(None)
}

/// 记录 runner 终态；step 4 占位返 None。
///
/// step 5 将实装：append step（Success "✅ ..." / Failed "❌ ..." / Aborted "⚠️ ..." / Timeout "⏱️ ..."）。
#[allow(clippy::unused_async)]
pub async fn record_end(
    _lark: &dyn LarkRunner,
    _bot: &BotRole,
    _chat_id: &str,
    _source_message_id: &str,
    _outcome: &EndOutcome,
    _result_text: &str,
) -> Result<Option<TaskRef>, RelayTaskError> {
    // TODO(step 5): cache lookup + append step 按 outcome 走对应文案。
    Ok(None)
}

/// 记录 /adjust 重启；step 4 占位返 Ok。
///
/// step 5 将实装：append step "🔁 用户调整 (attempt N): ..."。
#[allow(clippy::unused_async)]
pub async fn record_adjust(
    _lark: &dyn LarkRunner,
    _bot: &BotRole,
    _task_ref: &TaskRef,
    _adjust_text: &str,
    _attempt: u32,
) -> Result<(), RelayTaskError> {
    // TODO(step 5): append step "🔁 ..." 记录调整请求。
    Ok(())
}
