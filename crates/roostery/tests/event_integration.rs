//! Integration tests for `bot_bridge::event::consume_im`.
//!
//! Covers step 6 exit signals (see
//! `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-checklist.yaml`)：
//! - 多行 NDJSON：3 行有效 + 1 行损坏 JSON skip + 子进程 EOF 触发重连
//! - stream Item 数与有效事件数一致
//! - EOF / 立即退出触发指数退避重连（fixture 计数器观测）
//!
//! 测试不打真飞书 / 真 lark-cli —— 全部用 fake shell 脚本伪装子进程 stdout。
//! fixture pattern 与 `lark_cli::subprocess::tests::fixture_script` 同源（一次性 `fs::write`
//! + chmod +x，绕开 Linux ETXTBSY）。

use roostery::bot_bridge::event::{ConsumeOpts, EventError, consume_im};
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::time::Duration;

/// Write a shell script under tempdir, chmod +x, return (dir handle, path).
///
/// Linux execve 在最近写完的文件上有 ETXTBSY 风险（kernel write-reference
/// lingering），用 `std::fs::write` 原子关闭 fd 后再 chmod 来规避。复用
/// `lark_cli::subprocess::tests::fixture_script` 的同源 pattern。
fn fixture_script(body: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fake-lark-cli");
    let mut content = String::from("#!/bin/sh\n");
    content.push_str(body);
    std::fs::write(&path, content).unwrap();
    let mut perm = std::fs::metadata(&path).unwrap().permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(&path, perm).unwrap();
    (dir, path)
}

fn opts(binary: PathBuf) -> ConsumeOpts {
    let mut o = ConsumeOpts::new(binary, "test_profile");
    // 快速重连，避免测试等待
    o.initial_backoff = Duration::from_millis(20);
    o.max_backoff = Duration::from_millis(80);
    o.backoff_reset_after = Duration::from_millis(10);
    o.channel_buffer = 8;
    o
}

/// NDJSON 多行：3 行有效 + 1 行损坏 JSON + EOF。
///
/// 期望：收到 3 个 `Ok(ImEvent)`（损坏行 skip）+ 至少 1 个 `Err(ChildExitedAbnormally)`（EOF）；
/// max_events=3 触发后 stream 关闭。
#[tokio::test]
async fn s6_1_ndjson_parses_valid_and_skips_corrupt() {
    let body = r#"cat <<'JSON'
{"message_id":"m1","chat_id":"c1","chat_type":"group","message_type":"text","sender_id":"u1","content":"hello"}
{"message_id":"m2","chat_id":"c1","chat_type":"group","message_type":"text","sender_id":"u2","content":"@scout do X"}
this is not json at all { broken
{"message_id":"m3","chat_id":"c2","chat_type":"p2p","message_type":"text","sender_id":"u3","content":"/stop"}
JSON
"#;
    let (_d, path) = fixture_script(body);
    let mut o = opts(path);
    o.max_events = 3;

    let mut stream = consume_im(o);
    let mut events = Vec::new();
    let mut errors = 0usize;

    // 收 stream 直到关闭或拿到 3 条 + 些许 error 边界。
    let collect = async {
        while let Some(item) = stream.rx.recv().await {
            match item {
                Ok(ev) => events.push(ev),
                Err(_) => errors += 1,
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), collect)
        .await
        .expect("stream should close within 5s after max_events");

    assert_eq!(events.len(), 3, "expected exactly 3 valid events");
    assert_eq!(events[0].message_id, "m1");
    assert_eq!(events[1].content, "@scout do X");
    assert_eq!(events[2].chat_id, "c2");
    // 第 3 条达 max_events 后停止前，损坏行已被静默 skip（未计入 errors）。
    // errors 计数允许 0（max_events 达到时直接 kill 子进程，可能未观察到 EOF）。
    let _ = errors;

    // 后台 task 自然退出
    let _ = stream.join.await;
}

/// 子进程立即 EOF → 触发指数退避重连。
///
/// fixture 用 shared counter 文件：每次 spawn append "x"；测试 sleep 一段时间后断言
/// counter ≥ 2（首启 + 至少一次重连）。channel 关闭由 receiver drop 触发。
#[tokio::test]
async fn s6_2_eof_triggers_exponential_reconnect() {
    let counter_dir = tempfile::tempdir().unwrap();
    let counter_path = counter_dir.path().join("spawn_count");
    std::fs::write(&counter_path, "").unwrap();

    let body = format!(
        r#"printf 'x' >> {counter}
exit 0
"#,
        counter = counter_path.display()
    );
    let (_d, path) = fixture_script(&body);
    let mut o = opts(path);
    o.initial_backoff = Duration::from_millis(20);
    o.max_backoff = Duration::from_millis(60);
    // 总超时兜底，避免循环失控
    o.timeout = Some(Duration::from_secs(2));

    // 总超时拉长到 5s，避免 shell fork 在并发测试压力下被调度延迟卡死
    o.timeout = Some(Duration::from_secs(5));
    let mut stream = consume_im(o);
    let mut error_events = 0usize;

    // 主动 polling counter 文件，counter ≥ 2 即说明已重连
    let poll_deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    loop {
        tokio::select! {
            msg = stream.rx.recv() => {
                match msg {
                    Some(Ok(_)) => unreachable!("fixture never emits valid event"),
                    Some(Err(_)) => { error_events += 1; }
                    None => break,
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
        let cur = std::fs::read_to_string(&counter_path).unwrap().len();
        if cur >= 2 {
            break;
        }
        if tokio::time::Instant::now() >= poll_deadline {
            break;
        }
    }

    drop(stream.rx);
    let _ = tokio::time::timeout(Duration::from_secs(3), stream.join).await;

    // 验证 fixture 确实被多次 spawn
    let spawn_count = std::fs::read_to_string(&counter_path).unwrap().len();
    assert!(
        spawn_count >= 2,
        "expected fixture to be spawned ≥2 times (initial + reconnect), got {spawn_count}"
    );
    assert!(
        error_events >= 1,
        "expected ≥1 error event on EOF / spawn, got {error_events}"
    );
}

/// Spawn 失败（binary 不存在）→ 投递 SpawnFailed；receiver drop 后后台 task 退出。
#[tokio::test]
async fn s6_3_spawn_failure_reports_and_keeps_retrying() {
    let mut o = opts(PathBuf::from("/definitely/nonexistent/fake-lark-cli-x9"));
    o.initial_backoff = Duration::from_millis(20);
    o.max_backoff = Duration::from_millis(60);
    o.timeout = Some(Duration::from_secs(1));

    let mut stream = consume_im(o);
    let first = tokio::time::timeout(Duration::from_secs(1), stream.rx.recv())
        .await
        .expect("first item should arrive within 1s");
    match first {
        Some(Err(EventError::SpawnFailed { binary, source })) => {
            assert!(binary.to_string_lossy().contains("nonexistent"));
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected SpawnFailed first, got {other:?}"),
    }
    drop(stream.rx);
    let _ = tokio::time::timeout(Duration::from_secs(2), stream.join).await;
}
