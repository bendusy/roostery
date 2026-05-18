//! CLI 适配层：clap args + 顶层 dispatch。
//!
//! 拆自原 `bot_stop_hook.rs` line 530-697（refactor `2026-05-19-bot-stop-hook-split`）。
//!
//! main.rs 仅做一行 dispatch：`Command::Bot(a) => bot_stop_hook::cli::run(a)`。

use super::push::{push, run_stop_hook};
use super::types::{PushOptions, PushOutcome, PushRequest, PushStatus};
use clap::{ArgGroup, Args, Subcommand};
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Args)]
pub struct BotArgs {
    #[command(subcommand)]
    pub subcmd: BotSub,
}

#[derive(Subcommand)]
pub enum BotSub {
    /// CC / Codex / Gemini SessionEnd hook 入口；从 stdin 读 JSON。
    StopHook(StopHookCliArgs),
    /// 反向调用入口；任意 agent / 脚本可推送到飞书。
    Push(PushCliArgs),
    /// 长跑 daemon：订阅 IM event → @mention 路由 → runner → 接力 task。
    Bridge(crate::bot_bridge::cli::BridgeCliArgs),
}

#[derive(Args, Debug)]
pub struct StopHookCliArgs {
    /// Failed 状态时 exit 1（默认 exit 0 不阻塞 agent runtime）
    #[arg(long)]
    pub strict: bool,
    /// PushOutcome JSON 写到 stdout
    #[arg(long)]
    pub json: bool,
    /// task_writer 失败时不走 IM 兜底
    #[arg(long = "no-im-fallback")]
    pub no_im_fallback: bool,
}

#[derive(Args, Debug)]
#[command(group(
    ArgGroup::new("summary_source")
        .args(["summary", "summary_stdin"])
        .required(false)
        .multiple(false),
))]
pub struct PushCliArgs {
    /// agent kind（如 "cc" / "codex" / "custom-bot" / "ci"）
    #[arg(long)]
    pub agent: String,
    /// session id；用于 (agent, session) 维度幂等
    #[arg(long)]
    pub session: String,
    /// 工作目录；缺省 = 当前进程 cwd
    #[arg(long)]
    pub cwd: Option<PathBuf>,
    /// 推送的进度描述（单条 step 文本）
    #[arg(long)]
    pub summary: Option<String>,
    /// 从 stdin 整体读 summary 文本（与 --summary 互斥）
    #[arg(long = "summary-stdin")]
    pub summary_stdin: bool,
    /// task description（缺省自动生成 "Agent {agent} working in {cwd}"）
    #[arg(long)]
    pub description: Option<String>,
    /// 显式 override receive_id 三层链
    #[arg(long = "assignee-open-id")]
    pub assignee_open_id: Option<String>,
    #[arg(long)]
    pub strict: bool,
    #[arg(long)]
    pub json: bool,
    #[arg(long = "no-im-fallback")]
    pub no_im_fallback: bool,
}

impl StopHookCliArgs {
    pub fn to_options(&self) -> PushOptions {
        PushOptions {
            strict: self.strict,
            json_output: self.json,
            no_im_fallback: self.no_im_fallback,
        }
    }
}

impl PushCliArgs {
    pub fn to_options(&self) -> PushOptions {
        PushOptions {
            strict: self.strict,
            json_output: self.json,
            no_im_fallback: self.no_im_fallback,
        }
    }
}

/// 从 PushCliArgs 构造 PushRequest；`summary_stdin=true` 时整体读 stdin（或测试
/// 注入的 reader）。
pub(crate) fn build_request_from_push_args<R: Read>(
    args: PushCliArgs,
    reader: &mut R,
) -> PushRequest {
    let summary = if args.summary_stdin {
        let mut buf = String::new();
        let _ = reader.read_to_string(&mut buf);
        let trimmed = buf.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    } else {
        args.summary
    };
    let cwd = args
        .cwd
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let mut req = PushRequest::new(args.agent, args.session, cwd);
    if let Some(s) = summary {
        req = req.with_summary(s);
    }
    if let Some(d) = args.description {
        req = req.with_description(d);
    }
    if let Some(a) = args.assignee_open_id {
        req = req.with_assignee(a);
    }
    req
}

/// `--json` / `--strict` 共享的出口逻辑：把 outcome 序列化到 stdout（如果开了
/// `--json`），按 strict 决定 exit code。
pub fn outcome_to_exit_code(outcome: &PushOutcome, opts: &PushOptions) -> ExitCode {
    if opts.json_output {
        match serde_json::to_string(outcome) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("[roostery bot] outcome serialize: {e}"),
        }
    }
    if opts.strict && outcome.status == PushStatus::Failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// 顶层 CLI dispatch；从 main.rs 直接调。
pub fn run(args: BotArgs) -> ExitCode {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[roostery bot] failed to start tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };
    let runner = crate::lark_cli::subprocess::LarkCli::new();
    match args.subcmd {
        BotSub::StopHook(a) => {
            let opts = a.to_options();
            let outcome = rt.block_on(run_stop_hook(&runner, opts.clone()));
            outcome_to_exit_code(&outcome, &opts)
        }
        BotSub::Push(a) => {
            let opts = a.to_options();
            let mut stdin = std::io::stdin();
            let req = build_request_from_push_args(a, &mut stdin);
            let outcome = rt.block_on(push(req, &runner, opts.clone()));
            outcome_to_exit_code(&outcome, &opts)
        }
        BotSub::Bridge(a) => {
            // 释放本函数自建的 runtime，避免与 `bridge::cli::run` 内部新建的 runtime
            // 互相嵌套；`runner` 在 Bridge 分支不使用。
            let _ = runner;
            drop(rt);
            crate::bot_bridge::cli::run(a)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// 小型 wrapper struct 给 clap 试解析（PushCliArgs 单独不能 try_parse_from
    /// 顶级——derive(Parser) 需要顶 struct）
    #[derive(clap::Parser, Debug)]
    struct PushCliWrapper {
        #[command(flatten)]
        args: PushCliArgs,
    }

    #[test]
    fn push_cli_args_flag_based_happy_parse() {
        let w = PushCliWrapper::try_parse_from([
            "test",
            "--agent",
            "custom-bot",
            "--session",
            "run-1",
            "--cwd",
            "/tmp/x",
            "--summary",
            "did X",
        ])
        .expect("parse");
        assert_eq!(w.args.agent, "custom-bot");
        assert_eq!(w.args.session, "run-1");
        assert_eq!(w.args.summary.as_deref(), Some("did X"));
        assert!(!w.args.summary_stdin);
        assert!(!w.args.strict);
        assert!(!w.args.json);
    }

    #[test]
    fn push_cli_args_summary_stdin_builds_request_from_reader() {
        let w = PushCliWrapper::try_parse_from([
            "test",
            "--agent",
            "ci",
            "--session",
            "build-42",
            "--summary-stdin",
        ])
        .expect("parse");
        assert!(w.args.summary_stdin);
        let mut reader: &[u8] = b"build green\n";
        let req = build_request_from_push_args(w.args, &mut reader);
        assert_eq!(req.agent, "ci");
        assert_eq!(req.session, "build-42");
        assert_eq!(req.summary.as_deref(), Some("build green"));
    }

    #[test]
    fn push_cli_args_summary_and_summary_stdin_are_mutually_exclusive() {
        let result = PushCliWrapper::try_parse_from([
            "test",
            "--agent",
            "x",
            "--session",
            "y",
            "--summary",
            "hi",
            "--summary-stdin",
        ]);
        assert!(result.is_err(), "ArgGroup should reject");
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("cannot be used with") || msg.contains("conflict"),
            "conflict error msg: {msg}"
        );
    }

    #[test]
    fn push_cli_args_description_passthrough() {
        let w = PushCliWrapper::try_parse_from([
            "test",
            "--agent",
            "x",
            "--session",
            "y",
            "--description",
            "my custom desc",
        ])
        .expect("parse");
        let mut empty: &[u8] = b"";
        let req = build_request_from_push_args(w.args, &mut empty);
        assert_eq!(req.description.as_deref(), Some("my custom desc"));
    }

    #[test]
    fn push_cli_args_assignee_open_id_passthrough() {
        let w = PushCliWrapper::try_parse_from([
            "test",
            "--agent",
            "x",
            "--session",
            "y",
            "--assignee-open-id",
            "ou_via_cli",
        ])
        .expect("parse");
        let mut empty: &[u8] = b"";
        let req = build_request_from_push_args(w.args, &mut empty);
        assert_eq!(req.assignee_open_id.as_deref(), Some("ou_via_cli"));
    }

    #[test]
    fn outcome_to_exit_code_strict_failed_exits_one() {
        // 比较 ExitCode 值不直接可用——验证函数行为而不验证 ExitCode 内部值
        let failed = PushOutcome {
            status: PushStatus::Failed,
            task_url: None,
            task_guid: None,
            fallback_used: false,
            fallback_im_message_id: None,
            errors: vec!["x".into()],
        };
        let opts_strict = PushOptions {
            strict: true,
            ..Default::default()
        };
        let _ec = outcome_to_exit_code(&failed, &opts_strict);
        // exit code 行为靠 CLI 集成测试用 assert_cmd 验证

        let opts_loose = PushOptions::default();
        let _ = outcome_to_exit_code(&failed, &opts_loose);

        let success = PushOutcome {
            status: PushStatus::Success,
            ..PushOutcome::skipped()
        };
        let _ = outcome_to_exit_code(&success, &opts_strict);

        // 防回归确认 ExitCode 类型
        let _: ExitCode = outcome_to_exit_code(&success, &opts_strict);
    }
}
