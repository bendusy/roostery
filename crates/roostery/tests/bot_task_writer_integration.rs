//! Integration tests for bot_task_writer.
//!
//! Tests use MockLarkRunner and isolate `~/.roostery/` via tempdir +
//! `ROOSTERY_HOME` env. Serialized by module-local Mutex (attention.md
//! pattern: env var mutation must be串行化).

#![allow(clippy::await_holding_lock)]

use roostery::bot_task_writer::{
    AppendStepsOptions, CreateTaskOptions, append_steps, create_task, get_or_create_for_session,
};
use roostery::lark_cli::mock::MockLarkRunner;
use serde_json::json;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn isolate(tmp: &tempfile::TempDir) {
    unsafe { std::env::set_var("ROOSTERY_HOME", tmp.path()) };
    unsafe { std::env::set_var("ROOSTERY_HOST", "integ-host") };
}
fn restore() {
    unsafe { std::env::remove_var("ROOSTERY_HOME") };
    unsafe { std::env::remove_var("ROOSTERY_HOST") };
}

fn task_create_response() -> serde_json::Value {
    json!({
        "ok": true,
        "data": {
            "guid": "integ_guid_1",
            "url": "https://feishu.cn/task/integ_1"
        }
    })
}

#[tokio::test]
async fn e2e_create_then_append_then_get_or_create_hits_cache() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    isolate(&tmp);
    let mock = MockLarkRunner::new();
    // 入队 4 个响应：create + 3 次 append（一次性 1 调）+ get_or_create 不再调
    mock.enqueue_ok(task_create_response()); // create_task
    mock.enqueue_ok(json!({"ok": true})); // append_steps

    // 第 1 步 create
    let ref_ = create_task(
        &mock,
        "cc",
        "/work/integ",
        "Integration run",
        CreateTaskOptions::new().with_assignee_open_id("ou_integ"),
    )
    .await
    .unwrap();
    assert_eq!(ref_.guid.as_str(), "integ_guid_1");

    // 第 2 步 append 3 个步骤
    append_steps(
        &mock,
        &ref_.guid,
        &["step A", "step B", "step C"],
        AppendStepsOptions::default(),
    )
    .await
    .unwrap();

    // 第 3 步 get_or_create_for_session 与上面同 session — 走 cache miss + create
    // 这里用新 session 避免和第 1 步的 cache（其实没写 cache）冲突
    mock.enqueue_ok(task_create_response()); // get_or_create 内的 create
    let r1 = get_or_create_for_session(
        &mock,
        "cc",
        "integ_session_1",
        "/work/integ",
        "Session run",
        CreateTaskOptions::new().with_assignee_open_id("ou_integ"),
    )
    .await
    .unwrap();

    // 第 4 步 同 session 再调一次 — 应 hit cache 不调 lark
    let r2 = get_or_create_for_session(
        &mock,
        "cc",
        "integ_session_1",
        "/work/integ",
        "Session run",
        CreateTaskOptions::new().with_assignee_open_id("ou_integ"),
    )
    .await
    .unwrap();
    assert_eq!(r1, r2);

    let calls = mock.calls();
    assert_eq!(
        calls.len(),
        3,
        "expected 3 lark calls: create / append / get_or_create.create"
    );
    restore();
}

#[tokio::test]
async fn host_suffix_applied_in_summary() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    isolate(&tmp);
    let mock = MockLarkRunner::new();
    mock.enqueue_ok(task_create_response());
    create_task(
        &mock,
        "cc",
        "/x",
        "Build feature",
        CreateTaskOptions::new().with_assignee_open_id("ou_y"),
    )
    .await
    .unwrap();
    let calls = mock.calls();
    // summary 应自动后缀 "· integ-host"
    let argv = &calls[0];
    assert!(
        argv.iter().any(|s| s == "Build feature · integ-host"),
        "expected host suffix in summary, got argv={argv:?}"
    );
    restore();
}

#[tokio::test]
async fn cache_file_lives_under_state_session_tasks() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    isolate(&tmp);
    let mock = MockLarkRunner::new();
    mock.enqueue_ok(task_create_response());
    get_or_create_for_session(
        &mock,
        "cc",
        "verify_path",
        "/x",
        "S",
        CreateTaskOptions::new().with_assignee_open_id("ou_y"),
    )
    .await
    .unwrap();
    let expected = tmp
        .path()
        .join("state")
        .join("session_tasks")
        .join("cc-verify_path.json");
    assert!(
        expected.exists(),
        "expected cache at {}",
        expected.display()
    );
    restore();
}
