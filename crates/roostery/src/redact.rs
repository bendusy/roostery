//! Logging-boundary scrubber: redact sensitive values from data flowing into
//! the local journal. See `.codestable/features/2026-05-15-core-redact/` for the
//! full design.

use std::sync::LazyLock;

use regex::Regex;

/// Placeholder string that replaces a sensitive value.
pub const MASK: &str = "***";

/// Default sensitive field-name list (compared after key normalization).
///
/// 11 entries: 7 from the Python baseline parity + 4 defensive-default
/// extensions covering common industry-wide secrets.
pub const SENSITIVE_KEYS: &[&str] = &[
    // Python baseline (7)
    "app_secret",
    "access_token",
    "refresh_token",
    "user_access_token",
    "tenant_access_token",
    "authorization",
    "api_key",
    // Defensive-default extensions (4)
    "password",
    "secret",
    "cookie",
    "private_key",
];

/// Recursively redact sensitive values in a JSON value.
///
/// Returns `(redacted_value, audit_paths)` where `audit_paths` are RFC 6901
/// JSON Pointer strings pointing at each redacted leaf.
pub fn scrub_value(value: &serde_json::Value) -> (serde_json::Value, Vec<String>) {
    let mut paths = Vec::new();
    let new_value = scrub_value_inner(value, "", &mut paths);
    (new_value, paths)
}

fn scrub_value_inner(
    value: &serde_json::Value,
    prefix: &str,
    paths: &mut Vec<String>,
) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut new_map = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                let segment = jsonptr_escape(k);
                let child_path = format!("{}/{}", prefix, segment);
                if is_sensitive_key(k) {
                    new_map.insert(k.clone(), Value::String(MASK.to_string()));
                    paths.push(child_path);
                } else {
                    new_map.insert(k.clone(), scrub_value_inner(v, &child_path, paths));
                }
            }
            Value::Object(new_map)
        }
        Value::Array(arr) => {
            let new_arr: Vec<Value> = arr
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let child_path = format!("{}/{}", prefix, i);
                    scrub_value_inner(v, &child_path, paths)
                })
                .collect();
            Value::Array(new_arr)
        }
        _ => value.clone(),
    }
}

/// RFC 6901 escape: `~` → `~0`, `/` → `~1`. Order matters (escape `~` first).
fn jsonptr_escape(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

/// Redact sensitive values inside a CLI argv array.
///
/// Returns `(redacted_argv, audit_paths)` where `audit_paths` are strings of
/// the form `argv[N]`.
pub fn scrub_argv(argv: &[String]) -> (Vec<String>, Vec<String>) {
    let mut out: Vec<String> = argv.to_vec();
    let mut paths: Vec<String> = Vec::new();
    let mut i = 0;
    while i < out.len() {
        let token = out[i].clone();
        // --flag value
        if is_sensitive_flag(&token) && i + 1 < out.len() {
            out[i + 1] = MASK.to_string();
            paths.push(format!("argv[{}]", i + 1));
            i += 2;
            continue;
        }
        // --flag=value
        if token.starts_with("--")
            && let Some((flag, _)) = token.split_once('=')
            && is_sensitive_flag(flag)
        {
            out[i] = format!("{}={}", flag, MASK);
            paths.push(format!("argv[{}]", i));
            i += 1;
            continue;
        }
        // --header "Name: value" / -H "Name: value"
        if (token == "--header" || token == "-H") && i + 1 < out.len() {
            if let Some(new_val) = scrub_header_value(&out[i + 1]) {
                out[i + 1] = new_val;
                paths.push(format!("argv[{}]", i + 1));
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    (out, paths)
}

/// Normalize a key: lowercase, replace `-` with `_`, strip leading `_`.
fn normalize_key(key: &str) -> String {
    let lower = key.to_ascii_lowercase().replace('-', "_");
    lower.trim_start_matches('_').to_string()
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = normalize_key(key);
    SENSITIVE_KEYS.iter().any(|k| *k == normalized)
}

fn is_sensitive_flag(flag: &str) -> bool {
    if let Some(stripped) = flag.strip_prefix("--") {
        is_sensitive_key(stripped)
    } else {
        false
    }
}

/// If the value parses as `Name: rest` and `Name` is a sensitive key,
/// return the redacted form `Name: ***`. Otherwise return None.
fn scrub_header_value(value: &str) -> Option<String> {
    let (name, _rest) = value.split_once(':')?;
    let name_trimmed = name.trim();
    if is_sensitive_key(name_trimmed) {
        Some(format!("{}: {}", name_trimmed, MASK))
    } else {
        None
    }
}

/// Redact sensitive values inside a raw text blob via regex substitution.
///
/// Does not emit audit paths — regex replacement loses positional accuracy.
pub fn scrub_text(text: &str) -> String {
    let mut out = text.to_string();
    for re in TEXT_JSON_PATTERNS.iter() {
        out = re
            .replace_all(&out, format!("${{1}}{}${{2}}", MASK))
            .into_owned();
    }
    for re in TEXT_YAML_PATTERNS.iter() {
        out = re
            .replace_all(&out, format!("${{1}}${{2}}{}", MASK))
            .into_owned();
    }
    out
}

/// Build a regex char-class fragment that matches either `_` or `-` for each
/// underscore in the key — accommodating `access_token` / `Access-Token` /
/// `access-Token` etc. without enumerating variants.
fn key_pattern_fragment(key: &str) -> String {
    key.replace('_', "[_-]")
}

/// JSON-string-form patterns: matches `"sensitive_key": "value"`.
/// Capture group 1 = `"key": "` (key casing preserved); group 2 = closing `"`.
static TEXT_JSON_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    SENSITIVE_KEYS
        .iter()
        .map(|key| {
            let key_pat = key_pattern_fragment(key);
            let pattern = format!(r#"(?i)("{}"\s*:\s*")[^"]*(")"#, key_pat);
            Regex::new(&pattern).expect("static regex compiles")
        })
        .collect()
});

/// YAML-form patterns: matches `sensitive_key: value` at line start.
/// Captures: 1 = line prefix (start-of-line or newline), 2 = `key: ` (with whitespace).
static TEXT_YAML_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    SENSITIVE_KEYS
        .iter()
        .map(|key| {
            let key_pat = key_pattern_fragment(key);
            let pattern = format!(r#"(?im)(^|\n)([ \t]*{}[ \t]*:[ \t]*)\S+"#, key_pat);
            Regex::new(&pattern).expect("static regex compiles")
        })
        .collect()
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_keys_has_eleven() {
        assert_eq!(SENSITIVE_KEYS.len(), 11);
    }

    #[test]
    fn mask_is_three_asterisks() {
        assert_eq!(MASK, "***");
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    // ----- scrub_argv tests (covering S2.1 - S2.7) -----

    #[test]
    fn scrub_argv_flag_space_value() {
        let argv = s(&["lark-cli", "--access-token", "abc", "--other", "x"]);
        let (out, paths) = scrub_argv(&argv);
        assert_eq!(out[2], MASK);
        assert_eq!(out[4], "x"); // unchanged
        assert_eq!(paths, vec!["argv[2]".to_string()]);
    }

    #[test]
    fn scrub_argv_flag_equals_value() {
        let argv = s(&["lark-cli", "--access-token=abc"]);
        let (out, paths) = scrub_argv(&argv);
        assert_eq!(out[1], format!("--access-token={}", MASK));
        assert_eq!(paths, vec!["argv[1]".to_string()]);
    }

    #[test]
    fn scrub_argv_header_long_form() {
        let argv = s(&["lark-cli", "--header", "Authorization: Bearer xyz"]);
        let (out, paths) = scrub_argv(&argv);
        assert_eq!(out[2], format!("Authorization: {}", MASK));
        assert_eq!(paths, vec!["argv[2]".to_string()]);
    }

    #[test]
    fn scrub_argv_header_short_form() {
        let argv = s(&["lark-cli", "-H", "Authorization: abc"]);
        let (out, paths) = scrub_argv(&argv);
        assert_eq!(out[2], format!("Authorization: {}", MASK));
        assert_eq!(paths, vec!["argv[2]".to_string()]);
    }

    #[test]
    fn scrub_argv_non_sensitive_flag_passes_through() {
        let argv = s(&["lark-cli", "--user", "alice"]);
        let (out, paths) = scrub_argv(&argv);
        assert_eq!(out, argv);
        assert!(paths.is_empty());
    }

    #[test]
    fn scrub_argv_sensitive_flag_last_no_value() {
        // edge: --access-token has no following value, must not panic
        let argv = s(&["lark-cli", "--access-token"]);
        let (out, paths) = scrub_argv(&argv);
        assert_eq!(out, argv);
        assert!(paths.is_empty());
    }

    #[test]
    fn scrub_argv_empty_input() {
        let argv: Vec<String> = vec![];
        let (out, paths) = scrub_argv(&argv);
        assert!(out.is_empty());
        assert!(paths.is_empty());
    }

    #[test]
    fn scrub_argv_header_non_sensitive() {
        // --header with non-sensitive header name passes through
        let argv = s(&["lark-cli", "--header", "Content-Type: application/json"]);
        let (out, paths) = scrub_argv(&argv);
        assert_eq!(out, argv);
        assert!(paths.is_empty());
    }

    #[test]
    fn scrub_argv_normalizes_dash_to_underscore() {
        // --access-token == access_token after normalization
        let argv = s(&["lark-cli", "--access-token", "abc"]);
        let (out, _) = scrub_argv(&argv);
        assert_eq!(out[2], MASK);
    }

    // ----- scrub_text tests (covering S3.1 - S3.5) -----

    #[test]
    fn scrub_text_json_string_form() {
        let input = r#"{"access_token": "abc123", "user": "alice"}"#;
        let out = scrub_text(input);
        assert!(out.contains(r#""access_token": "***""#));
        assert!(out.contains(r#""user": "alice""#));
    }

    #[test]
    fn scrub_text_yaml_form() {
        let input = "api_key: secret123\nuser: alice";
        let out = scrub_text(input);
        assert!(out.contains("api_key: ***"));
        assert!(out.contains("user: alice"));
    }

    #[test]
    fn scrub_text_case_insensitive() {
        let input = r#"{"Access-Token": "x"}"#;
        // YAML pattern matches `Access-Token` literally with case-insensitive flag
        // JSON pattern likewise. Both forms should redact.
        let out = scrub_text(input);
        // Key casing is preserved (only value replaced)
        assert!(
            out.contains(r#""Access-Token": "***""#) || out.contains(r#""Access-Token":"***""#),
            "expected case-preserving redaction, got: {}",
            out
        );
    }

    #[test]
    fn scrub_text_no_sensitive_key() {
        let input = r#"{"user": "alice", "city": "Shanghai"}"#;
        let out = scrub_text(input);
        assert_eq!(out, input);
    }

    #[test]
    fn scrub_text_empty_string() {
        let out = scrub_text("");
        assert_eq!(out, "");
    }

    #[test]
    fn scrub_text_multiple_keys_in_one_blob() {
        let input = r#"{"access_token": "a", "api_key": "b", "password": "c"}"#;
        let out = scrub_text(input);
        assert!(out.contains(r#""access_token": "***""#));
        assert!(out.contains(r#""api_key": "***""#));
        assert!(out.contains(r#""password": "***""#));
    }

    // ----- scrub_value tests (covering S1.1 - S1.8) -----

    use serde_json::{Value, json};

    #[test]
    fn scrub_value_simple_object() {
        let v = json!({"user": "alice", "access_token": "xyz"});
        let (out, paths) = scrub_value(&v);
        assert_eq!(out["access_token"], Value::String(MASK.to_string()));
        assert_eq!(out["user"], Value::String("alice".to_string()));
        assert_eq!(paths, vec!["/access_token".to_string()]);
    }

    #[test]
    fn scrub_value_nested_object() {
        let v = json!({"headers": {"Authorization": "Bearer abc"}});
        let (out, paths) = scrub_value(&v);
        assert_eq!(
            out["headers"]["Authorization"],
            Value::String(MASK.to_string())
        );
        assert_eq!(paths, vec!["/headers/Authorization".to_string()]);
    }

    #[test]
    fn scrub_value_array_elements() {
        let v = json!([{"api_key": "k1"}, {"api_key": "k2"}]);
        let (out, paths) = scrub_value(&v);
        assert_eq!(out[0]["api_key"], Value::String(MASK.to_string()));
        assert_eq!(out[1]["api_key"], Value::String(MASK.to_string()));
        assert_eq!(
            paths,
            vec!["/0/api_key".to_string(), "/1/api_key".to_string()]
        );
    }

    #[test]
    fn scrub_value_no_sensitive_keys() {
        let v = json!({"foo": "bar", "items": [1, 2, 3]});
        let (out, paths) = scrub_value(&v);
        assert_eq!(out, v);
        assert!(paths.is_empty());
    }

    #[test]
    fn scrub_value_dash_and_case_variants() {
        let v = json!({"Access-Token": "x", "API-KEY": "y"});
        let (out, paths) = scrub_value(&v);
        assert_eq!(out["Access-Token"], Value::String(MASK.to_string()));
        assert_eq!(out["API-KEY"], Value::String(MASK.to_string()));
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn scrub_value_primitive_top_level() {
        let cases = vec![
            json!("standalone string"),
            json!(42),
            json!(true),
            json!(null),
        ];
        for v in cases {
            let (out, paths) = scrub_value(&v);
            assert_eq!(out, v);
            assert!(paths.is_empty());
        }
    }

    #[test]
    fn scrub_value_idempotent() {
        let v = json!({"access_token": "xyz", "user": "alice"});
        let (once, paths1) = scrub_value(&v);
        let (twice, paths2) = scrub_value(&once);
        assert_eq!(once, twice);
        assert_eq!(paths1, paths2);
        // value is "***" both times
        assert_eq!(twice["access_token"], Value::String(MASK.to_string()));
    }

    #[test]
    fn scrub_value_all_eleven_keys_covered() {
        // exercise each entry in SENSITIVE_KEYS, including the 4 defensive extensions
        let v = json!({
            "app_secret": "1",
            "access_token": "2",
            "refresh_token": "3",
            "user_access_token": "4",
            "tenant_access_token": "5",
            "authorization": "6",
            "api_key": "7",
            "password": "8",
            "secret": "9",
            "cookie": "10",
            "private_key": "11",
        });
        let (_out, paths) = scrub_value(&v);
        assert_eq!(paths.len(), 11);
    }

    #[test]
    fn scrub_value_jsonptr_escape_slash_in_key() {
        // edge: key containing '/' must be escaped to '~1' per RFC 6901
        let mut map = serde_json::Map::new();
        map.insert("a/b".to_string(), json!({"access_token": "x"}));
        let v = Value::Object(map);
        let (_out, paths) = scrub_value(&v);
        assert_eq!(paths, vec!["/a~1b/access_token".to_string()]);
    }
}
