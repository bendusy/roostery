//! `lark-cli` PATH-prefix shim — intercepts agent-runtime calls to `lark-cli`,
//! tees stdout/stderr to the user while writing a [`JournalEntry`] for audit,
//! then delegates to the real `lark-cli` binary.
//!
//! See `.codestable/features/2026-05-17-lark-cli-shim/lark-cli-shim-design.md`
//! for the full behavior contract.

use roostery::journal::{JournalEntry, JournalResult};
use roostery::redact;
use roostery::remoterefs;
use serde_json::json;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use thiserror::Error;

const ENV_REAL_CLI: &str = "ROOSTERY_REAL_LARK_CLI";
const ENV_NOJOURNAL: &str = "ROOSTERY_NOJOURNAL";
const INTERACTIVE_VERBS: &[&str] = &["auth"];
const STDOUT_HEAD_CAP: usize = 64 * 1024;
const STDERR_HEAD_CAP: usize = 16 * 1024;

#[derive(Debug, Error)]
enum ShimError {
    #[error("ROOSTERY_REAL_LARK_CLI not set; run `roostery init`")]
    MissingRealCli,
    #[error(
        "real_lark_cli ({real:?}) resolves to shim itself ({shim:?}); abort to prevent recursion"
    )]
    Recursion { real: PathBuf, shim: PathBuf },
    #[error("real_lark_cli not found: {path:?}")]
    RealCliNotFound { path: PathBuf },
    #[error("journal append failed: {source}")]
    #[allow(dead_code)]
    JournalFailed { source: std::io::Error },
}

/// Resolve the real `lark-cli` path from env and guard against the shim
/// pointing at itself (would cause infinite recursion).
fn resolve_real_cli() -> Result<PathBuf, ShimError> {
    let raw = std::env::var_os(ENV_REAL_CLI).filter(|v| !v.is_empty());
    let raw = raw.ok_or(ShimError::MissingRealCli)?;
    let real = PathBuf::from(raw);
    if !real.exists() {
        return Err(ShimError::RealCliNotFound { path: real });
    }
    let real_canon = std::fs::canonicalize(&real)
        .map_err(|_| ShimError::RealCliNotFound { path: real.clone() })?;
    if let Ok(self_exe) = std::env::current_exe()
        && let Ok(self_canon) = std::fs::canonicalize(&self_exe)
        && real_canon == self_canon
    {
        return Err(ShimError::Recursion {
            real: real_canon,
            shim: self_canon,
        });
    }
    Ok(real_canon)
}

/// Decide whether the call must go through `execv` (interactive direct-through).
/// Three-stage check: TTY on any std fd, verb in [`INTERACTIVE_VERBS`], or
/// presence of `--interactive` / `-i` / `--repl` flag.
fn is_interactive(sub_argv: &[String]) -> bool {
    if std::io::stdin().is_terminal()
        || std::io::stdout().is_terminal()
        || std::io::stderr().is_terminal()
    {
        return true;
    }
    if let Some(verb) = sub_argv.first()
        && INTERACTIVE_VERBS.contains(&verb.as_str())
    {
        return true;
    }
    sub_argv
        .iter()
        .any(|a| a == "--interactive" || a == "-i" || a == "--repl")
}

/// Variant of `is_interactive` that ignores TTY state — testable in any env.
#[cfg(test)]
fn is_interactive_argv(sub_argv: &[String]) -> bool {
    if let Some(verb) = sub_argv.first()
        && INTERACTIVE_VERBS.contains(&verb.as_str())
    {
        return true;
    }
    sub_argv
        .iter()
        .any(|a| a == "--interactive" || a == "-i" || a == "--repl")
}

enum Outcome {
    Full {
        rc: i32,
        stdout_head: Vec<u8>,
        stderr_head: Vec<u8>,
        duration_ms: u64,
        stdin_present: bool,
    },
    Skipped {
        reason: &'static str,
    },
}

/// Pump bytes from `src` to `dst` while keeping the first `cap` bytes in
/// `head`. Writes to `dst` are best-effort (broken pipes are swallowed) so
/// transparency wins over journal completeness.
fn pump<R: std::io::Read, W: std::io::Write>(
    mut src: R,
    mut dst: W,
    cap: usize,
) -> std::io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(cap.min(4096));
    let mut buf = [0u8; 4096];
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        let _ = dst.write_all(chunk);
        let _ = dst.flush();
        if head.len() < cap {
            let take = cap - head.len();
            head.extend_from_slice(&chunk[..n.min(take)]);
        }
    }
    Ok(head)
}

/// Run the real `lark-cli` non-interactively: spawn, two pump threads
/// (stdout / stderr), wait, return (rc, stdout_head, stderr_head, duration_ms).
fn run_non_interactive(
    real_cli: &Path,
    sub_argv: &[String],
) -> std::io::Result<(i32, Vec<u8>, Vec<u8>, u64)> {
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let started = Instant::now();
    let mut child = Command::new(real_cli)
        .args(sub_argv)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let child_stdout = child.stdout.take().expect("stdout piped");
    let child_stderr = child.stderr.take().expect("stderr piped");

    let t_out = std::thread::spawn(move || pump(child_stdout, std::io::stdout(), STDOUT_HEAD_CAP));
    let t_err = std::thread::spawn(move || pump(child_stderr, std::io::stderr(), STDERR_HEAD_CAP));

    let status = child.wait()?;
    let stdout_head = t_out.join().unwrap_or(Ok(Vec::new())).unwrap_or_default();
    let stderr_head = t_err.join().unwrap_or(Ok(Vec::new())).unwrap_or_default();
    let rc = status.code().unwrap_or(1);
    let duration_ms = started.elapsed().as_millis() as u64;
    Ok((rc, stdout_head, stderr_head, duration_ms))
}

/// Build a `JournalEntry` for either a Full run or a Skipped placeholder.
/// Maps onto the 11-field schema (see roadmap §4.2); extras live under `params`.
fn build_entry(sub_argv: &[String], outcome: Outcome) -> JournalEntry {
    let verb = sub_argv
        .first()
        .cloned()
        .unwrap_or_else(|| "<empty>".into());
    let (scrubbed_argv, _) = redact::scrub_argv(sub_argv);
    match outcome {
        Outcome::Full {
            rc,
            stdout_head,
            stderr_head,
            duration_ms,
            stdin_present,
        } => {
            let stdout_str = String::from_utf8_lossy(&stdout_head);
            let stderr_str = String::from_utf8_lossy(&stderr_head);
            let stdout_clean = redact::scrub_text(&stdout_str);
            let stderr_clean = redact::scrub_text(&stderr_str);
            let refs = remoterefs::extract(sub_argv, &stdout_str);
            let refs_value = serde_json::to_value(&refs).unwrap_or(serde_json::Value::Null);
            let cwd = std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut e = JournalEntry::new("shim", format!("lark-cli:{}", verb));
            e.params = json!({
                "argv": scrubbed_argv,
                "cwd": cwd,
                "stdin_present": stdin_present,
                "stdout_head": stdout_clean,
                "stderr_head": stderr_clean,
                "remote_refs": refs_value.clone(),
            });
            e.duration_ms = duration_ms;
            e.result = if rc == 0 {
                JournalResult::Ok { value: refs_value }
            } else {
                JournalResult::Err {
                    kind: "NonZeroExit".into(),
                    message: format!(
                        "exit={} stderr={}",
                        rc,
                        stderr_clean.lines().next().unwrap_or("")
                    ),
                }
            };
            e
        }
        Outcome::Skipped { reason } => {
            let mut e = JournalEntry::new("shim", format!("lark-cli:{}:skipped", verb));
            e.params = json!({
                "argv": scrubbed_argv,
                "reason": reason,
            });
            e.duration_ms = 0;
            e
        }
    }
}

fn main() -> ExitCode {
    let sub_argv: Vec<String> = std::env::args().skip(1).collect();

    let real_cli = match resolve_real_cli() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[roostery] {e}");
            return ExitCode::from(127);
        }
    };

    let journal = roostery::journal::Journal::default();

    if is_interactive(&sub_argv) {
        let entry = build_entry(
            &sub_argv,
            Outcome::Skipped {
                reason: "interactive",
            },
        );
        if let Err(e) = journal.append(&entry) {
            tracing::warn!(error = %e, "shim: journal append failed (interactive)");
        }
        use std::os::unix::process::CommandExt as _;
        let err = std::process::Command::new(&real_cli).args(&sub_argv).exec();
        // exec only returns on failure.
        eprintln!("[roostery] exec failed: {err}");
        return ExitCode::from(127);
    }

    let nojournal = matches!(std::env::var(ENV_NOJOURNAL).as_deref(), Ok("1"));

    let (rc, stdout_head, stderr_head, duration_ms) =
        match run_non_interactive(&real_cli, &sub_argv) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[roostery] spawn failed: {e}");
                return ExitCode::from(127);
            }
        };

    let entry = if nojournal {
        build_entry(
            &sub_argv,
            Outcome::Skipped {
                reason: "nojournal",
            },
        )
    } else {
        let stdin_present = !std::io::stdin().is_terminal();
        build_entry(
            &sub_argv,
            Outcome::Full {
                rc,
                stdout_head,
                stderr_head,
                duration_ms,
                stdin_present,
            },
        )
    };

    if let Err(e) = journal.append(&entry) {
        tracing::warn!(error = %e, "shim: journal append failed");
    }

    rc_to_exitcode(rc)
}

fn rc_to_exitcode(rc: i32) -> ExitCode {
    if let Ok(byte) = u8::try_from(rc) {
        ExitCode::from(byte)
    } else {
        ExitCode::from(1)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    // codex audit finding-09: 统一用 crate-wide TEST_ENV_LOCK。shim 是 cargo bin
    // target，同 crate 的 lib 模块可直接 `use roostery::...` 访问。
    use roostery::paths::TEST_ENV_LOCK as ENV_LOCK;

    fn with_env<F: FnOnce()>(real: Option<&str>, body: F) {
        let _g = ENV_LOCK.lock().unwrap();
        // Safety: tests touching env are serialized via ENV_LOCK.
        unsafe {
            match real {
                Some(v) => std::env::set_var(ENV_REAL_CLI, v),
                None => std::env::remove_var(ENV_REAL_CLI),
            }
        }
        body();
        unsafe { std::env::remove_var(ENV_REAL_CLI) };
    }

    #[test]
    fn resolve_missing_env() {
        with_env(None, || {
            assert!(matches!(resolve_real_cli(), Err(ShimError::MissingRealCli)));
        });
    }

    #[test]
    fn resolve_empty_env_is_missing() {
        with_env(Some(""), || {
            assert!(matches!(resolve_real_cli(), Err(ShimError::MissingRealCli)));
        });
    }

    #[test]
    fn resolve_nonexistent_path() {
        with_env(Some("/definitely/not/here/lark-cli"), || {
            assert!(matches!(
                resolve_real_cli(),
                Err(ShimError::RealCliNotFound { .. })
            ));
        });
    }

    #[test]
    fn resolve_happy_path() {
        // /bin/sh exists on every Unix; canonicalize works.
        with_env(Some("/bin/sh"), || {
            let got = resolve_real_cli().expect("happy");
            assert!(got.exists());
            assert!(got.is_absolute());
        });
    }

    #[test]
    fn resolve_recursion_detected() {
        // Point ENV_REAL_CLI at current_exe(); canonicalize equal → Recursion.
        let self_exe = std::env::current_exe().expect("test binary");
        with_env(Some(self_exe.to_str().unwrap()), || {
            assert!(matches!(
                resolve_real_cli(),
                Err(ShimError::Recursion { .. })
            ));
        });
    }

    #[test]
    fn is_interactive_truth_table() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(is_interactive_argv(&s(&["auth", "login"])));
        assert!(is_interactive_argv(&s(&["any", "--interactive"])));
        assert!(is_interactive_argv(&s(&["any", "-i"])));
        assert!(is_interactive_argv(&s(&["any", "--repl"])));
        assert!(!is_interactive_argv(&s(&["im", "+messages-send"])));
    }

    #[test]
    fn build_entry_full_schema_locked() {
        let argv = vec!["im".into(), "+messages-send".into()];
        let entry = build_entry(
            &argv,
            Outcome::Full {
                rc: 0,
                stdout_head: br#"{"data":{"message_id":"om_abc","chat_id":"oc_x"}}"#.to_vec(),
                stderr_head: Vec::new(),
                duration_ms: 12,
                stdin_present: false,
            },
        );
        assert_eq!(entry.schema_version, roostery::SCHEMA_VERSION);
        assert_eq!(entry.source, "shim");
        assert_eq!(entry.action, "lark-cli:im");
        assert_eq!(entry.duration_ms, 12);
        let p = &entry.params;
        for key in [
            "argv",
            "cwd",
            "stdin_present",
            "stdout_head",
            "stderr_head",
            "remote_refs",
        ] {
            assert!(p.get(key).is_some(), "params missing {key}");
        }
        match entry.result {
            JournalResult::Ok { value } => {
                assert_eq!(value["message_id"], "om_abc");
            }
            JournalResult::Err { .. } => panic!("expected Ok"),
        }
    }

    #[test]
    fn build_entry_full_nonzero_is_err() {
        let entry = build_entry(
            &["docs".into()],
            Outcome::Full {
                rc: 7,
                stdout_head: Vec::new(),
                stderr_head: b"boom\n".to_vec(),
                duration_ms: 1,
                stdin_present: false,
            },
        );
        match entry.result {
            JournalResult::Err { kind, message } => {
                assert_eq!(kind, "NonZeroExit");
                assert!(message.contains("exit=7"));
            }
            JournalResult::Ok { .. } => panic!("expected Err"),
        }
    }

    #[test]
    fn build_entry_skipped_schema() {
        let entry = build_entry(
            &["auth".into(), "login".into()],
            Outcome::Skipped {
                reason: "interactive",
            },
        );
        assert_eq!(entry.source, "shim");
        assert_eq!(entry.action, "lark-cli:auth:skipped");
        assert_eq!(entry.duration_ms, 0);
        assert_eq!(entry.params["reason"], "interactive");
        assert!(matches!(entry.result, JournalResult::Ok { .. }));
    }

    #[test]
    fn build_entry_edge_cases() {
        let e1 = build_entry(
            &[],
            Outcome::Skipped {
                reason: "nojournal",
            },
        );
        assert_eq!(e1.action, "lark-cli:<empty>:skipped");

        let argv = vec![
            "im".into(),
            "--access-token".into(),
            "xyz".into(),
            "send".into(),
        ];
        let e2 = build_entry(
            &argv,
            Outcome::Full {
                rc: 0,
                stdout_head: Vec::new(),
                stderr_head: Vec::new(),
                duration_ms: 1,
                stdin_present: false,
            },
        );
        assert_eq!(e2.params["argv"].as_array().unwrap()[2], "***");
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
    fn run_happy_and_exit_passthrough() {
        let (_d, p) = fixture_script("echo hello && echo err >&2 && exit 0");
        let (rc, out, err, _) = run_non_interactive(&p, &[]).unwrap();
        assert_eq!(
            (rc, &out[..], &err[..]),
            (0, &b"hello\n"[..], &b"err\n"[..])
        );
        let (_d2, p2) = fixture_script("exit 42");
        assert_eq!(run_non_interactive(&p2, &[]).unwrap().0, 42);
    }

    #[test]
    fn run_head_caps() {
        let (_d, p) = fixture_script("head -c 204800 /dev/zero | tr '\\0' A");
        let (_, out, _, _) = run_non_interactive(&p, &[]).unwrap();
        assert_eq!(out.len(), STDOUT_HEAD_CAP);
        assert!(out.iter().all(|b| *b == b'A'));
        let (_d2, p2) = fixture_script("head -c 65536 /dev/zero | tr '\\0' B >&2");
        assert_eq!(
            run_non_interactive(&p2, &[]).unwrap().2.len(),
            STDERR_HEAD_CAP
        );
    }
}
