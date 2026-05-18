//! End-to-end integration tests for the `hooks_merge` module.

use roostery::hooks_merge::{
    self, CC_STOP_HOOK_JSON, CODEX_STOP_HOOK_JSON, STOP_HOOK_AGENT_NOTIFY_SH,
};
use serde_json::Value;

#[test]
fn cc_and_codex_coexist_in_one_settings_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");

    hooks_merge::apply_template(CC_STOP_HOOK_JSON, &path, "/bin/sh-cc").unwrap();
    hooks_merge::apply_template(CODEX_STOP_HOOK_JSON, &path, "/bin/sh-codex").unwrap();

    let body = std::fs::read_to_string(&path).unwrap();
    let v: Value = serde_json::from_str(&body).unwrap();
    let session_end = v["hooks"]["SessionEnd"].as_array().unwrap();
    // Both hooks share matcher "*" → same bucket, two commands appended
    assert_eq!(session_end.len(), 1);
    let bucket_hooks = session_end[0]["hooks"].as_array().unwrap();
    assert_eq!(bucket_hooks.len(), 2);
    let commands: Vec<&str> = bucket_hooks
        .iter()
        .map(|h| h["command"].as_str().unwrap())
        .collect();
    assert!(commands.contains(&"ROOSTERY_AGENT=cc /bin/sh-cc"));
    assert!(commands.contains(&"ROOSTERY_AGENT=codex /bin/sh-codex"));
}

#[test]
fn output_is_indent_two_with_trailing_newline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    hooks_merge::apply_template(CC_STOP_HOOK_JSON, &path, "/x").unwrap();
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.ends_with('\n'), "must end with \\n");
    // First nested key starts with two spaces
    assert!(body.contains("\n  \"hooks\""));
    // SessionEnd array element start indented with 6 spaces (3 levels × 2)
    assert!(body.contains("\n      {"));
}

#[test]
fn three_consts_are_nonempty_strings() {
    assert!(!CC_STOP_HOOK_JSON.is_empty());
    assert!(!CODEX_STOP_HOOK_JSON.is_empty());
    assert!(!STOP_HOOK_AGENT_NOTIFY_SH.is_empty());
    // Roundtrip parse the JSON templates (placeholder is still there but valid JSON)
    let _: Value = serde_json::from_str(CC_STOP_HOOK_JSON).unwrap();
    let _: Value = serde_json::from_str(CODEX_STOP_HOOK_JSON).unwrap();
}

#[test]
fn double_apply_is_byte_for_byte_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    hooks_merge::apply_template(CC_STOP_HOOK_JSON, &path, "/sh").unwrap();
    let first = std::fs::read(&path).unwrap();
    hooks_merge::apply_template(CC_STOP_HOOK_JSON, &path, "/sh").unwrap();
    let second = std::fs::read(&path).unwrap();
    assert_eq!(first, second);
}
