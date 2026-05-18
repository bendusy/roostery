use clap::{Args, Parser, Subcommand};
use roostery::hooks_merge::AgentKind;
use roostery::lark_cli::subprocess::LarkCli;
use roostery::onboarding::{self, InitOptions};
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
}

#[derive(Args)]
struct InitArgs {
    /// Print what would be done without modifying any file.
    #[arg(long)]
    dry_run: bool,
    /// Skip a specific agent's hook installation. May be repeated.
    #[arg(long = "skip-agent", value_name = "AGENT")]
    skip_agents: Vec<String>,
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
