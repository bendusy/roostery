//! `bot_bridge` — Phase 5 Module F 收尾子 feature 的 Rust 子目录。
//!
//! 见 `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md`。
//!
//! 7 子模块按职责切分（design §1.3 D1）：
//! - `role`            BotRole / BotsConfig / bots.yaml 加载
//! - `hitl`            HitlDecision / IM event 关键词分类
//! - `active_registry` 进程内活跃 runner 表 + oneshot HITL 信号通道
//! - `relay_task`      chat_id → TaskRef 缓存 + step 文案 + record_start/end/adjust
//! - `event`           IM 事件源（lark-cli im_messages_subscribe NDJSON tail）
//! - `runner`          handle_event 编排
//! - `daemon`          run_bridge 长跑主循环 + tokio spawn
//!
//! `cli` 是挂载点适配层（compound `2026-05-18-decision-cli-subcommand-module-layout.md`）。

pub mod active_registry;
pub mod cli;
pub mod daemon;
pub mod event;
pub mod hitl;
pub mod relay_task;
pub mod role;
pub mod runner;

pub use daemon::{BridgeError, BridgeOptions, BridgeReport, run_bridge};
