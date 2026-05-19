//! `bot_bridge::runner` — handle_event 编排节点。
//!
//! 见 `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md`
//! §2.1 + §2.2（handle_event / BotAction / HandleEventError / ADJUST_MAX 循环）。
//!
//! 本模块是 `daemon` 主循环和 `dispatcher::runners::Runner` 之间的编排层：
//! - 抽 prompt → 查 runner_registry → 占位 record_start → register active → `tokio::select!`
//!   等 runner_result vs kill_signal → Adjust 走 ADJUST_MAX 限制的重启循环 → record_end
//! - 飞书 reply 由 daemon（step 7）按 BotAction 字段拼模板，本 step 不调 lark_cli reply
//!
//! relay_task 仍为占位（step 5 实装）；占位调用点签名已与目标实装一致。

use std::sync::Arc;

use chrono::Utc;
use serde_json::json;

use crate::bot_bridge::active_registry::{
    ActiveRunnerRegistry, HitlSignal, HitlSignalError, RunnerHandle,
};
use crate::bot_bridge::event::ImEvent;
use crate::bot_bridge::relay_task::{EndOutcome, RelayTaskError};
use crate::bot_bridge::role::{BotRole, extract_message_body};
use crate::bot_task_writer::TaskGuid;
use crate::dispatcher::hook_event::{HOOK_EVENT_SCHEMA_VERSION, HookEvent};
use crate::dispatcher::runners::{RunOutcome, RunnerError, RunnerRegistry, RunnerStatus};
use crate::dispatcher::trace::TraceContext;
use crate::journal::{Journal, JournalEntry, JournalResult};
use crate::lark_cli::LarkRunner;

/// `/adjust` 重启上限（design §1.3 D9 + design §2.1）。
///
/// Python parity；命中后再次 Adjust → 转 Aborted{reason="adjust attempts exhausted"}。
pub const ADJUST_MAX: u32 = 1;

/// handle_event 终态——daemon 据此拼飞书 reply / 聚合 BridgeReport。
///
/// 与 design §2.1 `BotAction { runner_outcome: EndOutcome }` 的"struct 形态"
/// 偏离：本 step 在 spec 指导下以 enum 表达 5 态（含 Skipped），让 daemon
/// 端 match 一次到位，避免 caller 还要再 match runner_outcome 子枚举。
/// 实际语义与 design 字段子集一致：除 `Skipped` 外，每条变体都对应原
/// `EndOutcome` 的一个变体。
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum BotAction {
    /// runner 正常跑完 = Success；含此次 /adjust 重启次数（0 = 没调整）。
    Success {
        bot_app_id: String,
        chat_id: String,
        source_message_id: String,
        result_text: String,
        adjust_attempts: u32,
    },
    /// runner 非 0 退出 / RunnerStatus::Failed。
    Failed {
        bot_app_id: String,
        chat_id: String,
        source_message_id: String,
        reason: String,
    },
    /// 用户 `/stop` 或 `/adjust` 触上限 → 中止。
    Aborted {
        bot_app_id: String,
        chat_id: String,
        source_message_id: String,
        reason: String,
    },
    /// runner 调用 timeout（来自 `RunnerError::Timeout`）。
    Timeout {
        bot_app_id: String,
        chat_id: String,
        source_message_id: String,
    },
    /// runner kind 在 registry 找不到（design §3 E3）；不调 runner.run。
    Skipped {
        bot_app_id: String,
        chat_id: String,
        source_message_id: String,
        reason: String,
    },
}

/// handle_event 错误（design §2.1 "典型"四变体，本 step 落 4 变体）。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HandleEventError {
    #[error("runner kind `{kind}` not registered")]
    RunnerKindNotRegistered { kind: String },
    #[error("runner spawn failed: {0}")]
    RunnerSpawnFailed(#[source] RunnerError),
    #[error("journal append failed: {0}")]
    JournalFailed(#[source] std::io::Error),
    #[error("active registry signal error: {0}")]
    ActiveRegistryFailed(#[source] HitlSignalError),
    #[error("relay_task error: {0}")]
    RelayTask(#[from] RelayTaskError),
}

/// 编排 IM event → runner 调用 → 终态映射的主流程。
///
/// design §2.2 主流程图：
/// 1. 抽 prompt（`extract_message_body`）
/// 2. 查 `runner_registry` 找 Runner trait 实例；未注册 → `BotAction::Skipped` + journal
/// 3. `record_start`（step 5 才真实写 task；本期占位）
/// 4. `active_registry.register` 含 oneshot kill_rx
/// 5. `tokio::select! { runner_future, kill_rx }`：
///    - runner 自然结束 → 看 `RunOutcome.status` 映射 Success / Failed
///    - kill = Abort → `BotAction::Aborted`
///    - kill = Adjust：attempts+1；attempts > ADJUST_MAX → `Aborted`，否则带新 prompt 重跑
/// 6. unregister + record_end + journal `event:handle_complete`
///
/// **生命周期边界**：调用方负责保证 `active_registry` / `runner_registry` 在
/// handle_event 协程存活期间不被 drop。
pub async fn handle_event(
    event: &ImEvent,
    bot: &BotRole,
    lark: &dyn LarkRunner,
    runner_registry: &RunnerRegistry,
    active: &Arc<ActiveRunnerRegistry>,
    journal: &Journal,
) -> Result<BotAction, HandleEventError> {
    // --- 1) 抽 prompt + 写 event:received journal --------------------
    let prompt_body = extract_message_body(event, bot).to_string();
    write_journal(
        journal,
        "event:received",
        json!({
            "bot_app_id": bot.app_id,
            "chat_id": event.chat_id,
            "message_id": event.message_id,
        }),
        JournalResult::Ok {
            value: serde_json::Value::Null,
        },
    )?;

    // --- 2) 查 runner kind --------------------------------------------
    let Some(runner) = runner_registry.find(&bot.runner) else {
        let action = BotAction::Skipped {
            bot_app_id: bot.app_id.clone(),
            chat_id: event.chat_id.clone(),
            source_message_id: event.message_id.clone(),
            reason: format!("unknown runner kind: {}", bot.runner),
        };
        write_journal(
            journal,
            "event:skipped",
            json!({
                "bot_app_id": bot.app_id,
                "chat_id": event.chat_id,
                "message_id": event.message_id,
                "runner_kind": bot.runner,
            }),
            JournalResult::Err {
                kind: "RunnerKindNotRegistered".to_string(),
                message: format!("unknown runner kind: {}", bot.runner),
            },
        )?;
        return Ok(action);
    };

    // --- 3) record_start（step 5 实装；本 step 占位返 None） ----------
    // 即便占位，调用点位 / 错误传播保留——step 5 替换不影响 runner.rs。
    let brief = take_head(&prompt_body, 80);
    let task_ref = crate::bot_bridge::relay_task::record_start(lark, bot, event, &brief).await?;
    let (task_guid, task_url) = match task_ref.as_ref() {
        Some(r) => (r.guid.clone(), r.url.clone()),
        // 占位路径：用一个稳定派生 guid，便于 active_registry 注册 + 测试断言。
        None => (
            TaskGuid::from_existing(format!("placeholder-{}", event.message_id)),
            String::new(),
        ),
    };

    // --- 4-5) Adjust 重启循环 ----------------------------------------
    let mut current_prompt = prompt_body.clone();
    let mut adjust_attempts: u32 = 0;
    let outcome: HandleOutcome = loop {
        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<HitlSignal>();
        active.register(RunnerHandle {
            kill_tx,
            task_guid: task_guid.clone(),
            task_url: task_url.clone(),
            chat_id: event.chat_id.clone(),
            started_at: Utc::now(),
        });

        // 调用 runner.run；HookEvent + TraceContext 是 trait 必填参数。
        // bot_bridge 没有上游 hook，因此从 ImEvent 合成最小 HookEvent；
        // TraceContext 走 new_root（IM 事件即 trace 起点）。
        let hook_event = synth_hook_event(event);
        let ctx = TraceContext::new_root(None, 8);
        let args = json!({ "prompt": current_prompt });

        let runner_future = runner.run(&hook_event, &ctx, &args);

        tokio::pin!(runner_future);
        let select_outcome = tokio::select! {
            biased;
            sig = kill_rx => sig,
            res = &mut runner_future => {
                // runner 自然完成；active_registry 把 handle 留在表里
                // （send_signal 是 remove-on-send，runner 自然完成路径需自行 unregister）。
                let _ = active.unregister(&task_guid);
                break map_runner_result(res, adjust_attempts);
            }
        };

        // 走到这里 = kill_rx 拿到信号；handle 已被 send_signal 内的 remove 摘掉。
        // runner_future 还在跑；我们 drop 它（select! 已 drop pin），让其自然结束被忽略。
        match select_outcome {
            Ok(HitlSignal::Abort { reason }) => {
                break HandleOutcome::Aborted { reason };
            }
            Ok(HitlSignal::Adjust { body }) => {
                adjust_attempts = adjust_attempts.saturating_add(1);
                if adjust_attempts > ADJUST_MAX {
                    break HandleOutcome::Aborted {
                        reason: "adjust attempts exhausted".to_string(),
                    };
                }
                write_journal(
                    journal,
                    "event:hitl_adjust",
                    json!({
                        "bot_app_id": bot.app_id,
                        "chat_id": event.chat_id,
                        "attempt": adjust_attempts,
                    }),
                    JournalResult::Ok {
                        value: serde_json::Value::Null,
                    },
                )?;
                // record_adjust 占位调用（step 5 实装真实 step 写入）
                if let Some(ref tr) = task_ref {
                    crate::bot_bridge::relay_task::record_adjust(
                        lark,
                        bot,
                        tr,
                        &body,
                        adjust_attempts,
                    )
                    .await?;
                }
                // 拼新 prompt = 原 prompt + adjust body，重启循环
                current_prompt = format!("{prompt_body}\n\n[ADJUST] {body}");
                continue;
            }
            Err(_recv_err) => {
                // oneshot 端意外 drop——异常情形，按 aborted 处理
                break HandleOutcome::Aborted {
                    reason: "kill channel closed unexpectedly".to_string(),
                };
            }
        }
    };

    // --- 6) record_end + 写 complete journal + 返 BotAction -----------
    let (action, end_outcome) = match outcome {
        HandleOutcome::Success {
            result_text,
            adjust_attempts: attempts,
        } => (
            BotAction::Success {
                bot_app_id: bot.app_id.clone(),
                chat_id: event.chat_id.clone(),
                source_message_id: event.message_id.clone(),
                result_text: result_text.clone(),
                adjust_attempts: attempts,
            },
            EndOutcome::Success {
                adjust_attempts: attempts,
            },
        ),
        HandleOutcome::Failed { reason, exit_code } => (
            BotAction::Failed {
                bot_app_id: bot.app_id.clone(),
                chat_id: event.chat_id.clone(),
                source_message_id: event.message_id.clone(),
                reason: reason.clone(),
            },
            EndOutcome::Failed { exit_code },
        ),
        HandleOutcome::Aborted { reason } => (
            BotAction::Aborted {
                bot_app_id: bot.app_id.clone(),
                chat_id: event.chat_id.clone(),
                source_message_id: event.message_id.clone(),
                reason: reason.clone(),
            },
            EndOutcome::Aborted { reason },
        ),
        HandleOutcome::Timeout => (
            BotAction::Timeout {
                bot_app_id: bot.app_id.clone(),
                chat_id: event.chat_id.clone(),
                source_message_id: event.message_id.clone(),
            },
            EndOutcome::Timeout,
        ),
    };

    let result_text = match &action {
        BotAction::Success { result_text, .. } => result_text.clone(),
        BotAction::Failed { reason, .. } | BotAction::Aborted { reason, .. } => reason.clone(),
        BotAction::Timeout { .. } => "timeout".to_string(),
        BotAction::Skipped { reason, .. } => reason.clone(),
    };
    crate::bot_bridge::relay_task::record_end(
        lark,
        bot,
        &event.chat_id,
        &event.message_id,
        &end_outcome,
        &result_text,
    )
    .await?;

    write_journal(
        journal,
        "event:handle_complete",
        json!({
            "bot_app_id": bot.app_id,
            "chat_id": event.chat_id,
            "message_id": event.message_id,
            "task_guid": task_guid.as_str(),
            "outcome": end_outcome_kind(&end_outcome),
        }),
        JournalResult::Ok {
            value: serde_json::Value::Null,
        },
    )?;

    Ok(action)
}

// --- internal helpers ------------------------------------------------------

/// runner 内部循环出口（含每变体所需字段）；不对外暴露。
enum HandleOutcome {
    Success {
        result_text: String,
        adjust_attempts: u32,
    },
    Failed {
        reason: String,
        exit_code: i32,
    },
    Aborted {
        reason: String,
    },
    Timeout,
}

fn map_runner_result(res: Result<RunOutcome, RunnerError>, adjust_attempts: u32) -> HandleOutcome {
    match res {
        Ok(outcome) => match outcome.status {
            RunnerStatus::Success => HandleOutcome::Success {
                result_text: outcome.stdout,
                adjust_attempts,
            },
            RunnerStatus::Failed { reason } => HandleOutcome::Failed {
                reason,
                exit_code: -1,
            },
            RunnerStatus::Skipped { reason } => HandleOutcome::Failed {
                reason: format!("runner skipped: {reason}"),
                exit_code: -1,
            },
        },
        Err(RunnerError::Timeout { .. }) => HandleOutcome::Timeout,
        Err(e) => HandleOutcome::Failed {
            reason: format!("runner error: {e}"),
            exit_code: -1,
        },
    }
}

fn end_outcome_kind(o: &EndOutcome) -> &'static str {
    match o {
        EndOutcome::Success { .. } => "success",
        EndOutcome::Failed { .. } => "failed",
        EndOutcome::Aborted { .. } => "aborted",
        EndOutcome::Timeout => "timeout",
    }
}

fn synth_hook_event(event: &ImEvent) -> HookEvent {
    HookEvent {
        schema_version: HOOK_EVENT_SCHEMA_VERSION,
        hook_source: "bot_bridge".to_string(),
        session_id: event.chat_id.clone(),
        workspace: std::path::PathBuf::from("/"),
        trigger_meta: json!({
            "chat_id": event.chat_id,
            "message_id": event.message_id,
            "sender_id": event.sender_id,
        }),
        trace: None,
    }
}

fn take_head(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn write_journal(
    journal: &Journal,
    action: &str,
    params: serde_json::Value,
    result: JournalResult,
) -> Result<(), HandleEventError> {
    let mut entry = JournalEntry::new("bot_bridge:handle_event", action);
    entry.params = params;
    entry.result = result;
    journal
        .append(&entry)
        .map(|_| ())
        .map_err(HandleEventError::JournalFailed)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot_bridge::role::BotRole;
    use crate::dispatcher::hook_event::HookEvent;
    use crate::dispatcher::runners::{RunOutcome, Runner, RunnerError, RunnerStatus};
    use crate::dispatcher::trace::TraceContext;
    use crate::lark_cli::mock::MockLarkRunner;
    use async_trait::async_trait;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    // --- TestRunner: 可控 Runner trait 实现（inline，不外溢） ---------------

    /// 触发哪种 outcome。`SuccessAfterDelay` 用于让外部 abort 信号先到。
    enum TestKind {
        Success {
            stdout: String,
        },
        Failed {
            reason: String,
        },
        Timeout,
        /// sleep then return Success；用于 abort / adjust 测试
        SuccessAfterDelay {
            delay: Duration,
            stdout: String,
        },
    }

    struct TestRunner {
        kind_str: &'static str,
        /// 多次 run 对应不同行为，按 invocation_count 取
        scripts: std::sync::Mutex<Vec<TestKind>>,
        invocations: AtomicU32,
    }

    impl TestRunner {
        fn new(kind_str: &'static str, scripts: Vec<TestKind>) -> Self {
            Self {
                kind_str,
                scripts: std::sync::Mutex::new(scripts),
                invocations: AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl Runner for TestRunner {
        fn kind(&self) -> &'static str {
            self.kind_str
        }

        async fn run(
            &self,
            _event: &HookEvent,
            _ctx: &TraceContext,
            _args: &serde_json::Value,
        ) -> Result<RunOutcome, RunnerError> {
            let _idx = self.invocations.fetch_add(1, Ordering::SeqCst);
            let script = {
                let mut scripts = self.scripts.lock().unwrap();
                if scripts.is_empty() {
                    // 退化为 Success（防测试脚本短了陷无限循环）
                    TestKind::Success {
                        stdout: "default".into(),
                    }
                } else {
                    scripts.remove(0)
                }
            };
            match script {
                TestKind::Success { stdout } => Ok(RunOutcome {
                    status: RunnerStatus::Success,
                    stdout,
                    stderr: String::new(),
                    emitted_events: Vec::new(),
                    cost_usd: None,
                }),
                TestKind::Failed { reason } => Ok(RunOutcome {
                    status: RunnerStatus::Failed { reason },
                    stdout: String::new(),
                    stderr: String::new(),
                    emitted_events: Vec::new(),
                    cost_usd: None,
                }),
                TestKind::Timeout => Err(RunnerError::Timeout {
                    kind: "test_runner",
                    timeout_ms: 100,
                }),
                TestKind::SuccessAfterDelay { delay, stdout } => {
                    tokio::time::sleep(delay).await;
                    Ok(RunOutcome {
                        status: RunnerStatus::Success,
                        stdout,
                        stderr: String::new(),
                        emitted_events: Vec::new(),
                        cost_usd: None,
                    })
                }
            }
        }
    }

    // --- fixtures ---------------------------------------------------------

    fn mk_event(chat: &str, msg_id: &str, content: &str) -> ImEvent {
        ImEvent {
            message_id: msg_id.into(),
            chat_id: chat.into(),
            chat_type: "group".into(),
            message_type: "text".into(),
            sender_id: "u_1".into(),
            content: content.into(),
        }
    }

    fn mk_bot(runner_kind: &str) -> BotRole {
        BotRole {
            app_id: "cli_app".into(),
            role: "scout".into(),
            mention_alias: "tl".into(),
            runner: runner_kind.into(),
            default_cwd: PathBuf::from("/tmp"),
            prompt_template: "{message}".into(),
            reply_template: "{result}".into(),
            chat_whitelist: vec![],
            next_bot_mention: String::new(),
        }
    }

    fn mk_journal() -> (Journal, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path());
        (j, dir)
    }

    fn registry_with(runner: TestRunner) -> RunnerRegistry {
        RunnerRegistry::new().with_runner(Box::new(runner))
    }

    /// 给 MockLarkRunner 灌满 relay_task 调用所需 N 次 lark 响应（identity::current
    /// 占 2 次：auth status + profile list；create_task 占 1 次；append_steps 各占 1 次）。
    /// `appends` 是预计的 append_steps 次数（start + 0..n adjust + end）。
    fn enqueue_relay_for(mock: &MockLarkRunner, appends: usize) {
        // identity::current（assignee None 路径走 auth status + profile list）
        mock.enqueue_ok(serde_json::json!({
            "userOpenId": "ou_user_123",
            "userName": "TestUser",
            "appId": "cli_app",
            "tokenStatus": "valid",
        }));
        mock.enqueue_ok(serde_json::json!([{"name": "default", "active": true}]));
        // create_task
        mock.enqueue_ok(serde_json::json!({
            "ok": true,
            "data": { "guid": "g_test", "url": "https://feishu.cn/task/test" }
        }));
        for _ in 0..appends {
            mock.enqueue_ok(serde_json::json!({"ok": true}));
        }
    }

    /// 让本模块的 test 都跑在隔离的 ROOSTERY_HOME 下，避免污染 ~/.roostery。
    /// 调用方需要持锁 ENV_LOCK 并保存 TempDir 直到测试结束。
    fn isolate_for_test() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
        let g = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("ROOSTERY_HOME", tmp.path()) };
        unsafe { std::env::set_var("ROOSTERY_HOST", "m4") };
        (tmp, g)
    }
    fn restore_for_test() {
        unsafe { std::env::remove_var("ROOSTERY_HOME") };
        unsafe { std::env::remove_var("ROOSTERY_HOST") };
    }

    // --- tests -----------------------------------------------------------

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn success_outcome_returns_bot_action_success() {
        let (_tmp, _g) = isolate_for_test();
        let runner = TestRunner::new(
            "test_runner",
            vec![TestKind::Success {
                stdout: "all done".into(),
            }],
        );
        let reg = registry_with(runner);
        let active = Arc::new(ActiveRunnerRegistry::new());
        let (journal, _td) = mk_journal();
        let mock = MockLarkRunner::new();
        enqueue_relay_for(&mock, 2); // start + end
        let bot = mk_bot("test_runner");
        let ev = mk_event("oc_x", "om_1", "@tl do it");

        let action = handle_event(&ev, &bot, &mock, &reg, &active, &journal)
            .await
            .expect("handle ok");
        match action {
            BotAction::Success {
                result_text,
                adjust_attempts,
                chat_id,
                ..
            } => {
                assert_eq!(result_text, "all done");
                assert_eq!(adjust_attempts, 0);
                assert_eq!(chat_id, "oc_x");
            }
            other => panic!("expected Success, got {other:?}"),
        }
        restore_for_test();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn failed_outcome_returns_bot_action_failed() {
        let (_tmp, _g) = isolate_for_test();
        let runner = TestRunner::new(
            "test_runner",
            vec![TestKind::Failed {
                reason: "exit 42".into(),
            }],
        );
        let reg = registry_with(runner);
        let active = Arc::new(ActiveRunnerRegistry::new());
        let (journal, _td) = mk_journal();
        let mock = MockLarkRunner::new();
        enqueue_relay_for(&mock, 2);
        let bot = mk_bot("test_runner");
        let ev = mk_event("oc_x", "om_2", "@tl do it");

        let action = handle_event(&ev, &bot, &mock, &reg, &active, &journal)
            .await
            .expect("handle ok");
        match action {
            BotAction::Failed { reason, .. } => {
                assert!(reason.contains("exit 42"), "got reason={reason}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        restore_for_test();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn timeout_outcome_returns_bot_action_timeout() {
        let (_tmp, _g) = isolate_for_test();
        let runner = TestRunner::new("test_runner", vec![TestKind::Timeout]);
        let reg = registry_with(runner);
        let active = Arc::new(ActiveRunnerRegistry::new());
        let (journal, _td) = mk_journal();
        let mock = MockLarkRunner::new();
        enqueue_relay_for(&mock, 2);
        let bot = mk_bot("test_runner");
        let ev = mk_event("oc_x", "om_3", "@tl do it");

        let action = handle_event(&ev, &bot, &mock, &reg, &active, &journal)
            .await
            .expect("handle ok");
        assert!(matches!(action, BotAction::Timeout { .. }));
        restore_for_test();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn unknown_runner_kind_returns_skipped_and_writes_journal() {
        let (_tmp, _g) = isolate_for_test();
        let reg = RunnerRegistry::new(); // 空 registry
        let active = Arc::new(ActiveRunnerRegistry::new());
        let (journal, td) = mk_journal();
        let mock = MockLarkRunner::new();
        let bot = mk_bot("does_not_exist");
        let ev = mk_event("oc_x", "om_4", "@tl do it");

        let action = handle_event(&ev, &bot, &mock, &reg, &active, &journal)
            .await
            .expect("handle ok");
        match action {
            BotAction::Skipped { reason, .. } => {
                assert!(reason.contains("does_not_exist"), "got reason={reason}");
            }
            other => panic!("expected Skipped, got {other:?}"),
        }

        // journal 落档：扫目录确认至少出现 event:skipped 字符串。
        let dir = td.path();
        let mut found = false;
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            let body = std::fs::read_to_string(&p).unwrap();
            if body.contains("event:skipped") && body.contains("does_not_exist") {
                found = true;
                break;
            }
        }
        assert!(found, "expected event:skipped journal entry");
        restore_for_test();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn abort_signal_returns_bot_action_aborted() {
        let (_tmp, _g) = isolate_for_test();
        // runner 跑得久；外部信号 Abort 触发后 select 命中 kill_rx
        let active = Arc::new(ActiveRunnerRegistry::new());
        let bot = mk_bot("test_runner");
        let ev = mk_event("oc_abort", "om_abort", "@tl do it");

        let active2 = active.clone();
        let bot_clone = bot.clone();
        let ev_clone = ev.clone();
        // spawn handle_event；主测试线程稍等触发 abort
        let handle = tokio::spawn(async move {
            let (j, _td) = mk_journal();
            let mock = MockLarkRunner::new();
            // Aborted 路径 = start + end → 2 appends
            enqueue_relay_for(&mock, 2);
            let runner_inner = TestRunner::new(
                "test_runner",
                vec![TestKind::SuccessAfterDelay {
                    delay: Duration::from_secs(5),
                    stdout: "should not see".into(),
                }],
            );
            let reg_inner = registry_with(runner_inner);
            handle_event(&ev_clone, &bot_clone, &mock, &reg_inner, &active2, &j).await
        });

        // 等待 handle_event 注册 active handle（轮询）
        let guid_opt = wait_for_chat(&active, "oc_abort", Duration::from_millis(500)).await;
        let guid = guid_opt.expect("active handle should be registered");
        active
            .send_signal(
                &guid,
                HitlSignal::Abort {
                    reason: "/stop".into(),
                },
            )
            .expect("send abort");

        let action = handle.await.expect("join").expect("handle_event ok");
        match action {
            BotAction::Aborted { reason, .. } => {
                assert!(reason.contains("/stop"), "got reason={reason}");
            }
            other => panic!("expected Aborted, got {other:?}"),
        }
        restore_for_test();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn adjust_within_limit_then_success() {
        let (_tmp, _g) = isolate_for_test();
        // 第一次 run = 长跑（被 /adjust 打断）；第二次 run = 立即 Success
        let runner = TestRunner::new(
            "test_runner",
            vec![
                TestKind::SuccessAfterDelay {
                    delay: Duration::from_secs(5),
                    stdout: "first".into(),
                },
                TestKind::Success {
                    stdout: "second-final".into(),
                },
            ],
        );
        let reg = Arc::new(registry_with(runner));
        let active = Arc::new(ActiveRunnerRegistry::new());

        let active2 = active.clone();
        let reg2 = reg.clone();
        let bot = mk_bot("test_runner");
        let ev = mk_event("oc_adj", "om_adj", "@tl do it");

        let handle = tokio::spawn(async move {
            let (j, _td) = mk_journal();
            let mock = MockLarkRunner::new();
            // start + 1 adjust + end = 3 appends
            enqueue_relay_for(&mock, 3);
            handle_event(&ev, &bot, &mock, &reg2, &active2, &j).await
        });

        // 等首次 register → 发 Adjust
        let guid = wait_for_chat(&active, "oc_adj", Duration::from_millis(500))
            .await
            .expect("first register");
        active
            .send_signal(
                &guid,
                HitlSignal::Adjust {
                    body: "use sqlite".into(),
                },
            )
            .expect("send adjust");

        // 第二次 run 立即 Success，handle_event 走到 record_end → 返
        let action = handle.await.expect("join").expect("handle_event ok");
        match action {
            BotAction::Success {
                result_text,
                adjust_attempts,
                ..
            } => {
                assert_eq!(result_text, "second-final");
                assert_eq!(adjust_attempts, 1);
            }
            other => panic!("expected Success after adjust, got {other:?}"),
        }
        restore_for_test();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn adjust_exceeding_limit_becomes_aborted() {
        let (_tmp, _g) = isolate_for_test();
        // 第一、二次都是长跑；连发两次 Adjust → 第二次超 ADJUST_MAX → Aborted
        let runner = TestRunner::new(
            "test_runner",
            vec![
                TestKind::SuccessAfterDelay {
                    delay: Duration::from_secs(5),
                    stdout: "first".into(),
                },
                TestKind::SuccessAfterDelay {
                    delay: Duration::from_secs(5),
                    stdout: "second".into(),
                },
            ],
        );
        let reg = Arc::new(registry_with(runner));
        let active = Arc::new(ActiveRunnerRegistry::new());

        let active2 = active.clone();
        let reg2 = reg.clone();
        let bot = mk_bot("test_runner");
        let ev = mk_event("oc_adj2", "om_adj2", "@tl do it");

        let handle = tokio::spawn(async move {
            let (j, _td) = mk_journal();
            let mock = MockLarkRunner::new();
            // start + 1 adjust + end = 3 appends（第 2 次 adjust 命中上限直接转 aborted，
            // 不再触发 record_adjust 的 append）
            enqueue_relay_for(&mock, 3);
            handle_event(&ev, &bot, &mock, &reg2, &active2, &j).await
        });

        // 第 1 次 Adjust（attempts 0→1，允许）
        let guid1 = wait_for_chat(&active, "oc_adj2", Duration::from_millis(500))
            .await
            .expect("first register");
        active
            .send_signal(
                &guid1,
                HitlSignal::Adjust {
                    body: "first adjust".into(),
                },
            )
            .expect("send adjust 1");

        // 第 2 次 Adjust（attempts 1→2，超 ADJUST_MAX=1 → Aborted）
        let guid2 = wait_for_chat(&active, "oc_adj2", Duration::from_millis(500))
            .await
            .expect("second register");
        active
            .send_signal(
                &guid2,
                HitlSignal::Adjust {
                    body: "second adjust".into(),
                },
            )
            .expect("send adjust 2");

        let action = handle.await.expect("join").expect("handle_event ok");
        match action {
            BotAction::Aborted { reason, .. } => {
                assert!(
                    reason.contains("adjust attempts exhausted"),
                    "got reason={reason}"
                );
            }
            other => panic!("expected Aborted, got {other:?}"),
        }
        restore_for_test();
    }

    /// 轮询 active_registry 直到出现指定 chat_id 的 handle 或超时。
    async fn wait_for_chat(
        active: &ActiveRunnerRegistry,
        chat_id: &str,
        timeout: Duration,
    ) -> Option<TaskGuid> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(g) = active.lookup_by_chat_id(chat_id) {
                return Some(g);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
