//! `lark-cli` upgrade-compatibility smoke probe matrix.
//!
//! Runs 6 `lark-cli {sub} ... --dry-run` probes (im / docs / drive) sequentially,
//! classifies each result (`Dry Run` marker + rc==0 = ok; "unknown flag/command"
//! = mismatch; everything else = unexpected), writes a JSON snapshot to
//! `~/.roostery/state/smoke.json` (atomic `.tmp` + rename), and exposes
//! [`ensure_ready`] as a gate for downstream `roostery init` / `daily_report`.
//!
//! See `.codestable/features/2026-05-17-roostery-smoke/roostery-smoke-design.md`.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use thiserror::Error;

const ENV_BIN: &str = "ROOSTERY_LARK_CLI_BIN";
const DEFAULT_BIN: &str = "lark-cli";
const PROBE_TIMEOUT_SECS: u64 = 10;
const HEAD_BYTES: usize = 500;
const DRY_RUN_MARKER: &str = "Dry Run";

struct Probe {
    name: &'static str,
    argv: &'static [&'static str],
}

const PROBE_MATRIX: &[Probe] = &[
    Probe {
        name: "im_messages_send",
        argv: &[
            "im",
            "+messages-send",
            "--user-id",
            "ou_smoke",
            "--text",
            "probe",
            "--dry-run",
        ],
    },
    Probe {
        name: "docs_create_v2",
        argv: &[
            "docs",
            "+create",
            "--api-version",
            "v2",
            "--folder-token",
            "fld_smoke",
            "--content",
            "# probe",
            "--doc-format",
            "markdown",
            "--dry-run",
        ],
    },
    Probe {
        name: "docs_update_overwrite",
        argv: &[
            "docs",
            "+update",
            "--doc",
            "doc_smoke",
            "--mode",
            "overwrite",
            "--markdown",
            "# probe",
            "--new-title",
            "smoke",
            "--dry-run",
        ],
    },
    Probe {
        name: "drive_files_list",
        argv: &[
            "drive",
            "files",
            "list",
            "--params",
            r#"{"folder_token":"fld_smoke","page_size":5}"#,
            "--as",
            "user",
            "--dry-run",
        ],
    },
    Probe {
        name: "drive_create_folder",
        argv: &[
            "drive",
            "+create-folder",
            "--folder-token",
            "fld_smoke",
            "--name",
            "smoke",
            "--as",
            "user",
            "--dry-run",
        ],
    },
    Probe {
        name: "drive_move",
        argv: &[
            "drive",
            "+move",
            "--file-token",
            "doc_smoke",
            "--folder-token",
            "fld_smoke",
            "--type",
            "docx",
            "--as",
            "user",
            "--dry-run",
        ],
    },
];

/// Single probe result; serialized into `SmokeReport.probes`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProbeResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rc: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Full smoke run report; 1:1 with `~/.roostery/state/smoke.json`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SmokeReport {
    pub schema_version: u32,
    pub binary: String,
    pub lark_cli_version: Option<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub all_ok: bool,
    pub probes: BTreeMap<String, ProbeResult>,
}

/// Caller-facing failures for [`ensure_ready`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SmokeError {
    #[error("smoke probe never run; execute `roostery smoke` first")]
    NeverRun,
    #[error(
        "smoke probe last run reported failures: {failed_probes:?}; re-run `roostery smoke` after fixing"
    )]
    LastFailed { failed_probes: Vec<String> },
    #[error("smoke state file load failed: {source}")]
    StateLoadFailed {
        #[from]
        source: std::io::Error,
    },
    #[error("lark-cli binary not found: {path:?}")]
    BinaryNotFound { path: PathBuf },
}

/// Run a single probe with timeout; classify result.
fn probe_one(binary: &str, argv: &[&str], timeout: std::time::Duration) -> ProbeResult {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let mut child = match Command::new(binary)
        .args(argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ProbeResult {
                ok: false,
                rc: None,
                head: None,
                reason: Some(format!("binary not found: {binary}")),
            };
        }
        Err(e) => {
            return ProbeResult {
                ok: false,
                rc: None,
                head: None,
                reason: Some(format!("spawn failed: {e}")),
            };
        }
    };

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return ProbeResult {
                        ok: false,
                        rc: None,
                        head: None,
                        reason: Some(format!("timeout after {}s", timeout.as_secs())),
                    };
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                return ProbeResult {
                    ok: false,
                    rc: None,
                    head: None,
                    reason: Some(format!("wait failed: {e}")),
                };
            }
        }
    };

    let mut stdout_buf = Vec::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_end(&mut stdout_buf);
    }
    let mut stderr_buf = Vec::new();
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_end(&mut stderr_buf);
    }
    let combined =
        String::from_utf8_lossy(&stdout_buf).into_owned() + &String::from_utf8_lossy(&stderr_buf);
    let head: String = combined.chars().take(HEAD_BYTES).collect();
    let rc = status.code().unwrap_or(-1);

    if rc == 0 && combined.contains(DRY_RUN_MARKER) {
        ProbeResult {
            ok: true,
            rc: Some(0),
            head: Some(head),
            reason: None,
        }
    } else {
        let lower = combined.to_lowercase();
        let reason = if lower.contains("unknown flag") || lower.contains("unknown command") {
            "flag/command mismatch (lark-cli upgrade?)".to_string()
        } else {
            format!("unexpected exit {rc} or missing Dry Run marker")
        };
        ProbeResult {
            ok: false,
            rc: Some(rc),
            head: Some(head),
            reason: Some(reason),
        }
    }
}

fn resolve_binary() -> String {
    std::env::var_os(ENV_BIN)
        .filter(|v| !v.is_empty())
        .and_then(|v| v.into_string().ok())
        .unwrap_or_else(|| DEFAULT_BIN.to_string())
}

fn fetch_lark_cli_version(binary: &str) -> Option<String> {
    use std::process::{Command, Stdio};
    let output = Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout);
    s.lines().next().map(|l| l.trim().to_string())
}

fn save_report(path: &std::path::Path, report: &SmokeReport) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let buf = serde_json::to_vec_pretty(report).expect("SmokeReport serializes");
    std::fs::write(&tmp, &buf)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn load_last(path: &std::path::Path) -> Result<SmokeReport, SmokeError> {
    let bytes = std::fs::read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SmokeError::NeverRun
        } else {
            SmokeError::StateLoadFailed { source: e }
        }
    })?;
    serde_json::from_slice(&bytes).map_err(|e| SmokeError::StateLoadFailed {
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, e),
    })
}

/// Run the full PROBE_MATRIX and persist the resulting report.
pub fn run() -> SmokeReport {
    let binary = resolve_binary();
    let lark_cli_version = fetch_lark_cli_version(&binary);
    let started_at = chrono::Utc::now();
    let timeout = std::time::Duration::from_secs(PROBE_TIMEOUT_SECS);

    let mut probes = BTreeMap::new();
    for probe in PROBE_MATRIX {
        let result = probe_one(&binary, probe.argv, timeout);
        probes.insert(probe.name.to_string(), result);
    }
    let all_ok = probes.values().all(|p| p.ok);

    let report = SmokeReport {
        schema_version: crate::SCHEMA_VERSION,
        binary,
        lark_cli_version,
        started_at,
        all_ok,
        probes,
    };

    if let Err(e) = save_report(&paths::smoke_state_path(), &report) {
        tracing::warn!(error = %e, "smoke: state file save failed");
    }

    report
}

/// Gate API for downstream features (`roostery init`, `daily_report`).
pub fn ensure_ready() -> Result<(), SmokeError> {
    let report = load_last(&paths::smoke_state_path())?;
    if report.all_ok {
        return Ok(());
    }
    let failed_probes = report
        .probes
        .into_iter()
        .filter(|(_, r)| !r.ok)
        .map(|(name, _)| name)
        .collect();
    Err(SmokeError::LastFailed { failed_probes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_matrix_has_six_entries() {
        assert_eq!(PROBE_MATRIX.len(), 6);
    }

    #[test]
    fn probe_matrix_names_match_python_parity() {
        let names: Vec<&str> = PROBE_MATRIX.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            vec![
                "im_messages_send",
                "docs_create_v2",
                "docs_update_overwrite",
                "drive_files_list",
                "drive_create_folder",
                "drive_move",
            ]
        );
    }

    #[test]
    fn smoke_report_round_trip() {
        let mut probes = BTreeMap::new();
        probes.insert(
            "im_messages_send".into(),
            ProbeResult {
                ok: true,
                rc: Some(0),
                head: Some("=== Dry Run ===".into()),
                reason: None,
            },
        );
        let original = SmokeReport {
            schema_version: 1,
            binary: "/usr/local/bin/lark-cli".into(),
            lark_cli_version: Some("1.0.29".into()),
            started_at: chrono::Utc::now(),
            all_ok: true,
            probes,
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: SmokeReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn probe_result_optional_fields_skipped_when_none() {
        let r = ProbeResult {
            ok: true,
            rc: None,
            head: None,
            reason: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#"{"ok":true}"#);
    }

    fn fixture_script(body: &str) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt as _;
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

    #[test]
    fn probe_one_happy() {
        let (_d, p) = fixture_script(r#"echo "=== Dry Run ==="; exit 0"#);
        let r = probe_one(
            p.to_str().unwrap(),
            &["any"],
            std::time::Duration::from_secs(5),
        );
        assert!(r.ok);
        assert_eq!(r.rc, Some(0));
        assert!(r.reason.is_none());
        assert!(r.head.unwrap().contains("Dry Run"));
    }

    #[test]
    fn probe_one_unknown_flag() {
        let (_d, p) = fixture_script(r#"echo "unknown flag: --foo" >&2; exit 2"#);
        let r = probe_one(
            p.to_str().unwrap(),
            &["any"],
            std::time::Duration::from_secs(5),
        );
        assert!(!r.ok);
        assert_eq!(r.rc, Some(2));
        assert!(r.reason.unwrap().contains("flag/command mismatch"));
    }

    #[test]
    fn probe_one_unexpected_exit() {
        let (_d, p) = fixture_script("echo nothing; exit 5");
        let r = probe_one(
            p.to_str().unwrap(),
            &["any"],
            std::time::Duration::from_secs(5),
        );
        assert!(!r.ok);
        assert_eq!(r.rc, Some(5));
        assert!(r.reason.unwrap().contains("unexpected exit 5"));
    }

    #[test]
    fn probe_one_timeout() {
        let (_d, p) = fixture_script("sleep 30");
        let r = probe_one(
            p.to_str().unwrap(),
            &["any"],
            std::time::Duration::from_millis(200),
        );
        assert!(!r.ok);
        assert!(r.reason.unwrap().contains("timeout"));
    }

    #[test]
    fn probe_one_binary_not_found() {
        let r = probe_one(
            "/definitely/not/here/lark-cli",
            &["any"],
            std::time::Duration::from_secs(5),
        );
        assert!(!r.ok);
        assert!(r.reason.unwrap().contains("binary not found"));
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("smoke.json");
        let mut probes = BTreeMap::new();
        probes.insert(
            "x".into(),
            ProbeResult {
                ok: true,
                rc: Some(0),
                head: None,
                reason: None,
            },
        );
        let original = SmokeReport {
            schema_version: 1,
            binary: "/bin/sh".into(),
            lark_cli_version: Some("1.0.29".into()),
            started_at: chrono::Utc::now(),
            all_ok: true,
            probes,
        };
        save_report(&path, &original).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists(), "tmp cleaned up");
        let loaded = load_last(&path).unwrap();
        assert_eq!(loaded.binary, original.binary);
        assert!(loaded.all_ok);
    }

    #[test]
    fn load_last_missing_returns_never_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("smoke.json");
        match load_last(&path) {
            Err(SmokeError::NeverRun) => {}
            other => panic!("expected NeverRun, got {other:?}"),
        }
    }

    #[test]
    fn load_last_bad_json_returns_state_load_failed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("smoke.json");
        std::fs::write(&path, b"not json").unwrap();
        match load_last(&path) {
            Err(SmokeError::StateLoadFailed { .. }) => {}
            other => panic!("expected StateLoadFailed, got {other:?}"),
        }
    }

    #[test]
    fn run_with_fake_binary_writes_state() {
        // Override ROOSTERY_HOME + ROOSTERY_LARK_CLI_BIN, run().
        let home = tempfile::tempdir().unwrap();
        let (_d, script) = fixture_script(r#"echo "=== Dry Run ==="; exit 0"#);
        // Lock env globally for this test.
        let _g = ENV_GUARD.lock().unwrap();
        unsafe {
            std::env::set_var("ROOSTERY_HOME", home.path());
            std::env::set_var("ROOSTERY_LARK_CLI_BIN", &script);
        }
        let report = run();
        assert!(report.all_ok);
        assert_eq!(report.probes.len(), 6);
        assert_eq!(report.schema_version, 1);
        let state_file = home.path().join("state").join("smoke.json");
        assert!(state_file.exists());
        unsafe {
            std::env::remove_var("ROOSTERY_HOME");
            std::env::remove_var("ROOSTERY_LARK_CLI_BIN");
        }
    }

    #[test]
    fn run_with_missing_binary_marks_all_failed() {
        let home = tempfile::tempdir().unwrap();
        let _g = ENV_GUARD.lock().unwrap();
        unsafe {
            std::env::set_var("ROOSTERY_HOME", home.path());
            std::env::set_var("ROOSTERY_LARK_CLI_BIN", "/definitely/not/here/lark-cli");
        }
        let report = run();
        assert!(!report.all_ok);
        assert_eq!(report.probes.len(), 6);
        assert!(report.probes.values().all(|p| !p.ok));
        unsafe {
            std::env::remove_var("ROOSTERY_HOME");
            std::env::remove_var("ROOSTERY_LARK_CLI_BIN");
        }
    }

    static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn write_state(home: &std::path::Path, json: &str) -> std::path::PathBuf {
        let dir = home.join("state");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("smoke.json");
        std::fs::write(&path, json).unwrap();
        path
    }

    #[test]
    fn ensure_ready_never_run() {
        let home = tempfile::tempdir().unwrap();
        let _g = ENV_GUARD.lock().unwrap();
        unsafe { std::env::set_var("ROOSTERY_HOME", home.path()) };
        match ensure_ready() {
            Err(SmokeError::NeverRun) => {}
            other => panic!("expected NeverRun, got {other:?}"),
        }
        unsafe { std::env::remove_var("ROOSTERY_HOME") };
    }

    #[test]
    fn ensure_ready_happy() {
        let home = tempfile::tempdir().unwrap();
        let _g = ENV_GUARD.lock().unwrap();
        unsafe { std::env::set_var("ROOSTERY_HOME", home.path()) };
        let json = r#"{"schema_version":1,"binary":"/bin/sh","lark_cli_version":null,"started_at":"2026-05-17T00:00:00Z","all_ok":true,"probes":{}}"#;
        write_state(home.path(), json);
        ensure_ready().unwrap();
        unsafe { std::env::remove_var("ROOSTERY_HOME") };
    }

    #[test]
    fn ensure_ready_last_failed() {
        let home = tempfile::tempdir().unwrap();
        let _g = ENV_GUARD.lock().unwrap();
        unsafe { std::env::set_var("ROOSTERY_HOME", home.path()) };
        let json = r#"{"schema_version":1,"binary":"/bin/sh","lark_cli_version":null,"started_at":"2026-05-17T00:00:00Z","all_ok":false,"probes":{"docs_create_v2":{"ok":false,"reason":"unknown flag"},"im_messages_send":{"ok":true,"rc":0}}}"#;
        write_state(home.path(), json);
        match ensure_ready() {
            Err(SmokeError::LastFailed { failed_probes }) => {
                assert_eq!(failed_probes, vec!["docs_create_v2".to_string()]);
            }
            other => panic!("expected LastFailed, got {other:?}"),
        }
        unsafe { std::env::remove_var("ROOSTERY_HOME") };
    }

    #[test]
    fn ensure_ready_state_load_failed() {
        let home = tempfile::tempdir().unwrap();
        let _g = ENV_GUARD.lock().unwrap();
        unsafe { std::env::set_var("ROOSTERY_HOME", home.path()) };
        write_state(home.path(), "not json");
        match ensure_ready() {
            Err(SmokeError::StateLoadFailed { .. }) => {}
            other => panic!("expected StateLoadFailed, got {other:?}"),
        }
        unsafe { std::env::remove_var("ROOSTERY_HOME") };
    }

    #[test]
    fn smoke_error_display() {
        let e = SmokeError::LastFailed {
            failed_probes: vec!["im_messages_send".into(), "drive_move".into()],
        };
        let s = format!("{e}");
        assert!(s.contains("im_messages_send"));
        assert!(s.contains("drive_move"));
    }
}
