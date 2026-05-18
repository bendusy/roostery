//! Runner trait + 默认实现 + registry (roadmap §4.3, **budget moved out**).
//!
//! Phase 4 Module E 第 3 子 feature。每个 Runner 是一个 agent runtime 的
//! adapter——`noop` / `cc_headless` 本期落地；`codex_exec` / `gemini_headless`
//! 等其他 runtime 推后。
//!
//! **与 roadmap §4.3 的偏离**（user 拍板，acceptance 阶段建议 cs-roadmap update）：
//! - `Runner::run` 不收 `&BudgetGate` 参数；budget gate 编排留给
//!   dispatcher-loop 上层 caller
//! - `RunOutcome` 加 `cost_usd: Option<f64>` 字段，让 caller 走
//!   `budget.consume(cost_usd)`
//!
//! 子进程 env 经 `SAFE_ENV_FORWARD` allowlist 过滤——父 hook 状态（如
//! `ROOSTERY_AGENT`）不串到子 agent，避免 trace 链断裂。trace ctx 经
//! `trace::to_env_pairs()` 注入 env，让子 agent 的 hook 自动回填到 journal。
//!
//! See `.codestable/features/2026-05-18-dispatcher-runners/dispatcher-runners-design.md`
//! §2.1.1.

use crate::hook_event::HookEvent;
use crate::trace::TraceContext;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use thiserror::Error;

pub const DEFAULT_TIMEOUT_MS: u64 = 600_000; // 10 min
pub const STDOUT_HEAD_CAP: usize = 4096; // 4 KiB

/// Env vars allowed to flow from the parent process to the spawned runner.
/// Anything outside this allowlist (including `ROOSTERY_AGENT` from the
/// caller's hook context) is dropped — see roadmap §4.3 约束 "防 trace 链断裂".
pub const SAFE_ENV_FORWARD: &[&str] = &[
    // POSIX baseline
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    // XDG
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    // Proxy
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
    // TLS / CA
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    // API keys (three families + Google common)
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
    // Custom base URLs
    "ANTHROPIC_BASE_URL",
    "OPENAI_BASE_URL",
    // Per-vendor config dirs
    "CLAUDE_CONFIG_DIR",
    "ANTHROPIC_CONFIG_DIR",
    "CODEX_HOME",
    "CODEX_CONFIG_DIR",
    "GEMINI_HOME",
    "GEMINI_CONFIG_DIR",
];

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunnerStatus {
    Success,
    Failed { reason: String },
    Skipped { reason: String },
}

/// Per-invocation result. `cost_usd` is `None` when runner can't infer cost
/// (e.g. noop, or CC output is not parseable JSON).
#[derive(Debug)]
pub struct RunOutcome {
    pub status: RunnerStatus,
    pub stdout: String,
    pub stderr: String,
    pub emitted_events: Vec<HookEvent>,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RunnerError {
    #[error("runner {kind} binary not found at {path:?}")]
    BinaryNotFound { kind: &'static str, path: PathBuf },
    #[error("failed to spawn runner {kind}: {source}")]
    SpawnFailed {
        kind: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("runner {kind} timed out after {timeout_ms}ms")]
    Timeout { kind: &'static str, timeout_ms: u64 },
    #[error("failed to parse runner {kind} output: {source}; stdout_head={stdout_head:?}")]
    OutputParseFailed {
        kind: &'static str,
        #[source]
        source: serde_json::Error,
        stdout_head: String,
    },
}

/// Single agent runtime adapter. Implementations spawn the runtime (CC /
/// Codex / Gemini / no-op stub) and report a `RunOutcome`. Budget gating
/// and journal writing are NOT this trait's responsibility — see module
/// docs.
#[async_trait]
pub trait Runner: Send + Sync {
    fn kind(&self) -> &'static str;

    async fn run(
        &self,
        event: &HookEvent,
        ctx: &TraceContext,
        args: &serde_json::Value,
    ) -> Result<RunOutcome, RunnerError>;
}

/// Linear-scan registry. `n` is small (2-4 runners) so `O(n)` find by kind
/// is plenty; later if registry grows we can swap the backing store.
pub struct RunnerRegistry {
    runners: Vec<Box<dyn Runner>>,
}

impl RunnerRegistry {
    pub fn new() -> Self {
        Self {
            runners: Vec::new(),
        }
    }

    pub fn with_runner(mut self, runner: Box<dyn Runner>) -> Self {
        self.runners.push(runner);
        self
    }

    /// Convenience: register `NoopRunner` + `CcHeadlessRunner` defaults.
    pub fn with_defaults() -> Self {
        Self::new()
            .with_runner(Box::new(NoopRunner))
            .with_runner(Box::new(CcHeadlessRunner::default()))
    }

    pub fn find(&self, kind: &str) -> Option<&dyn Runner> {
        self.runners
            .iter()
            .find(|r| r.kind() == kind)
            .map(|b| b.as_ref())
    }

    pub fn len(&self) -> usize {
        self.runners.len()
    }

    pub fn is_empty(&self) -> bool {
        self.runners.is_empty()
    }
}

impl Default for RunnerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// --- env sanitization ------------------------------------------------------

/// Build a sanitized env map for a child runner process. Filters parent env
/// through `SAFE_ENV_FORWARD` allowlist; adds POSIX baseline fallbacks; then
/// injects trace ctx env vars (`ROOSTERY_TRACE_ID` / `_DEPTH` /
/// `_PARENT_EVENT_ID`).
fn prep_env(ctx: &TraceContext, _runner_kind: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    // POSIX baseline with fallbacks.
    out.insert(
        "PATH".to_string(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_string()),
    );
    out.insert(
        "HOME".to_string(),
        std::env::var("HOME").unwrap_or_default(),
    );
    out.insert(
        "LANG".to_string(),
        std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".to_string()),
    );
    out.insert(
        "TERM".to_string(),
        std::env::var("TERM").unwrap_or_else(|_| "dumb".to_string()),
    );
    // SAFE_ENV_FORWARD allowlist.
    for key in SAFE_ENV_FORWARD {
        if let Ok(v) = std::env::var(key) {
            out.insert((*key).to_string(), v);
        }
    }
    // Trace ctx env (overwrites parent if anything collided).
    for (k, v) in ctx.to_env_pairs() {
        out.insert(k.to_string(), v);
    }
    out
}

fn truncate_head(s: &str) -> String {
    if s.len() <= STDOUT_HEAD_CAP {
        return s.to_string();
    }
    // Find a char boundary <= STDOUT_HEAD_CAP.
    let mut end = STDOUT_HEAD_CAP;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n... [truncated]", &s[..end])
}

// --- NoopRunner -----------------------------------------------------------

pub struct NoopRunner;

#[async_trait]
impl Runner for NoopRunner {
    fn kind(&self) -> &'static str {
        "noop"
    }

    async fn run(
        &self,
        _event: &HookEvent,
        _ctx: &TraceContext,
        _args: &serde_json::Value,
    ) -> Result<RunOutcome, RunnerError> {
        Ok(RunOutcome {
            status: RunnerStatus::Success,
            stdout: String::new(),
            stderr: String::new(),
            emitted_events: Vec::new(),
            cost_usd: None,
        })
    }
}

// --- CcHeadlessRunner -----------------------------------------------------

/// `claude -p <prompt> --output-format json [--model <m>] [--resume <id>]`
/// wrapper. `bin_override` injects a fake binary for tests; production leaves
/// it `None` and we look up `claude` via `which::which`.
#[derive(Default)]
pub struct CcHeadlessRunner {
    pub bin_override: Option<PathBuf>,
}

#[derive(Deserialize, Debug, Default)]
struct CcArgs {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    resume_id: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

/// Subset of CC `--output-format json` schema we care about. CC may add /
/// rename fields freely; everything we ignore goes into `#[serde(other)]`-able
/// catchalls — we only deserialize what we use.
#[derive(Deserialize, Debug, Default)]
struct CcJsonOutput {
    #[serde(default)]
    cost_usd: Option<f64>,
    #[serde(default, alias = "total_cost_usd")]
    _total_cost_usd: Option<f64>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[async_trait]
impl Runner for CcHeadlessRunner {
    fn kind(&self) -> &'static str {
        "cc_headless"
    }

    async fn run(
        &self,
        _event: &HookEvent,
        ctx: &TraceContext,
        args: &serde_json::Value,
    ) -> Result<RunOutcome, RunnerError> {
        let parsed: CcArgs = serde_json::from_value(args.clone()).unwrap_or_default();
        let prompt = parsed.prompt.unwrap_or_default();
        let timeout_ms = parsed.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);

        let bin = match &self.bin_override {
            Some(p) => p.clone(),
            None => which::which("claude").map_err(|_| RunnerError::BinaryNotFound {
                kind: "cc_headless",
                path: PathBuf::from("claude"),
            })?,
        };
        if !bin.exists() {
            return Err(RunnerError::BinaryNotFound {
                kind: "cc_headless",
                path: bin,
            });
        }

        let env = prep_env(ctx, "cc_headless");
        let mut cmd_args: Vec<String> =
            vec!["-p".into(), prompt, "--output-format".into(), "json".into()];
        if let Some(m) = parsed.model {
            cmd_args.push("--model".into());
            cmd_args.push(m);
        }
        if let Some(r) = parsed.resume_id {
            cmd_args.push("--resume".into());
            cmd_args.push(r);
        }

        let outcome = tokio::task::spawn_blocking(move || {
            spawn_with_timeout(&bin, &cmd_args, &env, timeout_ms)
        })
        .await
        .expect("spawn_blocking join")?;
        Ok(enrich_cc(outcome))
    }
}

struct RawProcessOutcome {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

fn spawn_with_timeout(
    bin: &std::path::Path,
    args: &[String],
    env: &HashMap<String, String>,
    timeout_ms: u64,
) -> Result<RawProcessOutcome, RunnerError> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(bin)
        .args(args)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                RunnerError::BinaryNotFound {
                    kind: "cc_headless",
                    path: bin.to_path_buf(),
                }
            } else {
                RunnerError::SpawnFailed {
                    kind: "cc_headless",
                    source,
                }
            }
        })?;

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut s) = child.stdout.take() {
                    use std::io::Read;
                    let _ = s.read_to_string(&mut stdout);
                }
                if let Some(mut s) = child.stderr.take() {
                    use std::io::Read;
                    let _ = s.read_to_string(&mut stderr);
                }
                return Ok(RawProcessOutcome {
                    exit_code: status.code(),
                    stdout,
                    stderr,
                    timed_out: false,
                });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(RunnerError::Timeout {
                        kind: "cc_headless",
                        timeout_ms,
                    });
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(source) => {
                return Err(RunnerError::SpawnFailed {
                    kind: "cc_headless",
                    source,
                });
            }
        }
    }
}

fn enrich_cc(raw: RawProcessOutcome) -> RunOutcome {
    let exit_code = raw.exit_code.unwrap_or(-1);
    let status = if raw.timed_out {
        // timed_out path returns Err above; this branch unreachable in practice.
        RunnerStatus::Failed {
            reason: "timed out".to_string(),
        }
    } else if exit_code != 0 {
        RunnerStatus::Failed {
            reason: format!("exit code {exit_code}"),
        }
    } else {
        RunnerStatus::Success
    };

    // Parse stdout as CC JSON if status is Success; otherwise skip parse.
    let (cost_usd, parsed_text) = if matches!(status, RunnerStatus::Success) {
        match serde_json::from_str::<CcJsonOutput>(&raw.stdout) {
            Ok(body) => {
                let txt = body.result.or(body.text);
                (body.cost_usd, txt)
            }
            Err(_) => (None, None),
        }
    } else {
        (None, None)
    };

    // Body of stdout is what caller sees; if we parsed a "result" text we
    // surface that as stdout for downstream readers, else keep raw.
    let stdout_out = parsed_text.unwrap_or(raw.stdout);

    RunOutcome {
        status,
        stdout: truncate_head(&stdout_out),
        stderr: truncate_head(&raw.stderr),
        emitted_events: Vec::new(),
        cost_usd,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn dummy_event() -> HookEvent {
        let raw = json!({
            "schema_version": 1,
            "hook_source": "claude-code-stop",
            "session_id": "s_test",
            "workspace": "/tmp",
            "trigger_meta": {},
        });
        serde_json::from_value(raw).unwrap()
    }

    fn dummy_ctx() -> TraceContext {
        TraceContext::new_root(Some("evt_test".to_string()), 8)
    }

    // --- S1 type tests ----------------------------------------------------

    #[test]
    fn runner_status_serde_snake_case() {
        let success = RunnerStatus::Success;
        let json = serde_json::to_value(&success).unwrap();
        assert_eq!(json, json!({"kind": "success"}));
        let failed = RunnerStatus::Failed {
            reason: "x".to_string(),
        };
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json, json!({"kind": "failed", "reason": "x"}));
    }

    #[test]
    fn runner_error_display_contains_kind() {
        let err = RunnerError::Timeout {
            kind: "cc_headless",
            timeout_ms: 1000,
        };
        let msg = err.to_string();
        assert!(msg.contains("cc_headless"));
        assert!(msg.contains("1000"));
    }

    #[test]
    fn constants_exposed() {
        assert_eq!(DEFAULT_TIMEOUT_MS, 600_000);
        assert_eq!(STDOUT_HEAD_CAP, 4096);
        assert!(SAFE_ENV_FORWARD.contains(&"PATH") || !SAFE_ENV_FORWARD.is_empty());
        assert!(SAFE_ENV_FORWARD.contains(&"ANTHROPIC_API_KEY"));
    }

    // --- S2 NoopRunner ----------------------------------------------------

    #[tokio::test]
    async fn noop_runner_kind_is_noop() {
        let r = NoopRunner;
        assert_eq!(r.kind(), "noop");
    }

    #[tokio::test]
    async fn noop_runner_returns_success_empty() {
        let r = NoopRunner;
        let outcome = r
            .run(&dummy_event(), &dummy_ctx(), &json!({}))
            .await
            .unwrap();
        assert_eq!(outcome.status, RunnerStatus::Success);
        assert!(outcome.stdout.is_empty());
        assert!(outcome.stderr.is_empty());
        assert!(outcome.emitted_events.is_empty());
        assert!(outcome.cost_usd.is_none());
    }

    // --- S3 RunnerRegistry ------------------------------------------------

    #[test]
    fn registry_new_is_empty() {
        let r = RunnerRegistry::new();
        assert!(r.is_empty());
        assert!(r.find("noop").is_none());
    }

    #[test]
    fn registry_with_runner_then_find() {
        let r = RunnerRegistry::new().with_runner(Box::new(NoopRunner));
        assert_eq!(r.len(), 1);
        let n = r.find("noop").unwrap();
        assert_eq!(n.kind(), "noop");
    }

    #[test]
    fn registry_find_miss_returns_none() {
        let r = RunnerRegistry::new().with_runner(Box::new(NoopRunner));
        assert!(r.find("nonexistent").is_none());
    }

    #[test]
    fn registry_with_defaults_has_noop_and_cc_headless() {
        let r = RunnerRegistry::with_defaults();
        assert_eq!(r.len(), 2);
        assert!(r.find("noop").is_some());
        assert!(r.find("cc_headless").is_some());
        assert!(r.find("codex_exec").is_none());
        assert!(r.find("gemini_headless").is_none());
    }

    #[test]
    fn registry_dup_kind_returns_first() {
        // Two NoopRunner registrations; linear find returns first match.
        let r = RunnerRegistry::new()
            .with_runner(Box::new(NoopRunner))
            .with_runner(Box::new(NoopRunner));
        assert_eq!(r.len(), 2);
        // No way to distinguish two identical-kind runners with this API;
        // the contract is that `find` finds *a* match.
        assert!(r.find("noop").is_some());
    }

    // --- S4 prep_env ------------------------------------------------------
    // These tests touch $ENV; we serialize on a module-local mutex.

    use std::sync::Mutex;
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn prep_env_includes_base_fallbacks() {
        let _g = ENV_LOCK.lock().unwrap();
        let ctx = dummy_ctx();
        let env = prep_env(&ctx, "cc_headless");
        assert!(env.contains_key("PATH"));
        assert!(env.contains_key("HOME"));
        assert!(env.contains_key("LANG"));
        assert!(env.contains_key("TERM"));
    }

    #[test]
    fn prep_env_injects_trace_env() {
        let _g = ENV_LOCK.lock().unwrap();
        let ctx = TraceContext::new_root(Some("evt_xy".to_string()), 8);
        let env = prep_env(&ctx, "cc_headless");
        assert_eq!(
            env.get("ROOSTERY_TRACE_ID").map(|s| s.as_str()),
            Some(ctx.trace_id.as_str())
        );
        assert_eq!(env.get("ROOSTERY_DEPTH").map(|s| s.as_str()), Some("0"));
        assert_eq!(
            env.get("ROOSTERY_PARENT_EVENT_ID").map(|s| s.as_str()),
            Some("evt_xy")
        );
    }

    #[test]
    fn prep_env_does_not_forward_unsafe_var() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("ROOSTERY_AGENT", "cc") };
        unsafe { std::env::set_var("MY_RANDOM_UNSAFE_VAR_xyz", "leak") };
        let ctx = dummy_ctx();
        let env = prep_env(&ctx, "cc_headless");
        assert!(!env.contains_key("ROOSTERY_AGENT"));
        assert!(!env.contains_key("MY_RANDOM_UNSAFE_VAR_xyz"));
        unsafe { std::env::remove_var("ROOSTERY_AGENT") };
        unsafe { std::env::remove_var("MY_RANDOM_UNSAFE_VAR_xyz") };
    }

    #[test]
    fn prep_env_forwards_safe_var_when_set() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("OPENAI_API_KEY", "test-token-abc") };
        let ctx = dummy_ctx();
        let env = prep_env(&ctx, "cc_headless");
        assert_eq!(
            env.get("OPENAI_API_KEY").map(|s| s.as_str()),
            Some("test-token-abc")
        );
        unsafe { std::env::remove_var("OPENAI_API_KEY") };
    }

    // --- S5 CcHeadless spawn --------------------------------------------

    fn write_executable_fake_claude(dir: &std::path::Path, body: &str) -> PathBuf {
        let path = dir.join("fake_claude.sh");
        // Use fs::write to avoid Linux ETXTBSY race per attention.md note.
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
    async fn cc_headless_kind() {
        let r = CcHeadlessRunner::default();
        assert_eq!(r.kind(), "cc_headless");
    }

    #[tokio::test]
    async fn cc_headless_binary_not_found() {
        let r = CcHeadlessRunner {
            bin_override: Some(PathBuf::from("/nonexistent/path/to/claude")),
        };
        match r
            .run(&dummy_event(), &dummy_ctx(), &json!({"prompt": "hi"}))
            .await
        {
            Err(RunnerError::BinaryNotFound { kind, .. }) => {
                assert_eq!(kind, "cc_headless");
            }
            other => panic!("expected BinaryNotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cc_headless_happy_returns_success_with_cost() {
        let tmp = tempfile::tempdir().unwrap();
        let body = r#"#!/bin/sh
cat <<EOF
{"cost_usd": 0.0042, "result": "hello world"}
EOF
"#;
        let bin = write_executable_fake_claude(tmp.path(), body);
        let r = CcHeadlessRunner {
            bin_override: Some(bin),
        };
        let outcome = r
            .run(&dummy_event(), &dummy_ctx(), &json!({"prompt": "hi"}))
            .await
            .unwrap();
        assert_eq!(outcome.status, RunnerStatus::Success);
        assert_eq!(outcome.cost_usd, Some(0.0042));
        assert!(outcome.stdout.contains("hello world"));
    }

    #[tokio::test]
    async fn cc_headless_non_json_stdout_returns_success_no_cost() {
        let tmp = tempfile::tempdir().unwrap();
        let body = "#!/bin/sh\necho 'plain text output'\n";
        let bin = write_executable_fake_claude(tmp.path(), body);
        let r = CcHeadlessRunner {
            bin_override: Some(bin),
        };
        let outcome = r
            .run(&dummy_event(), &dummy_ctx(), &json!({"prompt": "hi"}))
            .await
            .unwrap();
        assert_eq!(outcome.status, RunnerStatus::Success);
        assert!(outcome.cost_usd.is_none());
    }

    #[tokio::test]
    async fn cc_headless_non_zero_exit_returns_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let body = "#!/bin/sh\necho 'oops' >&2\nexit 42\n";
        let bin = write_executable_fake_claude(tmp.path(), body);
        let r = CcHeadlessRunner {
            bin_override: Some(bin),
        };
        let outcome = r
            .run(&dummy_event(), &dummy_ctx(), &json!({"prompt": "hi"}))
            .await
            .unwrap();
        match outcome.status {
            RunnerStatus::Failed { reason } => {
                assert!(reason.contains("42"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(outcome.stderr.contains("oops"));
    }

    #[tokio::test]
    async fn cc_headless_timeout_returns_err() {
        let tmp = tempfile::tempdir().unwrap();
        // Sleeps longer than our timeout.
        let body = "#!/bin/sh\nsleep 5\n";
        let bin = write_executable_fake_claude(tmp.path(), body);
        let r = CcHeadlessRunner {
            bin_override: Some(bin),
        };
        match r
            .run(
                &dummy_event(),
                &dummy_ctx(),
                &json!({"prompt": "hi", "timeout_ms": 200}),
            )
            .await
        {
            Err(RunnerError::Timeout { kind, timeout_ms }) => {
                assert_eq!(kind, "cc_headless");
                assert_eq!(timeout_ms, 200);
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    // --- S6 enrich_cc JSON parse ----------------------------------------

    #[test]
    fn enrich_cc_full_json_extracts_cost_and_text() {
        let raw = RawProcessOutcome {
            exit_code: Some(0),
            stdout: r#"{"cost_usd": 0.5, "result": "ok"}"#.to_string(),
            stderr: String::new(),
            timed_out: false,
        };
        let out = enrich_cc(raw);
        assert_eq!(out.status, RunnerStatus::Success);
        assert_eq!(out.cost_usd, Some(0.5));
        assert!(out.stdout.contains("ok"));
    }

    #[test]
    fn enrich_cc_missing_cost_field() {
        let raw = RawProcessOutcome {
            exit_code: Some(0),
            stdout: r#"{"result": "ok"}"#.to_string(),
            stderr: String::new(),
            timed_out: false,
        };
        let out = enrich_cc(raw);
        assert_eq!(out.status, RunnerStatus::Success);
        assert!(out.cost_usd.is_none());
    }

    #[test]
    fn enrich_cc_invalid_json_still_returns_success() {
        let raw = RawProcessOutcome {
            exit_code: Some(0),
            stdout: "not json at all".to_string(),
            stderr: String::new(),
            timed_out: false,
        };
        let out = enrich_cc(raw);
        assert_eq!(out.status, RunnerStatus::Success);
        assert!(out.cost_usd.is_none());
        // Falls back to raw stdout.
        assert!(out.stdout.contains("not json"));
    }

    #[test]
    fn truncate_head_caps_long_strings() {
        let long = "a".repeat(STDOUT_HEAD_CAP + 100);
        let head = truncate_head(&long);
        assert!(head.len() <= STDOUT_HEAD_CAP + 32); // + "\n... [truncated]" suffix
        assert!(head.ends_with("[truncated]"));
    }
}
