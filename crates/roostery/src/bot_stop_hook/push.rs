//! 核心业务流：`push` + IM 兜底 + `run_stop_hook` 入口。
//!
//! 拆自原 `bot_stop_hook.rs` line 340-524 + 700-719（refactor `2026-05-19-bot-stop-hook-split`）。

use super::stop_input::{StopHookInput, resolve_summary_from_hook_input};
use super::types::{
    DEFAULT_SUMMARY, PushOptions, PushOutcome, PushRequest, PushStatus, SUMMARY_MAX_BYTES,
};
use super::util::{cwd_basename, resolve_receive_id, stable_idem_key, truncate_utf8};
use crate::lark_cli::LarkRunner;
use std::path::PathBuf;

/// 核心 lib fn——两路 CLI 共享的业务编排。
///
/// 编排：resolve receive_id 三层链 → if None 返 Skipped → task_writer
/// get_or_create_for_session + append_steps → 成功 Success / 任意错按
/// `opts.no_im_fallback` 决定走 IM 兜底 (lark-cli `im +messages-send`) 或直接 Failed。
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
        tracing::warn!(error = %e, "stdin read failed; treating as empty");
        return Ok(StopHookInput::default());
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(StopHookInput::default());
    }
    serde_json::from_str(trimmed)
}

pub(crate) fn build_request_from_stop_hook_input(input: StopHookInput) -> PushRequest {
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

#[cfg(test)]
#[allow(clippy::await_holding_lock)] // ENV_LOCK serializes env mutation (attention.md pattern)
mod tests {
    use super::*;
    use crate::bot_stop_hook::test_helpers::{
        im_send_response, install_tempdir_as_home, task_create_response,
    };
    use crate::lark_cli::LarkError;
    use crate::lark_cli::mock::MockLarkRunner;
    use crate::paths::TEST_ENV_LOCK as ENV_LOCK;
    use serde_json::json;

    // ----- push 核心 lib fn 集成单测 ------------------------------------

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
        let req = PushRequest::new("custom", "s1", "/tmp").with_assignee("ou_explicit");
        let out = push(req, &mock, PushOptions::default()).await;
        assert_eq!(out.status, PushStatus::Success);
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
        mock.enqueue_ok(json!({}))
            .enqueue_ok(json!([{"name":"default","active":true}]));
        let req = PushRequest::new("cc", "s1", "/tmp");
        let out = push(req, &mock, PushOptions::default()).await;
        assert_eq!(out.status, PushStatus::Skipped);
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
        let mock = MockLarkRunner::new();
        mock.enqueue_err(LarkError::Timeout { timeout_ms: 5 });
        let req = PushRequest::new("cc", "s1", "/tmp");
        let out = push(req, &mock, PushOptions::default()).await;
        assert_eq!(out.status, PushStatus::Skipped);
        assert_eq!(mock.calls().len(), 1);
    }

    // ----- run_stop_hook 适配层单测 -------------------------------------

    #[tokio::test]
    async fn run_stop_hook_cc_happy_stdin_routes_to_push() {
        let _g = ENV_LOCK.lock().unwrap();
        let _home = install_tempdir_as_home();
        unsafe { std::env::remove_var("ROOSTERY_NOTIFY_TO") };
        unsafe { std::env::set_var("ROOSTERY_AGENT", "cc") };
        unsafe { std::env::set_var("ROOSTERY_NOTIFY_TO", "ou_test") };

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
        mock.enqueue_ok(json!({}))
            .enqueue_ok(json!([{"name":"default","active":true}]));
        let out = run_stop_hook_with_reader(&mut reader, &mock, PushOptions::default()).await;
        assert_eq!(
            out.status,
            PushStatus::Skipped,
            "non-panic graceful fallback"
        );
    }
}
