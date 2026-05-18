//! `HookEvent` schema (roadmap §4.4) — dispatcher 入口数据形状。
//!
//! 跨模块契约：模块 D 的 stop hook 脚本（embedded template）把 runtime-
//! specific 输出拼成这个 schema 喂给 `roostery dispatcher fire`；模块 E
//! 的 dispatcher loop 消费；后续 Module F bot bridge 也会跨层传递。
//!
//! 外部 hook（CC / Codex / Gemini 触发的）`trace` 必为 `None`，由 dispatcher
//! 在 fire 时分配新 trace_id；内部 dispatcher → dispatcher 跨层传递时填。
//!
//! See `.codestable/features/2026-05-18-dispatcher-rules/dispatcher-rules-design.md`
//! §2.1.1.

use super::trace::TraceContext;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const HOOK_EVENT_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[non_exhaustive]
pub struct HookEvent {
    pub schema_version: u32,
    pub hook_source: String,
    pub session_id: String,
    pub workspace: PathBuf,
    pub trigger_meta: serde_json::Value,
    #[serde(default)]
    pub trace: Option<TraceContext>,
}

impl HookEvent {
    /// Dotted-path lookup into [`HookEvent::trigger_meta`]. Returns `None` if
    /// any segment along the path is missing or hits a non-object node.
    /// Used by `rules::matches` for `trigger_meta_eq`.
    pub fn trigger_meta_path(&self, path: &str) -> Option<&serde_json::Value> {
        let mut cur = &self.trigger_meta;
        for segment in path.split('.') {
            match cur {
                serde_json::Value::Object(map) => match map.get(segment) {
                    Some(v) => cur = v,
                    None => return None,
                },
                _ => return None,
            }
        }
        Some(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_event() -> HookEvent {
        HookEvent {
            schema_version: HOOK_EVENT_SCHEMA_VERSION,
            hook_source: "claude-code-stop".to_string(),
            session_id: "sess_abc".to_string(),
            workspace: PathBuf::from("/Users/ben/Projects/roostery"),
            trigger_meta: json!({
                "action": "stop",
                "user": {"role": "owner", "name": "Ben"},
                "tags": ["alpha", "beta"],
            }),
            trace: None,
        }
    }

    #[test]
    fn schema_version_const_is_one() {
        assert_eq!(HOOK_EVENT_SCHEMA_VERSION, 1);
    }

    #[test]
    fn serde_round_trip_preserves_fields() {
        let ev = sample_event();
        let json = serde_json::to_string(&ev).unwrap();
        let back: HookEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.schema_version, ev.schema_version);
        assert_eq!(back.hook_source, ev.hook_source);
        assert_eq!(back.session_id, ev.session_id);
        assert_eq!(back.workspace, ev.workspace);
        assert_eq!(back.trigger_meta, ev.trigger_meta);
        assert!(back.trace.is_none());
    }

    #[test]
    fn trace_field_defaults_to_none_on_missing() {
        let raw = r#"{
            "schema_version": 1,
            "hook_source": "codex-stop",
            "session_id": "s",
            "workspace": "/tmp",
            "trigger_meta": {}
        }"#;
        let ev: HookEvent = serde_json::from_str(raw).unwrap();
        assert!(ev.trace.is_none());
    }

    #[test]
    fn trigger_meta_path_single_segment_hit() {
        let ev = sample_event();
        let v = ev.trigger_meta_path("action").unwrap();
        assert_eq!(v, &json!("stop"));
    }

    #[test]
    fn trigger_meta_path_nested_hit() {
        let ev = sample_event();
        let v = ev.trigger_meta_path("user.role").unwrap();
        assert_eq!(v, &json!("owner"));
    }

    #[test]
    fn trigger_meta_path_missing_segment_returns_none() {
        let ev = sample_event();
        assert!(ev.trigger_meta_path("nonexistent").is_none());
        assert!(ev.trigger_meta_path("user.email").is_none());
    }

    #[test]
    fn trigger_meta_path_through_non_object_returns_none() {
        let ev = sample_event();
        // `tags` is an array; descending into it via dotted path must fail.
        assert!(ev.trigger_meta_path("tags.alpha").is_none());
        // `action` is a string; same.
        assert!(ev.trigger_meta_path("action.x").is_none());
    }
}
