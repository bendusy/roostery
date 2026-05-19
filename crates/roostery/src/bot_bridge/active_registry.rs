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
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};

use crate::bot_task_writer::TaskGuid;

/// 进程内 runner 实例唯一 id（codex round-7 P1-2 修复）。
///
/// 每次 `register` 拿一个新的 RunId 作为主键，避免 `TaskGuid` 在 relay_task
/// cache hit 场景下（同 chat 多次 record_start 复用同一 TaskGuid）多个并发
/// `handle_event` 注册时 BTreeMap 互相 overwrite —— 之前那条 race 会让
/// 前一个 runner 的 `kill_tx` 被 drop，runner 错误退出。
///
/// 来源是进程内 `AtomicU64` 单调计数器，进程重启 reset；HITL 信号不跨进程。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(u64);

impl RunId {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "run{}", self.0)
    }
}

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
/// - `NotFound`     ：run_id 在表中不存在（task 已结束 / 未 register）
/// - `ReceiverGone` ：oneshot::Sender::send 失败（runner 已 drop receiver，等价 runner 自然结束）
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HitlSignalError {
    #[error("no active runner for run_id={0}")]
    NotFound(RunId),
    #[error("runner receiver already dropped for run_id={0}")]
    ReceiverGone(RunId),
}

/// 活跃 runner 表（design §2.1）。
///
/// 内部 `Mutex<BTreeMap<RunId, RunnerHandle>>`——RunId 由 `next_id` 单调发放
/// 保证多个并发 `handle_event` 即便 TaskGuid 重复（cache hit 场景）也不互相
/// overwrite。`lookup_by_chat_id` 仍按 BTreeMap 顺序遍历（RunId 单增→最早
/// 注册的 run 在前）。
#[derive(Debug, Default)]
pub struct ActiveRunnerRegistry {
    inner: Mutex<BTreeMap<RunId, RunnerHandle>>,
    next_id: AtomicU64,
}

impl ActiveRunnerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一条 handle 并返回唯一 `RunId`。同 TaskGuid 多次 register（cache
    /// hit 场景）各自得独立 RunId，互不覆盖。
    pub fn register(&self, handle: RunnerHandle) -> RunId {
        let id = RunId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let mut guard = self
            .inner
            .lock()
            .expect("ActiveRunnerRegistry mutex poisoned");
        guard.insert(id, handle);
        id
    }

    /// 移除并返回 handle；用于 runner 自然结束后清理表（design §流程图）。
    pub fn unregister(&self, run_id: RunId) -> Option<RunnerHandle> {
        let mut guard = self
            .inner
            .lock()
            .expect("ActiveRunnerRegistry mutex poisoned");
        guard.remove(&run_id)
    }

    /// 同 chat 多 task 时取**第一个**命中（design 注释：不是全部）——按 RunId
    /// 升序意味着最早 register 的 run 先返回。
    ///
    /// 调用方期望"对该 chat 一条 HITL 信号"的语义；同 chat 并发多 run 时未
    /// 命中的 run 不受 HITL 影响（已知 trade-off，留待后续考虑"send 到全部"
    /// 语义）。
    pub fn lookup_by_chat_id(&self, chat_id: &str) -> Option<RunId> {
        let guard = self
            .inner
            .lock()
            .expect("ActiveRunnerRegistry mutex poisoned");
        guard
            .iter()
            .find(|(_, h)| h.chat_id == chat_id)
            .map(|(id, _)| *id)
    }

    /// 给 run_id 对应的 runner 发 HITL 信号。
    ///
    /// 实现细节：oneshot::Sender 的 send 消费 self，所以必须 `remove` handle 取所有权
    /// （send 成功后 handle 不再可用，表中清除天经地义；send 失败则 handle 已无用，
    /// 等价 receiver 端已退出，也清除）。
    pub fn send_signal(&self, run_id: RunId, sig: HitlSignal) -> Result<(), HitlSignalError> {
        let mut guard = self
            .inner
            .lock()
            .expect("ActiveRunnerRegistry mutex poisoned");
        let handle = guard
            .remove(&run_id)
            .ok_or(HitlSignalError::NotFound(run_id))?;
        handle
            .kill_tx
            .send(sig)
            .map_err(|_| HitlSignalError::ReceiverGone(run_id))?;
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
        let id = reg.register(h);

        reg.send_signal(
            id,
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
        let id = reg.register(h);

        reg.send_signal(
            id,
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
    fn send_signal_unknown_run_id_returns_not_found() {
        let reg = ActiveRunnerRegistry::new();
        let ghost = RunId(9999);
        let err = reg
            .send_signal(ghost, HitlSignal::Abort { reason: "x".into() })
            .unwrap_err();
        match err {
            HitlSignalError::NotFound(id) => assert_eq!(id.as_u64(), 9999),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn send_signal_receiver_dropped_returns_receiver_gone() {
        let reg = ActiveRunnerRegistry::new();
        let (h, rx) = mk_handle("g3", "oc_c");
        let id = reg.register(h);
        drop(rx);

        let err = reg
            .send_signal(id, HitlSignal::Abort { reason: "x".into() })
            .unwrap_err();
        assert!(matches!(err, HitlSignalError::ReceiverGone(_)));
    }

    #[test]
    fn lookup_by_chat_id_returns_first_registered_match_when_multiple_share_chat() {
        let reg = ActiveRunnerRegistry::new();
        // 同 chat 3 task，注册顺序 b → a → c；RunId 单增，最早 register 的（h_b）
        // 排首位——这是 codex round-7 P1-2 修复的语义：以 RunId 为主键避免同
        // TaskGuid 互相 overwrite，lookup_by_chat_id 返最早 RunId 命中的。
        let (h_a, _rx_a) = mk_handle("g_a", "oc_shared");
        let (h_b, _rx_b) = mk_handle("g_b", "oc_shared");
        let (h_c, _rx_c) = mk_handle("g_c", "oc_shared");
        let _id_b = reg.register(h_b);
        let _id_a = reg.register(h_a);
        let _id_c = reg.register(h_c);

        let found = reg
            .lookup_by_chat_id("oc_shared")
            .expect("should find at least one");
        assert_eq!(found.as_u64(), 0, "earliest-registered (RunId=0) wins");
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
        let id = reg.register(h);

        let popped = reg.unregister(id);
        assert!(popped.is_some());
        assert_eq!(popped.unwrap().task_guid.as_str(), "g1");

        // 再 unregister 同 id 应 None
        assert!(
            reg.unregister(id).is_none(),
            "second unregister should return None"
        );
    }

    #[test]
    fn register_same_task_guid_twice_yields_distinct_run_ids_no_overwrite() {
        // P1-2 核心回归测试：relay_task cache hit 场景下同一 TaskGuid 多次
        // register 必须各得独立 RunId，前一个 handle 不被覆盖。
        let reg = ActiveRunnerRegistry::new();
        let (h1, rx1) = mk_handle("g_same", "oc_x");
        let (h2, rx2) = mk_handle("g_same", "oc_x");
        let id1 = reg.register(h1);
        let id2 = reg.register(h2);
        assert_ne!(id1, id2);
        // 两个 receiver 都还能拿到信号（h1 没被 h2 覆盖掉）。
        reg.send_signal(id1, HitlSignal::Abort { reason: "s1".into() })
            .expect("id1 still alive");
        reg.send_signal(id2, HitlSignal::Abort { reason: "s2".into() })
            .expect("id2 still alive");
        drop((rx1, rx2));
    }
}
