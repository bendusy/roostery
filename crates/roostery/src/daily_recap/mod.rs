//! Daily recap engine — git log multi-repo aggregation + Runner-delegated summarization.
//!
//! See `.codestable/features/2026-05-19-report-recap-engine/2026-05-19-report-recap-engine-design.md`.
//!
//! Direct `RunnerRegistry::find(kind).run` invocation; **does not go through
//! `dispatcher::fire`** because daily-recap is a one-shot string-return call,
//! not a hook-event dispatch (design §0 D5). Self-managed `BudgetGuard` +
//! `JournalEntry`.

pub mod cli;
pub mod git_log;

use crate::config::BudgetCfg;
use crate::dispatcher::budget::{BudgetError, BudgetGuard};
use crate::dispatcher::hook_event::{HOOK_EVENT_SCHEMA_VERSION, HookEvent};
use crate::dispatcher::runners::{RunnerError, RunnerRegistry, RunnerStatus};
use crate::dispatcher::trace::TraceContext;
use crate::journal::{self, Journal, JournalEntry, JournalResult};
use crate::redact::{scrub_text, scrub_value};
use chrono::{FixedOffset, NaiveDate};
use git_log::{GitLogAggregate, GitLogError, RepoSpec, RepoSpecError};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Instant;
use thiserror::Error;

/// Compile-time embedded default prompt template. `{{ git_log }}` is the only
/// placeholder; consumers pass the rendered markdown from
/// [`git_log::render_markdown`] in.
pub const DEFAULT_PROMPT_TEMPLATE: &str = include_str!("templates/default-recap-prompt.md");

const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_ESTIMATED_COST_USD: f64 = 0.05;
const HOOK_SOURCE: &str = "cron.daily-recap";
const JOURNAL_SOURCE: &str = "daily_recap";
const PROMPT_HEAD_LIMIT: usize = 200;
const SUMMARY_HEAD_LIMIT: usize = 200;
const STDERR_HEAD_LIMIT: usize = 200;

/// Runtime context — bundles all collaborators so the public API doesn't
/// expose a 5-argument soup. Borrow-lifetime collapses onto `'a`.
pub struct RecapRuntime<'a> {
    pub registry: &'a RunnerRegistry,
    pub journal: &'a Journal,
    pub budget_cfg: &'a BudgetCfg,
    pub budget_path: &'a Path,
    pub trace_max_depth: u32,
    /// Estimated single-call cost USD (USD) passed to `state_mut.check_or_raise`
    /// as the gate threshold. Usually `config.recap.budget_estimated_cost_usd`,
    /// falls back to `DEFAULT_ESTIMATED_COST_USD` if 0.0.
    pub budget_estimated_cost_usd: f64,
}

#[derive(Debug, Clone)]
pub struct RecapRequest {
    pub date: NaiveDate,
    pub timezone: FixedOffset,
    pub repos: Vec<RepoSpec>,
    pub runner_kind: String,
    pub timeout_ms: u64,
    pub prompt_override: Option<String>,
}

impl Default for RecapRequest {
    fn default() -> Self {
        let now = chrono::Local::now();
        Self {
            date: now.date_naive(),
            timezone: *now.offset(),
            repos: Vec::new(),
            runner_kind: String::new(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            prompt_override: None,
        }
    }
}

/// Dry-run output — what `prepare()` returns. CLI uses this to print the
/// rendered prompt without invoking the runner / budget / journal.
#[derive(Debug)]
pub struct PreparedRecap {
    pub aggregate: GitLogAggregate,
    pub markdown: String,
    pub prompt: String,
}

/// daily-recap **runner convention** typed args. Serialized to
/// `serde_json::Value` before being passed to `Runner::run`. Not a Runner
/// trait extension — just a documented schema for what `prompt`-based runners
/// expect under `args.prompt` / `args.timeout_ms` / `args.model` /
/// `args.resume_id`.
#[derive(Debug, Serialize)]
pub struct PromptRunnerArgs<'a> {
    pub prompt: &'a str,
    pub timeout_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_id: Option<&'a str>,
}

#[derive(Debug)]
pub enum RecapOutcome {
    Summarized {
        summary: String,
        aggregate: GitLogAggregate,
        runner_kind: String,
        cost_usd: Option<f64>,
        duration_ms: u64,
    },
    RawDump {
        markdown: String,
        aggregate: GitLogAggregate,
        reason: NoSummaryReason,
    },
    Failed(RecapError),
}

#[derive(Debug)]
#[non_exhaustive]
pub enum NoSummaryReason {
    RunnerNotInRegistry { kind: String },
    BudgetUnavailable(BudgetError),
    BudgetExhausted(BudgetError),
    RunnerErrored(RunnerError),
    RunOutcomeFailed { reason: String, stderr_head: String },
    RunOutcomeSkipped { reason: String },
    EmptyOutput,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RecapError {
    #[error("git log collection failed: {0}")]
    GitLog(#[from] GitLogError),
    #[error("repo spec invalid: {0}")]
    RepoSpec(#[from] RepoSpecError),
    #[error("config missing recap.repos and CLI provided no --repo")]
    NoRepos,
    #[error("config missing recap.runner_kind and CLI provided no --runner")]
    NoRunnerKind,
    #[error("journal append failed: {0}")]
    JournalAppend(#[from] std::io::Error),
}

// --- prompt rendering -------------------------------------------------------

/// Render a prompt template with the given git log markdown. Replaces both
/// `{{ git_log }}` and `{{git_log}}` for tolerance.
pub fn render_prompt(template: &str, markdown: &str) -> String {
    template
        .replace("{{ git_log }}", markdown)
        .replace("{{git_log}}", markdown)
}

pub fn load_template_from_path(path: &Path) -> std::io::Result<String> {
    std::fs::read_to_string(path)
}

/// UTF-8-safe byte-length truncation — `&s[..max]` panics if `max` falls
/// inside a multi-byte char. We back up to the nearest char boundary.
fn truncate_utf8_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// --- prepare (dry-run) ------------------------------------------------------

/// Build aggregate + markdown + rendered prompt without invoking runner or
/// budget. CLI `--dry-run` consumes this directly.
pub fn prepare(req: &RecapRequest) -> Result<PreparedRecap, RecapError> {
    if req.repos.is_empty() {
        return Err(RecapError::NoRepos);
    }
    let aggregate = git_log::collect_aggregate(req.date, req.timezone, &req.repos)?;
    let markdown = git_log::render_markdown(&aggregate);
    let template = match &req.prompt_override {
        Some(t) => t.clone(),
        None => DEFAULT_PROMPT_TEMPLATE.to_string(),
    };
    let prompt = render_prompt(&template, &markdown);
    Ok(PreparedRecap {
        aggregate,
        markdown,
        prompt,
    })
}

// --- run (live) -------------------------------------------------------------

/// Live execution path. Builds prompt, opens BudgetGuard, calls Runner,
/// writes JournalEntry. Returns structured `RecapOutcome` (no `Result` —
/// soft failures are encoded as `RawDump`).
pub async fn run<'a>(req: RecapRequest, rt: RecapRuntime<'a>) -> RecapOutcome {
    let start = Instant::now();

    // -------- Resolve required inputs ----------
    if req.repos.is_empty() {
        return finalize_failed(rt, &req, None, RecapError::NoRepos, start).await;
    }
    if req.runner_kind.is_empty() {
        return finalize_failed(rt, &req, None, RecapError::NoRunnerKind, start).await;
    }

    // -------- Collect git log ----------
    let aggregate = match git_log::collect_aggregate(req.date, req.timezone, &req.repos) {
        Ok(a) => a,
        Err(e) => {
            return finalize_failed(rt, &req, None, RecapError::GitLog(e), start).await;
        }
    };
    let markdown = git_log::render_markdown(&aggregate);
    let template = match &req.prompt_override {
        Some(t) => t.clone(),
        None => DEFAULT_PROMPT_TEMPLATE.to_string(),
    };
    let prompt = render_prompt(&template, &markdown);

    // -------- Runner lookup ----------
    let runner = match rt.registry.find(&req.runner_kind) {
        Some(r) => r,
        None => {
            let reason = NoSummaryReason::RunnerNotInRegistry {
                kind: req.runner_kind.clone(),
            };
            return finalize_raw(rt, &req, aggregate, markdown, &prompt, reason, start).await;
        }
    };

    // -------- Open budget guard ----------
    let mut guard = match BudgetGuard::open_at(rt.budget_cfg, rt.budget_path) {
        Ok(g) => g,
        Err(e) => {
            let reason = NoSummaryReason::BudgetUnavailable(e);
            return finalize_raw(rt, &req, aggregate, markdown, &prompt, reason, start).await;
        }
    };

    // -------- Pre-flight budget check ----------
    let estimated = if rt.budget_estimated_cost_usd > 0.0 {
        rt.budget_estimated_cost_usd
    } else {
        DEFAULT_ESTIMATED_COST_USD
    };
    if let Err(e) = guard.state_mut().check_or_raise(estimated) {
        let reason = NoSummaryReason::BudgetExhausted(e);
        return finalize_raw(rt, &req, aggregate, markdown, &prompt, reason, start).await;
    }

    // -------- Build synthetic event + trace + args ----------
    let session_id = format!("daily-recap-{}-{}", req.date, journal::new_event_id());
    let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let event = HookEvent {
        schema_version: HOOK_EVENT_SCHEMA_VERSION,
        hook_source: HOOK_SOURCE.to_string(),
        session_id: session_id.clone(),
        workspace,
        trigger_meta: serde_json::json!({
            "date": req.date.to_string(),
            "timezone_offset_seconds": req.timezone.local_minus_utc(),
            "repo_count": aggregate.repos.len(),
        }),
        trace: None,
    };
    let trace = TraceContext::new_root(None, rt.trace_max_depth);
    let timeout = if req.timeout_ms > 0 {
        req.timeout_ms
    } else {
        DEFAULT_TIMEOUT_MS
    };
    let args = PromptRunnerArgs {
        prompt: &prompt,
        timeout_ms: timeout,
        model: None,
        resume_id: None,
    };
    let args_value = serde_json::to_value(&args)
        .expect("PromptRunnerArgs is Serialize-safe");

    // -------- Invoke runner ----------
    let invoke_res = runner.run(&event, &trace, &args_value).await;
    match invoke_res {
        Err(e) => {
            let reason = NoSummaryReason::RunnerErrored(e);
            finalize_raw(rt, &req, aggregate, markdown, &prompt, reason, start).await
        }
        Ok(outcome) => match outcome.status {
            RunnerStatus::Failed { reason } => {
                let stderr_head =
                    truncate_utf8_boundary(&outcome.stderr, STDERR_HEAD_LIMIT).to_string();
                let r = NoSummaryReason::RunOutcomeFailed {
                    reason,
                    stderr_head,
                };
                finalize_raw(rt, &req, aggregate, markdown, &prompt, r, start).await
            }
            RunnerStatus::Skipped { reason } => {
                let r = NoSummaryReason::RunOutcomeSkipped { reason };
                finalize_raw(rt, &req, aggregate, markdown, &prompt, r, start).await
            }
            RunnerStatus::Success => {
                let summary = outcome.stdout.trim().to_string();
                if summary.is_empty() {
                    let r = NoSummaryReason::EmptyOutput;
                    return finalize_raw(rt, &req, aggregate, markdown, &prompt, r, start).await;
                }
                // Consume budget — mirror dispatcher's "only on Success"
                // policy (design §2.2). cost_usd None still consumes 0.0 so
                // max_calls advances and prevents bypass.
                let cost = outcome.cost_usd.unwrap_or(0.0);
                guard.state_mut().consume(cost);
                if let Err(e) = guard.commit() {
                    let r = NoSummaryReason::BudgetUnavailable(e);
                    return finalize_raw(rt, &req, aggregate, markdown, &prompt, r, start).await;
                }
                finalize_summarized(
                    rt,
                    &req,
                    aggregate,
                    &prompt,
                    summary,
                    outcome.cost_usd,
                    start,
                )
                .await
            }
        },
    }
}

// --- finalize helpers --------------------------------------------------------

async fn finalize_summarized<'a>(
    rt: RecapRuntime<'a>,
    req: &RecapRequest,
    aggregate: GitLogAggregate,
    prompt: &str,
    summary: String,
    cost_usd: Option<f64>,
    start: Instant,
) -> RecapOutcome {
    let duration_ms = start.elapsed().as_millis() as u64;
    let entry = build_journal_entry(
        req,
        &aggregate,
        prompt,
        JournalResult::Ok {
            value: serde_json::json!({
                "outcome": "summarized",
                "cost_usd": cost_usd,
                "runner_kind": req.runner_kind.clone(),
                "summary_head": scrub_text(truncate_utf8_boundary(&summary, SUMMARY_HEAD_LIMIT)),
            }),
        },
        duration_ms,
    );
    if let Err(e) = rt.journal.append(&entry) {
        return RecapOutcome::Failed(RecapError::JournalAppend(e));
    }
    RecapOutcome::Summarized {
        summary,
        aggregate,
        runner_kind: req.runner_kind.clone(),
        cost_usd,
        duration_ms,
    }
}

async fn finalize_raw<'a>(
    rt: RecapRuntime<'a>,
    req: &RecapRequest,
    aggregate: GitLogAggregate,
    markdown: String,
    prompt: &str,
    reason: NoSummaryReason,
    start: Instant,
) -> RecapOutcome {
    let duration_ms = start.elapsed().as_millis() as u64;
    let reason_kind = no_summary_reason_kind(&reason);
    let entry = build_journal_entry(
        req,
        &aggregate,
        prompt,
        JournalResult::Ok {
            value: serde_json::json!({
                "outcome": "no_summary",
                "reason_kind": reason_kind,
            }),
        },
        duration_ms,
    );
    if let Err(e) = rt.journal.append(&entry) {
        return RecapOutcome::Failed(RecapError::JournalAppend(e));
    }
    RecapOutcome::RawDump {
        markdown,
        aggregate,
        reason,
    }
}

async fn finalize_failed<'a>(
    rt: RecapRuntime<'a>,
    req: &RecapRequest,
    aggregate: Option<GitLogAggregate>,
    err: RecapError,
    start: Instant,
) -> RecapOutcome {
    let duration_ms = start.elapsed().as_millis() as u64;
    let kind = recap_error_kind(&err);
    let message = err.to_string();
    // Best-effort journal write; ignore IO failures here since we're already
    // in a Failed path (don't shadow the original error).
    if let Some(agg) = &aggregate {
        let entry = build_journal_entry(
            req,
            agg,
            "",
            JournalResult::Err {
                kind: kind.to_string(),
                message: message.clone(),
            },
            duration_ms,
        );
        let _ = rt.journal.append(&entry);
    } else {
        // No aggregate (e.g. NoRepos / NoRunnerKind / GitLog failed before
        // aggregate built). Write a minimal entry.
        let mut entry = JournalEntry::new(JOURNAL_SOURCE, format!("runner:{}", req.runner_kind));
        entry.params = serde_json::json!({
            "date": req.date.to_string(),
            "runner_kind": req.runner_kind,
        });
        entry.result = JournalResult::Err {
            kind: kind.to_string(),
            message,
        };
        entry.duration_ms = duration_ms;
        let _ = rt.journal.append(&entry);
    }
    RecapOutcome::Failed(err)
}

fn build_journal_entry(
    req: &RecapRequest,
    aggregate: &GitLogAggregate,
    prompt: &str,
    result: JournalResult,
    duration_ms: u64,
) -> JournalEntry {
    let raw_params = serde_json::json!({
        "date": req.date.to_string(),
        "timezone_offset_seconds": req.timezone.local_minus_utc(),
        "repo_count": aggregate.repos.len(),
        "commit_count": aggregate.total_commits(),
        "runner_kind": req.runner_kind,
        "timeout_ms": req.timeout_ms,
        "prompt_head": scrub_text(truncate_utf8_boundary(prompt, PROMPT_HEAD_LIMIT)),
    });
    let (params, _redacted_paths) = scrub_value(&raw_params);
    let mut entry = JournalEntry::new(JOURNAL_SOURCE, format!("runner:{}", req.runner_kind));
    entry.params = params;
    entry.result = result;
    entry.duration_ms = duration_ms;
    entry
}

/// Internal: maps degradation reason to a stable string discriminant used
/// in journal entries + `--json` output.
pub(crate) fn no_summary_reason_kind(reason: &NoSummaryReason) -> &'static str {
    match reason {
        NoSummaryReason::RunnerNotInRegistry { .. } => "runner_not_in_registry",
        NoSummaryReason::BudgetUnavailable(_) => "budget_unavailable",
        NoSummaryReason::BudgetExhausted(_) => "budget_exhausted",
        NoSummaryReason::RunnerErrored(_) => "runner_errored",
        NoSummaryReason::RunOutcomeFailed { .. } => "run_outcome_failed",
        NoSummaryReason::RunOutcomeSkipped { .. } => "run_outcome_skipped",
        NoSummaryReason::EmptyOutput => "empty_output",
    }
}

// --- JSON DTO v1 stable contract (step 9) -----------------------------------

/// Stable JSON output for `--json` flag. `schema_version: 1` is a public
/// commitment; incompatible changes must bump and retain backwards
/// deserialization. Mirrors `bot push --json` precedent
/// (`bot_stop_hook/types.rs::PushOutcome`).
#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RecapJsonOutcome {
    Summarized {
        schema_version: u32,
        summary: String,
        runner_kind: String,
        cost_usd: Option<f64>,
        duration_ms: u64,
        commit_count: usize,
        repo_count: usize,
    },
    RawDump {
        schema_version: u32,
        markdown: String,
        reason: RecapJsonReason,
        commit_count: usize,
        repo_count: usize,
    },
    Failed {
        schema_version: u32,
        error_kind: String,
        message: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecapJsonReason {
    RunnerNotInRegistry { runner_kind: String },
    BudgetUnavailable { detail: String },
    BudgetExhausted { detail: String },
    RunnerErrored { variant: String, detail: String },
    RunOutcomeFailed { reason: String, stderr_head: String },
    RunOutcomeSkipped { reason: String },
    EmptyOutput,
}

const RECAP_JSON_SCHEMA_VERSION: u32 = 1;

impl From<&RecapOutcome> for RecapJsonOutcome {
    fn from(o: &RecapOutcome) -> Self {
        match o {
            RecapOutcome::Summarized {
                summary,
                runner_kind,
                cost_usd,
                duration_ms,
                aggregate,
                ..
            } => Self::Summarized {
                schema_version: RECAP_JSON_SCHEMA_VERSION,
                summary: summary.clone(),
                runner_kind: runner_kind.clone(),
                cost_usd: *cost_usd,
                duration_ms: *duration_ms,
                commit_count: aggregate.total_commits(),
                repo_count: aggregate.repos.len(),
            },
            RecapOutcome::RawDump {
                markdown,
                reason,
                aggregate,
            } => Self::RawDump {
                schema_version: RECAP_JSON_SCHEMA_VERSION,
                markdown: markdown.clone(),
                reason: reason.into(),
                commit_count: aggregate.total_commits(),
                repo_count: aggregate.repos.len(),
            },
            RecapOutcome::Failed(err) => Self::Failed {
                schema_version: RECAP_JSON_SCHEMA_VERSION,
                error_kind: recap_error_kind(err).to_string(),
                message: err.to_string(),
            },
        }
    }
}

impl From<&NoSummaryReason> for RecapJsonReason {
    fn from(r: &NoSummaryReason) -> Self {
        match r {
            NoSummaryReason::RunnerNotInRegistry { kind } => Self::RunnerNotInRegistry {
                runner_kind: kind.clone(),
            },
            NoSummaryReason::BudgetUnavailable(e) => Self::BudgetUnavailable {
                detail: e.to_string(),
            },
            NoSummaryReason::BudgetExhausted(e) => Self::BudgetExhausted {
                detail: e.to_string(),
            },
            NoSummaryReason::RunnerErrored(e) => Self::RunnerErrored {
                variant: runner_error_variant(e).to_string(),
                detail: e.to_string(),
            },
            NoSummaryReason::RunOutcomeFailed { reason, stderr_head } => Self::RunOutcomeFailed {
                reason: reason.clone(),
                stderr_head: stderr_head.clone(),
            },
            NoSummaryReason::RunOutcomeSkipped { reason } => Self::RunOutcomeSkipped {
                reason: reason.clone(),
            },
            NoSummaryReason::EmptyOutput => Self::EmptyOutput,
        }
    }
}

fn runner_error_variant(e: &RunnerError) -> &'static str {
    match e {
        RunnerError::BinaryNotFound { .. } => "binary_not_found",
        RunnerError::SpawnFailed { .. } => "spawn_failed",
        RunnerError::Timeout { .. } => "timeout",
        RunnerError::OutputParseFailed { .. } => "output_parse_failed",
        RunnerError::BadArgs { .. } => "bad_args",
    }
}

pub(crate) fn recap_error_kind(err: &RecapError) -> &'static str {
    match err {
        RecapError::GitLog(_) => "git_log",
        RecapError::RepoSpec(_) => "repo_spec",
        RecapError::NoRepos => "no_repos",
        RecapError::NoRunnerKind => "no_runner_kind",
        RecapError::JournalAppend(_) => "journal_append",
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::hook_event::HookEvent;
    use crate::dispatcher::runners::{RunOutcome, Runner};
    use crate::dispatcher::trace::TraceContext;
    use async_trait::async_trait;
    use std::process::Command;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // ---- helpers ----------------------------------------------------------

    fn tz_utc8() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    fn make_repo_with_commit(subject: &str) -> TempDir {
        let dir = TempDir::new().unwrap();
        let path = dir.path();
        run_git(path, &["init", "--initial-branch=main"]);
        run_git(path, &["config", "user.email", "t@example.com"]);
        run_git(path, &["config", "user.name", "Tester"]);
        run_git(path, &["config", "commit.gpgsign", "false"]);
        std::fs::write(path.join("f.txt"), subject).unwrap();
        run_git(path, &["add", "f.txt"]);
        run_git(path, &["commit", "-m", subject]);
        dir
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let out = Command::new("git").arg("-C").arg(cwd).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?} failed");
    }

    /// Test harness — creates fresh tempdirs for budget + journal + a
    /// configurable mock runner registry.
    struct Harness {
        _tmp: TempDir,
        budget_path: PathBuf,
        journal: Journal,
        registry: RunnerRegistry,
        budget_cfg: BudgetCfg,
    }

    impl Harness {
        fn new(runner: MockRunner) -> Self {
            let tmp = TempDir::new().unwrap();
            let budget_path = tmp.path().join("budget.json");
            let journal_dir = tmp.path().join("journal");
            std::fs::create_dir_all(&journal_dir).unwrap();
            let journal = Journal::open(journal_dir);
            let registry = RunnerRegistry::new().with_runner(Box::new(runner));
            let budget_cfg = BudgetCfg {
                max_calls: 100,
                max_cost_usd: 1.0,
            };
            Self {
                _tmp: tmp,
                budget_path,
                journal,
                registry,
                budget_cfg,
            }
        }

        fn rt(&self) -> RecapRuntime<'_> {
            RecapRuntime {
                registry: &self.registry,
                journal: &self.journal,
                budget_cfg: &self.budget_cfg,
                budget_path: &self.budget_path,
                trace_max_depth: 8,
                budget_estimated_cost_usd: 0.05,
            }
        }
    }

    /// Programmable mock Runner — returns whatever the caller sets.
    struct MockRunner {
        kind: &'static str,
        outcome: Mutex<Option<RunOutcome>>,
        error: Mutex<Option<RunnerError>>,
        last_args: Mutex<Option<serde_json::Value>>,
    }

    impl MockRunner {
        fn succeeds(stdout: &str, cost_usd: Option<f64>) -> Self {
            Self::with_outcome(RunOutcome {
                status: RunnerStatus::Success,
                stdout: stdout.to_string(),
                stderr: String::new(),
                emitted_events: Vec::new(),
                cost_usd,
            })
        }

        fn fails_status(reason: &str, stderr: &str) -> Self {
            Self::with_outcome(RunOutcome {
                status: RunnerStatus::Failed {
                    reason: reason.to_string(),
                },
                stdout: String::new(),
                stderr: stderr.to_string(),
                emitted_events: Vec::new(),
                cost_usd: None,
            })
        }

        fn skipped(reason: &str) -> Self {
            Self::with_outcome(RunOutcome {
                status: RunnerStatus::Skipped {
                    reason: reason.to_string(),
                },
                stdout: String::new(),
                stderr: String::new(),
                emitted_events: Vec::new(),
                cost_usd: None,
            })
        }

        fn errors(err: RunnerError) -> Self {
            Self {
                kind: "mock",
                outcome: Mutex::new(None),
                error: Mutex::new(Some(err)),
                last_args: Mutex::new(None),
            }
        }

        fn with_outcome(o: RunOutcome) -> Self {
            Self {
                kind: "mock",
                outcome: Mutex::new(Some(o)),
                error: Mutex::new(None),
                last_args: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl Runner for MockRunner {
        fn kind(&self) -> &'static str {
            self.kind
        }
        async fn run(
            &self,
            _event: &HookEvent,
            _ctx: &TraceContext,
            args: &serde_json::Value,
        ) -> Result<RunOutcome, RunnerError> {
            *self.last_args.lock().unwrap() = Some(args.clone());
            if let Some(err) = self.error.lock().unwrap().take() {
                return Err(err);
            }
            Ok(self.outcome.lock().unwrap().take().expect("outcome preset"))
        }
    }

    fn sample_req(repo: RepoSpec, runner_kind: &str) -> RecapRequest {
        RecapRequest {
            date: chrono::Utc::now().with_timezone(&tz_utc8()).date_naive(),
            timezone: tz_utc8(),
            repos: vec![repo],
            runner_kind: runner_kind.to_string(),
            timeout_ms: 30_000,
            prompt_override: None,
        }
    }

    // ---- truncate_utf8_boundary -------------------------------------------

    #[test]
    fn truncate_ascii() {
        assert_eq!(truncate_utf8_boundary("hello", 3), "hel");
    }

    #[test]
    fn truncate_utf8_no_panic_at_char_boundary() {
        // "中" is 3 bytes; max=2 should back up to 0
        let s = "中文";
        let out = truncate_utf8_boundary(s, 2);
        assert!(out.is_char_boundary(out.len()));
        assert_eq!(out, "");
    }

    #[test]
    fn truncate_utf8_keeps_whole_chars() {
        let s = "中文测试"; // 12 bytes, 4 chars
        let out = truncate_utf8_boundary(s, 6);
        assert_eq!(out, "中文");
    }

    #[test]
    fn truncate_no_op_when_short() {
        assert_eq!(truncate_utf8_boundary("hi", 100), "hi");
    }

    // ---- prompt rendering -------------------------------------------------

    #[test]
    fn default_template_has_placeholder() {
        assert!(DEFAULT_PROMPT_TEMPLATE.contains("{{ git_log }}"));
    }

    #[test]
    fn render_prompt_substitutes() {
        let out = render_prompt("before {{ git_log }} after", "MD");
        assert_eq!(out, "before MD after");
    }

    #[test]
    fn render_prompt_no_spaces_also_works() {
        let out = render_prompt("X {{git_log}} Y", "Z");
        assert_eq!(out, "X Z Y");
    }

    // ---- prepare ----------------------------------------------------------

    #[tokio::test]
    async fn prepare_returns_aggregate_markdown_prompt() {
        let dir = make_repo_with_commit("feat: hi");
        let spec = RepoSpec::new(dir.path()).unwrap();
        let req = sample_req(spec, "mock");
        let prepared = prepare(&req).unwrap();
        assert_eq!(prepared.aggregate.total_commits(), 1);
        assert!(prepared.markdown.contains("feat: hi"));
        assert!(prepared.prompt.contains("feat: hi"));
        assert!(prepared.prompt.contains("git log 数据"));
    }

    #[test]
    fn prepare_no_repos_fails() {
        let req = RecapRequest::default();
        match prepare(&req) {
            Err(RecapError::NoRepos) => {}
            other => panic!("expected NoRepos, got {other:?}"),
        }
    }

    // ---- run: happy path (Step 6 exit signal) -----------------------------

    #[tokio::test]
    async fn run_summarized_happy_path() {
        let dir = make_repo_with_commit("feat: integration");
        let spec = RepoSpec::new(dir.path()).unwrap();
        let h = Harness::new(MockRunner::succeeds("Today's main work: integration test scaffolding.", Some(0.012)));
        let req = sample_req(spec, "mock");

        let outcome = run(req, h.rt()).await;
        match outcome {
            RecapOutcome::Summarized {
                summary,
                runner_kind,
                cost_usd,
                duration_ms,
                aggregate,
                ..
            } => {
                assert_eq!(summary, "Today's main work: integration test scaffolding.");
                assert_eq!(runner_kind, "mock");
                assert_eq!(cost_usd, Some(0.012));
                // duration_ms is u64; just confirm it's set (any value valid)
                let _ = duration_ms;
                assert_eq!(aggregate.total_commits(), 1);
            }
            other => panic!("expected Summarized, got {other:?}"),
        }
        // Journal file should have one entry
        let journal_files: Vec<_> = std::fs::read_dir(h.journal.dir())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(journal_files.len(), 1);
        let body = std::fs::read_to_string(journal_files[0].path()).unwrap();
        assert!(body.contains("\"outcome\":\"summarized\""));
        assert!(body.contains("\"runner_kind\":\"mock\""));
    }

    // ---- run: degradation 7 branches (Step 7) -----------------------------

    #[tokio::test]
    async fn run_runner_not_in_registry() {
        let dir = make_repo_with_commit("c1");
        let spec = RepoSpec::new(dir.path()).unwrap();
        let h = Harness::new(MockRunner::succeeds("unused", None));
        let req = sample_req(spec, "nonexistent_runner_kind");
        match run(req, h.rt()).await {
            RecapOutcome::RawDump {
                reason: NoSummaryReason::RunnerNotInRegistry { kind },
                ..
            } => {
                assert_eq!(kind, "nonexistent_runner_kind");
            }
            other => panic!("expected RawDump RunnerNotInRegistry, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_budget_exhausted() {
        let dir = make_repo_with_commit("c1");
        let spec = RepoSpec::new(dir.path()).unwrap();
        let mut h = Harness::new(MockRunner::succeeds("unused", None));
        // Force tiny budget so estimated 0.05 immediately exceeds
        h.budget_cfg = BudgetCfg {
            max_calls: 100,
            max_cost_usd: 0.001,
        };
        let req = sample_req(spec, "mock");
        match run(req, h.rt()).await {
            RecapOutcome::RawDump {
                reason: NoSummaryReason::BudgetExhausted(_),
                ..
            } => {}
            other => panic!("expected BudgetExhausted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_runner_errored() {
        let dir = make_repo_with_commit("c1");
        let spec = RepoSpec::new(dir.path()).unwrap();
        let h = Harness::new(MockRunner::errors(RunnerError::Timeout {
            kind: "mock",
            timeout_ms: 100,
        }));
        let req = sample_req(spec, "mock");
        match run(req, h.rt()).await {
            RecapOutcome::RawDump {
                reason: NoSummaryReason::RunnerErrored(RunnerError::Timeout { .. }),
                ..
            } => {}
            other => panic!("expected RunnerErrored, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_run_outcome_failed() {
        let dir = make_repo_with_commit("c1");
        let spec = RepoSpec::new(dir.path()).unwrap();
        let h = Harness::new(MockRunner::fails_status("exit code 1", "boom"));
        let req = sample_req(spec, "mock");
        match run(req, h.rt()).await {
            RecapOutcome::RawDump {
                reason: NoSummaryReason::RunOutcomeFailed { reason, stderr_head },
                ..
            } => {
                assert_eq!(reason, "exit code 1");
                assert_eq!(stderr_head, "boom");
            }
            other => panic!("expected RunOutcomeFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_run_outcome_skipped() {
        let dir = make_repo_with_commit("c1");
        let spec = RepoSpec::new(dir.path()).unwrap();
        let h = Harness::new(MockRunner::skipped("rule disabled"));
        let req = sample_req(spec, "mock");
        match run(req, h.rt()).await {
            RecapOutcome::RawDump {
                reason: NoSummaryReason::RunOutcomeSkipped { reason },
                ..
            } => {
                assert_eq!(reason, "rule disabled");
            }
            other => panic!("expected RunOutcomeSkipped, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_empty_output() {
        let dir = make_repo_with_commit("c1");
        let spec = RepoSpec::new(dir.path()).unwrap();
        let h = Harness::new(MockRunner::succeeds("   \n  \t  ", Some(0.001)));
        let req = sample_req(spec, "mock");
        match run(req, h.rt()).await {
            RecapOutcome::RawDump {
                reason: NoSummaryReason::EmptyOutput,
                ..
            } => {}
            other => panic!("expected EmptyOutput, got {other:?}"),
        }
    }

    // ---- run: failed paths ------------------------------------------------

    #[tokio::test]
    async fn run_no_repos_failed() {
        let h = Harness::new(MockRunner::succeeds("unused", None));
        let req = RecapRequest {
            runner_kind: "mock".to_string(),
            ..Default::default()
        };
        match run(req, h.rt()).await {
            RecapOutcome::Failed(RecapError::NoRepos) => {}
            other => panic!("expected Failed NoRepos, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_no_runner_kind_failed() {
        let dir = make_repo_with_commit("c1");
        let spec = RepoSpec::new(dir.path()).unwrap();
        let h = Harness::new(MockRunner::succeeds("unused", None));
        let req = RecapRequest {
            repos: vec![spec],
            ..Default::default()
        };
        match run(req, h.rt()).await {
            RecapOutcome::Failed(RecapError::NoRunnerKind) => {}
            other => panic!("expected Failed NoRunnerKind, got {other:?}"),
        }
    }

    // ---- budget consume only on Success (mirrors dispatcher) --------------

    #[tokio::test]
    async fn budget_not_consumed_on_failure() {
        let dir = make_repo_with_commit("c1");
        let spec = RepoSpec::new(dir.path()).unwrap();
        let h = Harness::new(MockRunner::fails_status("nope", ""));
        let req = sample_req(spec, "mock");
        let _ = run(req, h.rt()).await;
        // Budget file should not exist or have 0 calls
        if h.budget_path.exists() {
            let body = std::fs::read_to_string(&h.budget_path).unwrap();
            assert!(
                !body.contains("\"calls\":1") && !body.contains("\"calls\": 1"),
                "budget should not consume on failure, got body: {body}"
            );
        }
    }

    // ---- RecapJsonOutcome v1 stable DTO (Step 9) --------------------------

    #[tokio::test]
    async fn json_dto_summarized_schema_v1() {
        let dir = make_repo_with_commit("c1");
        let spec = RepoSpec::new(dir.path()).unwrap();
        let h = Harness::new(MockRunner::succeeds("daily summary text", Some(0.02)));
        let req = sample_req(spec, "mock");
        let outcome = run(req, h.rt()).await;
        let dto = RecapJsonOutcome::from(&outcome);
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["outcome"], "summarized");
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["summary"], "daily summary text");
        assert_eq!(v["runner_kind"], "mock");
        assert_eq!(v["cost_usd"], 0.02);
        assert!(v["duration_ms"].is_number());
        assert_eq!(v["commit_count"], 1);
        assert_eq!(v["repo_count"], 1);
    }

    #[tokio::test]
    async fn json_dto_raw_dump_schema_v1() {
        let dir = make_repo_with_commit("c1");
        let spec = RepoSpec::new(dir.path()).unwrap();
        let h = Harness::new(MockRunner::fails_status("exit 1", "boom"));
        let req = sample_req(spec, "mock");
        let outcome = run(req, h.rt()).await;
        let dto = RecapJsonOutcome::from(&outcome);
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["outcome"], "raw_dump");
        assert_eq!(v["schema_version"], 1);
        assert!(v["markdown"].as_str().unwrap().contains("c1"));
        assert_eq!(v["reason"]["kind"], "run_outcome_failed");
        assert_eq!(v["reason"]["reason"], "exit 1");
        assert_eq!(v["reason"]["stderr_head"], "boom");
    }

    #[tokio::test]
    async fn json_dto_failed_schema_v1() {
        let h = Harness::new(MockRunner::succeeds("x", None));
        let req = RecapRequest {
            runner_kind: "mock".to_string(),
            ..Default::default()
        };
        let outcome = run(req, h.rt()).await;
        let dto = RecapJsonOutcome::from(&outcome);
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["outcome"], "failed");
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["error_kind"], "no_repos");
        assert!(v["message"].as_str().unwrap().contains("recap.repos"));
    }

    #[tokio::test]
    async fn budget_consumed_on_success() {
        let dir = make_repo_with_commit("c1");
        let spec = RepoSpec::new(dir.path()).unwrap();
        let h = Harness::new(MockRunner::succeeds("summary text", Some(0.01)));
        let req = sample_req(spec, "mock");
        let _ = run(req, h.rt()).await;
        assert!(h.budget_path.exists());
        let body = std::fs::read_to_string(&h.budget_path).unwrap();
        // Budget should have consumed one call
        assert!(
            body.contains("\"calls\": 1") || body.contains("\"calls\":1"),
            "expected calls=1, body: {body}"
        );
    }
}
