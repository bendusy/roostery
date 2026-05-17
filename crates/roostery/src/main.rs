use clap::{Parser, Subcommand};
use std::process::ExitCode;

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
    }
}
