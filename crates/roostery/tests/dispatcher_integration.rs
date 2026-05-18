//! Integration tests for the dispatcher main loop. Each test runs in an
//! isolated `ROOSTERY_HOME` (tempdir) so journal / budget state don't leak.
//!
//! Serialized via a module-local `Mutex` because they mutate process env.

#![allow(clippy::await_holding_lock)] // ENV_LOCK serializes ROOSTERY_HOME mutation (attention.md pattern)

use async_trait::async_trait;
use roostery::config::Config;
use roostery::dispatcher::hook_event::{HOOK_EVENT_SCHEMA_VERSION, HookEvent};
use roostery::dispatcher::rules;
use roostery::dispatcher::runners::{
    RunOutcome, Runner, RunnerError, RunnerRegistry, RunnerStatus,
};
use roostery::dispatcher::trace::TraceContext;
use roostery::dispatcher::{self, DispatchError, StepStatus};
use roostery::paths::TEST_ENV_LOCK as ENV_LOCK;
use serde_json::json;

fn isolate_home(tmp: &tempfile::TempDir) {
    unsafe { std::env::set_var("ROOSTERY_HOME", tmp.path()) };
}
fn restore_home() {
    unsafe { std::env::remove_var("ROOSTERY_HOME") };
}

fn build_event(hook_source: &str) -> HookEvent {
    let raw = json!({
        "schema_version": HOOK_EVENT_SCHEMA_VERSION,
        "hook_source": hook_source,
        "session_id": "sess_e2e",
        "workspace": "/tmp/integ",
        "trigger_meta": {},
    });
    serde_json::from_value(raw).unwrap()
}

fn build_cfg(max_depth: u32, max_calls: u32, max_cost: f64) -> Config {
    let mut c = Config::default();
    c.trace.max_depth = max_depth;
    c.budgets.default.max_calls = max_calls;
    c.budgets.default.max_cost_usd = max_cost;
    c
}

fn load_rules(yaml: &str) -> Vec<rules::CompiledRule> {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), yaml).unwrap();
    rules::load_from(tmp.path()).unwrap()
}

// --- Test runners ----------------------------------------------------------

struct ChainRunner {
    kind: &'static str,
    emit_count: usize,
    child_source: String,
}

#[async_trait]
impl Runner for ChainRunner {
    fn kind(&self) -> &'static str {
        self.kind
    }
    async fn run(
        &self,
        _: &HookEvent,
        _: &TraceContext,
        _: &serde_json::Value,
    ) -> Result<RunOutcome, RunnerError> {
        let emit: Vec<HookEvent> = (0..self.emit_count)
            .map(|_| build_event(&self.child_source))
            .collect();
        Ok(RunOutcome {
            status: RunnerStatus::Success,
            stdout: String::new(),
            stderr: String::new(),
            emitted_events: emit,
            cost_usd: Some(0.01),
        })
    }
}

// ---------------------------------------------------------------------------

#[tokio::test]
async fn fire_happy_with_noop_runner() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    isolate_home(&tmp);
    let cfg = build_cfg(8, 100, 10.0);
    let rules = load_rules(
        "schema_version: 1\nrules:\n  - name: r1\n    when: {hook_source: cc-stop}\n    action: {runner: noop, args: {}}\n",
    );
    let registry = RunnerRegistry::with_defaults();
    let outcome = dispatcher::fire(build_event("cc-stop"), &registry, &rules, &cfg).await;
    assert_eq!(outcome.dispatched.len(), 1);
    assert_eq!(outcome.dispatched[0].status, StepStatus::Success);
    restore_home();
}

#[tokio::test]
async fn fire_chain_two_layers_via_real_registry() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    isolate_home(&tmp);
    let cfg = build_cfg(8, 100, 10.0);
    let yaml = "schema_version: 1\nrules:\n  - name: r1\n    when: {hook_source: cc-stop}\n    action: {runner: chain, args: {}}\n  - name: r2\n    when: {hook_source: downstream}\n    action: {runner: noop, args: {}}\n";
    let rules = load_rules(yaml);
    let registry = RunnerRegistry::new()
        .with_runner(Box::new(ChainRunner {
            kind: "chain",
            emit_count: 2,
            child_source: "downstream".to_string(),
        }))
        .with_runner(Box::new(roostery::dispatcher::runners::NoopRunner));
    let outcome = dispatcher::fire(build_event("cc-stop"), &registry, &rules, &cfg).await;
    // 1 root + 2 children = 3
    assert_eq!(outcome.dispatched.len(), 3);
    assert_eq!(outcome.dispatched[0].depth, 0);
    assert_eq!(outcome.dispatched[0].fanout, 2);
    assert_eq!(outcome.dispatched[1].depth, 1);
    assert_eq!(outcome.dispatched[2].depth, 1);
    restore_home();
}

#[tokio::test]
async fn fire_over_budget_gate_rejected_after_n_calls() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    isolate_home(&tmp);
    // max_calls = 1：第一次成功，第二次 gate reject
    let cfg = build_cfg(8, 1, 100.0);
    let rules = load_rules(
        "schema_version: 1\nrules:\n  - name: r1\n    when: {hook_source: cc-stop}\n    action: {runner: noop, args: {}}\n",
    );
    let registry = RunnerRegistry::with_defaults();
    // 第一次 fire 成功
    let r1 = dispatcher::fire(build_event("cc-stop"), &registry, &rules, &cfg).await;
    assert_eq!(r1.dispatched[0].status, StepStatus::Success);
    // 第二次 fire 复用 persisted budget state，应 gate reject
    let r2 = dispatcher::fire(build_event("cc-stop"), &registry, &rules, &cfg).await;
    match &r2.dispatched[0].status {
        StepStatus::GateRejected { reason } => assert!(reason.contains("budget")),
        other => panic!("expected GateRejected, got {other:?}"),
    }
    restore_home();
}

#[tokio::test]
async fn fire_over_depth_gates_child_step() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    isolate_home(&tmp);
    let cfg = build_cfg(1, 100, 10.0);
    let yaml = "schema_version: 1\nrules:\n  - name: r1\n    when: {hook_source: cc-stop}\n    action: {runner: chain, args: {}}\n  - name: r2\n    when: {hook_source: downstream}\n    action: {runner: noop, args: {}}\n";
    let rules = load_rules(yaml);
    let registry = RunnerRegistry::new()
        .with_runner(Box::new(ChainRunner {
            kind: "chain",
            emit_count: 1,
            child_source: "downstream".to_string(),
        }))
        .with_runner(Box::new(roostery::dispatcher::runners::NoopRunner));
    let outcome = dispatcher::fire(build_event("cc-stop"), &registry, &rules, &cfg).await;
    assert_eq!(outcome.dispatched.len(), 2);
    assert_eq!(outcome.dispatched[0].status, StepStatus::Success);
    match &outcome.dispatched[1].status {
        StepStatus::GateRejected { reason } => assert!(reason.contains("depth")),
        other => panic!("expected GateRejected, got {other:?}"),
    }
    restore_home();
}

#[tokio::test]
async fn replay_roundtrip_creates_new_trace_id() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    isolate_home(&tmp);
    let cfg = build_cfg(8, 100, 10.0);
    let rules = load_rules(
        "schema_version: 1\nrules:\n  - name: r1\n    when: {hook_source: cc-stop}\n    action: {runner: noop, args: {}}\n",
    );
    let registry = RunnerRegistry::with_defaults();
    let first = dispatcher::fire(build_event("cc-stop"), &registry, &rules, &cfg).await;
    let orig = first.trace_id.as_str().to_string();
    let replayed = dispatcher::replay(&orig, &registry, &rules, &cfg)
        .await
        .unwrap();
    assert_eq!(replayed.dispatched[0].status, StepStatus::Success);
    assert_ne!(replayed.trace_id.as_str(), orig);
    restore_home();
}

#[tokio::test]
async fn replay_unknown_trace_returns_not_found() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    isolate_home(&tmp);
    std::fs::create_dir_all(roostery::paths::journal_dir()).unwrap();
    let cfg = build_cfg(8, 100, 10.0);
    let rules: Vec<rules::CompiledRule> = Vec::new();
    let registry = RunnerRegistry::with_defaults();
    match dispatcher::replay("ghost_trace", &registry, &rules, &cfg).await {
        Err(DispatchError::ReplayNotFound(tid)) => assert_eq!(tid, "ghost_trace"),
        other => panic!("expected ReplayNotFound, got {other:?}"),
    }
    restore_home();
}

#[tokio::test]
async fn test_rule_match_and_no_match() {
    let rules = load_rules(
        "schema_version: 1\nrules:\n  - name: r1\n    when: {hook_source: cc-stop}\n    action: {runner: noop, args: {}}\n",
    );
    let ev_hit = build_event("cc-stop");
    let hit = dispatcher::test_rule(&ev_hit, &rules);
    assert!(hit.is_some());
    let ev_miss = build_event("other");
    let miss = dispatcher::test_rule(&ev_miss, &rules);
    assert!(miss.is_none());
}
