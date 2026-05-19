//! End-to-end integration tests for `daily-recap` engine. Verifies through
//! public API only: builds a real tmp git repo, injects a mock Runner via
//! `RunnerRegistry::with_runner`, points budget + journal at tempfiles,
//! invokes `daily_recap::run`, then inspects the journal jsonl + budget
//! state file from disk.

#![cfg(feature = "daily-report")]

use async_trait::async_trait;
use chrono::FixedOffset;
use roostery::config::BudgetCfg;
use roostery::daily_recap::git_log::RepoSpec;
use roostery::daily_recap::{
    NoSummaryReason, RecapJsonOutcome, RecapOutcome, RecapRequest, RecapRuntime,
};
use roostery::dispatcher::hook_event::HookEvent;
use roostery::dispatcher::runners::{
    RunOutcome, Runner, RunnerError, RunnerRegistry, RunnerStatus,
};
use roostery::dispatcher::trace::TraceContext;
use roostery::journal::Journal;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use tempfile::TempDir;

// ---------- helpers ----------

fn tz() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).unwrap()
}

fn run_git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn make_repo_with_commits(subjects: &[&str]) -> TempDir {
    let dir = TempDir::new().unwrap();
    let path = dir.path();
    run_git(path, &["init", "--initial-branch=main"]);
    run_git(path, &["config", "user.email", "i@test"]);
    run_git(path, &["config", "user.name", "Integ"]);
    run_git(path, &["config", "commit.gpgsign", "false"]);
    for subject in subjects {
        std::fs::write(path.join("f.txt"), subject).unwrap();
        run_git(path, &["add", "f.txt"]);
        run_git(path, &["commit", "-m", subject]);
    }
    dir
}

struct MockOutcomeRunner {
    outcome: Mutex<Option<RunOutcome>>,
}

impl MockOutcomeRunner {
    fn ok(stdout: &str, cost: Option<f64>) -> Self {
        Self {
            outcome: Mutex::new(Some(RunOutcome {
                status: RunnerStatus::Success,
                stdout: stdout.to_string(),
                stderr: String::new(),
                emitted_events: Vec::new(),
                cost_usd: cost,
            })),
        }
    }

    fn failed(reason: &str, stderr: &str) -> Self {
        Self {
            outcome: Mutex::new(Some(RunOutcome {
                status: RunnerStatus::Failed {
                    reason: reason.to_string(),
                },
                stdout: String::new(),
                stderr: stderr.to_string(),
                emitted_events: Vec::new(),
                cost_usd: None,
            })),
        }
    }
}

#[async_trait]
impl Runner for MockOutcomeRunner {
    fn kind(&self) -> &'static str {
        "integ_mock"
    }
    async fn run(
        &self,
        _event: &HookEvent,
        _ctx: &TraceContext,
        _args: &serde_json::Value,
    ) -> Result<RunOutcome, RunnerError> {
        Ok(self
            .outcome
            .lock()
            .unwrap()
            .take()
            .expect("outcome preset"))
    }
}

struct IntegHarness {
    _tmp: TempDir,
    budget_path: PathBuf,
    journal_dir: PathBuf,
    journal: Journal,
    registry: RunnerRegistry,
    budget_cfg: BudgetCfg,
}

impl IntegHarness {
    fn new(runner: MockOutcomeRunner) -> Self {
        let tmp = TempDir::new().unwrap();
        let budget_path = tmp.path().join("budget.json");
        let journal_dir = tmp.path().join("journal");
        std::fs::create_dir_all(&journal_dir).unwrap();
        let journal = Journal::open(journal_dir.clone());
        let registry = RunnerRegistry::new().with_runner(Box::new(runner));
        Self {
            _tmp: tmp,
            budget_path,
            journal_dir,
            journal,
            registry,
            // BudgetCfg is #[non_exhaustive] — external crates can't use struct literal;
            // attention.md "non_exhaustive struct corollary" says deserialize JSON instead.
            budget_cfg: serde_json::from_str(r#"{"max_calls":100,"max_cost_usd":1.0}"#).unwrap(),
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

    fn read_journal_lines(&self) -> Vec<serde_json::Value> {
        let files: Vec<_> = std::fs::read_dir(&self.journal_dir)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(files.len(), 1, "expected exactly one journal file");
        let body = std::fs::read_to_string(files[0].path()).unwrap();
        body.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

fn sample_req(spec: RepoSpec, runner_kind: &str) -> RecapRequest {
    RecapRequest {
        date: chrono::Utc::now().with_timezone(&tz()).date_naive(),
        timezone: tz(),
        repos: vec![spec],
        runner_kind: runner_kind.to_string(),
        timeout_ms: 30_000,
        prompt_override: None,
    }
}

// ============================================================================
// N1: Summarized happy path with journal verification
// ============================================================================

#[tokio::test]
async fn integ_n1_summarized_writes_journal_and_budget() {
    let dir = make_repo_with_commits(&["feat: integration test work"]);
    let spec = RepoSpec::new(dir.path()).unwrap();
    let h = IntegHarness::new(MockOutcomeRunner::ok(
        "今日完成集成测试搭建，主要是 daily-recap 引擎的端到端 dogfood。",
        Some(0.0123),
    ));
    let req = sample_req(spec, "integ_mock");

    let outcome = roostery::daily_recap::run(req, h.rt()).await;
    let RecapOutcome::Summarized {
        summary,
        runner_kind,
        cost_usd,
        aggregate,
        ..
    } = &outcome
    else {
        panic!("expected Summarized, got {outcome:?}");
    };
    assert!(summary.contains("daily-recap"));
    assert_eq!(runner_kind, "integ_mock");
    assert_eq!(*cost_usd, Some(0.0123));
    assert_eq!(aggregate.total_commits(), 1);

    // Journal check
    let entries = h.read_journal_lines();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e["source"], "daily_recap");
    assert_eq!(e["action"], "runner:integ_mock");
    assert_eq!(e["result"]["outcome"], "ok");
    assert_eq!(e["result"]["value"]["outcome"], "summarized");
    assert_eq!(e["result"]["value"]["cost_usd"], 0.0123);

    // Budget should have consumed
    assert!(h.budget_path.exists());
    let budget_body = std::fs::read_to_string(&h.budget_path).unwrap();
    assert!(
        budget_body.contains("\"calls\": 1") || budget_body.contains("\"calls\":1"),
        "budget did not consume: {budget_body}"
    );
}

// ============================================================================
// JSON DTO v1 schema (RecapJsonOutcome::Summarized)
// ============================================================================

#[tokio::test]
async fn integ_json_dto_summarized_v1_schema() {
    let dir = make_repo_with_commits(&["c1"]);
    let spec = RepoSpec::new(dir.path()).unwrap();
    let h = IntegHarness::new(MockOutcomeRunner::ok("summary text", Some(0.02)));
    let req = sample_req(spec, "integ_mock");

    let outcome = roostery::daily_recap::run(req, h.rt()).await;
    let dto = RecapJsonOutcome::from(&outcome);
    let v = serde_json::to_value(&dto).unwrap();
    assert_eq!(v["outcome"], "summarized");
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["summary"], "summary text");
    assert_eq!(v["commit_count"], 1);
    assert_eq!(v["repo_count"], 1);
}

// ============================================================================
// D5: RunOutcome Failed (cc_headless non-zero exit pattern)
// ============================================================================

#[tokio::test]
async fn integ_d5_run_outcome_failed_no_budget_consume() {
    let dir = make_repo_with_commits(&["c1"]);
    let spec = RepoSpec::new(dir.path()).unwrap();
    let h = IntegHarness::new(MockOutcomeRunner::failed("exit code 1", "agent crashed"));
    let req = sample_req(spec, "integ_mock");

    let outcome = roostery::daily_recap::run(req, h.rt()).await;
    let RecapOutcome::RawDump { reason, .. } = &outcome else {
        panic!("expected RawDump, got {outcome:?}");
    };
    let NoSummaryReason::RunOutcomeFailed {
        reason: r,
        stderr_head,
    } = reason
    else {
        panic!("expected RunOutcomeFailed, got {reason:?}");
    };
    assert_eq!(r, "exit code 1");
    assert_eq!(stderr_head, "agent crashed");

    // Journal: NoSummary entry with run_outcome_failed reason_kind
    let entries = h.read_journal_lines();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["result"]["value"]["outcome"], "no_summary");
    assert_eq!(
        entries[0]["result"]["value"]["reason_kind"],
        "run_outcome_failed"
    );

    // Budget should NOT have consumed
    if h.budget_path.exists() {
        let body = std::fs::read_to_string(&h.budget_path).unwrap();
        assert!(
            !body.contains("\"calls\": 1") && !body.contains("\"calls\":1"),
            "budget consumed on failure: {body}"
        );
    }
}

// ============================================================================
// D1: RunnerNotInRegistry
// ============================================================================

#[tokio::test]
async fn integ_d1_runner_not_in_registry() {
    let dir = make_repo_with_commits(&["c1"]);
    let spec = RepoSpec::new(dir.path()).unwrap();
    let h = IntegHarness::new(MockOutcomeRunner::ok("unused", None));
    let req = sample_req(spec, "different_kind_not_registered");

    let outcome = roostery::daily_recap::run(req, h.rt()).await;
    let RecapOutcome::RawDump {
        reason: NoSummaryReason::RunnerNotInRegistry { kind },
        ..
    } = &outcome
    else {
        panic!("expected RawDump RunnerNotInRegistry, got {outcome:?}");
    };
    assert_eq!(kind, "different_kind_not_registered");
}

// ============================================================================
// F1 / F2: Failed paths
// ============================================================================

#[tokio::test]
async fn integ_f1_failed_no_repos() {
    let h = IntegHarness::new(MockOutcomeRunner::ok("unused", None));
    let req = RecapRequest {
        runner_kind: "integ_mock".to_string(),
        ..Default::default()
    };
    let outcome = roostery::daily_recap::run(req, h.rt()).await;
    assert!(matches!(
        outcome,
        RecapOutcome::Failed(roostery::daily_recap::RecapError::NoRepos)
    ));
}

#[tokio::test]
async fn integ_f2_failed_no_runner_kind() {
    let dir = make_repo_with_commits(&["c1"]);
    let spec = RepoSpec::new(dir.path()).unwrap();
    let h = IntegHarness::new(MockOutcomeRunner::ok("unused", None));
    let req = RecapRequest {
        repos: vec![spec],
        ..Default::default()
    };
    let outcome = roostery::daily_recap::run(req, h.rt()).await;
    assert!(matches!(
        outcome,
        RecapOutcome::Failed(roostery::daily_recap::RecapError::NoRunnerKind)
    ));
}

// ============================================================================
// Multi-repo aggregation
// ============================================================================

#[tokio::test]
async fn integ_multi_repo_aggregation() {
    let active = make_repo_with_commits(&["work A", "work B"]);
    let quiet = make_repo_with_commits(&["yesterday work"]);
    // Re-run the quiet repo to add a commit dated today, but the test just
    // verifies aggregate.repos has both entries with proper commit counts.

    let specs = vec![
        RepoSpec::new(active.path()).unwrap(),
        RepoSpec::new(quiet.path()).unwrap(),
    ];
    let h = IntegHarness::new(MockOutcomeRunner::ok("combined summary", Some(0.01)));
    let req = RecapRequest {
        date: chrono::Utc::now().with_timezone(&tz()).date_naive(),
        timezone: tz(),
        repos: specs,
        runner_kind: "integ_mock".to_string(),
        timeout_ms: 30_000,
        prompt_override: None,
    };
    let outcome = roostery::daily_recap::run(req, h.rt()).await;
    let RecapOutcome::Summarized { aggregate, .. } = &outcome else {
        panic!("expected Summarized");
    };
    assert_eq!(aggregate.repos.len(), 2);
}

// ============================================================================
// Prepare (dry-run path)
// ============================================================================

#[tokio::test]
async fn integ_prepare_does_not_touch_registry_budget_journal() {
    let dir = make_repo_with_commits(&["c1"]);
    let spec = RepoSpec::new(dir.path()).unwrap();
    let h = IntegHarness::new(MockOutcomeRunner::ok("never called", None));
    let req = sample_req(spec, "integ_mock");

    let prepared = roostery::daily_recap::prepare(&req).unwrap();
    assert_eq!(prepared.aggregate.total_commits(), 1);
    assert!(prepared.markdown.contains("c1"));
    assert!(prepared.prompt.contains("c1"));
    // Budget file should not exist (no open)
    assert!(!h.budget_path.exists());
    // Journal dir should be empty (no append)
    let files: Vec<_> = std::fs::read_dir(&h.journal_dir)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        files.is_empty(),
        "journal dir should be empty after dry-run, got {} files",
        files.len()
    );
}
