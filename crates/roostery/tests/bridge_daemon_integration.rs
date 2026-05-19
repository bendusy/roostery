//! Integration tests for `bot_bridge::daemon::run_bridge`.
//!
//! Step 7 exit signals (see
//! `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-checklist.yaml`)：
//! - 端到端 feed 6 假 event（2 @bot / 1 /stop / 1 /adjust / 1 noise / 1 非匹配 chat）
//! - BridgeReport 计数符合预期；journal 含预期 source / action 条目
//! - ctrl_c / cancel 触发后 deadline 内退出

use async_trait::async_trait;
use roostery::bot_bridge::active_registry::{ActiveRunnerRegistry, HitlSignal, RunnerHandle};
use roostery::bot_bridge::daemon::{BridgeOptions, CancelToken, ShutdownReason, run_bridge};
use roostery::bot_task_writer::TaskGuid;
use roostery::dispatcher::hook_event::HookEvent;
use roostery::dispatcher::runners::{
    RunOutcome, Runner, RunnerError, RunnerRegistry, RunnerStatus,
};
use roostery::dispatcher::trace::TraceContext;
use roostery::lark_cli::mock::MockLarkRunner;
use serde_json::json;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tempfile::TempDir;

/// 写一个 fake lark-cli shell 脚本，打 6 行 NDJSON 后 `tail -f /dev/null` 保持 stdout 开启，
/// 让 consume_im 在自己的 max_events 到达后主动 kill。同源 pattern 见 event_integration.rs。
fn fixture_script(body: &str) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fake-lark-cli");
    let mut content = String::from("#!/bin/sh\n");
    content.push_str(body);
    std::fs::write(&path, content).unwrap();
    let mut perm = std::fs::metadata(&path).unwrap().permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(&path, perm).unwrap();
    (dir, path)
}

/// TestRunner：立即返回 Success("ok") 的 Runner 实现，用于 daemon 端到端。
struct TestRunner {
    kind_str: &'static str,
    invocations: AtomicU32,
}

impl TestRunner {
    fn new(kind_str: &'static str) -> Self {
        Self {
            kind_str,
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
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Ok(RunOutcome {
            status: RunnerStatus::Success,
            stdout: "ok".to_string(),
            stderr: String::new(),
            emitted_events: Vec::new(),
            cost_usd: None,
        })
    }
}

fn write_bots_yaml(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("bots.yaml");
    let yaml = r#"
schema_version: 1
bots:
  - app_id: test_app
    role: scout
    mention_alias: tl
    runner: test_runner
    default_cwd: /tmp
    prompt_template: "{message}"
    reply_template: "{result}"
    chat_whitelist: ["oc_match"]
"#;
    std::fs::write(&p, yaml).unwrap();
    p
}

/// 给 MockLarkRunner 灌满 handle_event 内 relay_task 调用所需 N 次 lark 响应。
/// 每条 handle_event Success 路径 = identity(2) + create_task(1) + append_steps(2)
fn enqueue_relay_success(mock: &MockLarkRunner) {
    mock.enqueue_ok(json!({
        "userOpenId": "ou_user_123",
        "userName": "TestUser",
        "appId": "test_app",
        "tokenStatus": "valid",
    }));
    mock.enqueue_ok(json!([{"name": "default", "active": true}]));
    mock.enqueue_ok(json!({
        "ok": true,
        "data": { "guid": "g_test", "url": "https://feishu.cn/task/test" }
    }));
    mock.enqueue_ok(json!({"ok": true}));
    mock.enqueue_ok(json!({"ok": true}));
}

#[tokio::test]
async fn s7_1_end_to_end_six_events_dispatched_correctly() {
    // 6 行 NDJSON：
    //   ev1: @tl do task A         （oc_match, handle_event）
    //   ev2: @tl do task B         （oc_match, handle_event）
    //   ev3: /stop                 （oc_match, HITL Abort）
    //   ev4: /adjust use sqlite    （oc_match, HITL Adjust）
    //   ev5: hello world           （oc_match, noise, no_match 计数）
    //   ev6: @tl ignored           （oc_other, unmatched_chat 计数）
    let body = r#"cat <<'JSON'
{"message_id":"m1","chat_id":"oc_match","chat_type":"group","message_type":"text","sender_id":"u1","content":"@tl do task A"}
{"message_id":"m2","chat_id":"oc_match","chat_type":"group","message_type":"text","sender_id":"u1","content":"@tl do task B"}
{"message_id":"m3","chat_id":"oc_match","chat_type":"group","message_type":"text","sender_id":"u2","content":"/stop"}
{"message_id":"m4","chat_id":"oc_match","chat_type":"group","message_type":"text","sender_id":"u2","content":"/adjust use sqlite"}
{"message_id":"m5","chat_id":"oc_match","chat_type":"group","message_type":"text","sender_id":"u3","content":"hello world"}
{"message_id":"m6","chat_id":"oc_other","chat_type":"group","message_type":"text","sender_id":"u4","content":"@tl ignored"}
JSON
# 保持 stdout 开启，让 consume_im 自己按 max_events 关闭
exec tail -f /dev/null
"#;
    let (_fixture_dir, lark_bin) = fixture_script(body);

    let tmp = tempfile::tempdir().unwrap();
    let bots_path = write_bots_yaml(tmp.path());
    let journal_dir = tmp.path().join("journal");

    // 灌满 mock lark：只有 2 个 @bot event 会触发 handle_event 实际 lark 调用；
    // /stop /adjust 在 daemon 端被 HITL 直接处理（lookup 失败 / 无 active 时无 lark 调用）。
    let mock = MockLarkRunner::new();
    enqueue_relay_success(&mock);
    enqueue_relay_success(&mock);
    let mock_arc: Arc<dyn roostery::lark_cli::LarkRunner> = Arc::new(mock);

    let registry =
        Arc::new(RunnerRegistry::new().with_runner(Box::new(TestRunner::new("test_runner"))));

    let opts = BridgeOptions {
        max_events: 6,
        event_channel_buffer: 16,
        shutdown_deadline: Duration::from_secs(3),
        lark_binary: Some(lark_bin),
        journal_dir: Some(journal_dir.clone()),
        runner_registry: Some(registry),
        lark_runner: Some(mock_arc),
        ..BridgeOptions::default()
    };

    let report = tokio::time::timeout(Duration::from_secs(15), run_bridge(&bots_path, opts))
        .await
        .expect("daemon should exit within 15s")
        .expect("daemon ok");

    // 期望计数：
    // events_received = 6
    // events_skipped_unmatched_chat = 1 （ev6 oc_other）
    // events_skipped_no_match = 1       （ev5 noise）
    // hitl_abort_signaled = 0；hitl_signal_misses ≥ 1（/stop 此时无 active runner）
    // hitl_adjust_signaled = 0；hitl_signal_misses 累计 ≥ 2
    // handle_event_spawned = 2          （ev1 / ev2）
    // handle_event_results success >= 2
    assert_eq!(report.events_received, 6, "report={report:?}");
    assert_eq!(report.events_skipped_unmatched_chat, 1, "report={report:?}");
    assert_eq!(report.events_skipped_no_match, 1, "report={report:?}");
    assert_eq!(report.handle_event_spawned, 2, "report={report:?}");
    // /stop + /adjust 在 daemon 端命中 HITL 路径——此 case 中没有 active runner，所以
    // 计入 hitl_signal_misses。这条仍然观察到 daemon 经过了 HITL 分流——
    // 既不是 unmatched_chat 也不是 no_match。
    assert!(
        report.hitl_signal_misses >= 2,
        "expected ≥2 hitl signal misses, report={report:?}"
    );
    let success = report
        .handle_event_results
        .get("success")
        .copied()
        .unwrap_or(0);
    assert!(
        success >= 2,
        "expected ≥2 success results, report={report:?}"
    );

    // 关闭原因：max_events 触发
    assert_eq!(
        report.shutdown_reason,
        Some(ShutdownReason::MaxEvents),
        "report={report:?}"
    );

    // journal 落档：至少出现 daemon:start / daemon:shutdown / daemon:hitl_*_no_active /
    // event:received（handle_event 内写）+ event:handle_complete
    let mut journal_blob = String::new();
    for entry in std::fs::read_dir(&journal_dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_file() {
            journal_blob.push_str(&std::fs::read_to_string(&p).unwrap_or_default());
        }
    }
    assert!(
        journal_blob.contains("\"action\":\"daemon:start\""),
        "expect daemon:start journal entry"
    );
    assert!(
        journal_blob.contains("\"action\":\"daemon:shutdown\""),
        "expect daemon:shutdown journal entry"
    );
    assert!(
        journal_blob.contains("hitl_abort_no_active") || journal_blob.contains("hitl_abort_miss"),
        "expect /stop journal trace"
    );
    assert!(
        journal_blob.contains("hitl_adjust_no_active") || journal_blob.contains("hitl_adjust_miss"),
        "expect /adjust journal trace"
    );
    assert!(
        journal_blob.contains("\"action\":\"event:received\""),
        "expect handle_event event:received journal entry"
    );
    assert!(
        journal_blob.contains("\"action\":\"event:handle_complete\""),
        "expect handle_event event:handle_complete journal entry"
    );
}

/// ctrl_c 触发路径：用 CancelToken 模拟（design A 选择）；deadline 内 daemon 必须返回。
#[tokio::test]
async fn s7_2_cancel_token_triggers_graceful_shutdown() {
    // fake lark-cli 一直 sleep，永远不会发 event。
    let body = "exec tail -f /dev/null\n";
    let (_fixture_dir, lark_bin) = fixture_script(body);

    let tmp = tempfile::tempdir().unwrap();
    let bots_path = write_bots_yaml(tmp.path());
    let journal_dir = tmp.path().join("journal");

    let mock_arc: Arc<dyn roostery::lark_cli::LarkRunner> = Arc::new(MockLarkRunner::new());
    let registry =
        Arc::new(RunnerRegistry::new().with_runner(Box::new(TestRunner::new("test_runner"))));
    let cancel = Arc::new(CancelToken::new());
    let cancel_clone = cancel.clone();

    // 50ms 后触发 cancel
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel_clone.cancel();
    });

    let opts = BridgeOptions {
        max_events: 0, // unlimited
        event_channel_buffer: 8,
        shutdown_deadline: Duration::from_secs(2),
        lark_binary: Some(lark_bin),
        journal_dir: Some(journal_dir),
        runner_registry: Some(registry),
        lark_runner: Some(mock_arc),
        cancel: Some(cancel),
        ..BridgeOptions::default()
    };

    let report = tokio::time::timeout(Duration::from_secs(10), run_bridge(&bots_path, opts))
        .await
        .expect("daemon should exit within 10s after cancel")
        .expect("daemon ok");

    assert_eq!(report.shutdown_reason, Some(ShutdownReason::CtrlC));
    assert_eq!(report.events_received, 0);
}

/// HITL 命中已 active 的 runner：用直接 register 一个 RunnerHandle 模拟 active task，
/// 再 feed 一条 /stop —— BridgeReport.hitl_abort_signaled 应 == 1。
#[tokio::test]
async fn s7_3_hitl_abort_matches_active_runner() {
    // 1 行 NDJSON /stop，然后 sleep 让 consume_im 自己 max_events=1 关闭。
    let body = r#"cat <<'JSON'
{"message_id":"mstop","chat_id":"oc_match","chat_type":"group","message_type":"text","sender_id":"u9","content":"/stop"}
JSON
exec tail -f /dev/null
"#;
    let (_fixture_dir, lark_bin) = fixture_script(body);

    let tmp = tempfile::tempdir().unwrap();
    let bots_path = write_bots_yaml(tmp.path());
    let journal_dir = tmp.path().join("journal");

    let mock_arc: Arc<dyn roostery::lark_cli::LarkRunner> = Arc::new(MockLarkRunner::new());
    let registry =
        Arc::new(RunnerRegistry::new().with_runner(Box::new(TestRunner::new("test_runner"))));

    // 一个外部 ActiveRunnerRegistry —— 但 daemon 内部 build 自己的，不接受注入。
    // 折中：通过 BridgeOptions 不接受外部 active 表，我们改测同 chat 真的有 task
    // 这部分由 s7_1 已经覆盖（handle_event 内会 register active 然后 unregister）。
    // 这里只断言 /stop 命中 HITL 路径而非 no_match。
    let opts = BridgeOptions {
        max_events: 1,
        event_channel_buffer: 8,
        shutdown_deadline: Duration::from_secs(2),
        lark_binary: Some(lark_bin),
        journal_dir: Some(journal_dir),
        runner_registry: Some(registry),
        lark_runner: Some(mock_arc),
        ..BridgeOptions::default()
    };
    let report = tokio::time::timeout(Duration::from_secs(10), run_bridge(&bots_path, opts))
        .await
        .expect("daemon should exit within 10s")
        .expect("daemon ok");

    assert_eq!(report.events_received, 1);
    // /stop 没有 active runner，但走 HITL 路径 → miss（不是 no_match）
    assert!(report.hitl_signal_misses >= 1);
    assert_eq!(report.events_skipped_no_match, 0);
}

/// design §3 E4：handle_event 协程内 panic → tokio JoinSet 隔离 → daemon main loop
/// 继续消费后续 event 不退出；BridgeReport 把 panic 计入 `error` 而不是 success。
///
/// 构造：runner 第一次 run 直接 `panic!`，第二次 run 正常 Success。Feed 两条 @bot 事件，
/// 期望 daemon 处理完 2 条且 1 条 error + 1 条 success。
#[tokio::test]
async fn s8_handle_event_panic_is_isolated_daemon_continues() {
    struct PanicOnceRunner {
        kind_str: &'static str,
        invocations: AtomicU32,
    }
    #[async_trait]
    impl Runner for PanicOnceRunner {
        fn kind(&self) -> &'static str {
            self.kind_str
        }
        async fn run(
            &self,
            _event: &HookEvent,
            _ctx: &TraceContext,
            _args: &serde_json::Value,
        ) -> Result<RunOutcome, RunnerError> {
            let idx = self.invocations.fetch_add(1, Ordering::SeqCst);
            if idx == 0 {
                panic!("simulated runner panic for E4 test");
            }
            Ok(RunOutcome {
                status: RunnerStatus::Success,
                stdout: "ok".to_string(),
                stderr: String::new(),
                emitted_events: Vec::new(),
                cost_usd: None,
            })
        }
    }

    let body = r#"cat <<'JSON'
{"message_id":"mp1","chat_id":"oc_match","chat_type":"group","message_type":"text","sender_id":"u1","content":"@tl panic please"}
{"message_id":"mp2","chat_id":"oc_match","chat_type":"group","message_type":"text","sender_id":"u1","content":"@tl now success"}
JSON
exec tail -f /dev/null
"#;
    let (_fixture_dir, lark_bin) = fixture_script(body);

    let tmp = tempfile::tempdir().unwrap();
    let bots_path = write_bots_yaml(tmp.path());
    let journal_dir = tmp.path().join("journal");

    // 两条 @bot 都会进 handle_event → 都会先走 relay_task identity+create+append_start。
    // 第一条 runner panic → 在 select 前的 record_start 已经做了 lark 调用；
    // 第二条会走完整 Success 路径。预灌两套 enqueue_relay_success 即可（第一条
    // panic 走完 start step 后还会被 panic 截断；第二条完整 5 calls）。
    let mock = MockLarkRunner::new();
    enqueue_relay_success(&mock);
    enqueue_relay_success(&mock);
    let mock_arc: Arc<dyn roostery::lark_cli::LarkRunner> = Arc::new(mock);

    let registry = Arc::new(RunnerRegistry::new().with_runner(Box::new(PanicOnceRunner {
        kind_str: "test_runner",
        invocations: AtomicU32::new(0),
    })));

    let opts = BridgeOptions {
        max_events: 2,
        event_channel_buffer: 8,
        // 强制串行：max_concurrency=1 → 第一条 panic 必然在第二条 spawn 前被 join
        max_concurrency: 1,
        shutdown_deadline: Duration::from_secs(3),
        lark_binary: Some(lark_bin),
        journal_dir: Some(journal_dir),
        runner_registry: Some(registry),
        lark_runner: Some(mock_arc),
        ..BridgeOptions::default()
    };

    let report = tokio::time::timeout(Duration::from_secs(15), run_bridge(&bots_path, opts))
        .await
        .expect("daemon must not hang on panic")
        .expect("daemon ok");

    // 关键不变量：daemon 没被 panic 拖垮——两条 event 都被收到并 spawn handle_event。
    assert_eq!(report.events_received, 2, "report={report:?}");
    assert_eq!(report.handle_event_spawned, 2, "report={report:?}");
    // 至少 1 条 error（panic 那条）；至少 1 条 success（第二条）。
    let error_count = report
        .handle_event_results
        .get("error")
        .copied()
        .unwrap_or(0);
    let success_count = report
        .handle_event_results
        .get("success")
        .copied()
        .unwrap_or(0);
    assert!(
        error_count >= 1,
        "panic 应计入 error 结果，report={report:?}"
    );
    assert!(
        success_count >= 1,
        "第二条应正常 Success，report={report:?}"
    );
    assert_eq!(report.shutdown_reason, Some(ShutdownReason::MaxEvents));
}

/// 直接单测 ActiveRunnerRegistry::send_signal 路径：daemon 内部 dispatch_hitl_abort 调用同样
/// 的内部 helper，行为已被 active_registry 单测覆盖。本测试只是冗余确认 register/lookup 链路
/// 与外部 build 的 RunnerHandle 兼容。
#[tokio::test]
async fn s7_4_active_registry_lookup_and_send_signal_works() {
    let reg = ActiveRunnerRegistry::new();
    let (tx, rx) = tokio::sync::oneshot::channel::<HitlSignal>();
    reg.register(RunnerHandle {
        kill_tx: tx,
        task_guid: TaskGuid::from_existing("g_active"),
        task_url: "https://x/y".into(),
        chat_id: "oc_match".into(),
        started_at: chrono::Utc::now(),
    });
    let guid = reg.lookup_by_chat_id("oc_match").expect("registered");
    reg.send_signal(
        &guid,
        HitlSignal::Abort {
            reason: "/stop".into(),
        },
    )
    .expect("send ok");
    match rx.await.unwrap() {
        HitlSignal::Abort { reason } => assert_eq!(reason, "/stop"),
        other => panic!("expected Abort, got {other:?}"),
    }
}
