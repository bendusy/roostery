//! `bot_bridge::event` — 占位，将在 step 6 实装完整 `consume_im` + `EventError`。
//!
//! 见 `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md`
//! §2.1（ImEvent / consume_im / EventError）。
//!
//! 本 step（step 2）仅落 `ImEvent` 最小 stub，给 `role::event_matches_bot` /
//! `role::extract_message_body` 提供签名依赖；`consume_im` / `EventError` 在 step 6 实装。

use serde::Deserialize;

/// 飞书 IM 事件最小模型（design §2.1）。
///
/// 字段集与 design 一致，反序列化来自 lark-cli `im im_messages_subscribe` NDJSON 行。
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ImEvent {
    pub message_id: String,
    pub chat_id: String,
    pub chat_type: String,
    pub message_type: String,
    pub sender_id: String,
    pub content: String,
}
