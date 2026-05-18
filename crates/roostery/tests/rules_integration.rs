//! End-to-end: load rules.yaml from real disk → match real HookEvent.

use roostery::hook_event::HookEvent;
use roostery::rules::{self, RuleName};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

fn write_rules_yaml(dir: &std::path::Path, body: &str) -> PathBuf {
    let p = dir.join("rules.yaml");
    fs::write(&p, body).unwrap();
    p
}

fn build_event(hook_source: &str, workspace: &str, trigger_meta: serde_json::Value) -> HookEvent {
    let raw = json!({
        "schema_version": 1,
        "hook_source": hook_source,
        "session_id": "sess_e2e",
        "workspace": workspace,
        "trigger_meta": trigger_meta,
    });
    serde_json::from_value(raw).unwrap()
}

#[test]
fn load_and_match_real_yaml_cc_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let body = r#"schema_version: 1
rules:
  - name: cc-projects
    when:
      hook_source: claude-code-stop
      workspace_glob: "/Users/*/Projects/**"
    action:
      runner: cc_headless
      args:
        prompt: "Summarize session"
        model: sonnet-4
  - name: codex-fallback
    when:
      hook_source: codex-stop
    action:
      runner: codex_exec
      args: {}
"#;
    let path = write_rules_yaml(tmp.path(), body);
    let compiled = rules::load_from(&path).unwrap();
    assert_eq!(compiled.len(), 2);

    let event = build_event(
        "claude-code-stop",
        "/Users/ben/Projects/roostery",
        json!({"action": "stop"}),
    );
    let m = rules::matches(&compiled, &event).expect("cc rule must match");
    assert_eq!(m.rule_name, &RuleName::new("cc-projects"));
    assert_eq!(m.runner, "cc_headless");
    assert_eq!(m.args["prompt"], json!("Summarize session"));
    assert_eq!(m.args["model"], json!("sonnet-4"));
}

#[test]
fn load_and_match_codex_when_cc_workspace_glob_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let body = r#"schema_version: 1
rules:
  - name: cc-projects
    when:
      hook_source: claude-code-stop
      workspace_glob: "/Users/*/Projects/**"
    action:
      runner: cc_headless
      args: {}
  - name: codex-anywhere
    when:
      hook_source: codex-stop
    action:
      runner: codex_exec
      args:
        flag: true
"#;
    let path = write_rules_yaml(tmp.path(), body);
    let compiled = rules::load_from(&path).unwrap();

    // First-match attempt fails hook_source (event is codex-stop not CC);
    // second rule matches.
    let event = build_event("codex-stop", "/opt/foreign", json!({}));
    let m = rules::matches(&compiled, &event).expect("codex rule must match");
    assert_eq!(m.rule_name.as_str(), "codex-anywhere");
    assert_eq!(m.runner, "codex_exec");
    assert_eq!(m.args, &json!({"flag": true}));
}

#[test]
fn self_event_does_not_match_any_rule() {
    let tmp = tempfile::tempdir().unwrap();
    let body = r#"schema_version: 1
rules:
  - name: catch-all
    when: {}
    action:
      runner: noop
      args: {}
"#;
    let path = write_rules_yaml(tmp.path(), body);
    let compiled = rules::load_from(&path).unwrap();
    assert_eq!(compiled.len(), 1);

    let event = build_event("dispatcher.replay", "/tmp", json!({}));
    assert!(rules::matches(&compiled, &event).is_none());
}

#[test]
fn missing_rules_file_returns_empty_set() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("not-there.yaml");
    let compiled = rules::load_from(&path).unwrap();
    assert!(compiled.is_empty());

    let event = build_event("claude-code-stop", "/tmp", json!({}));
    assert!(rules::matches(&compiled, &event).is_none());
}

#[test]
fn trigger_meta_eq_with_nested_path_real_yaml() {
    let tmp = tempfile::tempdir().unwrap();
    let body = r#"schema_version: 1
rules:
  - name: owner-only
    when:
      trigger_meta_eq:
        "user.role": owner
        "action": stop
    action:
      runner: cc_headless
      args: {}
"#;
    let path = write_rules_yaml(tmp.path(), body);
    let compiled = rules::load_from(&path).unwrap();

    let match_event = build_event(
        "claude-code-stop",
        "/tmp",
        json!({"action": "stop", "user": {"role": "owner"}}),
    );
    assert!(rules::matches(&compiled, &match_event).is_some());

    let miss_event = build_event(
        "claude-code-stop",
        "/tmp",
        json!({"action": "stop", "user": {"role": "guest"}}),
    );
    assert!(rules::matches(&compiled, &miss_event).is_none());
}
