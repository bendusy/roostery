//! End-to-end integration tests for `roostery bot {stop-hook, push}` CLI surface.
//!
//! Uses a fake lark-cli script (injected via `ROOSTERY_LARK_CLI_BIN`) to verify
//! the full binary path: arg parse → push core → IM fallback → JSON outcome →
//! exit code semantics under `--strict`.

use serde_json::Value;
use std::io::Write;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn roostery_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_roostery"))
}

/// Fake lark-cli script: dispatches by argv pattern.
/// - `task +create` → emits task JSON
/// - `task agent_task_step_info append_task_steps` → emits ok
/// - `im +messages-send` → emits IM JSON
/// - otherwise → exits 2 to signal unexpected invocation
fn fixture_lark_cli(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("fake-lark-cli");
    let body = r#"#!/bin/sh
case "$*" in
    *"+create"*)
        cat <<'EOF'
{"ok":true,"data":{"guid":"integ_task_1","url":"https://feishu.cn/task/integ_1"}}
EOF
        exit 0
        ;;
    *"append_task_steps"*)
        echo '{"ok":true}'
        exit 0
        ;;
    *"+messages-send"*)
        echo '{"ok":true,"data":{"message_id":"om_integ_1"}}'
        exit 0
        ;;
    *)
        echo "unexpected lark-cli invocation: $*" >&2
        exit 2
        ;;
esac
"#;
    std::fs::write(&path, body).unwrap();
    let mut perm = std::fs::metadata(&path).unwrap().permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(&path, perm).unwrap();
    path
}

/// Fake lark-cli that always fails (for testing --strict + Failed path).
fn fixture_lark_cli_fail(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("fake-lark-cli-fail");
    let body = r#"#!/bin/sh
echo "simulated lark-cli failure" >&2
exit 1
"#;
    std::fs::write(&path, body).unwrap();
    let mut perm = std::fs::metadata(&path).unwrap().permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(&path, perm).unwrap();
    path
}

#[test]
fn cli_push_flag_based_happy_outputs_json_outcome() {
    let home = tempfile::tempdir().unwrap();
    let script_dir = tempfile::tempdir().unwrap();
    let lark = fixture_lark_cli(script_dir.path());

    let out = Command::new(roostery_exe())
        .args([
            "bot",
            "push",
            "--agent",
            "ci",
            "--session",
            "build-42",
            "--cwd",
            "/tmp/integ",
            "--summary",
            "build green",
            "--assignee-open-id",
            "ou_integ",
            "--json",
            "--strict",
        ])
        .env("ROOSTERY_HOME", home.path())
        .env("ROOSTERY_LARK_CLI_BIN", &lark)
        .env_remove("ROOSTERY_NOTIFY_TO")
        .output()
        .expect("spawn roostery");

    assert!(
        out.status.success(),
        "expected exit 0; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let outcome: Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    assert_eq!(outcome["status"], "success");
    assert_eq!(outcome["task_url"], "https://feishu.cn/task/integ_1");
    assert_eq!(outcome["task_guid"], "integ_task_1");
    assert_eq!(outcome["fallback_used"], false);
}

#[test]
fn cli_push_summary_stdin_reads_and_pushes() {
    let home = tempfile::tempdir().unwrap();
    let script_dir = tempfile::tempdir().unwrap();
    let lark = fixture_lark_cli(script_dir.path());

    let mut child = Command::new(roostery_exe())
        .args([
            "bot",
            "push",
            "--agent",
            "gha-runner",
            "--session",
            "run-1",
            "--cwd",
            "/tmp/integ",
            "--summary-stdin",
            "--assignee-open-id",
            "ou_integ",
            "--json",
        ])
        .env("ROOSTERY_HOME", home.path())
        .env("ROOSTERY_LARK_CLI_BIN", &lark)
        .env_remove("ROOSTERY_NOTIFY_TO")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"tests passed, ready to merge")
        .unwrap();
    drop(child.stdin.take());

    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "expected exit 0; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let outcome: Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    assert_eq!(outcome["status"], "success");
}

#[test]
fn cli_stop_hook_stdin_json_routes_to_push() {
    let home = tempfile::tempdir().unwrap();
    let script_dir = tempfile::tempdir().unwrap();
    let lark = fixture_lark_cli(script_dir.path());

    // CC SessionEnd-style stdin
    let stdin_json = serde_json::json!({
        "cwd": "/tmp/integ",
        "session_id": "cc-sess-1",
        "prompt_response": "implementation done",
    });

    let mut child = Command::new(roostery_exe())
        .args(["bot", "stop-hook", "--json"])
        .env("ROOSTERY_HOME", home.path())
        .env("ROOSTERY_LARK_CLI_BIN", &lark)
        .env("ROOSTERY_AGENT", "cc")
        .env("ROOSTERY_NOTIFY_TO", "ou_integ")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin_json.to_string().as_bytes())
        .unwrap();
    drop(child.stdin.take());

    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "expected exit 0; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let outcome: Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    assert_eq!(outcome["status"], "success");
    assert_eq!(outcome["task_url"], "https://feishu.cn/task/integ_1");
}

#[test]
fn cli_push_strict_with_no_im_fallback_task_fail_exits_one() {
    let home = tempfile::tempdir().unwrap();
    let script_dir = tempfile::tempdir().unwrap();
    let lark = fixture_lark_cli_fail(script_dir.path());

    let out = Command::new(roostery_exe())
        .args([
            "bot",
            "push",
            "--agent",
            "ci",
            "--session",
            "run-fail-1",
            "--cwd",
            "/tmp/integ",
            "--summary",
            "build broken",
            "--assignee-open-id",
            "ou_integ",
            "--strict",
            "--no-im-fallback",
            "--json",
        ])
        .env("ROOSTERY_HOME", home.path())
        .env("ROOSTERY_LARK_CLI_BIN", &lark)
        .env_remove("ROOSTERY_NOTIFY_TO")
        .output()
        .expect("spawn");

    assert!(
        !out.status.success(),
        "expected exit !0; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1));
    let outcome: Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    assert_eq!(outcome["status"], "failed");
    assert_eq!(outcome["fallback_used"], false);
    assert!(
        !outcome["errors"].as_array().unwrap().is_empty(),
        "should record task_writer error"
    );
}
