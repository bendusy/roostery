//! `bot_bridge::role` — BotRole / BotsConfig / bots.yaml 加载 + mention 匹配。
//!
//! 见 `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md`
//! §2.1（名词层）+ §3 验收契约 B1/B2/B3/B4。
//!
//! 来源：legacy/python/src/roostery/bot_role.py（参考，不维护）。

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::bot_bridge::event::ImEvent;

/// `~/.roostery/bots.yaml` 顶层 `schema_version` 公开承诺。
///
/// design §1.3 D8：schema 字段变更需 bump + cs-roadmap update + 旧版兼容反序列化。
pub const BOTS_SCHEMA_VERSION: u32 = 1;

/// 单条 bot 配置（design §2.1）。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct BotRole {
    /// lark-cli profile name 双关。
    pub app_id: String,
    /// 显示用："tech-lead" / "scout"。
    pub role: String,
    /// `@<alias>` 匹配键。
    pub mention_alias: String,
    /// Runner::kind() 值，如 "cc_headless"。
    pub runner: String,
    /// runner 默认工作目录。
    pub default_cwd: PathBuf,
    /// `{message}` `{sender}` `{chat_id}` 占位的 prompt 模板。
    pub prompt_template: String,
    /// reply 模板；缺省时为 "{result}"。
    #[serde(default = "default_reply_template")]
    pub reply_template: String,
    /// 空 = 不限制；非空时事件 chat_id 必须命中。
    #[serde(default)]
    pub chat_whitelist: Vec<String>,
    /// 接力链下一棒；可空字符串。
    #[serde(default)]
    pub next_bot_mention: String,
}

fn default_reply_template() -> String {
    "{result}".to_string()
}

/// `~/.roostery/bots.yaml` 顶层结构（design §2.1）。
///
/// `schema_version` 缺失时默认 = `BOTS_SCHEMA_VERSION`（向后兼容，B2）。
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct BotsConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub bots: Vec<BotRole>,
}

fn default_schema_version() -> u32 {
    BOTS_SCHEMA_VERSION
}

/// bots.yaml 加载 / 解析错误（design §2.1 4 变体）。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BotRoleError {
    #[error("bots.yaml load failed: {0}")]
    LoadFailed(#[source] std::io::Error),
    #[error("bots.yaml parse failed: {0}")]
    ParseFailed(#[source] serde_yml::Error),
    #[error("schema_version mismatch: found={found}, expected={expected}")]
    SchemaVersionMismatch { found: u32, expected: u32 },
    #[error("bots[{index}] missing required field {field}")]
    MissingField { index: usize, field: &'static str },
}

/// 必填字段（serde-default 之外的字段都必填）。
///
/// `reply_template` / `chat_whitelist` / `next_bot_mention` 有 serde default，不算必填。
const REQUIRED_FIELDS: &[&str] = &[
    "app_id",
    "role",
    "mention_alias",
    "runner",
    "default_cwd",
    "prompt_template",
];

/// 加载并校验 bots.yaml。
///
/// 流程：
/// 1. 读文件 → `LoadFailed`
/// 2. 解析为 `serde_yml::Value` → `ParseFailed`
/// 3. 顶层 `schema_version` 缺失 → 默认 = `BOTS_SCHEMA_VERSION`；显式且 ≠ 当前 → `SchemaVersionMismatch`
/// 4. 遍历 `bots` 数组，对每条 bot 的必填字段做存在性校验 → `MissingField{ index, field }`
/// 5. 反序列化为 `BotsConfig`
pub fn load_bots(path: &Path) -> Result<BotsConfig, BotRoleError> {
    let bytes = std::fs::read(path).map_err(BotRoleError::LoadFailed)?;
    let raw: serde_yml::Value = serde_yml::from_slice(&bytes).map_err(BotRoleError::ParseFailed)?;

    // schema_version 校验（缺失=1 向后兼容；显式 ≠1 报错）
    if let Some(found) = raw.get("schema_version").and_then(|v| v.as_u64())
        && found as u32 != BOTS_SCHEMA_VERSION
    {
        return Err(BotRoleError::SchemaVersionMismatch {
            found: found as u32,
            expected: BOTS_SCHEMA_VERSION,
        });
    }

    // 必填字段校验（先于 serde::Deserialize 报更友好的错）
    if let Some(seq) = raw.get("bots").and_then(|v| v.as_sequence()) {
        for (index, bot) in seq.iter().enumerate() {
            for field in REQUIRED_FIELDS {
                if bot.get(*field).is_none() {
                    return Err(BotRoleError::MissingField { index, field });
                }
            }
        }
    }

    serde_yml::from_value(raw).map_err(BotRoleError::ParseFailed)
}

/// 判定事件是否归属本 bot（design §2.1 + §3 B1/B4）。
///
/// 规则：
/// 1. `bot.chat_whitelist` 非空时，事件 `chat_id` 必须命中（B4）
/// 2. 内容以 `@<mention_alias>` 开头，且紧随分隔符 = U+0020 / U+00A0 / U+3000 之一（B1）
///    或文本就此结束（仅 mention 无后文，按命中处理）
pub fn event_matches_bot(event: &ImEvent, bot: &BotRole) -> bool {
    if !bot.chat_whitelist.is_empty() && !bot.chat_whitelist.iter().any(|c| c == &event.chat_id) {
        return false;
    }
    matches_mention_prefix(&event.content, &bot.mention_alias).is_some()
}

/// 从 IM event 抽出 mention 后的正文（design §2.1）。
///
/// 若 `event_matches_bot` 不命中，返回整段 content 兜底（design 签名为 `&str` 而非 Option）。
/// 命中时返回去掉 `@alias<分隔符>` 前缀后的 body（leading 分隔符已被剥掉）。
pub fn extract_message_body<'a>(event: &'a ImEvent, bot: &BotRole) -> &'a str {
    match matches_mention_prefix(&event.content, &bot.mention_alias) {
        Some(rest) => rest,
        None => event.content.as_str(),
    }
}

/// 三种空格容忍的 mention 前缀匹配。
///
/// 返回 `Some(rest)`：rest 是 `@alias` + 1 个分隔符之后的全部内容；
/// `@alias` 后直接 EOF 也视为命中，返回 `Some("")`。
fn matches_mention_prefix<'a>(content: &'a str, alias: &str) -> Option<&'a str> {
    let alias_with_at = format!("@{alias}");
    let rest = content.strip_prefix(&alias_with_at)?;
    // 紧随 alias 之后：要么是 EOF（纯 mention 无正文），要么是受支持的分隔符之一
    if rest.is_empty() {
        return Some(rest);
    }
    let mut chars = rest.chars();
    let first = chars.next()?;
    match first {
        ' ' | '\u{00A0}' | '\u{3000}' => {
            // 跳过 1 个分隔符；保留其余内容（design：剥前缀拿正文）
            Some(&rest[first.len_utf8()..])
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn mk_event(chat_id: &str, content: &str) -> ImEvent {
        // 通过 serde JSON 构造（ImEvent 是 non_exhaustive，但本 crate 内可直接构造）。
        ImEvent {
            message_id: "om_1".into(),
            chat_id: chat_id.into(),
            chat_type: "group".into(),
            message_type: "text".into(),
            sender_id: "u_1".into(),
            content: content.into(),
        }
    }

    fn mk_bot(alias: &str, whitelist: Vec<String>) -> BotRole {
        BotRole {
            app_id: "cli_app".into(),
            role: "scout".into(),
            mention_alias: alias.into(),
            runner: "cc_headless".into(),
            default_cwd: PathBuf::from("/tmp"),
            prompt_template: "{message}".into(),
            reply_template: default_reply_template(),
            chat_whitelist: whitelist,
            next_bot_mention: String::new(),
        }
    }

    #[test]
    fn yaml_roundtrip_full_config() {
        let yaml = r#"
schema_version: 1
bots:
  - app_id: cli_a
    role: tech-lead
    mention_alias: tl
    runner: cc_headless
    default_cwd: /tmp/cwd
    prompt_template: "do {message}"
    reply_template: "ok {result}"
    chat_whitelist: ["oc_1", "oc_2"]
    next_bot_mention: "@scout"
"#;
        let cfg: BotsConfig = serde_yml::from_str(yaml).unwrap();
        assert_eq!(cfg.schema_version, 1);
        assert_eq!(cfg.bots.len(), 1);
        let bot = &cfg.bots[0];
        assert_eq!(bot.app_id, "cli_a");
        assert_eq!(bot.role, "tech-lead");
        assert_eq!(bot.mention_alias, "tl");
        assert_eq!(bot.runner, "cc_headless");
        assert_eq!(bot.default_cwd, PathBuf::from("/tmp/cwd"));
        assert_eq!(bot.prompt_template, "do {message}");
        assert_eq!(bot.reply_template, "ok {result}");
        assert_eq!(bot.chat_whitelist, vec!["oc_1".to_string(), "oc_2".into()]);
        assert_eq!(bot.next_bot_mention, "@scout");
    }

    #[test]
    fn mention_matches_three_space_variants() {
        let bot = mk_bot("tl", vec![]);

        // U+0020 ASCII space
        let ev1 = mk_event("oc_x", "@tl hello world");
        assert!(event_matches_bot(&ev1, &bot));
        assert_eq!(extract_message_body(&ev1, &bot), "hello world");

        // U+00A0 NBSP
        let ev2 = mk_event("oc_x", "@tl\u{00A0}hi");
        assert!(event_matches_bot(&ev2, &bot));
        assert_eq!(extract_message_body(&ev2, &bot), "hi");

        // U+3000 全角空格
        let ev3 = mk_event("oc_x", "@tl\u{3000}你好");
        assert!(event_matches_bot(&ev3, &bot));
        assert_eq!(extract_message_body(&ev3, &bot), "你好");

        // 非空格紧贴：不识别（mention 边界保护）
        let ev4 = mk_event("oc_x", "@tlhi");
        assert!(!event_matches_bot(&ev4, &bot));

        // 不带 @：不识别
        let ev5 = mk_event("oc_x", "tl hi");
        assert!(!event_matches_bot(&ev5, &bot));
    }

    #[test]
    fn chat_whitelist_filters_unmatched_chats() {
        let bot = mk_bot("tl", vec!["oc_allow".into()]);

        let ev_in = mk_event("oc_allow", "@tl run");
        assert!(event_matches_bot(&ev_in, &bot));

        let ev_out = mk_event("oc_other", "@tl run");
        assert!(!event_matches_bot(&ev_out, &bot));

        // 空 whitelist = 不限制
        let bot_open = mk_bot("tl", vec![]);
        let ev_any = mk_event("oc_random", "@tl run");
        assert!(event_matches_bot(&ev_any, &bot_open));
    }

    #[test]
    fn schema_version_missing_defaults_to_one() {
        let yaml = r#"
bots:
  - app_id: a
    role: r
    mention_alias: m
    runner: cc_headless
    default_cwd: /tmp
    prompt_template: "{message}"
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let cfg = load_bots(f.path()).unwrap();
        assert_eq!(cfg.schema_version, BOTS_SCHEMA_VERSION);
        assert_eq!(cfg.schema_version, 1);
        assert_eq!(cfg.bots.len(), 1);
    }

    #[test]
    fn schema_version_two_returns_mismatch_error() {
        let yaml = r#"
schema_version: 2
bots: []
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let err = load_bots(f.path()).unwrap_err();
        match err {
            BotRoleError::SchemaVersionMismatch { found, expected } => {
                assert_eq!(found, 2);
                assert_eq!(expected, BOTS_SCHEMA_VERSION);
            }
            other => panic!("expected SchemaVersionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn missing_required_field_reports_index_and_field() {
        // bot[1] 缺 mention_alias
        let yaml = r#"
schema_version: 1
bots:
  - app_id: a
    role: r
    mention_alias: m
    runner: cc_headless
    default_cwd: /tmp
    prompt_template: "{message}"
  - app_id: b
    role: r2
    runner: cc_headless
    default_cwd: /tmp
    prompt_template: "{message}"
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let err = load_bots(f.path()).unwrap_err();
        match err {
            BotRoleError::MissingField { index, field } => {
                assert_eq!(index, 1);
                assert_eq!(field, "mention_alias");
            }
            other => panic!("expected MissingField, got {other:?}"),
        }
    }

    #[test]
    fn load_failed_when_file_missing() {
        let path = PathBuf::from("/nonexistent/path/does/not/exist/bots.yaml");
        let err = load_bots(&path).unwrap_err();
        assert!(matches!(err, BotRoleError::LoadFailed(_)));
    }

    #[test]
    fn parse_failed_on_invalid_yaml() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b": : :not valid yaml\n  - - -").unwrap();
        let err = load_bots(f.path()).unwrap_err();
        assert!(matches!(err, BotRoleError::ParseFailed(_)));
    }

    #[test]
    fn extract_message_body_falls_back_to_full_content_when_no_match() {
        let bot = mk_bot("tl", vec![]);
        let ev = mk_event("oc_x", "no mention here");
        assert_eq!(extract_message_body(&ev, &bot), "no mention here");
    }

    #[test]
    fn mention_alone_without_body_is_match_with_empty_rest() {
        let bot = mk_bot("tl", vec![]);
        let ev = mk_event("oc_x", "@tl");
        assert!(event_matches_bot(&ev, &bot));
        assert_eq!(extract_message_body(&ev, &bot), "");
    }
}
