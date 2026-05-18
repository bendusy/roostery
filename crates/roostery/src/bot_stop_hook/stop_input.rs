//! StopHookInput stdin JSON schema + summary 派生 + transcript jsonl tail。
//!
//! 拆自原 `bot_stop_hook.rs` line 136-281（refactor `2026-05-19-bot-stop-hook-split`）。

use super::types::SUMMARY_MAX_BYTES;
use super::util::truncate_utf8;
use serde::Deserialize;
use std::path::Path;

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

/// 从 stop-hook stdin input 推导 summary：transcript_reader 优先 → prompt_response
/// 兜底 → None。返 None 表示后续走 [`super::types::DEFAULT_SUMMARY`]。
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

#[cfg(test)]
mod tests {
    use super::*;

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
        writeln!(f, "not valid json").unwrap();
        writeln!(
            f,
            r#"{{"type":"system","message":{{"content":[{{"text":"sys"}}]}}}}"#
        )
        .unwrap();
        writeln!(f, r#"{{"type":"assistant"}}"#).unwrap();
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
