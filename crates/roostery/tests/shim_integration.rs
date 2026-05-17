//! End-to-end integration tests for the `shim` binary.
//!
//! Spawns the compiled shim with `ROOSTERY_REAL_LARK_CLI` pointing at a fixture
//! shell script and `ROOSTERY_HOME` pointing at a tempdir, then asserts both
//! exit code transparency and the on-disk JournalEntry shape.

use serde_json::Value;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::Command;

fn shim_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_shim"))
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

fn read_journal(home: &std::path::Path) -> Vec<Value> {
    let journal_dir = home.join("journal");
    let mut entries = Vec::new();
    let read = std::fs::read_dir(&journal_dir).expect("journal dir exists");
    for f in read {
        let f = f.unwrap();
        let body = std::fs::read_to_string(f.path()).unwrap();
        for line in body.lines() {
            if !line.is_empty() {
                entries.push(serde_json::from_str(line).unwrap());
            }
        }
    }
    entries
}

#[test]
fn non_interactive_writes_full_entry() {
    let home = tempfile::tempdir().unwrap();
    let script_dir = tempfile::tempdir().unwrap();
    let script = fixture_script(
        script_dir.path(),
        r#"echo '{"data":{"message_id":"om_int_abc"}}'"#,
    );

    let out = Command::new(shim_exe())
        .args(["im", "+messages-send"])
        .env("ROOSTERY_HOME", home.path())
        .env("ROOSTERY_REAL_LARK_CLI", &script)
        .env_remove("ROOSTERY_NOJOURNAL")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("shim spawn");

    assert!(
        out.status.success(),
        "shim should exit 0, got {:?}",
        out.status
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("om_int_abc"),
        "shim must tee stdout to user"
    );

    let entries = read_journal(home.path());
    assert_eq!(entries.len(), 1, "exactly one entry");
    let e = &entries[0];
    assert_eq!(e["source"], "shim");
    assert_eq!(e["action"], "lark-cli:im");
    assert_eq!(e["schema_version"], 1);
    assert_eq!(
        e["params"]["remote_refs"]["message_id"], "om_int_abc",
        "remote_refs.message_id extracted from stdout"
    );
    assert_eq!(e["result"]["outcome"], "ok");
}

#[test]
fn nojournal_writes_skipped_entry() {
    let home = tempfile::tempdir().unwrap();
    let script_dir = tempfile::tempdir().unwrap();
    let script = fixture_script(script_dir.path(), "echo hello && exit 0");

    let out = Command::new(shim_exe())
        .args(["docs", "+download"])
        .env("ROOSTERY_HOME", home.path())
        .env("ROOSTERY_REAL_LARK_CLI", &script)
        .env("ROOSTERY_NOJOURNAL", "1")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("shim spawn");

    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");

    let entries = read_journal(home.path());
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e["source"], "shim");
    assert_eq!(e["action"], "lark-cli:docs:skipped");
    assert_eq!(e["params"]["reason"], "nojournal");
    assert_eq!(e["duration_ms"], 0);
}

#[test]
fn missing_env_returns_127() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(shim_exe())
        .args(["any"])
        .env("ROOSTERY_HOME", home.path())
        .env_remove("ROOSTERY_REAL_LARK_CLI")
        .env_remove("ROOSTERY_NOJOURNAL")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("shim spawn");
    assert_eq!(out.status.code(), Some(127));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("ROOSTERY_REAL_LARK_CLI"),
        "stderr must mention env var"
    );
}

#[test]
fn exit_code_passthrough() {
    let home = tempfile::tempdir().unwrap();
    let script_dir = tempfile::tempdir().unwrap();
    let script = fixture_script(script_dir.path(), "exit 7");

    let out = Command::new(shim_exe())
        .args(["any"])
        .env("ROOSTERY_HOME", home.path())
        .env("ROOSTERY_REAL_LARK_CLI", &script)
        .env_remove("ROOSTERY_NOJOURNAL")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("shim spawn");
    assert_eq!(out.status.code(), Some(7));
    let entries = read_journal(home.path());
    assert_eq!(entries[0]["result"]["outcome"], "err");
    assert_eq!(entries[0]["result"]["kind"], "NonZeroExit");
}
