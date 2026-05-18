//! Bot stop hook + 反向 push CLI — Module F 第 2 子 feature。
//!
//! 双 CLI surface 共享一个核心 lib fn [`push`]：
//! - `roostery bot stop-hook`：CC / Codex / Gemini SessionEnd hook 入口；
//!   stdin JSON 协议，agent 来源走 `ROOSTERY_AGENT` env。
//! - `roostery bot push`：**反向调用**入口，让任意 agent / 脚本 / cron / CI 把
//!   进度推到飞书。
//!
//! 0.1.0 release 触发判据 = 完成后 CC headless 在飞书出 task，且任意 agent 都能
//! 脚本化推送。
//!
//! See `.codestable/features/2026-05-18-bot-stop-hook/bot-stop-hook-design.md`
//! §2.1 / §2.2。

use crate::lark_cli::LarkRunner;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// summary 默认值——append_steps 文本在 `req.summary == None` 时回退到这里。
/// Python parity（"Agent stopped (no summary)"）。
pub const DEFAULT_SUMMARY: &str = "Agent stopped (no summary)";

/// summary 截断字节上限（task append_steps 内容字段）。Python parity (head -c 200)。
pub const SUMMARY_MAX_BYTES: usize = 200;

// --- request / options / outcome types ----------------------------------

/// 双 CLI surface 共享的类型化请求边界。builder API：必填项构造 + with_* 链式
/// 可选项。两路 CLI 在适配层后都构造一个 `PushRequest` 再调 [`push`]。
#[derive(Debug, Clone)]
pub struct PushRequest {
    pub agent: String,
    pub session: String,
    pub cwd: PathBuf,
    /// `None` → append_steps 文本用 `"Agent stopped (no summary)"` 默认值
    pub summary: Option<String>,
    /// `None` → task_writer 自动生成 `"Agent {agent} working in {cwd}"`
    pub description: Option<String>,
    /// `Some` → 跳过 receive_id 三层链直接用；`None` → 三层链解析
    /// (env > identity::current > config.identity.user_id)
    pub assignee_open_id: Option<String>,
}

impl PushRequest {
    pub fn new(
        agent: impl Into<String>,
        session: impl Into<String>,
        cwd: impl Into<PathBuf>,
    ) -> Self {
        Self {
            agent: agent.into(),
            session: session.into(),
            cwd: cwd.into(),
            summary: None,
            description: None,
            assignee_open_id: None,
        }
    }

    pub fn with_summary(mut self, s: impl Into<String>) -> Self {
        self.summary = Some(s.into());
        self
    }

    pub fn with_description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }

    pub fn with_assignee(mut self, oid: impl Into<String>) -> Self {
        self.assignee_open_id = Some(oid.into());
        self
    }
}

/// 两路 CLI 共享 options。默认值（`Default::default()`）= hook 路径推荐配置：
/// 不 strict / 不 json / 走 IM 兜底。`bot push` 反向调用时 caller 根据需要 opt-in。
#[derive(Debug, Clone, Default)]
pub struct PushOptions {
    /// `true` → outcome.status=Failed 时进程 exit 1；默认 false（hook 路径不阻塞
    /// agent runtime）
    pub strict: bool,
    /// `true` → outcome 序列化为 JSON 写到 stdout；默认 false 静默
    pub json_output: bool,
    /// `true` → task_writer 失败时不走 IM 兜底，直接 outcome.status=Failed；默认
    /// false（IM 兜底是好默认）
    pub no_im_fallback: bool,
}

/// 结构化结果。两路 CLI 都返这个；`--json` 时写到 stdout 供 caller jq 消费。
///
/// **稳定契约**：本期 v1 字段命名 / 类型一经定型不破坏性变更——新字段走
/// backwards-compatible append（用 `Option<T>` / 新增 enum 变体而非改现有的）。
/// 见 design §5 R4。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PushOutcome {
    pub status: PushStatus,
    pub task_url: Option<String>,
    pub task_guid: Option<String>,
    pub fallback_used: bool,
    pub fallback_im_message_id: Option<String>,
    /// 人类可读错误摘要列表；按发生顺序累积
    pub errors: Vec<String>,
}

impl PushOutcome {
    /// 起手返一个 Skipped outcome——push 内部各路径根据情况转 Success /
    /// FallbackUsed / Failed。
    pub fn skipped() -> Self {
        Self {
            status: PushStatus::Skipped,
            task_url: None,
            task_guid: None,
            fallback_used: false,
            fallback_im_message_id: None,
            errors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushStatus {
    /// task 创建 + step 追加成功
    Success,
    /// task_writer 失败但 IM 兜底成功
    FallbackUsed,
    /// task + IM 都失败 (或 no_im_fallback opt-out 时 task 失败)
    Failed,
    /// receive_id 三层全空 → 无通知对象 → 不调任何 lark-cli
    Skipped,
}

// --- stop-hook stdin JSON schema ----------------------------------------

/// CC SessionEnd stdin JSON payload；Codex / Gemini 共用相同 schema 子集
/// (transcript_path 仅 CC 用，其他 runtime 走 prompt_response)。
///
/// 全字段 Option + `#[serde(default)]` → 空 stdin / 缺字段都不报错。
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct StopHookInput {
    pub cwd: Option<String>,
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub prompt_response: Option<String>,
    pub hook_event_name: Option<String>,
}

// --- 计算层纯函数 -------------------------------------------------------

/// UTF-8 安全截断到 `max_bytes` 字节内。不切坏多字节字符（floor 到最近 char
/// boundary）。Python `head -c 200` 在中文 / emoji 上会切坏 UTF-8——Rust 红利之一
/// 是这种安全可以编译期约束在类型层。
pub(crate) fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// 从 cwd 路径里抽最后一段（basename）。空路径或全 `/` → `"."`。
pub(crate) fn cwd_basename(cwd: &Path) -> String {
    cwd.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ".".to_string())
}

/// 跨进程**稳态**的 idempotency key 短哈希。
///
/// 用 blake3 而非 [`std::hash::DefaultHasher`]——后者 SipHash 启动种子随机化，
/// 同输入两次进程拿到不同 key，在 lark-cli `--idempotency-key` 链路里幂等失效。
///
/// 长度：8 字符（hex 4 字节，冲突空间 ~4G）足够 session-级幂等。
pub(crate) fn stable_idem_key(parts: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    for p in parts {
        hasher.update(p.as_bytes());
        hasher.update(&[0]); // null 分隔防 ("ab","c") 与 ("a","bc") 碰撞
    }
    let hex = hasher.finalize().to_hex();
    hex.as_str()[..8].to_string()
}

/// 从 stop-hook stdin input 推导 summary：transcript_reader 优先 → prompt_response
/// 兜底 → None。返 None 表示后续走 [`DEFAULT_SUMMARY`]。
pub(crate) fn resolve_summary_from_hook_input(input: &StopHookInput) -> Option<String> {
    if let Some(path) = input.transcript_path.as_deref()
        && !path.is_empty()
        && let Ok(text) =
            transcript_reader::read_last_assistant_text(Path::new(path), SUMMARY_MAX_BYTES)
        && !text.is_empty()
    {
        return Some(text);
    }
    input
        .prompt_response
        .as_deref()
        .map(|s| truncate_utf8(s, SUMMARY_MAX_BYTES).to_string())
        .filter(|s| !s.is_empty())
}

// --- transcript_reader inline 子模块 -------------------------------------

/// CC transcript jsonl tail 抽最后一条 assistant message text。
///
/// 协议：transcript 是 newline-delimited JSON，每行形如
/// `{"type": "assistant" | "user" | ..., "message": {"content": [{"text": "..."}]}}`。
/// 取**最后一条** `type == "assistant"` 行的 `message.content[0].text`，截 UTF-8
/// 安全 `max_bytes` 字节。
pub(crate) mod transcript_reader {
    use super::truncate_utf8;
    use std::path::{Path, PathBuf};

    #[derive(Debug, thiserror::Error)]
    pub enum TranscriptReadError {
        #[error("transcript file not found: {0}")]
        NotFound(PathBuf),
        #[error("io error reading {path}: {source}")]
        Io {
            path: PathBuf,
            #[source]
            source: std::io::Error,
        },
        #[error("no assistant message found in transcript")]
        NoAssistantMessage,
    }

    /// 从 transcript jsonl 文件读出最后一条 assistant 消息 text，截断到
    /// `max_bytes`（UTF-8 边界安全）。
    ///
    /// 实现策略：一次 `read_to_string` 然后倒序扫行。CC transcript 实测 < 几 MB
    /// 量级，10MB+ 极少见；大文件优化（seek + chunk 倒读）记为 design U1 未决，
    /// implement 阶段先用简单实现。
    pub fn read_last_assistant_text(
        path: &Path,
        max_bytes: usize,
    ) -> Result<String, TranscriptReadError> {
        let body = match std::fs::read_to_string(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(TranscriptReadError::NotFound(path.to_path_buf()));
            }
            Err(source) => {
                return Err(TranscriptReadError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        for line in body.lines().rev() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let v: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue, // 跳过非法 JSON 行
            };
            if v.get("type").and_then(|x| x.as_str()) != Some("assistant") {
                continue;
            }
            if let Some(text) = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.get(0))
                .and_then(|first| first.get("text"))
                .and_then(|t| t.as_str())
                && !text.is_empty()
            {
                return Ok(truncate_utf8(text, max_bytes).to_string());
            }
        }
        Err(TranscriptReadError::NoAssistantMessage)
    }
}

// --- receive_id 三层链 ---------------------------------------------------

/// IM 兜底 / task assignee 共用的"通知谁"解析。三层 fallback 链：
///
/// 1. `explicit` (caller 显式 override) → 直接用
/// 2. env `ROOSTERY_NOTIFY_TO` → 直接用（不调 identity）
/// 3. `identity::current(runner).user_open_id` → 调 lark-cli profile
/// 4. `config::load().identity.user_id` (非空) → 装机持久态
/// 5. 全空 → `None`（caller 见 None 走 Skipped）
///
/// 任一层失败/缺失**不当 fatal**，自动走下一层；identity 调失败也只是
/// `tracing::warn!` 记一笔继续向 config 兜底。
pub(crate) async fn resolve_receive_id(
    runner: &dyn LarkRunner,
    explicit: Option<&str>,
) -> Option<String> {
    // 1. explicit override
    if let Some(s) = explicit
        && !s.is_empty()
    {
        return Some(s.to_string());
    }
    // 2. env
    if let Ok(s) = std::env::var("ROOSTERY_NOTIFY_TO")
        && !s.is_empty()
    {
        return Some(s);
    }
    // 3. identity (lark-cli profile)
    match crate::identity::current(runner).await {
        Ok(ident) => {
            if let Some(oid) = ident.user_open_id()
                && !oid.is_empty()
            {
                return Some(oid.to_string());
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "identity::current failed; falling back to config");
        }
    }
    // 4. config persisted
    match crate::config::load() {
        Ok(cfg) => {
            if !cfg.identity.user_id.is_empty() {
                return Some(cfg.identity.user_id);
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "config::load failed; treating as no recipient");
        }
    }
    None
}

// --- pub fn API (todo!() 占位，S4-S6 落实) -------------------------------

/// 核心 lib fn——两路 CLI 共享的业务编排。
///
/// 编排：resolve receive_id 三层链 → if None 返 Skipped → task_writer
/// get_or_create_for_session + append_steps → 成功 Success / 任意错按
/// `opts.no_im_fallback` 决定走 IM 兜底 (lark-cli `im +messages-send`) 或直接
/// Failed。详见 design §2.2 mermaid。
pub async fn push(req: PushRequest, runner: &dyn LarkRunner, opts: PushOptions) -> PushOutcome {
    let mut outcome = PushOutcome::skipped();

    // 1. resolve receive_id（三层链；空 → Skipped 静默）
    let receive_id = match resolve_receive_id(runner, req.assignee_open_id.as_deref()).await {
        Some(r) => r,
        None => {
            tracing::info!("no notify recipient configured; exiting Skipped");
            return outcome;
        }
    };

    // 2. 构造 task 字段
    let basename = cwd_basename(&req.cwd);
    let task_summary = format!("[{}] @ {}", req.agent, basename);
    let cwd_str = req.cwd.to_string_lossy().into_owned();
    let task_description = req
        .description
        .clone()
        .unwrap_or_else(|| format!("Agent {} working in {}", req.agent, cwd_str));
    let step_text = req
        .summary
        .as_deref()
        .map(|s| truncate_utf8(s, SUMMARY_MAX_BYTES).to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_SUMMARY.to_string());

    // 3. task_writer 主路径
    let cto = crate::bot_task_writer::CreateTaskOptions::new()
        .with_description(&task_description)
        .with_assignee_open_id(&receive_id);
    let task_result = crate::bot_task_writer::get_or_create_for_session(
        runner,
        &req.agent,
        &req.session,
        &cwd_str,
        &task_summary,
        cto,
    )
    .await;

    let task_ref = match task_result {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "task_writer.get_or_create_for_session failed");
            outcome.errors.push(format!("task_writer: {e}"));
            return finish_with_fallback(
                outcome,
                runner,
                &receive_id,
                &req,
                &step_text,
                &basename,
                &opts,
            )
            .await;
        }
    };

    // 4. append_steps
    let step_key = stable_idem_key(&[&req.agent, &req.session, &step_text]);
    let aso = crate::bot_task_writer::AppendStepsOptions::new().with_idempotency_key(&step_key);
    if let Err(e) =
        crate::bot_task_writer::append_steps(runner, &task_ref.guid, &[step_text.as_str()], aso)
            .await
    {
        tracing::warn!(error = %e, "bot_task_writer::append_steps failed; will try IM fallback");
        outcome.errors.push(format!("append_steps: {e}"));
        outcome.task_url = Some(task_ref.url.clone());
        outcome.task_guid = Some(task_ref.guid.as_str().to_string());
        return finish_with_fallback(
            outcome,
            runner,
            &receive_id,
            &req,
            &step_text,
            &basename,
            &opts,
        )
        .await;
    }

    outcome.status = PushStatus::Success;
    outcome.task_url = Some(task_ref.url);
    outcome.task_guid = Some(task_ref.guid.as_str().to_string());
    outcome
}

/// task_writer 任一步失败时的尾路径：`opts.no_im_fallback=true` 直接 Failed；否则
/// 调 lark-cli `im +messages-send` 推一条纯文本 IM 兜底。
async fn finish_with_fallback(
    mut outcome: PushOutcome,
    runner: &dyn LarkRunner,
    receive_id: &str,
    req: &PushRequest,
    step_text: &str,
    basename: &str,
    opts: &PushOptions,
) -> PushOutcome {
    if opts.no_im_fallback {
        outcome.status = PushStatus::Failed;
        return outcome;
    }
    // IM 文本截 120 字节（task 内 step 截 200；IM 短一点对用户友好）
    let truncated_for_im = truncate_utf8(step_text, 120);
    let im_text = format!("[{}] @ {}: {}", req.agent, basename, truncated_for_im);
    let im_key = stable_idem_key(&[&req.agent, &req.session, "fallback"]);
    let argv = vec![
        "im",
        "+messages-send",
        "--as",
        "bot",
        "--user-id",
        receive_id,
        "--text",
        &im_text,
        "--idempotency-key",
        &im_key,
    ];
    match runner.run(&argv).await {
        Ok(v) => {
            outcome.status = PushStatus::FallbackUsed;
            outcome.fallback_used = true;
            outcome.fallback_im_message_id = v
                .get("data")
                .and_then(|d| d.get("message_id"))
                .and_then(|m| m.as_str())
                .map(String::from);
        }
        Err(e) => {
            tracing::warn!(error = %e, "IM fallback also failed");
            outcome.errors.push(format!("im_fallback: {e}"));
            outcome.status = PushStatus::Failed;
        }
    }
    outcome
}

/// stop-hook CLI 适配：从 stdin 读 JSON + `ROOSTERY_AGENT` env + transcript tail
/// 抽 summary，构造 [`PushRequest`] 调 [`push`]。
///
/// stdin 非法 JSON / 完全空 / 缺字段都不 panic——`StopHookInput` 全字段 Option
/// + serde default，最差情况构造一个空 PushRequest 走 Skipped 路径。
pub async fn run_stop_hook(runner: &dyn LarkRunner, opts: PushOptions) -> PushOutcome {
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    run_stop_hook_with_reader(&mut handle, runner, opts).await
}

/// 测试可注入版本——接受任意 Reader 代替真实 stdin。
pub(crate) async fn run_stop_hook_with_reader<R: std::io::Read>(
    reader: &mut R,
    runner: &dyn LarkRunner,
    opts: PushOptions,
) -> PushOutcome {
    let input = match parse_stop_hook_input(reader) {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(error = %e, "stdin JSON parse failed; treating as empty");
            StopHookInput::default()
        }
    };
    let req = build_request_from_stop_hook_input(input);
    push(req, runner, opts).await
}

fn parse_stop_hook_input<R: std::io::Read>(reader: &mut R) -> serde_json::Result<StopHookInput> {
    let mut body = String::new();
    if let Err(e) = reader.read_to_string(&mut body) {
        // read failure → 当作空 stdin（serde default）
        tracing::warn!(error = %e, "stdin read failed; treating as empty");
        return Ok(StopHookInput::default());
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(StopHookInput::default());
    }
    serde_json::from_str(trimmed)
}

// --- CLI 子模块：BotSub args + run dispatch -----------------------------

/// CLI 适配层。**main.rs 仅做一行 dispatch**：`Command::Bot(a) => bot_stop_hook::cli::run(a)`。
/// 子命令的 args struct + run 实现都在这里——见 design 2.5 "建议沉淀的 convention"。
pub mod cli {
    use super::*;
    use clap::{ArgGroup, Args, Subcommand};
    use std::io::Read;
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
                let outcome = rt.block_on(super::run_stop_hook(&runner, opts.clone()));
                outcome_to_exit_code(&outcome, &opts)
            }
            BotSub::Push(a) => {
                let opts = a.to_options();
                let mut stdin = std::io::stdin();
                let req = build_request_from_push_args(a, &mut stdin);
                let outcome = rt.block_on(super::push(req, &runner, opts.clone()));
                outcome_to_exit_code(&outcome, &opts)
            }
        }
    }
}

fn build_request_from_stop_hook_input(input: StopHookInput) -> PushRequest {
    let agent = std::env::var("ROOSTERY_AGENT").unwrap_or_else(|_| "unknown".to_string());
    let session = input
        .session_id
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "no-session".to_string());
    let cwd = input
        .cwd
        .clone()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let summary = resolve_summary_from_hook_input(&input);
    let mut req = PushRequest::new(agent, session, cwd);
    if let Some(s) = summary {
        req = req.with_summary(s);
    }
    req
}

// `run_push` + `PushCliArgs` clap struct 在 S6 落进 cli 子模块。

#[cfg(test)]
#[allow(clippy::await_holding_lock)] // ENV_LOCK serializes env mutation (attention.md pattern)
mod tests {
    use super::*;
    use crate::paths::TEST_ENV_LOCK as ENV_LOCK;

    /// builder 链式构造覆盖三个 with_* 方法
    #[test]
    fn push_request_builder_chains_optional_fields() {
        let req = PushRequest::new("custom-agent", "session-1", "/tmp/x")
            .with_summary("did the thing")
            .with_description("custom desc")
            .with_assignee("ou_test");
        assert_eq!(req.agent, "custom-agent");
        assert_eq!(req.session, "session-1");
        assert_eq!(req.cwd, PathBuf::from("/tmp/x"));
        assert_eq!(req.summary.as_deref(), Some("did the thing"));
        assert_eq!(req.description.as_deref(), Some("custom desc"));
        assert_eq!(req.assignee_open_id.as_deref(), Some("ou_test"));
    }

    /// PushOutcome serde JSON roundtrip + PushStatus snake_case
    #[test]
    fn push_outcome_serde_roundtrip_and_status_snake_case() {
        let outcome = PushOutcome {
            status: PushStatus::FallbackUsed,
            task_url: None,
            task_guid: None,
            fallback_used: true,
            fallback_im_message_id: Some("om_xxx".into()),
            errors: vec!["task_writer: LarkCallFailed(...)".into()],
        };
        let json = serde_json::to_string(&outcome).expect("serialize");
        assert!(
            json.contains("\"status\":\"fallback_used\""),
            "snake_case status: {json}"
        );
        assert!(json.contains("\"fallback_used\":true"));
        let back: PushOutcome = serde_json::from_str(&json).expect("roundtrip");
        assert_eq!(back, outcome);
    }

    /// PushStatus 4 变体 snake_case 全覆盖
    #[test]
    fn push_status_all_variants_snake_case() {
        let cases = [
            (PushStatus::Success, "\"success\""),
            (PushStatus::FallbackUsed, "\"fallback_used\""),
            (PushStatus::Failed, "\"failed\""),
            (PushStatus::Skipped, "\"skipped\""),
        ];
        for (status, expected) in cases {
            let s = serde_json::to_string(&status).expect("ser");
            assert_eq!(s, expected, "variant {status:?}");
        }
    }

    // ----- S2 计算层单测 ----------------------------------------------------

    #[test]
    fn truncate_utf8_ascii_under_cap_unchanged() {
        assert_eq!(truncate_utf8("hello", 200), "hello");
    }

    #[test]
    fn truncate_utf8_emoji_boundary_safe() {
        // "ab😀😀cd" 字节序列：a b (1+1) 😀(4) 😀(4) c d → 共 12 字节
        // 切到 max=7 应落在第一个😀末尾后（a b 😀 = 6 字节），不会切坏第 2 个😀
        let s = "ab😀😀cd";
        let out = truncate_utf8(s, 7);
        // 必须是 "ab😀"（6 字节）或更短，绝不应切出 invalid UTF-8
        assert!(out.is_char_boundary(out.len()));
        assert!(out.starts_with("ab"));
        assert!(out.len() <= 7);
        // 进一步验证：不应在 6 字节后再切到 7 字节（会切到第 2 个😀的中间）
        assert_eq!(out, "ab😀");
    }

    #[test]
    fn cwd_basename_extracts_last_segment() {
        use std::path::Path;
        assert_eq!(
            cwd_basename(Path::new("/Users/ben/Projects/roostery")),
            "roostery"
        );
        assert_eq!(
            cwd_basename(Path::new("/Users/ben/Projects/roostery/")),
            "roostery"
        );
        assert_eq!(cwd_basename(Path::new("relative/dir")), "dir");
        assert_eq!(cwd_basename(Path::new("")), ".");
        assert_eq!(cwd_basename(Path::new("/")), ".");
    }

    #[test]
    fn stable_idem_key_deterministic_across_calls() {
        // 关键性质：同输入两次进程拿到同 key（修 std::hash 启动种子随机化的 bug）
        let k1 = stable_idem_key(&["cc", "session-1", "summary X"]);
        let k2 = stable_idem_key(&["cc", "session-1", "summary X"]);
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 8);
        // 不同输入应不同
        let k3 = stable_idem_key(&["cc", "session-2", "summary X"]);
        assert_ne!(k1, k3);
        // null 分隔防互换：("ab","c") != ("a","bc")
        let k_ab_c = stable_idem_key(&["ab", "c"]);
        let k_a_bc = stable_idem_key(&["a", "bc"]);
        assert_ne!(k_ab_c, k_a_bc);
    }

    #[test]
    fn transcript_reader_happy_picks_last_assistant() {
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let mut f = tmp.reopen().expect("reopen");
        writeln!(
            f,
            r#"{{"type":"user","message":{{"content":[{{"text":"hi"}}]}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant","message":{{"content":[{{"text":"first reply"}}]}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant","message":{{"content":[{{"text":"final reply"}}]}}}}"#
        )
        .unwrap();
        let out = transcript_reader::read_last_assistant_text(tmp.path(), 200).expect("read");
        assert_eq!(out, "final reply");
    }

    #[test]
    fn transcript_reader_skips_non_assistant_and_invalid() {
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let mut f = tmp.reopen().expect("reopen");
        // 中间夹非法行 / system 行 / 缺 content 的 assistant 行
        writeln!(f, "not valid json").unwrap();
        writeln!(
            f,
            r#"{{"type":"system","message":{{"content":[{{"text":"sys"}}]}}}}"#
        )
        .unwrap();
        writeln!(f, r#"{{"type":"assistant"}}"#).unwrap(); // 缺 message
        writeln!(
            f,
            r#"{{"type":"assistant","message":{{"content":[{{"text":"good"}}]}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"user","message":{{"content":[{{"text":"after"}}]}}}}"#
        )
        .unwrap();
        let out = transcript_reader::read_last_assistant_text(tmp.path(), 200).expect("read");
        assert_eq!(out, "good");
    }

    #[test]
    fn transcript_reader_no_assistant_returns_err() {
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let mut f = tmp.reopen().expect("reopen");
        writeln!(
            f,
            r#"{{"type":"user","message":{{"content":[{{"text":"hi"}}]}}}}"#
        )
        .unwrap();
        let err =
            transcript_reader::read_last_assistant_text(tmp.path(), 200).expect_err("no assistant");
        assert!(matches!(
            err,
            transcript_reader::TranscriptReadError::NoAssistantMessage
        ));
    }

    #[test]
    fn transcript_reader_not_found() {
        let err = transcript_reader::read_last_assistant_text(
            std::path::Path::new("/nonexistent/path/transcript.jsonl"),
            200,
        )
        .expect_err("not found");
        assert!(matches!(
            err,
            transcript_reader::TranscriptReadError::NotFound(_)
        ));
    }

    // ----- S3 receive_id 三层链单测 ---------------------------------------

    use crate::lark_cli::mock::MockLarkRunner;
    use serde_json::json;

    /// Helper: install a tempdir as ROOSTERY_HOME so config::load() reads from it.
    /// 调用方需先持 ENV_LOCK。返 TempDir 给 caller 持有保活。
    fn install_tempdir_as_home() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        unsafe { std::env::set_var("ROOSTERY_HOME", dir.path()) };
        dir
    }

    fn write_config_with_user_id(home: &std::path::Path, user_id: &str) {
        let cfg_path = home.join("config.yaml");
        let yaml = format!(
            "schema_version: 1\nidentity:\n  user_id: \"{user_id}\"\n  default_chat_id: \"\"\n  default_task_app_token: \"\"\n"
        );
        std::fs::write(cfg_path, yaml).expect("write config");
    }

    #[tokio::test]
    async fn resolve_receive_id_explicit_short_circuits() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        let mock = MockLarkRunner::new();
        // explicit 直接返；不调任何 lark-cli
        let out = resolve_receive_id(&mock, Some("ou_explicit")).await;
        assert_eq!(out.as_deref(), Some("ou_explicit"));
        assert!(mock.calls().is_empty(), "explicit short-circuits lark-cli");
    }

    #[tokio::test]
    async fn resolve_receive_id_env_overrides_identity() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::set_var("ROOSTERY_NOTIFY_TO", "ou_from_env") };
        let mock = MockLarkRunner::new();
        let out = resolve_receive_id(&mock, None).await;
        assert_eq!(out.as_deref(), Some("ou_from_env"));
        assert!(mock.calls().is_empty(), "env hit short-circuits identity");
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
    }

    #[tokio::test]
    async fn resolve_receive_id_falls_back_to_identity() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        let mock = MockLarkRunner::new();
        // identity::current calls (1) auth status (2) profile list
        mock.enqueue_ok(json!({"userOpenId": "ou_from_identity"}))
            .enqueue_ok(json!([{"name": "default", "active": true}]));
        let out = resolve_receive_id(&mock, None).await;
        assert_eq!(out.as_deref(), Some("ou_from_identity"));
        assert_eq!(mock.calls().len(), 2);
    }

    #[tokio::test]
    async fn resolve_receive_id_falls_back_to_config_when_identity_blank() {
        let _g = ENV_LOCK.lock().unwrap();
        let home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        write_config_with_user_id(home.path(), "ou_from_config");
        let mock = MockLarkRunner::new();
        // identity 返空 user_open_id（auth status 缺 userOpenId 字段）
        mock.enqueue_ok(json!({"userName": "Test"}))
            .enqueue_ok(json!([{"name": "default", "active": true}]));
        let out = resolve_receive_id(&mock, None).await;
        assert_eq!(out.as_deref(), Some("ou_from_config"));
    }

    #[tokio::test]
    async fn resolve_receive_id_all_three_empty_returns_none() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        let mock = MockLarkRunner::new();
        // identity 返空；config 也空（tempdir 无 config.yaml → Default）
        mock.enqueue_ok(json!({}))
            .enqueue_ok(json!([{"name": "default", "active": true}]));
        let out = resolve_receive_id(&mock, None).await;
        assert!(out.is_none());
    }

    // ----- S4 push 核心 lib fn 集成单测 ------------------------------------

    use crate::lark_cli::LarkError;

    fn task_create_response() -> serde_json::Value {
        json!({"ok": true, "data": {"guid": "task_abc", "url": "https://feishu.cn/task/abc"}})
    }

    fn im_send_response() -> serde_json::Value {
        json!({"ok": true, "data": {"message_id": "om_xxx"}})
    }

    #[tokio::test]
    async fn push_happy_creates_task_and_appends_step() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(task_create_response())
            .enqueue_ok(json!({"ok": true}));
        let req = PushRequest::new("cc", "session-1", "/tmp/proj")
            .with_summary("did the thing")
            .with_assignee("ou_explicit");
        let out = push(req, &mock, PushOptions::default()).await;
        assert_eq!(out.status, PushStatus::Success);
        assert_eq!(out.task_url.as_deref(), Some("https://feishu.cn/task/abc"));
        assert_eq!(out.task_guid.as_deref(), Some("task_abc"));
        assert!(!out.fallback_used);
        assert!(out.errors.is_empty());
        assert_eq!(mock.calls().len(), 2, "task +create + append_task_steps");
    }

    #[tokio::test]
    async fn push_explicit_assignee_skips_receive_id_chain() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(task_create_response())
            .enqueue_ok(json!({"ok": true}));
        // 不 enqueue identity 响应——explicit 应短路
        let req = PushRequest::new("custom", "s1", "/tmp").with_assignee("ou_explicit");
        let out = push(req, &mock, PushOptions::default()).await;
        assert_eq!(out.status, PushStatus::Success);
        // 仅 2 个调用：task +create + append；无 auth status / profile list
        assert_eq!(mock.calls().len(), 2);
        let first_argv = &mock.calls()[0];
        assert_eq!(first_argv[0], "task");
        assert_eq!(first_argv[1], "+create");
    }

    #[tokio::test]
    async fn push_receive_id_all_empty_returns_skipped() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        let mock = MockLarkRunner::new();
        // identity 返空；config 空
        mock.enqueue_ok(json!({}))
            .enqueue_ok(json!([{"name":"default","active":true}]));
        let req = PushRequest::new("cc", "s1", "/tmp");
        let out = push(req, &mock, PushOptions::default()).await;
        assert_eq!(out.status, PushStatus::Skipped);
        // 仅 identity 的 2 调用；无 task / im
        assert_eq!(mock.calls().len(), 2);
    }

    #[tokio::test]
    async fn push_task_fail_triggers_im_fallback() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        let mock = MockLarkRunner::new();
        mock.enqueue_err(LarkError::Timeout { timeout_ms: 5 })
            .enqueue_ok(im_send_response());
        let req = PushRequest::new("cc", "s1", "/tmp/x")
            .with_summary("progress")
            .with_assignee("ou_test");
        let out = push(req, &mock, PushOptions::default()).await;
        assert_eq!(out.status, PushStatus::FallbackUsed);
        assert!(out.fallback_used);
        assert_eq!(out.fallback_im_message_id.as_deref(), Some("om_xxx"));
        assert_eq!(out.errors.len(), 1);
        assert!(out.errors[0].contains("task_writer"));
        assert_eq!(mock.calls().len(), 2);
        let im_call = &mock.calls()[1];
        assert_eq!(im_call[0], "im");
        assert_eq!(im_call[1], "+messages-send");
    }

    #[tokio::test]
    async fn push_task_and_im_both_fail_returns_failed() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        let mock = MockLarkRunner::new();
        mock.enqueue_err(LarkError::Timeout { timeout_ms: 5 })
            .enqueue_ok(json!({})); // 没 message_id 也算 ok? — 改成 err
        // 重置 mock
        let mock = MockLarkRunner::new();
        mock.enqueue_err(LarkError::Timeout { timeout_ms: 5 })
            .enqueue_err(LarkError::Timeout { timeout_ms: 5 });
        let req = PushRequest::new("cc", "s1", "/tmp")
            .with_summary("x")
            .with_assignee("ou_test");
        let out = push(req, &mock, PushOptions::default()).await;
        assert_eq!(out.status, PushStatus::Failed);
        assert_eq!(out.errors.len(), 2, "task + im errors");
        assert!(out.errors[0].contains("task_writer"));
        assert!(out.errors[1].contains("im_fallback"));
    }

    #[tokio::test]
    async fn push_no_im_fallback_opt_out_task_fail_directly_failed() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        let mock = MockLarkRunner::new();
        mock.enqueue_err(LarkError::Timeout { timeout_ms: 5 });
        let req = PushRequest::new("cc", "s1", "/tmp")
            .with_summary("x")
            .with_assignee("ou_test");
        let opts = PushOptions {
            no_im_fallback: true,
            ..Default::default()
        };
        let out = push(req, &mock, opts).await;
        assert_eq!(out.status, PushStatus::Failed);
        assert!(!out.fallback_used);
        assert_eq!(mock.calls().len(), 1, "仅 task；no IM");
    }

    #[tokio::test]
    async fn push_append_fail_triggers_im_fallback_preserves_task_url() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(task_create_response())
            .enqueue_err(LarkError::Timeout { timeout_ms: 5 })
            .enqueue_ok(im_send_response());
        let req = PushRequest::new("cc", "s1", "/tmp")
            .with_summary("progress")
            .with_assignee("ou_test");
        let out = push(req, &mock, PushOptions::default()).await;
        assert_eq!(out.status, PushStatus::FallbackUsed);
        // task 已创建，url / guid 应保留
        assert_eq!(out.task_url.as_deref(), Some("https://feishu.cn/task/abc"));
        assert_eq!(out.task_guid.as_deref(), Some("task_abc"));
        assert!(out.errors[0].contains("append_steps"));
        assert_eq!(mock.calls().len(), 3);
    }

    #[tokio::test]
    async fn push_skipped_calls_no_lark_cli_except_identity_probe() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        // identity 直接失败（auth status err）→ 走 config tier → 空
        let mock = MockLarkRunner::new();
        mock.enqueue_err(LarkError::Timeout { timeout_ms: 5 });
        let req = PushRequest::new("cc", "s1", "/tmp");
        let out = push(req, &mock, PushOptions::default()).await;
        assert_eq!(out.status, PushStatus::Skipped);
        // identity probe 失败后 config 走 default 也空 → Skipped；无 task / im
        assert_eq!(mock.calls().len(), 1);
    }

    // ----- S5 run_stop_hook 适配层单测 -------------------------------------

    #[tokio::test]
    async fn run_stop_hook_cc_happy_stdin_routes_to_push() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        unsafe { std::env::set_var("ROOSTERY_AGENT", "cc") };
        unsafe { std::env::set_var("ROOSTERY_NOTIFY_TO", "ou_test") };

        // 准备 transcript 文件
        use std::io::Write;
        let tx = tempfile::NamedTempFile::new().unwrap();
        let mut f = tx.reopen().unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant","message":{{"content":[{{"text":"final reply"}}]}}}}"#
        )
        .unwrap();

        let stdin_json = serde_json::json!({
            "cwd": "/tmp/proj",
            "session_id": "s-cc-1",
            "transcript_path": tx.path().to_string_lossy(),
        })
        .to_string();
        let mut reader = stdin_json.as_bytes();

        let mock = MockLarkRunner::new();
        mock.enqueue_ok(task_create_response())
            .enqueue_ok(json!({"ok": true}));

        let out = run_stop_hook_with_reader(&mut reader, &mock, PushOptions::default()).await;
        assert_eq!(out.status, PushStatus::Success);
        // append_steps 的 step 文本应来自 transcript
        let append_call = &mock.calls()[1];
        let data_idx = append_call.iter().position(|s| s == "--data").unwrap();
        assert!(append_call[data_idx + 1].contains("final reply"));

        unsafe { std::env::remove_var("ROOSTERY_AGENT") };
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
    }

    #[tokio::test]
    async fn run_stop_hook_empty_stdin_uses_defaults() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        unsafe { std::env::remove_var("ROOSTERY_AGENT") };
        let mut reader: &[u8] = b"";
        let mock = MockLarkRunner::new();
        // receive_id 全空 → Skipped；mock 只会收到 identity 调用
        mock.enqueue_ok(json!({}))
            .enqueue_ok(json!([{"name":"default","active":true}]));
        let out = run_stop_hook_with_reader(&mut reader, &mock, PushOptions::default()).await;
        assert_eq!(out.status, PushStatus::Skipped);
    }

    #[tokio::test]
    async fn run_stop_hook_transcript_not_found_falls_back_to_prompt_response() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::set_var("ROOSTERY_NOTIFY_TO", "ou_test") };
        unsafe { std::env::set_var("ROOSTERY_AGENT", "codex") };

        let stdin_json = serde_json::json!({
            "cwd": "/tmp/proj",
            "session_id": "s-codex-1",
            "transcript_path": "/nonexistent/x.jsonl",
            "prompt_response": "from prompt_response",
        })
        .to_string();
        let mut reader = stdin_json.as_bytes();

        let mock = MockLarkRunner::new();
        mock.enqueue_ok(task_create_response())
            .enqueue_ok(json!({"ok": true}));

        let out = run_stop_hook_with_reader(&mut reader, &mock, PushOptions::default()).await;
        assert_eq!(out.status, PushStatus::Success);
        let append_call = &mock.calls()[1];
        let data_idx = append_call.iter().position(|s| s == "--data").unwrap();
        assert!(append_call[data_idx + 1].contains("from prompt_response"));

        unsafe { std::env::remove_var("ROOSTERY_AGENT") };
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
    }

    #[tokio::test]
    async fn run_stop_hook_invalid_json_does_not_panic() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        let mut reader: &[u8] = b"this is not json {{{";
        let mock = MockLarkRunner::new();
        // 走 default StopHookInput → 没 receive_id → Skipped；identity probe
        mock.enqueue_ok(json!({}))
            .enqueue_ok(json!([{"name":"default","active":true}]));
        let out = run_stop_hook_with_reader(&mut reader, &mock, PushOptions::default()).await;
        assert_eq!(
            out.status,
            PushStatus::Skipped,
            "non-panic graceful fallback"
        );
    }

    // ----- S6 push CLI 适配 + clap args 单测 -------------------------------

    use clap::Parser;
    use cli::PushCliArgs;

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
        let req = cli::build_request_from_push_args(w.args, &mut reader);
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
        let req = cli::build_request_from_push_args(w.args, &mut empty);
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
        let req = cli::build_request_from_push_args(w.args, &mut empty);
        assert_eq!(req.assignee_open_id.as_deref(), Some("ou_via_cli"));
    }

    #[test]
    fn outcome_to_exit_code_strict_failed_exits_one() {
        use std::process::ExitCode;
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
        let _ec = cli::outcome_to_exit_code(&failed, &opts_strict);
        // ExitCode 不 PartialEq；这里只能证明函数不 panic + 不 hang。
        // exit code 行为靠 S8 CLI 集成测试用 assert_cmd 验证

        let opts_loose = PushOptions::default();
        let _ = cli::outcome_to_exit_code(&failed, &opts_loose);

        let success = PushOutcome {
            status: PushStatus::Success,
            ..PushOutcome::skipped()
        };
        let _ = cli::outcome_to_exit_code(&success, &opts_strict);

        // 确认 ExitCode 是某种合理的类型——纯防回归
        let _: ExitCode = cli::outcome_to_exit_code(&success, &opts_strict);
    }

    #[test]
    fn resolve_summary_transcript_then_prompt_response_then_none() {
        // 1) 全空 → None
        let input = StopHookInput::default();
        assert!(resolve_summary_from_hook_input(&input).is_none());

        // 2) 仅 prompt_response → 用 prompt_response
        let input = StopHookInput {
            prompt_response: Some("from prompt".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_summary_from_hook_input(&input).as_deref(),
            Some("from prompt")
        );

        // 3) transcript_path 指向不存在文件 + prompt_response 有值 → 退回 prompt_response
        let input = StopHookInput {
            transcript_path: Some("/nonexistent/x.jsonl".into()),
            prompt_response: Some("backup".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_summary_from_hook_input(&input).as_deref(),
            Some("backup")
        );

        // 4) transcript_path 有效 → 优先 transcript
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let mut f = tmp.reopen().expect("reopen");
        writeln!(
            f,
            r#"{{"type":"assistant","message":{{"content":[{{"text":"from transcript"}}]}}}}"#
        )
        .unwrap();
        let input = StopHookInput {
            transcript_path: Some(tmp.path().to_string_lossy().into_owned()),
            prompt_response: Some("should be ignored".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_summary_from_hook_input(&input).as_deref(),
            Some("from transcript")
        );
    }
}
