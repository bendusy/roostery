//! Rule engine: YAML schema v1 + match `HookEvent` → `Match`.
//!
//! Phase 4 Module E second sub-feature. Match dimensions (MVP, 3 项):
//! `hook_source` 精确字符串相等 / `workspace_glob` fnmatch（用 `globset`
//! 编译一次性 GlobMatcher）/ `trigger_meta_eq` 点路径取值字面量相等。
//! Action 形状是 opaque `{ runner: String, args: serde_json::Value }`——rules
//! 不解释 args，透传给 Runner impl（dispatcher-runners feature 落地后才有
//! 真正的 Runner trait 消费）。
//!
//! 行为约束（不变量）：
//! - `load` 缺文件 → `Ok(vec![])`，让 first-run dispatcher-loop 不报错
//! - Self-event 短路：`hook_source` starts_with `["dispatcher.", "roostery."]`
//!   直接返 `None`，防 dispatcher 自激（roosery 自己产生的事件不该再触发
//!   新派发）
//! - First-match-wins：`matches` 返 `Option<Match>`，匹配到第一条即返
//! - AND 多维度：`when` 中所有非 None / 非空字段必须满足
//!
//! See `.codestable/features/2026-05-18-dispatcher-rules/dispatcher-rules-design.md`
//! §2.1.2.

use super::hook_event::HookEvent;
use crate::paths;
use globset::{Glob, GlobMatcher};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const RULES_SCHEMA_VERSION: u32 = 1;

const SELF_EVENT_PREFIXES: &[&str] = &["dispatcher.", "roostery."];

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct RuleName(String);

impl RuleName {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RuleName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct RulesConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub rules: Vec<RawRule>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct RawRule {
    pub name: RuleName,
    #[serde(default)]
    pub when: RuleWhen,
    pub action: RuleAction,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct RuleWhen {
    #[serde(default)]
    pub hook_source: Option<String>,
    #[serde(default)]
    pub workspace_glob: Option<String>,
    #[serde(default)]
    pub trigger_meta_eq: BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct RuleAction {
    pub runner: String,
    #[serde(default)]
    pub args: serde_json::Value,
}

/// Compiled-form rule. `workspace_glob` resolved once into `GlobMatcher`.
/// Typestate-lite: caller holding `Vec<CompiledRule>` is guaranteed every
/// glob has already been validated at load time.
#[derive(Debug)]
pub struct CompiledRule {
    pub name: RuleName,
    pub hook_source: Option<String>,
    pub workspace: Option<GlobMatcher>,
    pub trigger_meta_eq: BTreeMap<String, serde_json::Value>,
    pub action: RuleAction,
}

#[derive(Debug, Clone)]
pub struct Match<'a> {
    pub rule_name: &'a RuleName,
    pub runner: &'a str,
    pub args: &'a serde_json::Value,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RulesError {
    #[error("failed to read rules file {path}: {source}")]
    LoadFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse rules YAML {path}: {source}")]
    ParseFailed {
        path: PathBuf,
        #[source]
        source: serde_yml::Error,
    },
    #[error("rules schema version {found} not supported (expected {expected})")]
    SchemaVersionMismatch { found: u32, expected: u32 },
    #[error("duplicate rule name {name}")]
    DuplicateRuleName { name: RuleName },
    #[error("rule {name} has invalid workspace_glob {glob:?}: {source}")]
    InvalidGlob {
        name: RuleName,
        glob: String,
        #[source]
        source: globset::Error,
    },
}

/// Load + compile in one go. Missing file → `Ok(vec![])`.
pub fn load() -> Result<Vec<CompiledRule>, RulesError> {
    let path = paths::rules_path();
    load_from(&path)
}

pub fn load_from(path: &Path) -> Result<Vec<CompiledRule>, RulesError> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(RulesError::LoadFailed {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let raw: RulesConfig =
        serde_yml::from_slice(&bytes).map_err(|source| RulesError::ParseFailed {
            path: path.to_path_buf(),
            source,
        })?;
    if raw.schema_version != RULES_SCHEMA_VERSION {
        return Err(RulesError::SchemaVersionMismatch {
            found: raw.schema_version,
            expected: RULES_SCHEMA_VERSION,
        });
    }

    let mut seen: BTreeSet<RuleName> = BTreeSet::new();
    let mut out = Vec::with_capacity(raw.rules.len());
    for rule in raw.rules {
        if !seen.insert(rule.name.clone()) {
            return Err(RulesError::DuplicateRuleName { name: rule.name });
        }
        out.push(compile_rule(rule)?);
    }
    Ok(out)
}

fn compile_rule(raw: RawRule) -> Result<CompiledRule, RulesError> {
    let workspace = match raw.when.workspace_glob.as_deref() {
        None => None,
        Some(pattern) => {
            let glob = Glob::new(pattern).map_err(|source| RulesError::InvalidGlob {
                name: raw.name.clone(),
                glob: pattern.to_string(),
                source,
            })?;
            Some(glob.compile_matcher())
        }
    };
    Ok(CompiledRule {
        name: raw.name,
        hook_source: raw.when.hook_source,
        workspace,
        trigger_meta_eq: raw.when.trigger_meta_eq,
        action: raw.action,
    })
}

/// First-match-wins. Returns `None` when no rule matches or when the event is
/// a self-event (Roostery-originated event, prevents dispatcher self-feed).
pub fn matches<'a>(rules: &'a [CompiledRule], event: &'a HookEvent) -> Option<Match<'a>> {
    if is_self_event(event) {
        return None;
    }
    for rule in rules {
        if matches_rule(rule, event) {
            return Some(Match {
                rule_name: &rule.name,
                runner: &rule.action.runner,
                args: &rule.action.args,
            });
        }
    }
    None
}

fn is_self_event(event: &HookEvent) -> bool {
    SELF_EVENT_PREFIXES
        .iter()
        .any(|p| event.hook_source.starts_with(p))
}

fn matches_rule(rule: &CompiledRule, event: &HookEvent) -> bool {
    if let Some(want) = rule.hook_source.as_deref()
        && want != event.hook_source
    {
        return false;
    }
    if let Some(matcher) = &rule.workspace
        && !matcher.is_match(&event.workspace)
    {
        return false;
    }
    for (path, expected) in &rule.trigger_meta_eq {
        match event.trigger_meta_path(path) {
            Some(actual) if actual == expected => continue,
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn ev(hook_source: &str, workspace: &str, trigger_meta: serde_json::Value) -> HookEvent {
        HookEvent {
            schema_version: 1,
            hook_source: hook_source.to_string(),
            session_id: "s".to_string(),
            workspace: PathBuf::from(workspace),
            trigger_meta,
            trace: None,
        }
    }

    fn rule(name: &str, when: RuleWhen, runner: &str, args: serde_json::Value) -> CompiledRule {
        compile_rule(RawRule {
            name: RuleName::new(name),
            when,
            action: RuleAction {
                runner: runner.to_string(),
                args,
            },
        })
        .unwrap()
    }

    // --- S2 type tests ----------------------------------------------------

    #[test]
    fn rule_name_serde_transparent() {
        let n = RuleName::new("foo");
        let s = serde_json::to_string(&n).unwrap();
        assert_eq!(s, "\"foo\"");
        let back: RuleName = serde_json::from_str("\"foo\"").unwrap();
        assert_eq!(back, n);
    }

    #[test]
    fn rules_error_display_contains_path() {
        let err = RulesError::SchemaVersionMismatch {
            found: 2,
            expected: 1,
        };
        let msg = err.to_string();
        assert!(msg.contains('2'));
        assert!(msg.contains('1'));
    }

    #[test]
    fn rules_schema_version_const_is_one() {
        assert_eq!(RULES_SCHEMA_VERSION, 1);
    }

    #[test]
    fn self_event_prefixes_include_dispatcher_and_roostery() {
        assert!(SELF_EVENT_PREFIXES.contains(&"dispatcher."));
        assert!(SELF_EVENT_PREFIXES.contains(&"roostery."));
    }

    // --- S3 load + compile tests ------------------------------------------

    #[test]
    fn load_missing_file_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("nope.yaml");
        assert!(load_from(&p).unwrap().is_empty());
    }

    #[test]
    fn load_invalid_yaml_returns_parse_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("bad.yaml");
        fs::write(&p, b"this is :: not :: valid yaml [\n").unwrap();
        match load_from(&p) {
            Err(RulesError::ParseFailed { .. }) => {}
            other => panic!("expected ParseFailed, got {other:?}"),
        }
    }

    #[test]
    fn load_wrong_schema_version_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("v2.yaml");
        fs::write(&p, b"schema_version: 2\nrules: []\n").unwrap();
        match load_from(&p) {
            Err(RulesError::SchemaVersionMismatch { found, expected }) => {
                assert_eq!(found, 2);
                assert_eq!(expected, 1);
            }
            other => panic!("expected SchemaVersionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn load_duplicate_rule_name_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("dup.yaml");
        let body = r#"schema_version: 1
rules:
  - name: foo
    when: {}
    action:
      runner: noop
  - name: foo
    when: {}
    action:
      runner: noop
"#;
        fs::write(&p, body).unwrap();
        match load_from(&p) {
            Err(RulesError::DuplicateRuleName { name }) => {
                assert_eq!(name.as_str(), "foo");
            }
            other => panic!("expected DuplicateRuleName, got {other:?}"),
        }
    }

    #[test]
    fn load_invalid_glob_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("bad_glob.yaml");
        let body = r#"schema_version: 1
rules:
  - name: bad
    when:
      workspace_glob: "["
    action:
      runner: noop
"#;
        fs::write(&p, body).unwrap();
        match load_from(&p) {
            Err(RulesError::InvalidGlob { name, glob, .. }) => {
                assert_eq!(name.as_str(), "bad");
                assert_eq!(glob, "[");
            }
            other => panic!("expected InvalidGlob, got {other:?}"),
        }
    }

    #[test]
    fn load_happy_two_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("ok.yaml");
        let body = r#"schema_version: 1
rules:
  - name: cc-projects
    when:
      hook_source: claude-code-stop
      workspace_glob: "/Users/*/Projects/**"
    action:
      runner: cc_headless
      args:
        prompt: hi
  - name: codex-only
    when:
      hook_source: codex-stop
    action:
      runner: codex_exec
      args: {}
"#;
        fs::write(&p, body).unwrap();
        let rules = load_from(&p).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].name.as_str(), "cc-projects");
        assert_eq!(rules[0].hook_source.as_deref(), Some("claude-code-stop"));
        assert!(rules[0].workspace.is_some());
        assert_eq!(rules[0].action.runner, "cc_headless");
        assert_eq!(rules[1].action.runner, "codex_exec");
    }

    // --- S4 matches tests -------------------------------------------------

    #[test]
    fn matches_empty_rules_returns_none() {
        let event = ev("claude-code-stop", "/tmp", json!({}));
        assert!(matches(&[], &event).is_none());
    }

    #[test]
    fn matches_self_event_dispatcher_short_circuits() {
        let event = ev("dispatcher.replay", "/tmp", json!({}));
        let r = rule("any", RuleWhen::default(), "noop", json!({}));
        assert!(matches(std::slice::from_ref(&r), &event).is_none());
    }

    #[test]
    fn matches_self_event_roostery_short_circuits() {
        let event = ev("roostery.internal", "/tmp", json!({}));
        let r = rule("any", RuleWhen::default(), "noop", json!({}));
        assert!(matches(std::slice::from_ref(&r), &event).is_none());
    }

    #[test]
    fn matches_hook_source_eq_hit() {
        let event = ev("claude-code-stop", "/tmp", json!({}));
        let when = RuleWhen {
            hook_source: Some("claude-code-stop".to_string()),
            ..RuleWhen::default()
        };
        let r = rule("cc", when, "cc_headless", json!({}));
        let m = matches(std::slice::from_ref(&r), &event).unwrap();
        assert_eq!(m.rule_name.as_str(), "cc");
        assert_eq!(m.runner, "cc_headless");
    }

    #[test]
    fn matches_hook_source_eq_miss_skips() {
        let event = ev("codex-stop", "/tmp", json!({}));
        let when = RuleWhen {
            hook_source: Some("claude-code-stop".to_string()),
            ..RuleWhen::default()
        };
        let r = rule("cc", when, "cc_headless", json!({}));
        assert!(matches(std::slice::from_ref(&r), &event).is_none());
    }

    #[test]
    fn matches_workspace_glob_double_star_hit() {
        let event = ev(
            "claude-code-stop",
            "/Users/ben/Projects/roostery/crates/foo",
            json!({}),
        );
        let when = RuleWhen {
            workspace_glob: Some("/Users/*/Projects/**".to_string()),
            ..RuleWhen::default()
        };
        let r = rule("proj", when, "noop", json!({}));
        let m = matches(std::slice::from_ref(&r), &event).unwrap();
        assert_eq!(m.rule_name.as_str(), "proj");
    }

    #[test]
    fn matches_trigger_meta_eq_hit() {
        let event = ev(
            "claude-code-stop",
            "/tmp",
            json!({"action": "stop", "user": {"role": "owner"}}),
        );
        let mut meta = BTreeMap::new();
        meta.insert("action".to_string(), json!("stop"));
        meta.insert("user.role".to_string(), json!("owner"));
        let when = RuleWhen {
            trigger_meta_eq: meta,
            ..RuleWhen::default()
        };
        let r = rule("meta", when, "noop", json!({}));
        assert!(matches(std::slice::from_ref(&r), &event).is_some());
    }

    #[test]
    fn matches_trigger_meta_path_missing_skips_rule() {
        let event = ev("claude-code-stop", "/tmp", json!({"action": "stop"}));
        let mut meta = BTreeMap::new();
        meta.insert("user.role".to_string(), json!("owner"));
        let when = RuleWhen {
            trigger_meta_eq: meta,
            ..RuleWhen::default()
        };
        let r = rule("meta", when, "noop", json!({}));
        assert!(matches(std::slice::from_ref(&r), &event).is_none());
    }

    #[test]
    fn matches_and_dimensions_all_pass() {
        let event = ev(
            "claude-code-stop",
            "/Users/ben/Projects/roostery",
            json!({"action": "stop"}),
        );
        let mut meta = BTreeMap::new();
        meta.insert("action".to_string(), json!("stop"));
        let when = RuleWhen {
            hook_source: Some("claude-code-stop".to_string()),
            workspace_glob: Some("/Users/*/Projects/**".to_string()),
            trigger_meta_eq: meta,
        };
        let r = rule("triple", when, "cc_headless", json!({"k": "v"}));
        let m = matches(std::slice::from_ref(&r), &event).unwrap();
        assert_eq!(m.runner, "cc_headless");
        assert_eq!(m.args, &json!({"k": "v"}));
    }

    #[test]
    fn matches_and_dimensions_partial_fail() {
        let event = ev(
            "claude-code-stop",
            "/Users/ben/Projects/roostery",
            json!({"action": "other"}),
        );
        let mut meta = BTreeMap::new();
        meta.insert("action".to_string(), json!("stop"));
        let when = RuleWhen {
            hook_source: Some("claude-code-stop".to_string()),
            workspace_glob: Some("/Users/*/Projects/**".to_string()),
            trigger_meta_eq: meta,
        };
        let r = rule("triple", when, "cc_headless", json!({}));
        assert!(matches(std::slice::from_ref(&r), &event).is_none());
    }

    #[test]
    fn matches_first_match_wins_skips_later_rules() {
        let event = ev("claude-code-stop", "/tmp", json!({}));
        let when_a = RuleWhen {
            hook_source: Some("claude-code-stop".to_string()),
            ..RuleWhen::default()
        };
        let r_a = rule("first", when_a, "alpha", json!({}));
        let when_b = RuleWhen {
            hook_source: Some("claude-code-stop".to_string()),
            ..RuleWhen::default()
        };
        let r_b = rule("second", when_b, "beta", json!({}));
        let rules = vec![r_a, r_b];
        let m = matches(&rules, &event).unwrap();
        assert_eq!(m.rule_name.as_str(), "first");
        assert_eq!(m.runner, "alpha");
    }
}
