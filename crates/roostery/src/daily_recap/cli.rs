//! CLI surface for `roostery daily-recap`.
//!
//! `--dry-run` calls [`super::prepare`] and prints markdown + prompt to stdout.
//! Live mode calls [`super::run`] and prints summary (or raw markdown on
//! degradation) or structured JSON via `--json`. See feature design §4.2.

use super::{PreparedRecap, RecapOutcome, RecapRequest, RecapRuntime, RepoSpec};
use crate::config;
use crate::dispatcher::runners::RunnerRegistry;
use crate::journal::Journal;
use crate::paths;
use chrono::{FixedOffset, NaiveDate};
use clap::Args;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct DailyRecapArgs {
    /// Override statistics date (default: today in local timezone).
    #[arg(long)]
    pub date: Option<String>,
    /// Override config.recap.repos (repeatable).
    #[arg(long = "repo")]
    pub repos: Vec<PathBuf>,
    /// Override config.recap.runner_kind.
    #[arg(long)]
    pub runner: Option<String>,
    /// Path to a prompt template file (overrides embedded default).
    #[arg(long)]
    pub prompt_override: Option<PathBuf>,
    /// Dry-run: print rendered prompt + markdown without invoking the runner /
    /// touching budget / writing journal.
    #[arg(long)]
    pub dry_run: bool,
    /// Output structured JSON (RecapJsonOutcome v1 schema) instead of plain text.
    #[arg(long)]
    pub json: bool,
}

/// Entry point invoked by `main.rs` for `Command::DailyRecap`. Returns
/// `ExitCode::SUCCESS` for Summarized / RawDump, `ExitCode::FAILURE` for
/// hard `Failed` outcomes (or CLI prep errors).
pub fn run(args: DailyRecapArgs) -> ExitCode {
    let cfg = match config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[daily-recap] config load failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    let req = match build_request(&args, &cfg) {
        Ok(r) => r,
        Err(msg) => {
            eprintln!("[daily-recap] {msg}");
            return ExitCode::FAILURE;
        }
    };

    if args.dry_run {
        return run_dry_run(req, args.json);
    }
    run_live(req, args.json, &cfg)
}

fn build_request(args: &DailyRecapArgs, cfg: &config::Config) -> Result<RecapRequest, String> {
    let now = chrono::Local::now();
    let timezone: FixedOffset = *now.offset();
    let date: NaiveDate = match &args.date {
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|e| format!("bad --date {s:?}: {e}"))?,
        None => now.date_naive(),
    };
    let runner_kind = args
        .runner
        .clone()
        .unwrap_or_else(|| cfg.recap.runner_kind.clone());

    let repos = if !args.repos.is_empty() {
        args.repos
            .iter()
            .map(|p| RepoSpec::new(p).map_err(|e| format!("invalid --repo {p:?}: {e}")))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        cfg.recap
            .repos
            .iter()
            .map(|r| match &r.name {
                Some(name) => RepoSpec::with_name(&r.path, name.clone())
                    .map_err(|e| format!("invalid config repo {:?}: {e}", r.path)),
                None => RepoSpec::new(&r.path)
                    .map_err(|e| format!("invalid config repo {:?}: {e}", r.path)),
            })
            .collect::<Result<Vec<_>, _>>()?
    };

    let prompt_override = match &args.prompt_override {
        Some(path) => {
            let body = super::load_template_from_path(path)
                .map_err(|e| format!("read prompt-override {path:?}: {e}"))?;
            Some(body)
        }
        None => match &cfg.recap.prompt_override_path {
            Some(path) => {
                let body = super::load_template_from_path(path)
                    .map_err(|e| format!("read configured prompt_override_path {path:?}: {e}"))?;
                Some(body)
            }
            None => None,
        },
    };

    let timeout_ms = if cfg.recap.timeout_ms > 0 {
        cfg.recap.timeout_ms
    } else {
        0 // run() will substitute DEFAULT_TIMEOUT_MS
    };

    Ok(RecapRequest {
        date,
        timezone,
        repos,
        runner_kind,
        timeout_ms,
        prompt_override,
    })
}

fn run_dry_run(req: RecapRequest, json: bool) -> ExitCode {
    match super::prepare(&req) {
        Ok(PreparedRecap {
            aggregate,
            markdown,
            prompt,
        }) => {
            if json {
                // Minimal dry-run JSON: just metadata
                let v = serde_json::json!({
                    "outcome": "dry_run",
                    "date": aggregate.date.to_string(),
                    "repo_count": aggregate.repos.len(),
                    "commit_count": aggregate.total_commits(),
                    "prompt": prompt,
                });
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
            } else {
                println!("# git markdown\n");
                println!("{markdown}");
                println!("---\n# rendered prompt\n");
                println!("{prompt}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[daily-recap] dry-run failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_live(req: RecapRequest, json: bool, cfg: &config::Config) -> ExitCode {
    let tokio_rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[daily-recap] tokio init failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let registry = RunnerRegistry::with_defaults();
    let journal = Journal::open(cfg.journal.dir.clone());
    let budget_path = paths::budget_state_path();
    let rt = RecapRuntime {
        registry: &registry,
        journal: &journal,
        budget_cfg: &cfg.budgets.default,
        budget_path: &budget_path,
        trace_max_depth: cfg.trace.max_depth,
        budget_estimated_cost_usd: cfg.recap.budget_estimated_cost_usd,
    };

    let outcome = tokio_rt.block_on(super::run(req, rt));
    emit_outcome(&outcome, json)
}

fn emit_outcome(outcome: &RecapOutcome, json: bool) -> ExitCode {
    if json {
        let dto = super::RecapJsonOutcome::from(outcome);
        match serde_json::to_string_pretty(&dto) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("[daily-recap] json serialize failed: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        match outcome {
            RecapOutcome::Summarized {
                summary,
                runner_kind,
                cost_usd,
                duration_ms,
                ..
            } => {
                println!("{summary}");
                let cost = cost_usd
                    .map(|c| format!("${c:.4}"))
                    .unwrap_or_else(|| "n/a".to_string());
                eprintln!(
                    "[daily-recap] summarized via {runner_kind} ({duration_ms}ms, cost {cost})"
                );
            }
            RecapOutcome::RawDump {
                markdown, reason, ..
            } => {
                eprintln!("[daily-recap] summary unavailable ({reason:?}), printing raw markdown:");
                println!("{markdown}");
            }
            RecapOutcome::Failed(err) => {
                eprintln!("[daily-recap] FAILED: {err}");
                return ExitCode::FAILURE;
            }
        }
    }
    match outcome {
        RecapOutcome::Failed(_) => ExitCode::FAILURE,
        _ => ExitCode::SUCCESS,
    }
}


