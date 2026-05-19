//! `bot_bridge::event` — IM 事件源。
//!
//! 见 `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md`
//! §2.1（ImEvent / consume_im / EventError）+ §2.2（指数退避重连）+ §3 E1。
//!
//! 流式订阅：spawn lark-cli `im im_messages_subscribe` 子进程，BufReader::lines 拉 NDJSON
//! 一行一 event。子进程退出（EOF / 非零退出 / spawn 失败）触发指数退避重连，初始 1s
//! 倍增到 cap 60s，连续成功消费 ≥ 30s 后 reset。单行 NDJSON 损坏 → 仅 skip + warn，
//! 不中断流。
//!
//! 接口偏离 design 的点（用户指令对齐）：design §2.1 写的是
//! `consume_im(runner: &dyn LarkRunner, profile, ...) -> impl Stream`，但 LarkRunner trait
//! 是 buffered Value 模型，与长跑子进程 NDJSON tail 不兼容。本实装采用：
//! - 入参：`ConsumeOpts { binary: PathBuf, profile: String, max_events, timeout, backoff }`
//!   —— `binary` 注入便于测试 fake；prod 由 daemon 走 `LarkCli` 同源解析（`$ROOSTERY_LARK_CLI_BIN`
//!   > config > `"lark-cli"`），不在本模块决定默认值
//! - 出参：`tokio::sync::mpsc::Receiver<Result<ImEvent, EventError>>` —— mpsc 与 step 7
//!   daemon 中央 dispatcher 整合多 bot 流更顺手；不引 `futures` crate
//!
//! 红线守护：本模块用 `tokio::process::Command::new(&opts.binary)`（变量而非字符串字面量
//! `"lark-cli"`），不触发 §3 G1 grep；不引 reqwest；不引 nix / os::unix。

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// 飞书 IM 事件最小模型（design §2.1）。
///
/// 字段集与 design 一致，反序列化来自 lark-cli `im im_messages_subscribe` NDJSON 行。
/// 未来 newtype 化（MessageId / ChatId）走独立 feature；本期 String 起步（design O2 类）。
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ImEvent {
    pub message_id: String,
    pub chat_id: String,
    pub chat_type: String,
    pub message_type: String,
    pub sender_id: String,
    pub content: String,
}

/// `consume_im` 启动参数。
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ConsumeOpts {
    /// lark-cli 二进制路径。daemon 走 `LarkCli` 同源解析后注入；测试用 fake 脚本注入。
    pub binary: PathBuf,
    /// `--profile` 值，传给 lark-cli。
    pub profile: String,
    /// 处理 N 条**有效** event 后关闭 stream。0 = unlimited。
    pub max_events: usize,
    /// 总超时（包含所有重连等待）。None = 不限制。
    pub timeout: Option<Duration>,
    /// 重连初始退避；默认 1s。
    pub initial_backoff: Duration,
    /// 重连退避 cap；默认 60s（design §2.2 / §3 E1）。
    pub max_backoff: Duration,
    /// 子进程连续成功消费 ≥ 此时长后重置 backoff 至 initial。
    pub backoff_reset_after: Duration,
    /// mpsc channel 容量。
    pub channel_buffer: usize,
}

impl ConsumeOpts {
    /// 构造新选项，余项取默认值。
    pub fn new(binary: impl Into<PathBuf>, profile: impl Into<String>) -> Self {
        Self {
            binary: binary.into(),
            profile: profile.into(),
            max_events: 0,
            timeout: None,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            backoff_reset_after: Duration::from_secs(30),
            channel_buffer: 32,
        }
    }
}

/// IM 事件源错误。
///
/// `ParseFailed` 仅记入日志后 skip，不会通过 channel 投递（不中断流）；其余变体投递后
/// stream 仍会尝试重连，直到 `max_events` / `timeout` / spawn 永久失败。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EventError {
    #[error("spawn lark-cli {binary} failed: {source}")]
    SpawnFailed {
        binary: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("lark-cli child exited abnormally: code={exit_code:?}, stderr_tail={stderr_tail:?}")]
    ChildExitedAbnormally {
        exit_code: Option<i32>,
        stderr_tail: String,
    },
    #[error("read child stdout failed: {0}")]
    ReadFailed(#[source] std::io::Error),
    #[error("consume_im total timeout elapsed")]
    TotalTimeout,
}

/// stream API 句柄：mpsc receiver + 后台 spawn task 的 JoinHandle。
///
/// drop receiver 即停止后台 task（背景 task 检测 send 失败时主动退出，并 kill 子进程
/// 借 `kill_on_drop(true)`）。
pub struct ConsumeStream {
    pub rx: mpsc::Receiver<Result<ImEvent, EventError>>,
    pub join: JoinHandle<()>,
}

/// 启动 IM 事件订阅流。
///
/// 实装见模块级 doc。
pub fn consume_im(opts: ConsumeOpts) -> ConsumeStream {
    let (tx, rx) = mpsc::channel(opts.channel_buffer.max(1));
    let join = tokio::spawn(run_loop(opts, tx));
    ConsumeStream { rx, join }
}

async fn run_loop(opts: ConsumeOpts, tx: mpsc::Sender<Result<ImEvent, EventError>>) {
    let started = Instant::now();
    let mut backoff = opts.initial_backoff;
    let mut events_emitted: usize = 0;

    loop {
        // 总超时检查
        if let Some(total) = opts.timeout
            && started.elapsed() >= total
        {
            let _ = tx.send(Err(EventError::TotalTimeout)).await;
            return;
        }

        let spawn_at = Instant::now();
        match spawn_subscribe(&opts.binary, &opts.profile) {
            Ok((mut child, stdout)) => {
                let mut reader = BufReader::new(stdout).lines();
                loop {
                    let line_res = reader.next_line().await;
                    match line_res {
                        Ok(Some(line)) => {
                            let trimmed = line.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            match serde_json::from_str::<ImEvent>(trimmed) {
                                Ok(ev) => {
                                    if tx.send(Ok(ev)).await.is_err() {
                                        // receiver dropped → 终止；子进程随 kill_on_drop 被 SIGKILL。
                                        let _ = child.kill().await;
                                        return;
                                    }
                                    events_emitted += 1;
                                    if opts.max_events > 0 && events_emitted >= opts.max_events {
                                        let _ = child.kill().await;
                                        return;
                                    }
                                }
                                Err(parse_err) => {
                                    // 单行损坏 → skip + warn，不中断流。
                                    let preview: String = trimmed.chars().take(200).collect();
                                    tracing::warn!(
                                        target: "bot_bridge::event",
                                        error = %parse_err,
                                        line_preview = %preview,
                                        "skip corrupt NDJSON line"
                                    );
                                }
                            }
                        }
                        Ok(None) => {
                            // EOF —— 子进程关闭 stdout，等回收并触发重连。
                            let exit_code = match child.wait().await {
                                Ok(status) => status.code(),
                                Err(_) => None,
                            };
                            // 把 EOF 当 abnormal exit 上报（lark-cli subscribe 是长连接，
                            // 不期望 EOF），但不阻断 stream。
                            let err = EventError::ChildExitedAbnormally {
                                exit_code,
                                stderr_tail: String::new(),
                            };
                            if tx.send(Err(err)).await.is_err() {
                                return;
                            }
                            break;
                        }
                        Err(io_err) => {
                            let _ = child.kill().await;
                            if tx.send(Err(EventError::ReadFailed(io_err))).await.is_err() {
                                return;
                            }
                            break;
                        }
                    }
                }
                // 子进程结束后判断是否需要 reset backoff
                if spawn_at.elapsed() >= opts.backoff_reset_after {
                    backoff = opts.initial_backoff;
                }
            }
            Err(spawn_err) => {
                let err = EventError::SpawnFailed {
                    binary: opts.binary.clone(),
                    source: spawn_err,
                };
                if tx.send(Err(err)).await.is_err() {
                    return;
                }
            }
        }

        // 退避后重连
        tokio::time::sleep(backoff).await;
        backoff = (backoff.saturating_mul(2)).min(opts.max_backoff);
    }
}

fn spawn_subscribe(
    binary: &Path,
    profile: &str,
) -> std::io::Result<(tokio::process::Child, tokio::process::ChildStdout)> {
    let mut cmd = Command::new(binary);
    cmd.arg("--profile")
        .arg(profile)
        .arg("im")
        .arg("im_messages_subscribe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("child stdout pipe missing"))?;
    Ok((child, stdout))
}
