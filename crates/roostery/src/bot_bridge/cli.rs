//! `bot_bridge::cli` — clap 适配 + 顶层 dispatch。
//!
//! 见 `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md`
//! §2.1 / §2.3（BridgeCliArgs 5 flags + `roostery bot bridge` 挂载点）。
//!
//! step 1：仅落 args 形状 + 把 BridgeOptions 转给 `daemon::run_bridge` 空实现。
//! 真正的运行时（lark-cli 注入 / Runner 注册表）由 step 4-7 接入。

use clap::Args;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use super::daemon::{BridgeOptions, run_bridge};

/// `roostery bot bridge` 的 clap args。
#[derive(Args, Debug)]
pub struct BridgeCliArgs {
    /// bots.yaml 路径；缺省 = `~/.roostery/bots.yaml`。
    #[arg(long, default_value = "~/.roostery/bots.yaml")]
    pub bots: PathBuf,
    /// `--profile` 可重复；空 = 跑 bots.yaml 全部 BotRole。
    #[arg(long)]
    pub profile: Vec<String>,
    /// per-bot handle_event 并发上限；默认 8。
    #[arg(long, default_value_t = 8)]
    pub max_concurrency: usize,
    /// 处理 N 条 event 后正常退出。0 = unlimited（step 1 表现为立即返回）。
    #[arg(long, default_value_t = 0)]
    pub max_events: usize,
    /// 单 event 处理总超时（秒）。不传 = 不限制。
    #[arg(long)]
    pub timeout: Option<u64>,
}

impl BridgeCliArgs {
    /// 把 CLI args 翻成 `BridgeOptions`；纯映射，方便后续 step 单测。
    pub fn to_options(&self) -> BridgeOptions {
        BridgeOptions {
            max_concurrency: self.max_concurrency,
            max_events: self.max_events,
            timeout: self.timeout.map(Duration::from_secs),
            profile_filter: self.profile.clone(),
            ..BridgeOptions::default()
        }
    }
}

/// 顶层 dispatch；由 `bot_stop_hook::cli::BotSub::Bridge` 转发到这里。
pub fn run(args: BridgeCliArgs) -> ExitCode {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[roostery bot bridge] failed to start tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };
    let opts = args.to_options();
    let bots_path = args.bots.clone();
    match rt.block_on(run_bridge(&bots_path, opts)) {
        Ok(_report) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[roostery bot bridge] {e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(clap::Parser, Debug)]
    struct Wrapper {
        #[command(flatten)]
        args: BridgeCliArgs,
    }

    #[test]
    fn bridge_cli_args_defaults_parse() {
        let w = Wrapper::try_parse_from(["test"]).expect("parse");
        assert_eq!(w.args.max_concurrency, 8);
        assert_eq!(w.args.max_events, 0);
        assert!(w.args.timeout.is_none());
        assert!(w.args.profile.is_empty());
    }

    #[test]
    fn bridge_cli_args_all_flags_parse() {
        let w = Wrapper::try_parse_from([
            "test",
            "--bots",
            "/tmp/bots.yaml",
            "--profile",
            "app_a",
            "--profile",
            "app_b",
            "--max-concurrency",
            "4",
            "--max-events",
            "10",
            "--timeout",
            "30",
        ])
        .expect("parse");
        assert_eq!(w.args.bots, PathBuf::from("/tmp/bots.yaml"));
        assert_eq!(
            w.args.profile,
            vec!["app_a".to_string(), "app_b".to_string()]
        );
        assert_eq!(w.args.max_concurrency, 4);
        assert_eq!(w.args.max_events, 10);
        assert_eq!(w.args.timeout, Some(30));
    }

    #[test]
    fn bridge_cli_args_to_options_roundtrip() {
        let w = Wrapper::try_parse_from(["test", "--timeout", "5"]).expect("parse");
        let opts = w.args.to_options();
        assert_eq!(opts.timeout, Some(Duration::from_secs(5)));
        assert_eq!(opts.max_events, 0);
    }
}
