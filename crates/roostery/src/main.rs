use clap::{Args, Parser, Subcommand};
use roostery::bot_stop_hook;
use roostery::config;
use roostery::dispatcher::hook_event::{HOOK_EVENT_SCHEMA_VERSION, HookEvent};
use roostery::dispatcher::rules;
use roostery::dispatcher::runners::RunnerRegistry;
use roostery::dispatcher::{self, DispatchError};
use roostery::hooks_merge::AgentKind;
use roostery::lark_cli::subprocess::LarkCli;
use roostery::onboarding::{self, InitOptions};
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

#[derive(Parser)]
#[command(
    name = "roostery",
    version = concat!(env!("CARGO_PKG_VERSION"), " (rust)"),
    about = "🪺 Vendor-neutral agent broker, Feishu-native.",
    long_about = None,
    disable_help_subcommand = true,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run lark-cli probe matrix; persist state and gate downstream features.
    Smoke,
    /// Install Roostery on this host: shim + hooks + shell rc patch.
    Init(InitArgs),
    /// Dispatcher main loop: fire / replay / test-rule.
    Dispatcher(DispatcherArgs),
    /// Bot bridge: stop-hook (passive) + push (reverse-call) — Module F.
    Bot(bot_stop_hook::cli::BotArgs),
}

#[derive(Args)]
struct DispatcherArgs {
    #[command(subcommand)]
    sub: DispatcherSub,
}

#[derive(Subcommand)]
enum DispatcherSub {
    /// Fire a HookEvent through the dispatcher main loop.
    /// Always exits 0; failures are journaled.
    Fire(FireArgs),
    /// Replay a journaled trace by trace_id; allocates a new trace_id.
    Replay(ReplayArgs),
    /// Dry-run the rule engine without invoking any runner.
    TestRule(TestRuleArgs),
}

#[derive(Args)]
struct FireArgs {
    /// Agent kind ("cc" / "codex" / "gemini" / ...); maps to hook_source "{agent}-stop".
    #[arg(long)]
    agent: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long)]
    summary: Option<String>,
    /// Read full HookEvent JSON from stdin (overrides flag inputs).
    #[arg(long)]
    stdin_event: bool,
    /// Print DispatchOutcome summary to stdout.
    #[arg(long)]
    verbose: bool,
}

#[derive(Args)]
struct ReplayArgs {
    #[arg(long)]
    trace: String,
    #[arg(long)]
    verbose: bool,
}

#[derive(Args)]
struct TestRuleArgs {
    #[arg(long)]
    agent: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long)]
    summary: Option<String>,
    #[arg(long)]
    stdin_event: bool,
}

#[derive(Args)]
struct InitArgs {
    /// Print what would be done without modifying any file.
    #[arg(long)]
    dry_run: bool,
    /// Skip a specific agent's hook installation. May be repeated.
    #[arg(long = "skip-agent", value_name = "AGENT")]
    skip_agents: Vec<String>,
    /// Explicit path to the real lark-cli binary, bypassing PATH search.
    /// Priority: this flag > `ROOSTERY_LARK_CLI_BIN` env > PATH search.
    /// Useful when npm-installed lark-cli (default `~/.local/bin/lark-cli`)
    /// collides with the shim install target at the same path.
    #[arg(long = "real-lark-cli", value_name = "PATH")]
    real_lark_cli: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        None => {
            println!(
                "roostery {} (rust) — see https://github.com/bendusy/roostery",
                roostery::VERSION
            );
            ExitCode::SUCCESS
        }
        Some(Command::Smoke) => {
            let report = roostery::smoke::run();
            match serde_json::to_string_pretty(&report) {
                Ok(s) => println!("{s}"),
                Err(e) => eprintln!("[roostery] failed to serialize smoke report: {e}"),
            }
            if report.all_ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Some(Command::Init(args)) => run_init(args),
        Some(Command::Dispatcher(args)) => run_dispatcher(args),
        Some(Command::Bot(args)) => bot_stop_hook::cli::run(args),
    }
}

fn run_dispatcher(args: DispatcherArgs) -> ExitCode {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[roostery dispatcher] failed to start tokio runtime: {e}");
            return ExitCode::from(1);
        }
    };
    match args.sub {
        DispatcherSub::Fire(a) => rt.block_on(run_fire(a)),
        DispatcherSub::Replay(a) => rt.block_on(run_replay(a)),
        DispatcherSub::TestRule(a) => run_test_rule(a),
    }
}

/// Synthesize a HookEvent from CLI flags (or read full event from stdin).
fn synth_hook_event(
    stdin_event: bool,
    agent: &Option<String>,
    session: &Option<String>,
    cwd: &Option<PathBuf>,
    summary: &Option<String>,
) -> Result<HookEvent, DispatchError> {
    if stdin_event {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| DispatchError::BadCliInput(format!("stdin read: {e}")))?;
        return serde_json::from_str(&buf)
            .map_err(|e| DispatchError::BadCliInput(format!("HookEvent JSON parse: {e}")));
    }
    let agent_kind = agent.as_deref().unwrap_or("unknown");
    let hook_source = format!("{agent_kind}-stop");
    let session_id = session.clone().unwrap_or_else(|| "no-session".to_string());
    let workspace = cwd.clone().unwrap_or_else(|| PathBuf::from("."));
    let trigger_meta = match summary {
        Some(s) => serde_json::json!({"summary": s}),
        None => serde_json::json!({}),
    };
    let raw = serde_json::json!({
        "schema_version": HOOK_EVENT_SCHEMA_VERSION,
        "hook_source": hook_source,
        "session_id": session_id,
        "workspace": workspace,
        "trigger_meta": trigger_meta,
        "trace": null,
    });
    serde_json::from_value(raw)
        .map_err(|e| DispatchError::BadCliInput(format!("HookEvent build: {e}")))
}

async fn run_fire(args: FireArgs) -> ExitCode {
    // fire 始终 exit 0；input parse 失败也走 journal 走不通就只能 eprintln
    let event = match synth_hook_event(
        args.stdin_event,
        &args.agent,
        &args.session,
        &args.cwd,
        &args.summary,
    ) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[roostery dispatcher fire] {e}");
            return ExitCode::SUCCESS;
        }
    };
    let cfg = match config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[roostery dispatcher fire] config load failed: {e}");
            return ExitCode::SUCCESS;
        }
    };
    let rules = match rules::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[roostery dispatcher fire] rules load failed: {e}");
            return ExitCode::SUCCESS;
        }
    };
    let registry = RunnerRegistry::with_defaults();
    let outcome = dispatcher::fire(event, &registry, &rules, &cfg).await;
    if args.verbose {
        print_outcome(&outcome);
    }
    ExitCode::SUCCESS
}

async fn run_replay(args: ReplayArgs) -> ExitCode {
    let cfg = match config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[roostery dispatcher replay] config load failed: {e}");
            return ExitCode::from(1);
        }
    };
    let rules = match rules::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[roostery dispatcher replay] rules load failed: {e}");
            return ExitCode::from(1);
        }
    };
    let registry = RunnerRegistry::with_defaults();
    match dispatcher::replay(&args.trace, &registry, &rules, &cfg).await {
        Ok(outcome) => {
            if args.verbose {
                print_outcome(&outcome);
            } else {
                println!("replay ok: new trace_id={}", outcome.trace_id);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[roostery dispatcher replay] {e}");
            ExitCode::from(1)
        }
    }
}

fn run_test_rule(args: TestRuleArgs) -> ExitCode {
    let event = match synth_hook_event(
        args.stdin_event,
        &args.agent,
        &args.session,
        &args.cwd,
        &args.summary,
    ) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[roostery dispatcher test-rule] {e}");
            return ExitCode::from(1);
        }
    };
    let rules = match rules::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[roostery dispatcher test-rule] rules load failed: {e}");
            return ExitCode::from(1);
        }
    };
    match dispatcher::test_rule(&event, &rules) {
        Some(m) => {
            println!(
                "MATCH: rule={} runner={} args={}",
                m.rule_name.as_str(),
                m.runner,
                serde_json::to_string(m.args).unwrap_or_else(|_| "<unserializable>".to_string()),
            );
        }
        None => println!("NO MATCH"),
    }
    ExitCode::SUCCESS
}

fn print_outcome(outcome: &dispatcher::DispatchOutcome) {
    println!("trace_id: {}", outcome.trace_id);
    println!("root_event_id: {}", outcome.root_event_id);
    println!("dispatched: {} step(s)", outcome.dispatched.len());
    for (i, step) in outcome.dispatched.iter().enumerate() {
        println!(
            "  [{i}] depth={} hook_source={} rule={:?} runner={:?} status={:?} fanout={}",
            step.depth,
            step.hook_source,
            step.matched_rule,
            step.runner_kind,
            step.status,
            step.fanout,
        );
    }
}

fn run_init(args: InitArgs) -> ExitCode {
    let skip_agents = match parse_skip_agents(&args.skip_agents) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("[roostery init] {msg}");
            return ExitCode::from(2);
        }
    };
    let opts = InitOptions {
        dry_run: args.dry_run,
        skip_agents,
        real_lark_cli_override: args.real_lark_cli,
    };
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[roostery init] failed to start tokio runtime: {e}");
            return ExitCode::from(1);
        }
    };
    let runner = LarkCli::new();
    let result = rt.block_on(onboarding::run(&runner, opts));
    match result {
        Ok(report) => {
            println!("{}", onboarding::format_report(&report));
            if report.had_errors() {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("[roostery init] {e}");
            ExitCode::from(1)
        }
    }
}

fn parse_skip_agents(raw: &[String]) -> Result<Vec<AgentKind>, String> {
    raw.iter()
        .map(|s| AgentKind::from_str(s).map_err(|e| e.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Top-level wrapper so InitArgs can be exercised via `try_parse_from`.
    #[derive(Parser)]
    struct InitWrapper {
        #[command(flatten)]
        args: InitArgs,
    }

    #[test]
    fn init_args_parses_real_lark_cli_flag() {
        let w = InitWrapper::try_parse_from(["test", "--real-lark-cli", "/opt/feishu/lark-cli"])
            .expect("parse");
        assert_eq!(
            w.args.real_lark_cli.as_deref(),
            Some(std::path::Path::new("/opt/feishu/lark-cli"))
        );
        assert!(!w.args.dry_run);
        assert!(w.args.skip_agents.is_empty());
    }
}
