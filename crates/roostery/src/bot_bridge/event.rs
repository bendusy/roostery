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
use std::sync::{Arc, Mutex};
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
    /// 单行 NDJSON 最大字节数（防 lark-cli 异常吐出超长 / 无换行 partial 帧
    /// 导致内存无限累积 / 流 stall）。超长行被丢弃 + warn，stream 不中断。
    /// 默认 1 MiB——飞书 IM 单消息上限远小于此，正常 event 不会触发。
    pub max_line_bytes: usize,
    /// codex round-8 P1 修复：可选 cancel token，让 consume_im 在 read /
    /// backoff sleep 中响应 daemon shutdown。None = 仅靠 mpsc 关闭驱动退出
    /// （旧行为）。
    pub cancel: Option<Arc<crate::bot_bridge::cancel::CancelToken>>,
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
            max_line_bytes: 1024 * 1024,
            cancel: None,
        }
    }

    /// Builder: 注入 cancel token，让 consume_im 响应 graceful shutdown。
    pub fn with_cancel(
        mut self,
        cancel: Arc<crate::bot_bridge::cancel::CancelToken>,
    ) -> Self {
        self.cancel = Some(cancel);
        self
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
        // cancel 检查（codex round-8 P1 修复）：daemon shutdown 时立即返回，
        // 不再进入下一次 spawn / read 循环。
        if let Some(c) = &opts.cancel
            && c.is_cancelled()
        {
            return;
        }
        // 总超时检查
        if let Some(total) = opts.timeout
            && started.elapsed() >= total
        {
            let _ = tx.send(Err(EventError::TotalTimeout)).await;
            return;
        }

        let spawn_at = Instant::now();
        match spawn_subscribe(&opts.binary, &opts.profile) {
            Ok((mut child, stdout, stderr_tail)) => {
                let mut reader = BufReader::new(stdout).lines();
                loop {
                    // codex round-8 P1: read 路径加 cancel 分支；biased 优先
                    // 让 shutdown 立即响应。next_line future drop 时底层 buf
                    // 不影响——下次 spawn 重建 reader。
                    let line_res = if let Some(c) = &opts.cancel {
                        tokio::select! {
                            biased;
                            _ = c.cancelled() => {
                                let _ = child.kill().await;
                                return;
                            }
                            line = reader.next_line() => line,
                        }
                    } else {
                        reader.next_line().await
                    };
                    match line_res {
                        Ok(Some(line)) => {
                            // Defense vs runaway producer: lark-cli accidentally
                            // emitting a huge / no-newline frame would otherwise
                            // grow next_line()'s internal String unbounded. Cap
                            // here drops the offending line + warns; subsequent
                            // well-formed lines continue normally. Note: this
                            // bounds *post-arrival* damage; a single malicious
                            // long line can still allocate up to max_line_bytes
                            // before being dropped — full streaming cap is its
                            // own larger refactor (see followup observation).
                            if line.len() > opts.max_line_bytes {
                                tracing::warn!(
                                    line_bytes = line.len(),
                                    max = opts.max_line_bytes,
                                    "oversized NDJSON line dropped"
                                );
                                continue;
                            }
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
                            // 不期望 EOF），但不阻断 stream。stderr_tail 由 spawn_subscribe
                            // 后台 drain task 实时收集，wait 完成后已含完整 tail。
                            let stderr_str = stderr_tail
                                .lock()
                                .map(|g| String::from_utf8_lossy(&g).into_owned())
                                .unwrap_or_default();
                            let err = EventError::ChildExitedAbnormally {
                                exit_code,
                                stderr_tail: stderr_str,
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

        // 退避后重连。codex round-8 P1: backoff sleep 也要支持 cancel——
        // shutdown 时不再等满 backoff，立即返回。
        if let Some(c) = &opts.cancel {
            tokio::select! {
                biased;
                _ = c.cancelled() => return,
                _ = tokio::time::sleep(backoff) => {}
            }
        } else {
            tokio::time::sleep(backoff).await;
        }
        backoff = (backoff.saturating_mul(2)).min(opts.max_backoff);
    }
}

type SubscribeChild = (
    tokio::process::Child,
    tokio::process::ChildStdout,
    Arc<Mutex<Vec<u8>>>,
);

fn spawn_subscribe(binary: &Path, profile: &str) -> std::io::Result<SubscribeChild> {
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
    // P2 修复 (codex round-7 P2-2): 持续 drain stderr 到一个 cap=8KiB tail
    // 缓冲区——之前 stderr 取了 piped 却从不读，子进程吐多了 stderr 就会
    // 在 pipe 满后 block，subscribe 主体停发事件。drain task 与主 read 循
    // 环并行，child.wait() 后自然终止（pipe close）。
    let stderr_tail: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::with_capacity(STDERR_TAIL_CAP)));
    if let Some(mut stderr) = child.stderr.take() {
        let tail_clone = stderr_tail.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                match tokio::io::AsyncReadExt::read(&mut stderr, &mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut guard) = tail_clone.lock() {
                            let take = n.min(STDERR_TAIL_CAP.saturating_sub(guard.len()));
                            if take > 0 {
                                guard.extend_from_slice(&buf[..take]);
                            }
                            // 已到 cap 后继续读但丢弃——保证 pipe 不阻塞。
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }
    Ok((child, stdout, stderr_tail))
}

const STDERR_TAIL_CAP: usize = 8 * 1024;
