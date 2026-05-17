//! End-to-end integration tests for the `roostery smoke` subcommand.

use serde_json::Value;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::Command;

fn roostery_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_roostery"))
}

fn fixture_script(dir: &std::path::Path, body: &str) -> PathBuf {
    let path = dir.join("fake-lark-cli");
    let mut content = String::from("#!/bin/sh\n");
    content.push_str(body);
    std::fs::write(&path, content).unwrap();
    let mut perm = std::fs::metadata(&path).unwrap().permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(&path, perm).unwrap();
    path
}

#[test]
fn smoke_all_ok_exits_zero() {
    let home = tempfile::tempdir().unwrap();
    let script_dir = tempfile::tempdir().unwrap();
    let script = fixture_script(script_dir.path(), r#"echo "=== Dry Run ==="; exit 0"#);

    let out = Command::new(roostery_exe())
        .arg("smoke")
        .env("ROOSTERY_HOME", home.path())
        .env("ROOSTERY_LARK_CLI_BIN", &script)
        .output()
        .expect("roostery spawn");

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}, stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let report: Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["all_ok"], true);
    assert_eq!(report["probes"].as_object().unwrap().len(), 6);

    let state_file = home.path().join("state").join("smoke.json");
    assert!(state_file.exists());
    let persisted: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    assert_eq!(persisted["all_ok"], true);
}

#[test]
fn smoke_partial_failure_exits_one() {
    let home = tempfile::tempdir().unwrap();
    let script_dir = tempfile::tempdir().unwrap();
    // Fail only when --user-id is in argv (im_messages_send probe);
    // every other probe outputs the marker and exits 0.
    let script = fixture_script(
        script_dir.path(),
        r#"
for arg in "$@"; do
  if [ "$arg" = "--user-id" ]; then
    echo "unknown flag: --user-id" >&2
    exit 2
  fi
done
echo "=== Dry Run ==="
exit 0
"#,
    );

    let out = Command::new(roostery_exe())
        .arg("smoke")
        .env("ROOSTERY_HOME", home.path())
        .env("ROOSTERY_LARK_CLI_BIN", &script)
        .output()
        .expect("roostery spawn");

    assert_eq!(out.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    assert_eq!(report["all_ok"], false);
    assert_eq!(report["probes"]["im_messages_send"]["ok"], false);
    assert!(
        report["probes"]["im_messages_send"]["reason"]
            .as_str()
            .unwrap()
            .contains("flag/command mismatch")
    );
}

#[test]
fn version_string_locked() {
    let out = Command::new(roostery_exe())
        .arg("--version")
        .output()
        .expect("roostery spawn");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "roostery 0.0.0 (rust)"
    );
}

#[test]
fn no_args_prints_welcome() {
    let out = Command::new(roostery_exe())
        .output()
        .expect("roostery spawn");
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("roostery 0.0.0 (rust)"));
    assert!(s.contains("https://github.com/bendusy/roostery"));
}
