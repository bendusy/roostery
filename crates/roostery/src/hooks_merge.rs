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
//! Rust-idiom-first refactor (2026-05-18，per
//! `.codestable/compound/2026-05-18-decision-rust-idiom-first.md` B1+B2+B4)：
//! - `HookFragment` / `MatcherEntry` / `HookCommand` 强类型 serde derive 替代
//!   `serde_json::Value` 字符串 indexing + `.unwrap()` 满天飞
//! - `HooksError::FragmentInvalid { reason: String }` 拆为具体变体让 caller
//!   能 `match` 编译期穷尽
//! - `AgentKind` enum 替代 `"cc"` / `"codex"` 字符串字面量散落
//!
//! See `.codestable/features/2026-05-18-hooks-merge/hooks-merge-design.md`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;
use thiserror::Error;

pub const CC_STOP_HOOK_JSON: &str = include_str!("templates/cc_stop_hook.json");
pub const CODEX_STOP_HOOK_JSON: &str = include_str!("templates/codex_stop_hook.json");
pub const GEMINI_STOP_HOOK_JSON: &str = include_str!("templates/gemini_stop_hook.json");
pub const STOP_HOOK_AGENT_NOTIFY_SH: &str = include_str!("templates/agent_stop_notify.sh");

const HOOK_SCRIPT_PLACEHOLDER: &str = "{{HOOK_SCRIPT}}";
const DEFAULT_MATCHER: &str = "*";

// --- AgentKind (B4) -------------------------------------------------------

/// Identifies which agent runtime fires the hook. Replaces stringly-typed
/// `"cc"` / `"codex"` literals scattered through templates and downstream
/// caller code (see `ROOSTERY_AGENT` env in `templates/agent_stop_notify.sh`).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum AgentKind {
    Cc,
    Codex,
    Gemini,
}

impl AgentKind {
    /// Returns the embedded hook fragment template for this runtime.
    pub fn template(self) -> &'static str {
        match self {
            AgentKind::Cc => CC_STOP_HOOK_JSON,
            AgentKind::Codex => CODEX_STOP_HOOK_JSON,
            AgentKind::Gemini => GEMINI_STOP_HOOK_JSON,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AgentKind::Cc => "cc",
            AgentKind::Codex => "codex",
            AgentKind::Gemini => "gemini",
        }
    }
}

impl std::fmt::Display for AgentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AgentKind {
    type Err = UnknownAgentKind;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "cc" => Ok(AgentKind::Cc),
            "codex" => Ok(AgentKind::Codex),
            "gemini" => Ok(AgentKind::Gemini),
            other => Err(UnknownAgentKind(other.to_string())),
        }
    }
}

#[derive(Debug, Error)]
#[error("unknown agent kind: {0:?} (expected one of: cc / codex / gemini)")]
pub struct UnknownAgentKind(pub String);

// --- Typed hook fragment (B1) ---------------------------------------------

/// Top-level shape of a hook fragment JSON: `{ "hooks": { "<event>": [...] } }`.
///
/// Both our embedded templates (CC / Codex) and the on-disk
/// `~/.claude/settings.json` follow this shape. Typed wrapper lets the merge
/// algorithm work without raw `Value` indexing + `.unwrap()` ladders.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct HookFragment {
    pub hooks: BTreeMap<String, Vec<MatcherEntry>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MatcherEntry {
    #[serde(default = "default_matcher")]
    pub matcher: String,
    pub hooks: Vec<HookCommand>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct HookCommand {
    #[serde(rename = "type")]
    pub kind: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u32>,
}

fn default_matcher() -> String {
    DEFAULT_MATCHER.to_string()
}

impl HookFragment {
    /// Parse a JSON value into a fragment AND validate it has exactly the
    /// shape our merge algorithm expects (1 event key, ≥1 matcher entry,
    /// each entry has ≥1 hook command with non-empty `command` string).
    pub fn from_value(value: &Value) -> Result<Self, FragmentError> {
        let fragment: HookFragment =
            serde_json::from_value(value.clone()).map_err(FragmentError::Shape)?;
        fragment.validate()?;
        Ok(fragment)
    }

    fn validate(&self) -> Result<(), FragmentError> {
        match self.hooks.len() {
            0 => return Err(FragmentError::NoEventKey),
            1 => {}
            n => return Err(FragmentError::MultipleEventKeys { found: n }),
        }
        let (event, matchers) = self.hooks.iter().next().unwrap();
        if matchers.is_empty() {
            return Err(FragmentError::EmptyMatcherArray {
                event: event.clone(),
            });
        }
        let first = &matchers[0];
        if first.hooks.is_empty() {
            return Err(FragmentError::EmptyHooksArray {
                event: event.clone(),
                matcher: first.matcher.clone(),
            });
        }
        if first.hooks[0].command.is_empty() {
            return Err(FragmentError::MissingCommand {
                event: event.clone(),
                matcher: first.matcher.clone(),
            });
        }
        Ok(())
    }

    /// The single event key in this fragment (validated to be exactly 1).
    pub fn event_key(&self) -> &str {
        self.hooks.keys().next().expect("validated by from_value")
    }

    /// The first (and typically only) matcher entry.
    pub fn first_matcher(&self) -> &MatcherEntry {
        &self.hooks[self.event_key()][0]
    }

    /// The first hook command in the first matcher entry.
    pub fn first_command(&self) -> &HookCommand {
        &self.first_matcher().hooks[0]
    }
}

// --- Errors (B2) ----------------------------------------------------------

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
    #[error("fragment is invalid: {0}")]
    Fragment(#[from] FragmentError),
    #[error("save hook file failed: {source}")]
    SaveFailed { source: std::io::Error },
}

/// Specific reasons a hook fragment is invalid; replaces the old
/// `HooksError::FragmentInvalid { reason: String }` catch-all.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FragmentError {
    #[error("fragment.hooks has no event key")]
    NoEventKey,
    #[error("fragment.hooks must have exactly 1 event key, got {found}")]
    MultipleEventKeys { found: usize },
    #[error("fragment.hooks.{event} matcher array is empty")]
    EmptyMatcherArray { event: String },
    #[error("fragment.hooks.{event}[matcher={matcher:?}].hooks array is empty")]
    EmptyHooksArray { event: String, matcher: String },
    #[error(
        "fragment.hooks.{event}[matcher={matcher:?}].hooks[0].command must be non-empty string"
    )]
    MissingCommand { event: String, matcher: String },
    #[error("fragment shape does not match expected schema: {0}")]
    Shape(serde_json::Error),
    #[error("existing target top-level must be JSON object, got {type_name}")]
    TargetTopLevelNotObject { type_name: &'static str },
    #[error("existing target {field:?} field must be JSON object")]
    TargetFieldNotObject { field: &'static str },
    #[error("existing target hooks.{event} must be JSON array")]
    TargetEventNotArray { event: String },
    #[error("existing target matcher entry hooks field must be JSON array")]
    TargetMatcherHooksNotArray,
}

// --- Render --------------------------------------------------------------

/// Render template by **shell-quoting** `hook_script` and replacing the
/// `{{HOOK_SCRIPT}}` placeholder.
///
/// **codex audit finding-07 fix**：旧实现先 `str::replace` raw JSON text 再
/// parse——`hook_script` 含空格 / 单引号 / 双引号会破坏 JSON 解析或 shell 拆词
/// （`HOME=/Users/Ben Smith/...` 实际场景）。新实现：
///
/// 1. 先 parse template 为 JSON tree（结构正确性独立保证）
/// 2. 用 shell_quote 包装路径（防 shell 拆词 / metachar 注入）
/// 3. 用 walker 把 placeholder 替换发生在 JSON String 值内（JSON 重新序列化
///    时自动处理 string 内的 `"` `\` 等 JSON escape）
///
/// 两层 escape 各管一层不互相干扰。
pub fn render_template(template_src: &str, hook_script: &str) -> Result<Value, HooksError> {
    let mut tree: Value =
        serde_json::from_str(template_src).map_err(|e| HooksError::ParseFailed { source: e })?;
    let quoted = shell_quote_path(hook_script);
    replace_placeholder_in_strings(&mut tree, &quoted);
    Ok(tree)
}

/// POSIX shell single-quote 安全包装。alphanumeric / `/_-.` 不需引号；其他
/// 用 `'...'` 包裹，内部 `'` 转义为 `'\\''`。
fn shell_quote_path(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn replace_placeholder_in_strings(v: &mut Value, replacement: &str) {
    match v {
        Value::String(s) if s.contains(HOOK_SCRIPT_PLACEHOLDER) => {
            *s = s.replace(HOOK_SCRIPT_PLACEHOLDER, replacement);
        }
        Value::Array(a) => a
            .iter_mut()
            .for_each(|x| replace_placeholder_in_strings(x, replacement)),
        Value::Object(o) => o
            .values_mut()
            .for_each(|x| replace_placeholder_in_strings(x, replacement)),
        _ => {}
    }
}

// --- Target file IO ------------------------------------------------------

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
        return Err(HooksError::Fragment(
            FragmentError::TargetTopLevelNotObject {
                type_name: value_type_name(&v),
            },
        ));
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

// --- Merge ---------------------------------------------------------------

/// Merge `fragment` into existing JSON at `target_path`; idempotent by
/// (event key, matcher, command tail). `fragment` must satisfy
/// [`HookFragment::from_value`] shape requirements.
pub fn merge_event_hook(target_path: &Path, fragment: &Value) -> Result<Value, HooksError> {
    let typed = HookFragment::from_value(fragment)?;
    let event = typed.event_key().to_string();
    let new_matcher_entry = typed.first_matcher().clone();
    let new_matcher = new_matcher_entry.matcher.clone();
    let new_hook = typed.first_command().clone();

    let mut data = load_existing(target_path)?;
    let obj = data.as_object_mut().expect("load_existing returns Object");
    let hooks_obj = obj
        .entry("hooks")
        .or_insert_with(|| Value::Object(Default::default()));
    if !hooks_obj.is_object() {
        return Err(HooksError::Fragment(FragmentError::TargetFieldNotObject {
            field: "hooks",
        }));
    }
    let hooks_map = hooks_obj.as_object_mut().expect("checked is_object above");
    let arr_value = hooks_map
        .entry(event.clone())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !arr_value.is_array() {
        return Err(HooksError::Fragment(FragmentError::TargetEventNotArray {
            event,
        }));
    }
    let arr = arr_value.as_array_mut().expect("checked is_array above");

    let bucket_idx = arr.iter().position(|item| {
        item.get("matcher")
            .and_then(|m| m.as_str())
            .unwrap_or(DEFAULT_MATCHER)
            == new_matcher
    });

    match bucket_idx {
        None => arr.push(serde_json::to_value(&new_matcher_entry).expect("typed → Value")),
        Some(idx) => append_or_dedupe_in_bucket(&mut arr[idx], &new_hook)?,
    }

    Ok(data)
}

fn append_or_dedupe_in_bucket(
    bucket: &mut Value,
    new_hook: &HookCommand,
) -> Result<(), HooksError> {
    let bucket_hooks = bucket
        .get_mut("hooks")
        .and_then(|h| h.as_array_mut())
        .ok_or(HooksError::Fragment(
            FragmentError::TargetMatcherHooksNotArray,
        ))?;
    let new_tail = command_tail(&new_hook.command);
    let dup_idx = bucket_hooks.iter().position(|h| {
        h.get("command")
            .and_then(|c| c.as_str())
            .map(|c| command_tail(c) == new_tail)
            .unwrap_or(false)
    });
    match dup_idx {
        None => bucket_hooks.push(serde_json::to_value(new_hook).expect("typed → Value")),
        Some(i) => {
            if let Some(new_timeout) = new_hook.timeout
                && let Some(obj) = bucket_hooks[i].as_object_mut()
            {
                obj.insert("timeout".into(), serde_json::json!(new_timeout));
            }
        }
    }
    Ok(())
}

// --- Atomic save ---------------------------------------------------------

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
    use serde_json::json;

    fn write_existing(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        let path = dir.join("settings.json");
        std::fs::write(&path, body).unwrap();
        path
    }

    fn cc_fragment() -> Value {
        render_template(CC_STOP_HOOK_JSON, "/sh/path").unwrap()
    }

    // --- AgentKind (B4) ---------------------------------------------------

    #[test]
    fn agent_kind_display_and_parse() {
        assert_eq!(AgentKind::Cc.to_string(), "cc");
        assert_eq!(AgentKind::Codex.to_string(), "codex");
        assert_eq!(AgentKind::Gemini.to_string(), "gemini");
        assert_eq!(AgentKind::Cc.as_str(), "cc");
        assert_eq!("cc".parse::<AgentKind>().unwrap(), AgentKind::Cc);
        assert_eq!("codex".parse::<AgentKind>().unwrap(), AgentKind::Codex);
        assert_eq!("gemini".parse::<AgentKind>().unwrap(), AgentKind::Gemini);
        assert!("nushell".parse::<AgentKind>().is_err());
    }

    #[test]
    fn agent_kind_template_returns_matching_const() {
        assert_eq!(AgentKind::Cc.template(), CC_STOP_HOOK_JSON);
        assert_eq!(AgentKind::Codex.template(), CODEX_STOP_HOOK_JSON);
        assert_eq!(AgentKind::Gemini.template(), GEMINI_STOP_HOOK_JSON);
    }

    #[test]
    fn agent_kind_serde_lowercase() {
        let v = serde_json::to_string(&AgentKind::Cc).unwrap();
        assert_eq!(v, "\"cc\"");
        let k: AgentKind = serde_json::from_str("\"codex\"").unwrap();
        assert_eq!(k, AgentKind::Codex);
        let g: AgentKind = serde_json::from_str("\"gemini\"").unwrap();
        assert_eq!(g, AgentKind::Gemini);
    }

    #[test]
    fn gemini_template_nonempty_and_parseable() {
        assert!(!GEMINI_STOP_HOOK_JSON.is_empty());
        let v: serde_json::Value = serde_json::from_str(GEMINI_STOP_HOOK_JSON).unwrap();
        // Same shape as cc/codex: { hooks: { SessionEnd: [...] } }
        assert!(v["hooks"]["SessionEnd"].is_array());
    }

    #[test]
    fn agent_kind_unknown_error_msg_lists_all_three() {
        let err = "fish".parse::<AgentKind>().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cc"));
        assert!(msg.contains("codex"));
        assert!(msg.contains("gemini"));
    }

    // --- HookFragment typed (B1) ------------------------------------------

    #[test]
    fn fragment_parse_cc_template_typed() {
        let v = cc_fragment();
        let frag = HookFragment::from_value(&v).unwrap();
        assert_eq!(frag.event_key(), "SessionEnd");
        assert_eq!(frag.first_matcher().matcher, "*");
        assert_eq!(frag.first_command().command, "ROOSTERY_AGENT=cc /sh/path");
        assert_eq!(frag.first_command().kind, "command");
        assert_eq!(frag.first_command().timeout, Some(10));
    }

    // --- Templates baseline (kept from earlier) ---------------------------

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
    fn sh_template_calls_roostery_bot_stop_hook() {
        // Phase 5 (bot-stop-hook feature): sh wrapper 退化为极简 stdin 直透
        // 调 `roostery bot stop-hook`，由 Rust 端原生处理 transcript / push。
        assert!(STOP_HOOK_AGENT_NOTIFY_SH.contains("roostery bot stop-hook"));
        assert!(!STOP_HOOK_AGENT_NOTIFY_SH.contains("roostery dispatcher fire"));
        assert!(!STOP_HOOK_AGENT_NOTIFY_SH.contains("python3 -m roostery"));
        assert!(STOP_HOOK_AGENT_NOTIFY_SH.contains("ROOSTERY_AGENT"));
        assert!(!STOP_HOOK_AGENT_NOTIFY_SH.contains("FEISHU_HUB_AGENT"));
        // 极简性核查：<= 10 非空非注释行
        let code_lines = STOP_HOOK_AGENT_NOTIFY_SH
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#')
            })
            .count();
        assert!(
            code_lines <= 10,
            "极简 wrapper：non-comment lines = {code_lines}; 期望 <= 10"
        );
        // 极简性核查：不再 jq / tail / extract
        assert!(!STOP_HOOK_AGENT_NOTIFY_SH.contains("jq"));
        assert!(!STOP_HOOK_AGENT_NOTIFY_SH.contains("tac"));
    }

    // --- render ----------------------------------------------------------

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

    // --- codex audit finding-07: shell-quote 安全注入 -----------------

    #[test]
    fn render_path_with_space_is_single_quoted() {
        let v =
            render_template(CC_STOP_HOOK_JSON, "/Users/Ben Smith/.roostery/scripts/n.sh").unwrap();
        let cmd = v["hooks"]["SessionEnd"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        // 含空格的路径必须被单引号包裹防 shell 拆词
        assert_eq!(
            cmd,
            "ROOSTERY_AGENT=cc '/Users/Ben Smith/.roostery/scripts/n.sh'"
        );
    }

    #[test]
    fn render_path_with_single_quote_is_escaped() {
        // POSIX shell '\'' 是单引号字面量的标准 escape 路径
        let v = render_template(CC_STOP_HOOK_JSON, "/path/it's/here.sh").unwrap();
        let cmd = v["hooks"]["SessionEnd"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(cmd, "ROOSTERY_AGENT=cc '/path/it'\\''s/here.sh'");
    }

    #[test]
    fn render_path_with_double_quote_safe_via_json_layer() {
        // 含 " 的路径——shell_quote 用单引号包，JSON 层正常 escape \"
        let v = render_template(CC_STOP_HOOK_JSON, r#"/path/has"quote.sh"#).unwrap();
        // 再次 serialize 应可解析；不破坏 JSON
        let s = serde_json::to_string(&v).unwrap();
        let _: Value = serde_json::from_str(&s).expect("roundtrip");
        let cmd = v["hooks"]["SessionEnd"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.starts_with("ROOSTERY_AGENT=cc '"));
        assert!(cmd.contains(r#""quote.sh"#));
    }

    #[test]
    fn shell_quote_path_safe_chars_passthrough() {
        assert_eq!(
            shell_quote_path("/usr/local/bin/lark"),
            "/usr/local/bin/lark"
        );
        assert_eq!(shell_quote_path("abc_def-123.sh"), "abc_def-123.sh");
    }

    #[test]
    fn shell_quote_path_empty_string_quoted() {
        assert_eq!(shell_quote_path(""), "''");
    }

    // --- command_tail ----------------------------------------------------

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

    // --- merge -----------------------------------------------------------

    #[test]
    fn merge_into_missing_target() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("none.json");
        let merged = merge_event_hook(&path, &cc_fragment()).unwrap();
        // Merging into empty target should produce an object containing exactly
        // our fragment's event key.
        assert!(merged["hooks"]["SessionEnd"].is_array());
        let cmd = merged["hooks"]["SessionEnd"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(cmd, "ROOSTERY_AGENT=cc /sh/path");
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

    // --- FragmentError specific variants (B2) -----------------------------

    #[test]
    fn no_event_key() {
        let frag = json!({"hooks": {}});
        match merge_event_hook(std::path::Path::new("/nonexistent"), &frag) {
            Err(HooksError::Fragment(FragmentError::NoEventKey)) => {}
            other => panic!("expected NoEventKey, got {other:?}"),
        }
    }

    #[test]
    fn multiple_event_keys() {
        let frag = json!({
            "hooks": {
                "Stop": [{"matcher":"*","hooks":[{"type":"command","command":"x"}]}],
                "SessionEnd": [{"matcher":"*","hooks":[{"type":"command","command":"y"}]}]
            }
        });
        match merge_event_hook(std::path::Path::new("/nonexistent"), &frag) {
            Err(HooksError::Fragment(FragmentError::MultipleEventKeys { found: 2 })) => {}
            other => panic!("expected MultipleEventKeys{{2}}, got {other:?}"),
        }
    }

    #[test]
    fn empty_matcher_array() {
        let frag = json!({"hooks": {"SessionEnd": []}});
        match merge_event_hook(std::path::Path::new("/nonexistent"), &frag) {
            Err(HooksError::Fragment(FragmentError::EmptyMatcherArray { event })) => {
                assert_eq!(event, "SessionEnd");
            }
            other => panic!("expected EmptyMatcherArray, got {other:?}"),
        }
    }

    #[test]
    fn empty_hooks_array() {
        let frag = json!({"hooks": {"SessionEnd": [{"matcher":"*","hooks":[]}]}});
        match merge_event_hook(std::path::Path::new("/nonexistent"), &frag) {
            Err(HooksError::Fragment(FragmentError::EmptyHooksArray { event, matcher })) => {
                assert_eq!(event, "SessionEnd");
                assert_eq!(matcher, "*");
            }
            other => panic!("expected EmptyHooksArray, got {other:?}"),
        }
    }

    #[test]
    fn missing_command() {
        let frag = json!({
            "hooks": {
                "SessionEnd": [{"matcher":"*","hooks":[{"type":"command","command":""}]}]
            }
        });
        match merge_event_hook(std::path::Path::new("/nonexistent"), &frag) {
            Err(HooksError::Fragment(FragmentError::MissingCommand { .. })) => {}
            other => panic!("expected MissingCommand, got {other:?}"),
        }
    }

    #[test]
    fn fragment_shape_error_for_arbitrary_json() {
        // String-typed where object expected → Shape error
        let frag = json!({"hooks": "not an object"});
        match merge_event_hook(std::path::Path::new("/nonexistent"), &frag) {
            Err(HooksError::Fragment(FragmentError::Shape(_))) => {}
            other => panic!("expected Shape, got {other:?}"),
        }
    }

    // --- target file errors -----------------------------------------------

    #[test]
    fn target_top_level_not_object() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_existing(dir.path(), "[1, 2, 3]");
        match merge_event_hook(&path, &cc_fragment()) {
            Err(HooksError::Fragment(FragmentError::TargetTopLevelNotObject {
                type_name: "array",
            })) => {}
            other => panic!("expected TargetTopLevelNotObject{{array}}, got {other:?}"),
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

    // --- apply_template end-to-end ---------------------------------------

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

    /// Convenience: AgentKind::template() + apply_template chain works.
    #[test]
    fn apply_template_via_agent_kind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        apply_template(AgentKind::Codex.template(), &path, "/sh").unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("ROOSTERY_AGENT=codex /sh"));
    }
}
