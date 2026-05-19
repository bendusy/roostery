//! `LarkCli` subprocess implementation of `LarkRunner`. See module-level docs.

use crate::lark_cli::error::{LarkError, truncate_args, truncate_field};
use crate::lark_cli::runner::{LarkRunner, RunOptions};
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time;

const ENV_BIN: &str = "ROOSTERY_LARK_CLI_BIN";
const DEFAULT_BIN: &str = "lark-cli";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Default subprocess implementation of [`LarkRunner`].
///
/// Binary path resolution: `$ROOSTERY_LARK_CLI_BIN` > `with_binary` > `"lark-cli"`
/// (PATH lookup). The legacy Python `FEISHU_HUB_LARK_CLI_BIN` env var is
/// intentionally not consulted.
pub struct LarkCli {
    binary: PathBuf,
    default_timeout: Duration,
}

impl LarkCli {
    /// Per design D6 we intentionally do not provide a `Default` impl
    /// (would be near-duplicate of `new()`; we'd rather keep one path).
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let binary = std::env::var_os(ENV_BIN)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_BIN));
        Self {
            binary,
            default_timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            default_timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_default_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Resolved binary path (honors `ROOSTERY_LARK_CLI_BIN` env override).
    /// Exposed so streaming-wrapper callers (e.g., `bot_bridge::event::consume_im`)
    /// that bypass the buffered `LarkRunner` trait can still share the same
    /// binary resolution rule instead of falling back to a literal `"lark-cli"`.
    pub fn binary(&self) -> &Path {
        &self.binary
    }
}

#[async_trait]
impl LarkRunner for LarkCli {
    async fn run_with_options(&self, args: &[&str], opts: RunOptions) -> Result<Value, LarkError> {
        let timeout = opts.timeout.unwrap_or(self.default_timeout);

        // Build full argv: [profile?] + args
        let mut full_args: Vec<String> = Vec::with_capacity(args.len() + 2);
        if let Some(profile) = &opts.profile {
            full_args.push("--profile".into());
            full_args.push(profile.clone());
        }
        full_args.extend(args.iter().map(|s| s.to_string()));

        let mut cmd = Command::new(&self.binary);
        cmd.args(&full_args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        if opts.stdin.is_some() {
            cmd.stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }
        cmd.kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|source| {
            let mut program_args = full_args.clone();
            truncate_args(&mut program_args);
            LarkError::Spawn {
                path: self.binary.clone(),
                program_args,
                source,
            }
        })?;

        // codex audit round-3 finding：原实现 `let _ = write/shutdown` 静默吞
        // 错让 caller 误以为 stdin 已送达；改为传播 StdinWriteFailed。
        if let Some(stdin_data) = opts.stdin
            && let Some(mut stdin) = child.stdin.take()
        {
            let bytes = stdin_data.as_bytes();
            if let Err(source) = stdin.write_all(bytes).await {
                // 主动 reap child 防 zombie
                let _ = child.kill().await;
                return Err(LarkError::StdinWriteFailed {
                    bytes_written: 0,
                    source,
                });
            }
            if let Err(source) = stdin.shutdown().await {
                let _ = child.kill().await;
                return Err(LarkError::StdinWriteFailed {
                    bytes_written: bytes.len(),
                    source,
                });
            }
        }

        let wait_fut = child.wait_with_output();
        let output = match time::timeout(timeout, wait_fut).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                let mut program_args = full_args.clone();
                truncate_args(&mut program_args);
                return Err(LarkError::Spawn {
                    path: self.binary.clone(),
                    program_args,
                    source: e,
                });
            }
            Err(_elapsed) => {
                // wait_with_output consumed the child; kill_on_drop already
                // attempted SIGKILL when the future was dropped on timeout
                // unwind. Return Timeout.
                return Err(LarkError::Timeout {
                    timeout_ms: timeout.as_millis() as u64,
                });
            }
        };

        let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let exit_code = output.status.code().unwrap_or(-1);

        if !output.status.success() {
            truncate_field(&mut stdout);
            truncate_field(&mut stderr);
            let (body_code, message) = parse_error_body(&stdout, &stderr);
            return Err(LarkError::NonZeroExit {
                exit_code,
                body_code,
                message,
                stdout,
                stderr,
            });
        }

        let trimmed = stdout.trim();
        if trimmed.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str::<Value>(trimmed).map_err(|source| {
            truncate_field(&mut stdout);
            LarkError::OutputParse { source, stdout }
        })
    }
}

/// Try to extract `(code, msg)` from a JSON error body in stdout (lark-cli
/// convention: `{"code": int, "msg": str, ...}`). Falls back to stderr / stdout
/// summary as message.
fn parse_error_body(stdout: &str, stderr: &str) -> (Option<i64>, String) {
    let trimmed = stdout.trim();
    if trimmed.starts_with('{')
        && let Ok(Value::Object(map)) = serde_json::from_str::<Value>(trimmed)
    {
        let code = map.get("code").and_then(Value::as_i64);
        let msg = map
            .get("msg")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        if code.is_some() || msg.is_some() {
            let message = msg.unwrap_or_else(|| summary(stderr, stdout));
            return (code, message);
        }
    }
    (None, summary(stderr, stdout))
}

/// 取 stderr / stdout 第一行作为人类可读 summary，截断到 500 char。
fn summary(stderr: &str, stdout: &str) -> String {
    let raw = if !stderr.trim().is_empty() {
        stderr
    } else {
        stdout
    };
    let line = raw.lines().next().unwrap_or("").trim();
    let mut out = line.to_string();
    if out.len() > 500 {
        let mut cut = 500;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Write a shell script under tempdir, chmod +x, return path.
    ///
    /// Uses `std::fs::write` (closes fd atomically before return) instead
    /// of `File::create + write_all + drop` — Linux can reject `execve`
    /// on a recently-written file with `ExecutableFileBusy` (ETXTBSY) if
    /// the kernel's write-reference lingers past the close syscall.
    /// macOS doesn't enforce this so the bug is invisible on Darwin.
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

    #[tokio::test]
    async fn s2_1_happy_path() {
        let (_d, path) = fixture_script(r#"echo '{"foo":"bar"}'"#);
        let cli = LarkCli::with_binary(path);
        let v = cli.run(&["any"]).await.unwrap();
        assert_eq!(v, json!({"foo":"bar"}));
    }

    #[tokio::test]
    async fn s2_2_empty_stdout_returns_null() {
        let (_d, path) = fixture_script("printf ''");
        let cli = LarkCli::with_binary(path);
        let v = cli.run(&["any"]).await.unwrap();
        assert_eq!(v, Value::Null);
    }

    #[tokio::test]
    async fn s2_3_non_json_stdout_is_output_parse() {
        let (_d, path) = fixture_script("echo 'not json'");
        let cli = LarkCli::with_binary(path);
        match cli.run(&["any"]).await {
            Err(LarkError::OutputParse { stdout, .. }) => {
                assert!(stdout.contains("not json"));
            }
            other => panic!("expected OutputParse, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn s2_4_non_zero_exit_with_stderr() {
        let (_d, path) = fixture_script(
            r#"echo 'whatever' >&2
exit 1"#,
        );
        let cli = LarkCli::with_binary(path);
        match cli.run(&["any"]).await {
            Err(LarkError::NonZeroExit {
                exit_code,
                body_code,
                stderr,
                ..
            }) => {
                assert_eq!(exit_code, 1);
                assert_eq!(body_code, None);
                assert!(stderr.contains("whatever"));
            }
            other => panic!("expected NonZeroExit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn s2_4b_body_code_parsed_from_stdout_json() {
        let (_d, path) = fixture_script(
            r#"echo '{"code":99991663,"msg":"token expired"}'
exit 1"#,
        );
        let cli = LarkCli::with_binary(path);
        let err = cli.run(&["any"]).await.unwrap_err();
        match &err {
            LarkError::NonZeroExit {
                body_code, message, ..
            } => {
                assert_eq!(*body_code, Some(99991663));
                assert!(message.contains("token expired"));
            }
            other => panic!("expected NonZeroExit, got {other:?}"),
        }
        assert!(err.retriable(), "99991663 must be retriable");
    }

    #[tokio::test]
    async fn s2_5_spawn_failure_when_binary_missing() {
        let cli = LarkCli::with_binary("/definitely/nonexistent/lark-cli-bin");
        match cli.run(&["any"]).await {
            Err(LarkError::Spawn { path, source, .. }) => {
                assert!(path.to_string_lossy().contains("nonexistent"));
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("expected Spawn, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn s2_6_timeout_returns_promptly_and_marks_retriable() {
        // Fixture sleeps 30s. With timeout=500ms, run_with_options must
        // return within ~1s — proving the timeout fired and the future
        // didn't block on subprocess wait. tokio's `kill_on_drop(true)`
        // (set on the Command) handles SIGKILL during future drop — that
        // behavior is a tokio guarantee, not retested here.
        //
        // Note: an earlier iteration of this test polled a pidfile + ran
        // `kill -0 <pid>` to assert no zombie. That proved fundamentally
        // flaky on macOS under parallel test load (sh fork was not
        // scheduled within multi-second windows), and the assertion was
        // testing tokio's contract rather than our code. Reverted to a
        // duration assertion which proves what we own: timeout firing.
        let (_d, path) = fixture_script("sleep 30");
        let cli = LarkCli::with_binary(path);
        let opts = RunOptions::new().with_timeout(Duration::from_millis(500));
        let started = std::time::Instant::now();
        let err = cli.run_with_options(&["any"], opts).await.unwrap_err();
        let elapsed = started.elapsed();
        match &err {
            LarkError::Timeout { timeout_ms } => {
                assert_eq!(*timeout_ms, 500);
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
        assert!(err.retriable());
        assert!(
            elapsed < Duration::from_secs(5),
            "timeout should return promptly, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn s2_7_stdin_passthrough() {
        let (_d, path) = fixture_script("cat");
        let cli = LarkCli::with_binary(path);
        let opts = RunOptions::new().with_stdin(r#"{"x":1}"#);
        let v = cli.run_with_options(&["any"], opts).await.unwrap();
        assert_eq!(v, json!({"x":1}));
    }

    #[tokio::test]
    async fn s2_8_profile_flag_inserted_before_subcommand() {
        // Fixture echoes its own argv as a JSON array.
        let (_d, path) = fixture_script(
            r#"printf '['
sep=''
for a in "$@"; do
  printf '%s"%s"' "$sep" "$a"
  sep=','
done
printf ']'"#,
        );
        let cli = LarkCli::with_binary(path);
        let opts = RunOptions::new().with_profile("bot2");
        let v = cli.run_with_options(&["im", "+x"], opts).await.unwrap();
        assert_eq!(v, json!(["--profile", "bot2", "im", "+x"]));
    }
}
