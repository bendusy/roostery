//! 核心请求 / 响应类型 + 全局常量。
//!
//! 拆自原 `bot_stop_hook.rs` line 19-132（refactor `2026-05-19-bot-stop-hook-split`）。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// summary 默认值——append_steps 文本在 `req.summary == None` 时回退到这里。
/// Python parity（"Agent stopped (no summary)"）。
pub const DEFAULT_SUMMARY: &str = "Agent stopped (no summary)";

/// summary 截断字节上限（task append_steps 内容字段）。Python parity (head -c 200)。
pub const SUMMARY_MAX_BYTES: usize = 200;

/// 双 CLI surface 共享的类型化请求边界。builder API：必填项构造 + with_* 链式
/// 可选项。两路 CLI 在适配层后都构造一个 `PushRequest` 再调 [`super::push`]。
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
