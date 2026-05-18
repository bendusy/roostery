//! Integration: RunnerRegistry::with_defaults + NoopRunner + CcHeadlessRunner
//! against fake `claude` shell scripts.

use roostery::hook_event::HookEvent;
use roostery::runners::{CcHeadlessRunner, NoopRunner, RunnerRegistry, RunnerStatus};
use roostery::trace::TraceContext;
use serde_json::json;
use std::path::{Path, PathBuf};

fn build_event() -> HookEvent {
    let raw = json!({
        "schema_version": 1,
        "hook_source": "claude-code-stop",
        "session_id": "sess_e2e",
        "workspace": "/tmp",
        "trigger_meta": {},
    });
    serde_json::from_value(raw).unwrap()
}

fn build_ctx() -> TraceContext {
    TraceContext::new_root(Some("evt_root".to_string()), 8)
}

fn write_fake_claude(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("fake_claude.sh");
    std::fs::write(&path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
    }
    path
}

#[tokio::test]
async fn registry_defaults_contains_noop_and_cc_headless() {
    let r = RunnerRegistry::with_defaults();
    assert_eq!(r.len(), 2);
    assert!(r.find("noop").is_some());
    assert!(r.find("cc_headless").is_some());
    assert!(r.find("codex_exec").is_none());
}

#[tokio::test]
async fn noop_runner_runs_to_success_via_registry() {
    let r = RunnerRegistry::with_defaults();
    let runner = r.find("noop").unwrap();
    let outcome = runner
        .run(&build_event(), &build_ctx(), &json!({}))
        .await
        .unwrap();
    assert_eq!(outcome.status, RunnerStatus::Success);
    assert!(outcome.stdout.is_empty());
    assert!(outcome.cost_usd.is_none());
}

#[tokio::test]
async fn cc_headless_happy_with_fake_binary_via_registry() {
    let tmp = tempfile::tempdir().unwrap();
    let body = r#"#!/bin/sh
cat <<EOF
{"cost_usd": 0.012, "result": "integ test reply"}
EOF
"#;
    let bin = write_fake_claude(tmp.path(), body);

    // Build a custom registry that injects the fake binary.
    let cc = CcHeadlessRunner {
        bin_override: Some(bin),
    };
    let registry = RunnerRegistry::new()
        .with_runner(Box::new(NoopRunner))
        .with_runner(Box::new(cc));

    let runner = registry.find("cc_headless").unwrap();
    let outcome = runner
        .run(&build_event(), &build_ctx(), &json!({"prompt": "integ"}))
        .await
        .unwrap();
    assert_eq!(outcome.status, RunnerStatus::Success);
    assert_eq!(outcome.cost_usd, Some(0.012));
    assert!(outcome.stdout.contains("integ test reply"));
}

#[tokio::test]
async fn cc_headless_failure_with_fake_binary_via_registry() {
    let tmp = tempfile::tempdir().unwrap();
    let body = "#!/bin/sh\necho boom >&2\nexit 7\n";
    let bin = write_fake_claude(tmp.path(), body);

    let cc = CcHeadlessRunner {
        bin_override: Some(bin),
    };
    let registry = RunnerRegistry::new().with_runner(Box::new(cc));
    let runner = registry.find("cc_headless").unwrap();
    let outcome = runner
        .run(&build_event(), &build_ctx(), &json!({"prompt": "integ"}))
        .await
        .unwrap();
    match outcome.status {
        RunnerStatus::Failed { reason } => assert!(reason.contains("7")),
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(outcome.stderr.contains("boom"));
}

#[tokio::test]
async fn cc_headless_passes_trace_env_to_subprocess() {
    // Fake claude prints the trace env vars it received, in JSON form.
    let tmp = tempfile::tempdir().unwrap();
    let body = r#"#!/bin/sh
cat <<EOF
{"cost_usd": 0.0, "result": "tid=$ROOSTERY_TRACE_ID depth=$ROOSTERY_DEPTH parent=$ROOSTERY_PARENT_EVENT_ID"}
EOF
"#;
    let bin = write_fake_claude(tmp.path(), body);

    let cc = CcHeadlessRunner {
        bin_override: Some(bin),
    };
    let registry = RunnerRegistry::new().with_runner(Box::new(cc));
    let runner = registry.find("cc_headless").unwrap();
    let ctx = TraceContext::new_root(Some("evt_e2e_trace".to_string()), 8);
    let outcome = runner
        .run(&build_event(), &ctx, &json!({"prompt": "hi"}))
        .await
        .unwrap();
    assert_eq!(outcome.status, RunnerStatus::Success);
    assert!(outcome.stdout.contains(ctx.trace_id.as_str()));
    assert!(outcome.stdout.contains("depth=0"));
    assert!(outcome.stdout.contains("parent=evt_e2e_trace"));
}
