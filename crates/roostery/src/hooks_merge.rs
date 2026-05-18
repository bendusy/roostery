//! `roostery init` 装机时的 Stop hook 模板嵌入 + JSON 深合并（Phase 3，feature `hooks-merge`）。
//!
//! 三个模板用 `include_str!` 编译期嵌入（roadmap §4.7）：
//! - [`CC_STOP_HOOK_JSON`]：CC `SessionEnd` hook fragment
//! - [`CODEX_STOP_HOOK_JSON`]：Codex `SessionEnd` hook fragment
//! - [`STOP_HOOK_AGENT_NOTIFY_SH`]：CC / Codex 共用的 stop bridge sh
//!
//! Merge 算法按 event key + matcher + command tail 幂等去重；env 前缀切到
//! `ROOSTERY_AGENT=cc|codex`（不沿用 Python `FEISHU_HUB_AGENT`，文档明示偏离）。
//!
//! See `.codestable/features/2026-05-18-hooks-merge/hooks-merge-design.md`.

use serde_json::Value;
use std::path::Path;
use thiserror::Error;

pub const CC_STOP_HOOK_JSON: &str = include_str!("templates/cc_stop_hook.json");
pub const CODEX_STOP_HOOK_JSON: &str = include_str!("templates/codex_stop_hook.json");
pub const STOP_HOOK_AGENT_NOTIFY_SH: &str = include_str!("templates/agent_stop_notify.sh");

const HOOK_SCRIPT_PLACEHOLDER: &str = "{{HOOK_SCRIPT}}";

/// Caller-facing failures.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HooksError {
    #[error("read existing hook file failed: {source}")]
    ReadFailed {
        #[from]
        source: std::io::Error,
    },
    #[error("parse existing hook file failed: {source}")]
    ParseFailed { source: serde_json::Error },
    #[error("fragment invalid: {reason}")]
    FragmentInvalid { reason: String },
    #[error("save hook file failed: {source}")]
    SaveFailed { source: std::io::Error },
}

/// Replace `{{HOOK_SCRIPT}}` placeholder in template and parse as JSON.
pub fn render_template(template_src: &str, hook_script: &str) -> Result<Value, HooksError> {
    let rendered = template_src.replace(HOOK_SCRIPT_PLACEHOLDER, hook_script);
    serde_json::from_str(&rendered).map_err(|e| HooksError::ParseFailed { source: e })
}

fn load_existing(target_path: &Path) -> Result<Value, HooksError> {
    let bytes = match std::fs::read(target_path) {
        Ok(b) if b.iter().all(|c| c.is_ascii_whitespace()) => {
            return Ok(Value::Object(Default::default()));
        }
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Value::Object(Default::default()));
        }
        Err(e) => return Err(HooksError::ReadFailed { source: e }),
    };
    let v: Value =
        serde_json::from_slice(&bytes).map_err(|e| HooksError::ParseFailed { source: e })?;
    if !v.is_object() {
        return Err(HooksError::FragmentInvalid {
            reason: format!(
                "existing {} top-level must be object, got {}",
                target_path.display(),
                value_type_name(&v)
            ),
        });
    }
    Ok(v)
}

fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Strip leading `KEY=VAL` env assignments from a hook command, return remainder.
fn command_tail(cmd: &str) -> &str {
    let trimmed = cmd.trim_start();
    let mut rest = trimmed;
    loop {
        let next = rest.split_whitespace().next().unwrap_or("");
        if next.is_empty() || !next.contains('=') {
            return rest;
        }
        rest = rest.trim_start_matches(next).trim_start();
    }
}

fn detect_event_key(fragment: &Value) -> Result<String, HooksError> {
    let hooks =
        fragment
            .get("hooks")
            .and_then(|v| v.as_object())
            .ok_or(HooksError::FragmentInvalid {
                reason: "fragment.hooks must be an object".into(),
            })?;
    if hooks.len() != 1 {
        return Err(HooksError::FragmentInvalid {
            reason: format!(
                "fragment.hooks must have exactly 1 event key, got {}",
                hooks.len()
            ),
        });
    }
    Ok(hooks.keys().next().unwrap().clone())
}

fn extract_first_matcher_entry(fragment: &Value, event: &str) -> Result<Value, HooksError> {
    let arr = fragment["hooks"][event]
        .as_array()
        .ok_or(HooksError::FragmentInvalid {
            reason: format!("fragment.hooks.{event} must be an array"),
        })?;
    let first = arr.first().ok_or(HooksError::FragmentInvalid {
        reason: format!("fragment.hooks.{event} array is empty"),
    })?;
    let hooks_arr =
        first
            .get("hooks")
            .and_then(|v| v.as_array())
            .ok_or(HooksError::FragmentInvalid {
                reason: "fragment matcher entry has no hooks array".into(),
            })?;
    if hooks_arr.is_empty() {
        return Err(HooksError::FragmentInvalid {
            reason: "fragment matcher entry hooks array is empty".into(),
        });
    }
    let cmd_present = hooks_arr[0]
        .get("command")
        .and_then(|c| c.as_str())
        .is_some();
    if !cmd_present {
        return Err(HooksError::FragmentInvalid {
            reason: "fragment first hook has no command string".into(),
        });
    }
    Ok(first.clone())
}

/// Merge `fragment` into existing JSON at `target_path`; idempotent by
/// (event key, matcher, command tail).
pub fn merge_event_hook(target_path: &Path, fragment: &Value) -> Result<Value, HooksError> {
    let event = detect_event_key(fragment)?;
    let new_matcher_entry = extract_first_matcher_entry(fragment, &event)?;
    let new_matcher = new_matcher_entry
        .get("matcher")
        .and_then(|m| m.as_str())
        .unwrap_or("*")
        .to_string();
    let new_hook = new_matcher_entry["hooks"][0].clone();
    let new_cmd = new_hook["command"].as_str().unwrap().to_string();

    let mut data = load_existing(target_path)?;
    let obj = data.as_object_mut().unwrap();
    let hooks_obj = obj
        .entry("hooks")
        .or_insert_with(|| Value::Object(Default::default()));
    if !hooks_obj.is_object() {
        return Err(HooksError::FragmentInvalid {
            reason: "existing target hooks field must be object".into(),
        });
    }
    let hooks_map = hooks_obj.as_object_mut().unwrap();
    let arr = hooks_map
        .entry(event.clone())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !arr.is_array() {
        return Err(HooksError::FragmentInvalid {
            reason: format!("existing target hooks.{event} must be array"),
        });
    }
    let arr = arr.as_array_mut().unwrap();

    let bucket_idx = arr.iter().position(|item| {
        item.get("matcher").and_then(|m| m.as_str()).unwrap_or("*") == new_matcher
    });

    match bucket_idx {
        None => {
            arr.push(new_matcher_entry);
        }
        Some(idx) => {
            let bucket = &mut arr[idx];
            let bucket_hooks = bucket
                .get_mut("hooks")
                .and_then(|h| h.as_array_mut())
                .ok_or(HooksError::FragmentInvalid {
                    reason: "existing matcher entry has no hooks array".into(),
                })?;
            let new_tail = command_tail(&new_cmd);
            let dup_idx = bucket_hooks.iter().position(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .map(|c| command_tail(c) == new_tail)
                    .unwrap_or(false)
            });
            match dup_idx {
                None => bucket_hooks.push(new_hook),
                Some(i) => {
                    if let Some(new_timeout) = new_hook.get("timeout") {
                        bucket_hooks[i]
                            .as_object_mut()
                            .unwrap()
                            .insert("timeout".into(), new_timeout.clone());
                    }
                }
            }
        }
    }

    Ok(data)
}

fn write_json_atomic(target_path: &Path, data: &Value) -> Result<(), HooksError> {
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| HooksError::SaveFailed { source: e })?;
    }
    let mut body =
        serde_json::to_vec_pretty(data).map_err(|e| HooksError::ParseFailed { source: e })?;
    body.push(b'\n');
    let extension = target_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("json");
    let tmp = target_path.with_extension(format!("{extension}.tmp"));
    std::fs::write(&tmp, &body).map_err(|e| HooksError::SaveFailed { source: e })?;
    std::fs::rename(&tmp, target_path).map_err(|e| HooksError::SaveFailed { source: e })?;
    Ok(())
}

/// One-shot: render → merge → atomic write. Returns the path actually written.
pub fn apply_template(
    template_src: &str,
    target_path: &Path,
    hook_script: &str,
) -> Result<std::path::PathBuf, HooksError> {
    let fragment = render_template(template_src, hook_script)?;
    let merged = merge_event_hook(target_path, &fragment)?;
    write_json_atomic(target_path, &merged)?;
    Ok(target_path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_consts_nonempty() {
        assert!(!CC_STOP_HOOK_JSON.is_empty());
        assert!(!CODEX_STOP_HOOK_JSON.is_empty());
        assert!(!STOP_HOOK_AGENT_NOTIFY_SH.is_empty());
    }

    #[test]
    fn cc_template_uses_roostery_agent_env() {
        assert!(CC_STOP_HOOK_JSON.contains("ROOSTERY_AGENT=cc"));
        assert!(!CC_STOP_HOOK_JSON.contains("FEISHU_HUB_AGENT"));
        assert!(CC_STOP_HOOK_JSON.contains(HOOK_SCRIPT_PLACEHOLDER));
    }

    #[test]
    fn codex_template_uses_roostery_agent_env() {
        assert!(CODEX_STOP_HOOK_JSON.contains("ROOSTERY_AGENT=codex"));
        assert!(!CODEX_STOP_HOOK_JSON.contains("FEISHU_HUB_AGENT"));
        assert!(CODEX_STOP_HOOK_JSON.contains(HOOK_SCRIPT_PLACEHOLDER));
    }

    #[test]
    fn render_cc_template_happy() {
        let v = render_template(CC_STOP_HOOK_JSON, "/path/to/sh").unwrap();
        let cmd = v["hooks"]["SessionEnd"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(cmd, "ROOSTERY_AGENT=cc /path/to/sh");
    }

    #[test]
    fn render_codex_template_happy() {
        let v = render_template(CODEX_STOP_HOOK_JSON, "/usr/local/bin/notify.sh").unwrap();
        let cmd = v["hooks"]["SessionEnd"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(cmd, "ROOSTERY_AGENT=codex /usr/local/bin/notify.sh");
    }

    #[test]
    fn render_no_placeholder_left() {
        let v = render_template(CC_STOP_HOOK_JSON, "/x").unwrap();
        let s = serde_json::to_string(&v).unwrap();
        assert!(!s.contains(HOOK_SCRIPT_PLACEHOLDER));
    }

    #[test]
    fn render_invalid_json_returns_parse_failed() {
        match render_template("{ not json", "/x") {
            Err(HooksError::ParseFailed { .. }) => {}
            other => panic!("expected ParseFailed, got {other:?}"),
        }
    }

    use serde_json::json;

    fn write_existing(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        let path = dir.join("settings.json");
        std::fs::write(&path, body).unwrap();
        path
    }

    fn cc_fragment() -> Value {
        render_template(CC_STOP_HOOK_JSON, "/sh/path").unwrap()
    }

    #[test]
    fn command_tail_strips_env_prefix() {
        assert_eq!(command_tail("ROOSTERY_AGENT=cc /sh"), "/sh");
        assert_eq!(command_tail("FEISHU_HUB_AGENT=cc /sh"), "/sh");
        assert_eq!(
            command_tail("FOO=1 BAR=2 /usr/local/bin/sh arg"),
            "/usr/local/bin/sh arg"
        );
        assert_eq!(command_tail("/no/env /args"), "/no/env /args");
        assert_eq!(
            command_tail("   /leading/whitespace"),
            "/leading/whitespace"
        );
    }

    #[test]
    fn merge_into_missing_target() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("none.json");
        let merged = merge_event_hook(&path, &cc_fragment()).unwrap();
        assert_eq!(merged, cc_fragment());
    }

    #[test]
    fn merge_into_target_with_different_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_existing(
            dir.path(),
            r#"{"hooks":{"Stop":[{"matcher":"*","hooks":[{"type":"command","command":"echo s"}]}]}}"#,
        );
        let merged = merge_event_hook(&path, &cc_fragment()).unwrap();
        assert!(merged["hooks"]["Stop"].is_array());
        assert!(merged["hooks"]["SessionEnd"].is_array());
    }

    #[test]
    fn merge_into_same_event_different_matcher() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_existing(
            dir.path(),
            r#"{"hooks":{"SessionEnd":[{"matcher":"after-tool","hooks":[{"type":"command","command":"echo s"}]}]}}"#,
        );
        let merged = merge_event_hook(&path, &cc_fragment()).unwrap();
        let arr = merged["hooks"]["SessionEnd"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn merge_into_same_matcher_different_command_appends() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_existing(
            dir.path(),
            r#"{"hooks":{"SessionEnd":[{"matcher":"*","hooks":[{"type":"command","command":"echo other"}]}]}}"#,
        );
        let merged = merge_event_hook(&path, &cc_fragment()).unwrap();
        let bucket_hooks = merged["hooks"]["SessionEnd"][0]["hooks"]
            .as_array()
            .unwrap();
        assert_eq!(bucket_hooks.len(), 2);
    }

    #[test]
    fn merge_dedup_same_command_updates_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_existing(
            dir.path(),
            r#"{"hooks":{"SessionEnd":[{"matcher":"*","hooks":[{"type":"command","command":"ROOSTERY_AGENT=cc /sh/path","timeout":5}]}]}}"#,
        );
        let merged = merge_event_hook(&path, &cc_fragment()).unwrap();
        let bucket_hooks = merged["hooks"]["SessionEnd"][0]["hooks"]
            .as_array()
            .unwrap();
        assert_eq!(bucket_hooks.len(), 1, "idempotent: not appended");
        assert_eq!(bucket_hooks[0]["timeout"], 10);
    }

    #[test]
    fn merge_legacy_env_treated_as_same_command_by_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_existing(
            dir.path(),
            r#"{"hooks":{"SessionEnd":[{"matcher":"*","hooks":[{"type":"command","command":"FEISHU_HUB_AGENT=cc /sh/path","timeout":5}]}]}}"#,
        );
        let merged = merge_event_hook(&path, &cc_fragment()).unwrap();
        let bucket_hooks = merged["hooks"]["SessionEnd"][0]["hooks"]
            .as_array()
            .unwrap();
        // tail-match dedup: existing entry kept (legacy env preserved), timeout updated
        assert_eq!(bucket_hooks.len(), 1);
        assert!(
            bucket_hooks[0]["command"]
                .as_str()
                .unwrap()
                .starts_with("FEISHU_HUB_AGENT="),
            "existing command preserved (env migration is roostery-init's job)"
        );
        assert_eq!(bucket_hooks[0]["timeout"], 10);
    }

    #[test]
    fn fragment_with_zero_event_keys_invalid() {
        let frag = json!({"hooks": {}});
        match merge_event_hook(std::path::Path::new("/nonexistent"), &frag) {
            Err(HooksError::FragmentInvalid { .. }) => {}
            other => panic!("expected FragmentInvalid, got {other:?}"),
        }
    }

    #[test]
    fn fragment_with_two_event_keys_invalid() {
        let frag = json!({"hooks": {"Stop": [], "SessionEnd": []}});
        match merge_event_hook(std::path::Path::new("/nonexistent"), &frag) {
            Err(HooksError::FragmentInvalid { .. }) => {}
            other => panic!("expected FragmentInvalid, got {other:?}"),
        }
    }

    #[test]
    fn fragment_with_empty_matcher_array_invalid() {
        let frag = json!({"hooks": {"SessionEnd": []}});
        match merge_event_hook(std::path::Path::new("/nonexistent"), &frag) {
            Err(HooksError::FragmentInvalid { .. }) => {}
            other => panic!("expected FragmentInvalid, got {other:?}"),
        }
    }

    #[test]
    fn fragment_without_command_invalid() {
        let frag = json!({
            "hooks": {
                "SessionEnd": [{"matcher":"*","hooks":[{"type":"command"}]}]
            }
        });
        match merge_event_hook(std::path::Path::new("/nonexistent"), &frag) {
            Err(HooksError::FragmentInvalid { .. }) => {}
            other => panic!("expected FragmentInvalid, got {other:?}"),
        }
    }

    #[test]
    fn target_not_object_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_existing(dir.path(), "[1, 2, 3]");
        match merge_event_hook(&path, &cc_fragment()) {
            Err(HooksError::FragmentInvalid { .. }) => {}
            other => panic!("expected FragmentInvalid, got {other:?}"),
        }
    }

    #[test]
    fn target_invalid_json_returns_parse_failed() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_existing(dir.path(), "{not json");
        match merge_event_hook(&path, &cc_fragment()) {
            Err(HooksError::ParseFailed { .. }) => {}
            other => panic!("expected ParseFailed, got {other:?}"),
        }
    }

    #[test]
    fn apply_template_to_missing_target() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        apply_template(CC_STOP_HOOK_JSON, &path, "/sh/path").unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("ROOSTERY_AGENT=cc /sh/path"));
        assert!(body.ends_with('\n'));
        // indent=2 守护：subsequent line should start with two spaces
        assert!(body.contains("\n  \"hooks\""));
    }

    #[test]
    fn apply_template_creates_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c/settings.json");
        apply_template(CC_STOP_HOOK_JSON, &nested, "/x").unwrap();
        assert!(nested.exists());
    }

    #[test]
    fn apply_template_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        apply_template(CC_STOP_HOOK_JSON, &path, "/x").unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        apply_template(CC_STOP_HOOK_JSON, &path, "/x").unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            first, second,
            "second apply must be byte-for-byte identical"
        );
    }

    #[test]
    fn apply_template_preserves_other_event_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_existing(
            dir.path(),
            r#"{"hooks":{"Stop":[{"matcher":"*","hooks":[{"type":"command","command":"echo other"}]}]}}"#,
        );
        apply_template(CC_STOP_HOOK_JSON, &path, "/sh").unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(
            v["hooks"]["Stop"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                == "echo other"
        );
        assert!(
            v["hooks"]["SessionEnd"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("ROOSTERY_AGENT=cc")
        );
    }

    #[test]
    fn sh_template_calls_roostery_dispatcher_fire() {
        assert!(STOP_HOOK_AGENT_NOTIFY_SH.contains("roostery dispatcher fire"));
        assert!(!STOP_HOOK_AGENT_NOTIFY_SH.contains("python3 -m roostery"));
        assert!(STOP_HOOK_AGENT_NOTIFY_SH.contains("ROOSTERY_AGENT"));
        assert!(!STOP_HOOK_AGENT_NOTIFY_SH.contains("FEISHU_HUB_AGENT"));
    }
}
