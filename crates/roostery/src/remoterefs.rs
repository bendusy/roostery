//! Extract remote object tokens from `lark-cli` stdout (best-effort).
//!
//! 9 newtype-wrapped token kinds via single-pass match-walk over the parsed
//! JSON Value. Each newtype is `#[serde(transparent)]` so the JSON form is
//! the bare string (Python-version compatible) while Rust types remain
//! mutually incompatible — cross-wiring bugs caught at compile time.
//!
//! See `.codestable/features/2026-05-16-core-remoterefs/core-remoterefs-design.md`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

// --- 9 newtype token kinds --------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct MessageId(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct DocToken(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct FolderToken(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct RecordId(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct ChatId(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct AppToken(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct WikiToken(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct TaskId(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct ThreadId(pub String);

// AsRef<str> + Display for all 9 newtypes (caller ergonomics; no From cross-conversion).
macro_rules! impl_token_str {
    ($($t:ident),+ $(,)?) => {
        $(
            impl AsRef<str> for $t {
                fn as_ref(&self) -> &str {
                    &self.0
                }
            }
            impl fmt::Display for $t {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str(&self.0)
                }
            }
        )+
    };
}
impl_token_str!(
    MessageId,
    DocToken,
    FolderToken,
    RecordId,
    ChatId,
    AppToken,
    WikiToken,
    TaskId,
    ThreadId
);

// --- Container --------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct RemoteRefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<MessageId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_token: Option<DocToken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_token: Option<FolderToken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<RecordId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<ChatId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_token: Option<AppToken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wiki_token: Option<WikiToken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
}

// --- Public extract ---------------------------------------------------------

const MAX_DEPTH: u32 = 64;

/// Extract up to 9 remote object tokens from `lark-cli` stdout (best-effort).
/// Never panics; never returns `Result` — all failure modes collapse to
/// `RemoteRefs::default()`.
///
/// # Example
/// ```
/// use roostery::remoterefs::{extract, MessageId};
/// let argv = vec!["lark-cli".to_string(), "im".into(), "+messages-send".into()];
/// let stdout = r#"{"message_id":"om_abc","chat_id":"oc_xyz"}"#;
/// let refs = extract(&argv, stdout);
/// assert_eq!(refs.message_id, Some(MessageId("om_abc".into())));
/// ```
///
/// Type isolation is enforced at compile time:
/// ```compile_fail,E0308
/// use roostery::remoterefs::{MessageId, DocToken};
/// fn takes_msg(_: &MessageId) {}
/// let dt = DocToken("x".into());
/// takes_msg(&dt); // mismatched types
/// ```
///
/// `RemoteRefs` is `#[non_exhaustive]`; struct literals must use `..Default::default()`:
/// ```compile_fail,E0063
/// use roostery::remoterefs::RemoteRefs;
/// let _ = RemoteRefs { message_id: None }; // missing fields
/// ```
pub fn extract(argv: &[String], stdout: &str) -> RemoteRefs {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return RemoteRefs::default();
    }
    let first = trimmed.as_bytes()[0];
    if first != b'{' && first != b'[' {
        return RemoteRefs::default();
    }
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return RemoteRefs::default();
    };
    let argv_create_folder = argv.iter().any(|a| a.contains("create-folder"));
    let mut refs = RemoteRefs::default();
    walk(&value, 0, argv_create_folder, &mut refs);
    refs
}

fn as_token(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(|s| s.to_string())
}

fn walk(value: &Value, depth: u32, argv_create_folder: bool, refs: &mut RemoteRefs) {
    if depth > MAX_DEPTH {
        return;
    }
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                match k.as_str() {
                    "message_id" if refs.message_id.is_none() => {
                        refs.message_id = as_token(v).map(MessageId);
                    }
                    "document_id" | "doc_token" | "obj_token" if refs.doc_token.is_none() => {
                        refs.doc_token = as_token(v).map(DocToken);
                    }
                    "folder_token" if refs.folder_token.is_none() => {
                        refs.folder_token = as_token(v).map(FolderToken);
                    }
                    "token" if argv_create_folder && refs.folder_token.is_none() => {
                        refs.folder_token = as_token(v).map(FolderToken);
                    }
                    "record_id" if refs.record_id.is_none() => {
                        refs.record_id = as_token(v).map(RecordId);
                    }
                    "chat_id" if refs.chat_id.is_none() => {
                        refs.chat_id = as_token(v).map(ChatId);
                    }
                    "app_token" if refs.app_token.is_none() => {
                        refs.app_token = as_token(v).map(AppToken);
                    }
                    "wiki_token" if refs.wiki_token.is_none() => {
                        refs.wiki_token = as_token(v).map(WikiToken);
                    }
                    "task_id" if refs.task_id.is_none() => {
                        refs.task_id = as_token(v).map(TaskId);
                    }
                    "thread_id" if refs.thread_id.is_none() => {
                        refs.thread_id = as_token(v).map(ThreadId);
                    }
                    _ => {}
                }
                walk(v, depth + 1, argv_create_folder, refs);
            }
        }
        Value::Array(arr) => arr
            .iter()
            .for_each(|v| walk(v, depth + 1, argv_create_folder, refs)),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_is_all_none_and_serializes_to_empty_object() {
        let r = RemoteRefs::default();
        assert!(r.message_id.is_none());
        assert!(r.doc_token.is_none());
        assert!(r.folder_token.is_none());
        assert!(r.record_id.is_none());
        assert!(r.chat_id.is_none());
        assert!(r.app_token.is_none());
        assert!(r.wiki_token.is_none());
        assert!(r.task_id.is_none());
        assert!(r.thread_id.is_none());
        assert_eq!(serde_json::to_string(&r).unwrap(), "{}");
    }

    #[test]
    fn newtype_as_ref_and_display() {
        macro_rules! check {
            ($t:ident) => {{
                let v = $t("x".into());
                assert_eq!(v.as_ref(), "x");
                assert_eq!(format!("{}", v), "x");
            }};
        }
        check!(MessageId);
        check!(DocToken);
        check!(FolderToken);
        check!(RecordId);
        check!(ChatId);
        check!(AppToken);
        check!(WikiToken);
        check!(TaskId);
        check!(ThreadId);
    }

    #[test]
    fn newtype_serializes_transparently_as_bare_string() {
        // Verifies #[serde(transparent)] for all 9 newtypes.
        assert_eq!(
            serde_json::to_value(MessageId("om_x".into())).unwrap(),
            json!("om_x")
        );
        assert_eq!(
            serde_json::to_value(DocToken("dt".into())).unwrap(),
            json!("dt")
        );
        assert_eq!(
            serde_json::to_value(FolderToken("fld".into())).unwrap(),
            json!("fld")
        );
        assert_eq!(
            serde_json::to_value(RecordId("rec".into())).unwrap(),
            json!("rec")
        );
        assert_eq!(
            serde_json::to_value(ChatId("oc".into())).unwrap(),
            json!("oc")
        );
        assert_eq!(
            serde_json::to_value(AppToken("app".into())).unwrap(),
            json!("app")
        );
        assert_eq!(
            serde_json::to_value(WikiToken("wk".into())).unwrap(),
            json!("wk")
        );
        assert_eq!(
            serde_json::to_value(TaskId("tsk".into())).unwrap(),
            json!("tsk")
        );
        assert_eq!(
            serde_json::to_value(ThreadId("omt".into())).unwrap(),
            json!("omt")
        );
    }

    // --- S1.1-S1.13 happy path (per field + aliases + folder_token disambiguation)

    #[test]
    fn s1_1_message_id() {
        let r = extract(&[], r#"{"message_id":"om_abc"}"#);
        assert_eq!(r.message_id, Some(MessageId("om_abc".into())));
    }

    #[test]
    fn s1_2_doc_token_from_document_id() {
        let r = extract(&[], r#"{"document_id":"doxbAaa"}"#);
        assert_eq!(r.doc_token, Some(DocToken("doxbAaa".into())));
    }

    #[test]
    fn s1_3_doc_token_from_doc_token_key() {
        let r = extract(&[], r#"{"doc_token":"shtbBbb"}"#);
        assert_eq!(r.doc_token, Some(DocToken("shtbBbb".into())));
    }

    #[test]
    fn s1_4_doc_token_from_obj_token() {
        let r = extract(&[], r#"{"obj_token":"bascCcc"}"#);
        assert_eq!(r.doc_token, Some(DocToken("bascCcc".into())));
    }

    #[test]
    fn s1_5_folder_token_explicit_key_no_argv() {
        let r = extract(&[], r#"{"folder_token":"fldDdd"}"#);
        assert_eq!(r.folder_token, Some(FolderToken("fldDdd".into())));
    }

    #[test]
    fn s1_6_folder_token_via_argv_disambiguation() {
        let argv = vec![
            "lark-cli".to_string(),
            "drive".into(),
            "+create-folder".into(),
        ];
        let r = extract(&argv, r#"{"token":"fldEee"}"#);
        assert_eq!(r.folder_token, Some(FolderToken("fldEee".into())));
    }

    #[test]
    fn s1_7_folder_token_argv_no_match_does_not_extract_generic_token() {
        let argv = vec!["lark-cli".to_string(), "im".into(), "+messages-send".into()];
        let r = extract(&argv, r#"{"token":"unknown"}"#);
        assert!(r.folder_token.is_none());
    }

    #[test]
    fn s1_8_record_id() {
        let r = extract(&[], r#"{"record_id":"recFff"}"#);
        assert_eq!(r.record_id, Some(RecordId("recFff".into())));
    }

    #[test]
    fn s1_9_chat_id() {
        let r = extract(&[], r#"{"chat_id":"oc_Ggg"}"#);
        assert_eq!(r.chat_id, Some(ChatId("oc_Ggg".into())));
    }

    #[test]
    fn s1_10_app_token() {
        let r = extract(&[], r#"{"app_token":"bascHhh"}"#);
        assert_eq!(r.app_token, Some(AppToken("bascHhh".into())));
    }

    #[test]
    fn s1_11_wiki_token() {
        let r = extract(&[], r#"{"wiki_token":"wikIii"}"#);
        assert_eq!(r.wiki_token, Some(WikiToken("wikIii".into())));
    }

    #[test]
    fn s1_12_task_id() {
        let r = extract(&[], r#"{"task_id":"tsk_Jjj"}"#);
        assert_eq!(r.task_id, Some(TaskId("tsk_Jjj".into())));
    }

    #[test]
    fn s1_13_thread_id() {
        let r = extract(&[], r#"{"thread_id":"omt_Kkk"}"#);
        assert_eq!(r.thread_id, Some(ThreadId("omt_Kkk".into())));
    }

    // --- S2.1-S2.6 multi-field / nesting / ordering

    #[test]
    fn s2_1_multiple_fields_in_one_object() {
        let r = extract(&[], r#"{"message_id":"om_x","chat_id":"oc_y"}"#);
        assert_eq!(r.message_id, Some(MessageId("om_x".into())));
        assert_eq!(r.chat_id, Some(ChatId("oc_y".into())));
    }

    #[test]
    fn s2_2_nested_object_walks_through() {
        let r = extract(&[], r#"{"data":{"message_id":"om_z"}}"#);
        assert_eq!(r.message_id, Some(MessageId("om_z".into())));
    }

    #[test]
    fn s2_3_array_first_match_wins() {
        let r = extract(
            &[],
            r#"{"items":[{"record_id":"rec1"},{"record_id":"rec2"}]}"#,
        );
        assert_eq!(r.record_id, Some(RecordId("rec1".into())));
    }

    #[test]
    fn s2_4_top_level_array() {
        let r = extract(&[], r#"[{"message_id":"om_a"}]"#);
        assert_eq!(r.message_id, Some(MessageId("om_a".into())));
    }

    #[test]
    fn s2_5_sibling_key_dictionary_order_locks_invariant_8() {
        // serde_json default uses BTreeMap → keys iterated in dict order ("a" before "b").
        let r = extract(
            &[],
            r#"{"b":{"message_id":"om_b"},"a":{"message_id":"om_a"}}"#,
        );
        assert_eq!(
            r.message_id,
            Some(MessageId("om_a".into())),
            "sibling-key walk order is BTreeMap dict order, not stdout physical order"
        );
    }

    #[test]
    fn s2_6_doc_token_aliases_dict_order_first() {
        // Dict order: "doc_token" < "document_id" < "obj_token" → "dt" wins.
        let r = extract(
            &[],
            r#"{"document_id":"dx","doc_token":"dt","obj_token":"ot"}"#,
        );
        assert_eq!(r.doc_token, Some(DocToken("dt".into())));
    }

    // --- S3.1-S3.2 boundary / error fallback

    #[test]
    fn s3_1_error_fallback_returns_default() {
        // 8 sub-cases per design §3.1 S3.1.
        for (label, stdout) in [
            ("empty", ""),
            ("whitespace", "   \n  "),
            ("non_json", "hello world"),
            ("parse_fail", "{invalid json"),
            ("no_target_key", r#"{"foo":"bar"}"#),
            ("value_not_string", r#"{"message_id":123}"#),
            ("value_empty_string", r#"{"chat_id":""}"#),
            ("primitive_top", r#""standalone string""#),
        ] {
            let r = extract(&[], stdout);
            assert_eq!(r, RemoteRefs::default(), "case: {label}");
        }
    }

    #[test]
    fn s3_2_deep_nesting_does_not_overflow_locks_invariant_9() {
        // Build {"a":{"a":{...100 layers...}}} with message_id at depth ~30 (< 64 OK)
        // and chat_id at depth ~80 (> 64 not picked up due to MAX_DEPTH).
        fn nest(depth: u32, payload: &str) -> String {
            if depth == 0 {
                payload.to_string()
            } else {
                format!(r#"{{"a":{}}}"#, nest(depth - 1, payload))
            }
        }
        // 30 layers of {"a":...} wrapping {"message_id":"shallow"}.
        let shallow_nest = nest(30, r#"{"message_id":"shallow"}"#);
        // Then 50 more layers wrapping {"chat_id":"deep"}, glued under the shallow payload.
        // Easier: build two independent nests and combine as siblings at root.
        let deep_nest = nest(80, r#"{"chat_id":"deep"}"#);
        let combined = format!(r#"{{"shallow":{shallow_nest},"deep":{deep_nest}}}"#);

        let r = extract(&[], &combined);
        // Shallow message_id is at depth ~32 from root (< 64): should be picked up.
        assert_eq!(
            r.message_id,
            Some(MessageId("shallow".into())),
            "shallow field (<64 depth) should be extracted"
        );
        // Deep chat_id is at depth ~82 from root (> 64): should be cut off.
        assert!(
            r.chat_id.is_none(),
            "deep field (>64 depth) should be cut off by MAX_DEPTH"
        );
    }

    // --- S5.x serialize / type behavior (S4.x compile_fail are doctest-driven)

    #[test]
    fn s5_2_all_none_serializes_to_empty_object() {
        let r = RemoteRefs::default();
        assert_eq!(serde_json::to_string(&r).unwrap(), "{}");
    }

    #[test]
    fn s5_3_partial_serialize_only_non_none_fields() {
        let r = RemoteRefs {
            message_id: Some(MessageId("om_x".into())),
            chat_id: Some(ChatId("oc_y".into())),
            ..Default::default()
        };
        let v: Value = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(v, json!({"message_id":"om_x","chat_id":"oc_y"}));
    }

    #[test]
    fn s5_4_extract_roundtrip() {
        let stdout = r#"{"message_id":"om_x","task_id":"tsk_y","thread_id":"omt_z"}"#;
        let r1 = extract(&[], stdout);
        let serialized = serde_json::to_string(&r1).unwrap();
        let r2: RemoteRefs = serde_json::from_str(&serialized).unwrap();
        assert_eq!(r1, r2);
    }
}
