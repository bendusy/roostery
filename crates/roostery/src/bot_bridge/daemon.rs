//! `bot_bridge::daemon` — run_bridge 长跑主循环。
//!
//! 见 `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md`
//! §2.1 / §2.2（BridgeOptions / BridgeReport / 主流程图 + tokio::signal::ctrl_c
//! graceful shutdown）+ §3 G1-G6（红线）+ checklist step 7。
//!
//! 编排：
//! 1. `role::load_bots` 读 bots.yaml；按 `profile_filter` 过滤
//! 2. 每个 bot spawn 一条 `event::consume_im` 流，转发到 central `mpsc<(BotRole, Item)>`
//! 3. 主循环 `tokio::select!`：
//!    - mpsc.recv() →
//!      a. event_matches_bot 否 → events_skipped_unmatched_chat += 1
//!      b. hitl::classify 命中 Abort/Adjust → `active.lookup_by_chat_id` → send_signal
//!      c. Pass → `tokio::spawn handle_event`，handle 入 `JoinSet`
//!    - ctrl_c / cancel_token → break shutdown
//!    - max_events / max_duration 触上限 → break shutdown
//! 4. shutdown：drop 中央 sender → consume_im tasks 自然退出（kill_on_drop）→
//!    给已 spawn 的 handle_event 一个 deadline，到期还在跑的 → abort JoinSet 兜底。
//!
//! 红线守护：本模块不直接 `Command::new("lark-cli")`、不 `reqwest`、不 `os::unix`，
//! 飞书 API 全部走 `LarkRunner` trait；CLI 子进程交由 `event::consume_im`。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::json;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinSet;

use crate::bot_bridge::active_registry::{ActiveRunnerRegistry, HitlSignal};
use crate::bot_bridge::event::{ConsumeOpts, EventError, ImEvent, consume_im};
use crate::bot_bridge::hitl::{HitlDecision, classify};
use crate::bot_bridge::role::{
    BotRole, BotRoleError, event_matches_bot, extract_message_body, load_bots,
};
use crate::bot_bridge::runner::{HandleEventError, handle_event};
use crate::dispatcher::runners::RunnerRegistry;
use crate::journal::{Journal, JournalEntry, JournalResult};
use crate::lark_cli::LarkRunner;
use crate::lark_cli::subprocess::LarkCli;

/// daemon shutdown 原因——`BridgeReport::shutdown_reason` 取值集合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShutdownReason {
    /// 收到 Ctrl-C / 外部 cancel
    CtrlC,
    /// `--max-events` 阈值触发
    MaxEvents,
    /// `--timeout` 阈值触发
    MaxDuration,
    /// 所有事件源退出后无法重连——event channel 关闭
    EventSourceClosed,
    /// bots.yaml 内无可订阅 bot（filter 后空）—— 没有事件源可启动
    NoBots,
}

/// 进程内可取消令牌：daemon 接受外部注入 → CLI 入口在 ctrl_c handler 内 cancel。
///
/// 设计选择 A（仓库无 tokio-util，手撸 `Arc<AtomicBool>` + `Notify` 即足）。
#[derive(Debug, Default)]
pub struct CancelToken {
    flag: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        if !self.flag.swap(true, Ordering::SeqCst) {
            self.notify.notify_waiters();
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        loop {
            let waited = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            waited.await;
            if self.is_cancelled() {
                return;
            }
        }
    }
}

/// daemon 启动参数集合（design §2.1 + 测试可观察性扩展）。
pub struct BridgeOptions {
    pub max_concurrency: usize,
    pub max_events: usize,
    pub timeout: Option<Duration>,
    pub profile_filter: Vec<String>,
    /// 中央事件 mpsc buffer。
    pub event_channel_buffer: usize,
    /// graceful shutdown 留给 handle_event 的退出 deadline。
    pub shutdown_deadline: Duration,
    /// 注入给 `consume_im` 的 lark-cli 二进制路径；None = "lark-cli"。
    pub lark_binary: Option<PathBuf>,
    /// 注入 journal 目录；None = `paths::journal_dir()`。
    pub journal_dir: Option<PathBuf>,
    /// 注入 runner 注册表；None = `RunnerRegistry::with_defaults()`。
    pub runner_registry: Option<Arc<RunnerRegistry>>,
    /// 注入 LarkRunner 实例；None = `LarkCli::new()`。
    pub lark_runner: Option<Arc<dyn LarkRunner>>,
    /// 注入可取消令牌；None = 仅响应 ctrl_c。
    pub cancel: Option<Arc<CancelToken>>,
}

impl std::fmt::Debug for BridgeOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgeOptions")
            .field("max_concurrency", &self.max_concurrency)
            .field("max_events", &self.max_events)
            .field("timeout", &self.timeout)
            .field("profile_filter", &self.profile_filter)
            .field("event_channel_buffer", &self.event_channel_buffer)
            .field("shutdown_deadline", &self.shutdown_deadline)
            .field("lark_binary", &self.lark_binary)
            .field("journal_dir", &self.journal_dir)
            .field("runner_registry", &self.runner_registry.is_some())
            .field("lark_runner", &self.lark_runner.is_some())
            .field("cancel", &self.cancel.is_some())
            .finish()
    }
}

impl Clone for BridgeOptions {
    fn clone(&self) -> Self {
        Self {
            max_concurrency: self.max_concurrency,
            max_events: self.max_events,
            timeout: self.timeout,
            profile_filter: self.profile_filter.clone(),
            event_channel_buffer: self.event_channel_buffer,
            shutdown_deadline: self.shutdown_deadline,
            lark_binary: self.lark_binary.clone(),
            journal_dir: self.journal_dir.clone(),
            runner_registry: self.runner_registry.clone(),
            lark_runner: self.lark_runner.clone(),
            cancel: self.cancel.clone(),
        }
    }
}

impl Default for BridgeOptions {
    fn default() -> Self {
        Self {
            max_concurrency: 0,
            max_events: 0,
            timeout: None,
            profile_filter: Vec::new(),
            event_channel_buffer: 64,
            shutdown_deadline: Duration::from_secs(30),
            lark_binary: None,
            journal_dir: None,
            runner_registry: None,
            lark_runner: None,
            cancel: None,
        }
    }
}

/// daemon 退出聚合报告（design §2.1）。
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct BridgeReport {
    pub events_received: u32,
    pub events_skipped_unmatched_chat: u32,
    pub events_skipped_no_match: u32,
    pub hitl_abort_signaled: u32,
    pub hitl_adjust_signaled: u32,
    pub hitl_signal_misses: u32,
    pub handle_event_spawned: u32,
    /// kind ∈ {"success", "failed", "aborted", "timeout", "skipped", "error"}。
    pub handle_event_results: HashMap<String, u32>,
    pub event_source_errors: u32,
    pub bots_subscribed: u32,
    pub shutdown_reason: Option<ShutdownReason>,
}

impl BridgeReport {
    fn bump_result(&mut self, kind: &str) {
        *self
            .handle_event_results
            .entry(kind.to_string())
            .or_insert(0) += 1;
    }
}

/// daemon 启动 / 运行错误。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BridgeError {
    #[error("load bots.yaml failed: {0}")]
    LoadBots(#[from] BotRoleError),
}

/// daemon 主入口（design §2.1 / §2.2）。
pub async fn run_bridge(
    bots_path: &Path,
    opts: BridgeOptions,
) -> Result<BridgeReport, BridgeError> {
    let cfg = load_bots(bots_path)?;
    let filtered: Vec<BotRole> = if opts.profile_filter.is_empty() {
        cfg.bots
    } else {
        cfg.bots
            .into_iter()
            .filter(|b| opts.profile_filter.iter().any(|p| p == &b.app_id))
            .collect()
    };

    let journal_path = opts
        .journal_dir
        .clone()
        .unwrap_or_else(crate::paths::journal_dir);
    let journal = Arc::new(Journal::open(journal_path));

    let lark: Arc<dyn LarkRunner> = opts
        .lark_runner
        .clone()
        .unwrap_or_else(|| Arc::new(LarkCli::new()));
    let runners: Arc<RunnerRegistry> = opts
        .runner_registry
        .clone()
        .unwrap_or_else(|| Arc::new(RunnerRegistry::with_defaults()));
    let active = Arc::new(ActiveRunnerRegistry::new());
    let cancel = opts
        .cancel
        .clone()
        .unwrap_or_else(|| Arc::new(CancelToken::new()));

    let mut report = BridgeReport {
        bots_subscribed: filtered.len() as u32,
        ..Default::default()
    };

    let _ = write_journal(
        &journal,
        "daemon:start",
        json!({
            "bots": filtered.iter().map(|b| b.app_id.clone()).collect::<Vec<_>>(),
            "max_events": opts.max_events,
            "max_concurrency": opts.max_concurrency,
        }),
        JournalResult::Ok {
            value: serde_json::Value::Null,
        },
    );

    if filtered.is_empty() {
        report.shutdown_reason = Some(ShutdownReason::NoBots);
        let _ = write_journal(
            &journal,
            "daemon:shutdown",
            json!({ "reason": "no_bots" }),
            JournalResult::Ok {
                value: serde_json::Value::Null,
            },
        );
        return Ok(report);
    }

    // --- 启动 per-bot consume_im → 中央 mpsc 转发 ----------------------
    let (central_tx, mut central_rx) =
        mpsc::channel::<(BotRole, Result<ImEvent, EventError>)>(opts.event_channel_buffer.max(1));
    let mut source_joins: JoinSet<()> = JoinSet::new();

    // P2 fix (codex audit 2026-05-19): 复用 LarkCli 同源 binary 解析
    // （含 `ROOSTERY_LARK_CLI_BIN` env override），而非 hardcode "lark-cli"
    // 字面量——保证 streaming consume_im 子进程跟 buffered RPC 调用走同一
    // 二进制路径。
    let lark_binary = opts
        .lark_binary
        .clone()
        .unwrap_or_else(|| crate::lark_cli::subprocess::LarkCli::new().binary().to_path_buf());
    for bot in &filtered {
        let bot_clone = bot.clone();
        let tx = central_tx.clone();
        let mut consume_opts = ConsumeOpts::new(lark_binary.clone(), bot.app_id.clone());
        consume_opts.max_events = opts.max_events;
        consume_opts.timeout = opts.timeout;
        let stream_cancel = cancel.clone();
        source_joins.spawn(async move {
            let mut stream = consume_im(consume_opts);
            let exit_reason: &'static str = loop {
                tokio::select! {
                    biased;
                    _ = stream_cancel.cancelled() => break "cancelled",
                    item = stream.rx.recv() => match item {
                        Some(it) => {
                            if tx.send((bot_clone.clone(), it)).await.is_err() {
                                break "central_tx_closed";
                            }
                        }
                        None => break "stream_eof",
                    }
                }
            };
            // P1 fix (codex audit 2026-05-19): drop rx + await inner join 让
            // consume_im 后台 task 完整退出（含子进程 kill_on_drop 收尾），
            // 而非源 task 一返回就留下 dangling lark-cli subscribe 子进程
            // 直到 tokio runtime 退出。
            drop(stream.rx);
            let _ = stream.join.await;
            tracing::debug!(exit_reason, "bot consume_im source task exited");
        });
    }
    drop(central_tx);

    // --- 主循环 ---------------------------------------------------------
    let mut handle_joins: JoinSet<&'static str> = JoinSet::new();
    let started = std::time::Instant::now();
    let ctrl_c_signal = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    tokio::pin!(ctrl_c_signal);

    let shutdown_reason = loop {
        if let Some(total) = opts.timeout
            && started.elapsed() >= total
        {
            break ShutdownReason::MaxDuration;
        }
        if opts.max_events > 0 && report.events_received >= opts.max_events as u32 {
            break ShutdownReason::MaxEvents;
        }
        if cancel.is_cancelled() {
            break ShutdownReason::CtrlC;
        }

        tokio::select! {
            biased;
            _ = &mut ctrl_c_signal => {
                cancel.cancel();
                break ShutdownReason::CtrlC;
            }
            _ = cancel.cancelled() => {
                break ShutdownReason::CtrlC;
            }
            Some(joined) = handle_joins.join_next(), if !handle_joins.is_empty() => {
                match joined {
                    Ok(kind) => report.bump_result(kind),
                    Err(_) => report.bump_result("error"),
                }
            }
            msg = central_rx.recv() => match msg {
                None => break ShutdownReason::EventSourceClosed,
                Some((bot, Err(e))) => {
                    report.event_source_errors = report.event_source_errors.saturating_add(1);
                    let _ = write_journal(
                        &journal,
                        "daemon:event_source_error",
                        json!({"bot_app_id": bot.app_id, "error": format!("{e}")}),
                        JournalResult::Err {
                            kind: "EventError".into(),
                            message: format!("{e}"),
                        },
                    );
                }
                Some((bot, Ok(ev))) => {
                    report.events_received = report.events_received.saturating_add(1);
                    if !bot.chat_whitelist.is_empty()
                        && !bot.chat_whitelist.iter().any(|c| c == &ev.chat_id)
                    {
                        report.events_skipped_unmatched_chat =
                            report.events_skipped_unmatched_chat.saturating_add(1);
                        continue;
                    }
                    let body_for_hitl = extract_message_body(&ev, &bot).to_string();
                    match classify(&body_for_hitl) {
                        HitlDecision::Abort { reason } => {
                            dispatch_hitl_abort(&ev, &bot, reason, &active, &journal, &mut report).await;
                            continue;
                        }
                        HitlDecision::Adjust { body } => {
                            dispatch_hitl_adjust(&ev, &bot, body, &active, &journal, &mut report).await;
                            continue;
                        }
                        HitlDecision::Pass => {}
                    }
                    if !event_matches_bot(&ev, &bot) {
                        report.events_skipped_no_match =
                            report.events_skipped_no_match.saturating_add(1);
                        continue;
                    }
                    if opts.max_concurrency > 0
                        && handle_joins.len() >= opts.max_concurrency
                        && let Some(joined) = handle_joins.join_next().await
                    {
                        match joined {
                            Ok(k) => report.bump_result(k),
                            Err(_) => report.bump_result("error"),
                        }
                    }
                    report.handle_event_spawned = report.handle_event_spawned.saturating_add(1);
                    let lark_c = lark.clone();
                    let runners_c = runners.clone();
                    let active_c = active.clone();
                    let journal_c = journal.clone();
                    let bot_c = bot.clone();
                    let ev_c = ev.clone();
                    handle_joins.spawn(async move {
                        let res = handle_event(
                            &ev_c,
                            &bot_c,
                            lark_c.as_ref(),
                            runners_c.as_ref(),
                            &active_c,
                            journal_c.as_ref(),
                        )
                        .await;
                        action_kind(res)
                    });
                }
            }
        }
    };

    report.shutdown_reason = Some(shutdown_reason);
    let _ = write_journal(
        &journal,
        "daemon:shutdown",
        json!({ "reason": shutdown_reason_str(shutdown_reason) }),
        JournalResult::Ok {
            value: serde_json::Value::Null,
        },
    );

    // --- graceful shutdown ---------------------------------------------
    cancel.cancel();
    drop(central_rx);
    let _ = tokio::time::timeout(opts.shutdown_deadline, async {
        while source_joins.join_next().await.is_some() {}
    })
    .await;
    if !source_joins.is_empty() {
        source_joins.abort_all();
    }

    let deadline_at = tokio::time::Instant::now() + opts.shutdown_deadline;
    loop {
        if handle_joins.is_empty() {
            break;
        }
        match tokio::time::timeout_at(deadline_at, handle_joins.join_next()).await {
            Ok(Some(Ok(kind))) => report.bump_result(kind),
            Ok(Some(Err(_))) => report.bump_result("error"),
            Ok(None) => break,
            Err(_) => break,
        }
    }

    if !handle_joins.is_empty() {
        handle_joins.abort_all();
        while let Some(joined) = handle_joins.join_next().await {
            match joined {
                Ok(kind) => report.bump_result(kind),
                Err(_) => report.bump_result("error"),
            }
        }
    }

    Ok(report)
}

async fn dispatch_hitl_abort(
    ev: &ImEvent,
    bot: &BotRole,
    reason: String,
    active: &ActiveRunnerRegistry,
    journal: &Journal,
    report: &mut BridgeReport,
) {
    if let Some(guid) = active.lookup_by_chat_id(&ev.chat_id) {
        match active.send_signal(
            &guid,
            HitlSignal::Abort {
                reason: reason.clone(),
            },
        ) {
            Ok(()) => {
                report.hitl_abort_signaled = report.hitl_abort_signaled.saturating_add(1);
                let _ = write_journal(
                    journal,
                    "daemon:hitl_abort_dispatched",
                    json!({
                        "bot_app_id": bot.app_id,
                        "chat_id": ev.chat_id,
                        "task_guid": guid.as_str(),
                        "reason": reason,
                    }),
                    JournalResult::Ok {
                        value: serde_json::Value::Null,
                    },
                );
            }
            Err(e) => {
                report.hitl_signal_misses = report.hitl_signal_misses.saturating_add(1);
                let _ = write_journal(
                    journal,
                    "daemon:hitl_abort_miss",
                    json!({"bot_app_id": bot.app_id, "chat_id": ev.chat_id, "error": format!("{e}")}),
                    JournalResult::Err {
                        kind: "HitlSignalError".into(),
                        message: format!("{e}"),
                    },
                );
            }
        }
    } else {
        report.hitl_signal_misses = report.hitl_signal_misses.saturating_add(1);
        let _ = write_journal(
            journal,
            "daemon:hitl_abort_no_active",
            json!({"bot_app_id": bot.app_id, "chat_id": ev.chat_id}),
            JournalResult::Ok {
                value: serde_json::Value::Null,
            },
        );
    }
}

async fn dispatch_hitl_adjust(
    ev: &ImEvent,
    bot: &BotRole,
    body: String,
    active: &ActiveRunnerRegistry,
    journal: &Journal,
    report: &mut BridgeReport,
) {
    if let Some(guid) = active.lookup_by_chat_id(&ev.chat_id) {
        match active.send_signal(&guid, HitlSignal::Adjust { body: body.clone() }) {
            Ok(()) => {
                report.hitl_adjust_signaled = report.hitl_adjust_signaled.saturating_add(1);
                let _ = write_journal(
                    journal,
                    "daemon:hitl_adjust_dispatched",
                    json!({
                        "bot_app_id": bot.app_id,
                        "chat_id": ev.chat_id,
                        "task_guid": guid.as_str(),
                        "body_len": body.len(),
                    }),
                    JournalResult::Ok {
                        value: serde_json::Value::Null,
                    },
                );
            }
            Err(e) => {
                report.hitl_signal_misses = report.hitl_signal_misses.saturating_add(1);
                let _ = write_journal(
                    journal,
                    "daemon:hitl_adjust_miss",
                    json!({"bot_app_id": bot.app_id, "chat_id": ev.chat_id, "error": format!("{e}")}),
                    JournalResult::Err {
                        kind: "HitlSignalError".into(),
                        message: format!("{e}"),
                    },
                );
            }
        }
    } else {
        report.hitl_signal_misses = report.hitl_signal_misses.saturating_add(1);
        let _ = write_journal(
            journal,
            "daemon:hitl_adjust_no_active",
            json!({"bot_app_id": bot.app_id, "chat_id": ev.chat_id}),
            JournalResult::Ok {
                value: serde_json::Value::Null,
            },
        );
    }
}

fn action_kind(
    res: Result<crate::bot_bridge::runner::BotAction, HandleEventError>,
) -> &'static str {
    use crate::bot_bridge::runner::BotAction;
    match res {
        Ok(BotAction::Success { .. }) => "success",
        Ok(BotAction::Failed { .. }) => "failed",
        Ok(BotAction::Aborted { .. }) => "aborted",
        Ok(BotAction::Timeout { .. }) => "timeout",
        Ok(BotAction::Skipped { .. }) => "skipped",
        Err(_) => "error",
    }
}

fn shutdown_reason_str(r: ShutdownReason) -> &'static str {
    match r {
        ShutdownReason::CtrlC => "ctrl_c",
        ShutdownReason::MaxEvents => "max_events",
        ShutdownReason::MaxDuration => "max_duration",
        ShutdownReason::EventSourceClosed => "event_source_closed",
        ShutdownReason::NoBots => "no_bots",
    }
}

fn write_journal(
    journal: &Journal,
    action: &str,
    params: serde_json::Value,
    result: JournalResult,
) -> std::io::Result<PathBuf> {
    let mut entry = JournalEntry::new("bot_bridge:daemon", action);
    entry.params = params;
    entry.result = result;
    journal.append(&entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_token_cancelled_returns_promptly() {
        let tok = Arc::new(CancelToken::new());
        let tok2 = tok.clone();
        let h = tokio::spawn(async move {
            tok2.cancelled().await;
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        tok.cancel();
        let _ = tokio::time::timeout(Duration::from_millis(200), h)
            .await
            .expect("cancelled() returned promptly");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn run_bridge_with_empty_bots_yaml_returns_no_bots() {
        let g = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("ROOSTERY_HOME", tmp.path()) };

        let bots_path = tmp.path().join("bots.yaml");
        std::fs::write(&bots_path, "schema_version: 1\nbots: []\n").unwrap();
        let opts = BridgeOptions::default();
        let report = run_bridge(&bots_path, opts).await.expect("ok");
        assert_eq!(report.shutdown_reason, Some(ShutdownReason::NoBots));
        assert_eq!(report.bots_subscribed, 0);

        unsafe { std::env::remove_var("ROOSTERY_HOME") };
        drop(g);
    }
}
