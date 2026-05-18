//! `bot_bridge::active_registry` — 进程内活跃 runner 表 + oneshot HITL 信号通道。
//!
//! 见 `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md`
//! §2.1（ActiveRunnerRegistry / RunnerHandle / HitlSignal）+ §1.3 D3
//! （不落盘 sentinel；用 `tokio::sync::oneshot::Sender<HitlSignal>` 替代 POSIX 信号）。
//!
//! 与 `dispatcher::runners::RunnerRegistry` **同名不同概念**（D2）：后者是 "runner kind
//! 注册表"，本表是 "活跃 task 实例表"——这里加 `Active` 前缀避让，长期重构待 cs-refactor。
//!
//! 来源：legacy/python/src/roostery/runner_registry.py 的"进程内部分"
//! （Python 用落盘 abort.txt / adjust.txt 跨进程通信；Rust 同进程 tokio runtime，oneshot 即足）。

use std::collections::BTreeMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};

use crate::bot_task_writer::TaskGuid;

/// 传给运行中 runner 的 HITL 动作信号（design §2.1）。
///
/// 与 `HitlDecision` 区别：`HitlDecision::Pass` 不发信号（无动作可做），
/// 所以本 enum 只有两态——发到 oneshot 上的就是"runner 必须立即响应"的指令。
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum HitlSignal {
    Abort { reason: String },
    Adjust { body: String },
}

/// 一条活跃 runner 的 handle（design §2.1）。
///
/// `kill_tx` 是 oneshot::Sender，runner 协程在 `tokio::select!` 里 await
/// `kill_rx`；本表通过 `send_signal` 把 `HitlSignal` 注入对方 channel——
/// send 成功 = runner 已收到信号；send 失败 = receiver 已 drop（runner 自然结束）。
pub struct RunnerHandle {
    pub kill_tx: tokio::sync::oneshot::Sender<HitlSignal>,
    pub task_guid: TaskGuid,
    pub task_url: String,
    pub chat_id: String,
    pub started_at: DateTime<Utc>,
}

impl std::fmt::Debug for RunnerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // oneshot::Sender 不实现 Debug，单独写一份。
        f.debug_struct("RunnerHandle")
            .field("task_guid", &self.task_guid)
            .field("task_url", &self.task_url)
            .field("chat_id", &self.chat_id)
            .field("started_at", &self.started_at)
            .field("kill_tx", &"<oneshot::Sender<HitlSignal>>")
            .finish()
    }
}

/// 发送 HITL 信号时的错误（design §2.1 `Result<(), HitlSignalError>`）。
///
/// 两种实际可能：
/// - `NotFound`     ：guid 在表中不存在（task 已结束 / 未 register）
/// - `ReceiverGone` ：oneshot::Sender::send 失败（runner 已 drop receiver，等价 runner 自然结束）
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HitlSignalError {
    #[error("no active runner for task_guid={0}")]
    NotFound(TaskGuid),
    #[error("runner receiver already dropped for task_guid={0}")]
    ReceiverGone(TaskGuid),
}

/// 活跃 runner 表（design §2.1）。
///
/// 内部用 `Mutex<BTreeMap<TaskGuid, RunnerHandle>>`——BTreeMap 选自 design 显式标注，
/// 也方便 `lookup_by_chat_id` 按确定性顺序遍历（同 chat 多 task 时取第一个）。
#[derive(Debug, Default)]
pub struct ActiveRunnerRegistry {
    inner: Mutex<BTreeMap<TaskGuid, RunnerHandle>>,
}

impl ActiveRunnerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一条 handle；若同 guid 已存在则覆盖（design 未约束去重，简化处理）。
    pub fn register(&self, handle: RunnerHandle) {
        let mut guard = self
            .inner
            .lock()
            .expect("ActiveRunnerRegistry mutex poisoned");
        guard.insert(handle.task_guid.clone(), handle);
    }

    /// 移除并返回 handle；用于 runner 自然结束后清理表（design §流程图）。
    pub fn unregister(&self, guid: &TaskGuid) -> Option<RunnerHandle> {
        let mut guard = self
            .inner
            .lock()
            .expect("ActiveRunnerRegistry mutex poisoned");
        guard.remove(guid)
    }

    /// 同 chat 多 task 时取**第一个**命中（design 注释：不是全部）。
    ///
    /// 顺序由 BTreeMap 的 `TaskGuid` 字典序决定，调用方不要依赖具体哪条；
    /// 但 "第一个" 的语义保证了同输入同输出（确定性）。
    pub fn lookup_by_chat_id(&self, chat_id: &str) -> Option<TaskGuid> {
        let guard = self
            .inner
            .lock()
            .expect("ActiveRunnerRegistry mutex poisoned");
        guard
            .iter()
            .find(|(_, h)| h.chat_id == chat_id)
            .map(|(guid, _)| guid.clone())
    }

    /// 给 guid 对应的 runner 发 HITL 信号。
    ///
    /// 实现细节：oneshot::Sender 的 send 消费 self，所以必须 `remove` handle 取所有权
    /// （send 成功后 handle 不再可用，表中清除天经地义；send 失败则 handle 已无用，
    /// 等价 receiver 端已退出，也清除）。
    pub fn send_signal(&self, guid: &TaskGuid, sig: HitlSignal) -> Result<(), HitlSignalError> {
        let mut guard = self
            .inner
            .lock()
            .expect("ActiveRunnerRegistry mutex poisoned");
        let handle = guard
            .remove(guid)
            .ok_or_else(|| HitlSignalError::NotFound(guid.clone()))?;
        handle
            .kill_tx
            .send(sig)
            .map_err(|_| HitlSignalError::ReceiverGone(guid.clone()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_handle(
        guid: &str,
        chat_id: &str,
    ) -> (RunnerHandle, tokio::sync::oneshot::Receiver<HitlSignal>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let h = RunnerHandle {
            kill_tx: tx,
            task_guid: TaskGuid::from_existing(guid),
            task_url: format!("https://example/task/{guid}"),
            chat_id: chat_id.into(),
            started_at: Utc::now(),
        };
        (h, rx)
    }

    #[tokio::test]
    async fn oneshot_send_signal_delivers_abort_to_receiver() {
        let reg = ActiveRunnerRegistry::new();
        let (h, rx) = mk_handle("g1", "oc_a");
        reg.register(h);

        reg.send_signal(
            &TaskGuid::from_existing("g1"),
            HitlSignal::Abort {
                reason: "/stop".into(),
            },
        )
        .expect("send_signal ok");

        match rx.await.expect("receiver got value") {
            HitlSignal::Abort { reason } => assert_eq!(reason, "/stop"),
            other => panic!("expected Abort, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn oneshot_send_signal_delivers_adjust_to_receiver() {
        let reg = ActiveRunnerRegistry::new();
        let (h, rx) = mk_handle("g2", "oc_b");
        reg.register(h);

        reg.send_signal(
            &TaskGuid::from_existing("g2"),
            HitlSignal::Adjust {
                body: "use sqlite".into(),
            },
        )
        .expect("send_signal ok");

        match rx.await.expect("receiver got value") {
            HitlSignal::Adjust { body } => assert_eq!(body, "use sqlite"),
            other => panic!("expected Adjust, got {other:?}"),
        }
    }

    #[test]
    fn send_signal_unknown_guid_returns_not_found() {
        let reg = ActiveRunnerRegistry::new();
        let err = reg
            .send_signal(
                &TaskGuid::from_existing("ghost"),
                HitlSignal::Abort { reason: "x".into() },
            )
            .unwrap_err();
        match err {
            HitlSignalError::NotFound(g) => assert_eq!(g.as_str(), "ghost"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn send_signal_receiver_dropped_returns_receiver_gone() {
        let reg = ActiveRunnerRegistry::new();
        let (h, rx) = mk_handle("g3", "oc_c");
        reg.register(h);
        drop(rx);

        let err = reg
            .send_signal(
                &TaskGuid::from_existing("g3"),
                HitlSignal::Abort { reason: "x".into() },
            )
            .unwrap_err();
        assert!(matches!(err, HitlSignalError::ReceiverGone(_)));
    }

    #[test]
    fn lookup_by_chat_id_returns_first_match_when_multiple_tasks_share_chat() {
        let reg = ActiveRunnerRegistry::new();
        // 同 chat "oc_shared" 下 3 个 task，BTreeMap 字典序 g_a < g_b < g_c
        let (h_a, _rx_a) = mk_handle("g_a", "oc_shared");
        let (h_b, _rx_b) = mk_handle("g_b", "oc_shared");
        let (h_c, _rx_c) = mk_handle("g_c", "oc_shared");
        reg.register(h_b);
        reg.register(h_a);
        reg.register(h_c);

        let found = reg
            .lookup_by_chat_id("oc_shared")
            .expect("should find at least one");
        // BTreeMap 字典序遍历 → 首个是 g_a
        assert_eq!(found.as_str(), "g_a");
    }

    #[test]
    fn lookup_by_chat_id_returns_none_when_no_match() {
        let reg = ActiveRunnerRegistry::new();
        let (h, _rx) = mk_handle("g1", "oc_x");
        reg.register(h);
        assert!(reg.lookup_by_chat_id("oc_other").is_none());
    }

    #[test]
    fn register_unregister_round_trip() {
        let reg = ActiveRunnerRegistry::new();
        let (h, _rx) = mk_handle("g1", "oc_x");
        reg.register(h);

        let popped = reg.unregister(&TaskGuid::from_existing("g1"));
        assert!(popped.is_some());
        assert_eq!(popped.unwrap().task_guid.as_str(), "g1");

        // 再 unregister 同 guid 应 None
        assert!(
            reg.unregister(&TaskGuid::from_existing("g1")).is_none(),
            "second unregister should return None"
        );
    }
}
