//! Loop protection trace context (roadmap §4.5).
//!
//! Phase 4 Module E first sub-feature. `TraceContext` is the loop-guard
//! ledger threaded through dispatcher loop → bot bridge → lark_cli wrapper:
//! every dispatch links its parent via `parent_event_id`, increments
//! `depth`, and refuses to proceed when `depth >= max_depth`. Belongs to
//! `runtime-neutral` req — protection independent of which agent runtime
//! is executing.
//!
//! See `.codestable/features/2026-05-18-dispatcher-trace-budget/dispatcher-trace-budget-design.md`
//! §2.1.1.

use crate::journal::JournalEntry;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Env key carrying [`TraceContext::trace_id`] across process boundaries.
pub const ENV_TRACE_ID: &str = "ROOSTERY_TRACE_ID";
/// Env key carrying [`TraceContext::depth`] (as u32 decimal string).
pub const ENV_DEPTH: &str = "ROOSTERY_DEPTH";
/// Env key carrying [`TraceContext::parent_event_id`] when present.
pub const ENV_PARENT_EVENT_ID: &str = "ROOSTERY_PARENT_EVENT_ID";

/// Hex char length of a `TraceId` (16 random bytes → 32 hex chars).
pub const TRACE_ID_HEX_LEN: usize = 32;

/// Loop-guard trace identifier: 16 random bytes encoded as lowercase hex.
///
/// Transparent string serialization keeps JSON / YAML byte-for-byte
/// compatible with Python era `trace_id` field; in Rust the newtype prevents
/// accidental cross-wiring with other id-like strings (`event_id`,
/// `parent_event_id`, runner kind names, etc) per the
/// `business-identifier-newtype` decision.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct TraceId(String);

impl TraceId {
    /// Generate a fresh `TraceId` from 16 random bytes (32 lowercase hex chars).
    pub fn new_random() -> Self {
        let mut buf = [0u8; 16];
        getrandom::getrandom(&mut buf).expect("getrandom failed");
        let mut hex = String::with_capacity(TRACE_ID_HEX_LEN);
        for byte in buf {
            hex.push_str(&format!("{byte:02x}"));
        }
        Self(hex)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_existing(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl std::fmt::Display for TraceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Immutable trace context snapshot. Every dispatch layer creates a new
/// instance via [`TraceContext::new_root`] or [`TraceContext::child`] —
/// `depth` is monotonically increasing and there is no decrement API.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TraceContext {
    pub trace_id: TraceId,
    pub parent_event_id: Option<String>,
    pub depth: u32,
    pub max_depth: u32,
}

impl TraceContext {
    /// Link root: fresh trace_id, depth=0. `max_depth` is the caller-injected
    /// `Config.trace.max_depth` value.
    pub fn new_root(parent_event_id: Option<String>, max_depth: u32) -> Self {
        Self {
            trace_id: TraceId::new_random(),
            parent_event_id,
            depth: 0,
            max_depth,
        }
    }

    /// Derive a child context: same trace_id, depth+1, new parent_event_id.
    /// `max_depth` is preserved.
    ///
    /// `saturating_add` is a defense layer: a caller who forgets to call
    /// `check_depth` between dispatches could otherwise overflow `u32`
    /// after ~4B iterations, wrap depth to 0, and silently bypass the
    /// depth gate forever. Saturating at `u32::MAX` keeps the gate
    /// fail-closed (`u32::MAX > any sane max_depth`).
    pub fn child(&self, parent_event_id: Option<String>) -> Self {
        Self {
            trace_id: self.trace_id.clone(),
            parent_event_id,
            depth: self.depth.saturating_add(1),
            max_depth: self.max_depth,
        }
    }

    /// Returns `Err(DepthExceeded)` when `depth >= max_depth`. Call this
    /// before dispatching to the next runner layer.
    pub fn check_depth(&self) -> Result<(), TraceError> {
        if self.depth >= self.max_depth {
            return Err(TraceError::DepthExceeded {
                trace_id: self.trace_id.clone(),
                depth: self.depth,
                max_depth: self.max_depth,
            });
        }
        Ok(())
    }

    /// Emit `(KEY, VALUE)` pairs for env injection downstream. Drops
    /// `ROOSTERY_PARENT_EVENT_ID` when `parent_event_id` is `None`.
    pub fn to_env_pairs(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::with_capacity(3);
        out.push((ENV_TRACE_ID, self.trace_id.as_str().to_string()));
        out.push((ENV_DEPTH, self.depth.to_string()));
        if let Some(pid) = &self.parent_event_id {
            out.push((ENV_PARENT_EVENT_ID, pid.clone()));
        }
        out
    }

    /// Reconstruct a `TraceContext` from process env. Returns `Ok(None)` when
    /// `ROOSTERY_TRACE_ID` is absent (caller treats as link root). Returns
    /// `Err(EnvParseFailed)` only when `ROOSTERY_DEPTH` is present but
    /// non-numeric. Missing `ROOSTERY_DEPTH` defaults to 0.
    pub fn from_env(
        env_lookup: impl Fn(&str) -> Option<String>,
        max_depth: u32,
    ) -> Result<Option<Self>, TraceError> {
        let Some(tid) = env_lookup(ENV_TRACE_ID) else {
            return Ok(None);
        };
        let depth = match env_lookup(ENV_DEPTH) {
            None => 0,
            Some(raw) => raw
                .parse::<u32>()
                .map_err(|_| TraceError::EnvParseFailed { raw })?,
        };
        let parent_event_id = env_lookup(ENV_PARENT_EVENT_ID);
        Ok(Some(Self {
            trace_id: TraceId::from_existing(tid),
            parent_event_id,
            depth,
            max_depth,
        }))
    }

    /// Stamp trace fields onto a `JournalEntry` in place. Preserves all
    /// other entry fields untouched.
    pub fn stamp_journal(&self, entry: &mut JournalEntry) {
        entry.trace_id = Some(self.trace_id.as_str().to_string());
        entry.parent_event_id = self.parent_event_id.clone();
        entry.depth = self.depth;
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TraceError {
    #[error("trace {trace_id} depth {depth} >= max_depth {max_depth}")]
    DepthExceeded {
        trace_id: TraceId,
        depth: u32,
        max_depth: u32,
    },
    #[error("env value ROOSTERY_DEPTH not parseable as u32: {raw:?}")]
    EnvParseFailed { raw: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_id_random_is_32_hex_chars() {
        let id = TraceId::new_random();
        assert_eq!(id.as_str().len(), TRACE_ID_HEX_LEN);
        assert!(id.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn trace_id_serde_transparent_emits_bare_string() {
        let id = TraceId::from_existing("abc123");
        let s = serde_json::to_string(&id).unwrap();
        assert_eq!(s, "\"abc123\"");
        let back: TraceId = serde_json::from_str("\"abc123\"").unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn trace_error_display_includes_fields() {
        let err = TraceError::DepthExceeded {
            trace_id: TraceId::from_existing("tid"),
            depth: 8,
            max_depth: 8,
        };
        let msg = err.to_string();
        assert!(msg.contains("tid"));
        assert!(msg.contains("8"));
    }

    #[test]
    fn env_key_constants_use_roostery_prefix() {
        assert_eq!(ENV_TRACE_ID, "ROOSTERY_TRACE_ID");
        assert_eq!(ENV_DEPTH, "ROOSTERY_DEPTH");
        assert_eq!(ENV_PARENT_EVENT_ID, "ROOSTERY_PARENT_EVENT_ID");
    }

    // --- S2 calculation tests -------------------------------------------------

    #[test]
    fn child_at_u32_max_saturates_not_wraps() {
        // Defense-in-depth: a caller that forgets check_depth could chain
        // child() calls indefinitely; without saturating_add the u32 depth
        // would wrap to 0 after ~4B iterations and silently bypass the gate.
        let mut ctx = TraceContext::new_root(None, 8);
        ctx.depth = u32::MAX;
        let child = ctx.child(None);
        assert_eq!(child.depth, u32::MAX, "must saturate, not wrap");
        // Gate still rejects.
        assert!(child.check_depth().is_err());
    }

    #[test]
    fn new_root_starts_at_depth_zero() {
        let ctx = TraceContext::new_root(None, 8);
        assert_eq!(ctx.depth, 0);
        assert_eq!(ctx.max_depth, 8);
        assert!(ctx.parent_event_id.is_none());
        assert_eq!(ctx.trace_id.as_str().len(), TRACE_ID_HEX_LEN);
    }

    #[test]
    fn child_preserves_trace_id_and_increments_depth() {
        let root = TraceContext::new_root(Some("evt0".to_string()), 8);
        let child = root.child(Some("evt1".to_string()));
        assert_eq!(child.trace_id, root.trace_id);
        assert_eq!(child.depth, root.depth + 1);
        assert_eq!(child.parent_event_id.as_deref(), Some("evt1"));
        assert_eq!(child.max_depth, root.max_depth);
    }

    #[test]
    fn check_depth_at_max_rejects() {
        let mut ctx = TraceContext::new_root(None, 3);
        ctx.depth = 3;
        match ctx.check_depth() {
            Err(TraceError::DepthExceeded {
                depth, max_depth, ..
            }) => {
                assert_eq!(depth, 3);
                assert_eq!(max_depth, 3);
            }
            other => panic!("expected DepthExceeded, got {other:?}"),
        }
    }

    #[test]
    fn check_depth_below_max_passes() {
        let mut ctx = TraceContext::new_root(None, 8);
        ctx.depth = 7;
        ctx.check_depth().unwrap();
    }

    #[test]
    fn env_round_trip_preserves_fields() {
        let mut ctx = TraceContext::new_root(Some("evt_parent".to_string()), 8);
        ctx.depth = 3;
        let pairs = ctx.to_env_pairs();
        let lookup = |key: &str| -> Option<String> {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
        };
        let recovered = TraceContext::from_env(lookup, ctx.max_depth)
            .unwrap()
            .unwrap();
        assert_eq!(recovered.trace_id, ctx.trace_id);
        assert_eq!(recovered.depth, ctx.depth);
        assert_eq!(recovered.parent_event_id, ctx.parent_event_id);
        assert_eq!(recovered.max_depth, ctx.max_depth);
    }

    #[test]
    fn from_env_without_trace_id_returns_none() {
        let lookup = |_key: &str| -> Option<String> { None };
        let result = TraceContext::from_env(lookup, 8).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn from_env_with_invalid_depth_returns_err() {
        let lookup = |key: &str| -> Option<String> {
            match key {
                ENV_TRACE_ID => Some("abc".to_string()),
                ENV_DEPTH => Some("not-a-number".to_string()),
                _ => None,
            }
        };
        match TraceContext::from_env(lookup, 8) {
            Err(TraceError::EnvParseFailed { raw }) => {
                assert_eq!(raw, "not-a-number");
            }
            other => panic!("expected EnvParseFailed, got {other:?}"),
        }
    }

    #[test]
    fn from_env_missing_depth_defaults_to_zero() {
        let lookup = |key: &str| -> Option<String> {
            if key == ENV_TRACE_ID {
                Some("abc".to_string())
            } else {
                None
            }
        };
        let recovered = TraceContext::from_env(lookup, 8).unwrap().unwrap();
        assert_eq!(recovered.depth, 0);
        assert!(recovered.parent_event_id.is_none());
    }

    #[test]
    fn to_env_pairs_omits_parent_when_none() {
        let ctx = TraceContext::new_root(None, 8);
        let pairs = ctx.to_env_pairs();
        assert_eq!(pairs.len(), 2);
        assert!(!pairs.iter().any(|(k, _)| *k == ENV_PARENT_EVENT_ID));
    }

    #[test]
    fn stamp_journal_aligns_trace_fields_only() {
        let mut entry = JournalEntry::new("test", "act");
        let original_event_id = entry.event_id.clone();
        let original_action = entry.action.clone();
        let original_ts = entry.ts;

        let mut ctx = TraceContext::new_root(Some("parent".to_string()), 8);
        ctx.depth = 2;
        ctx.stamp_journal(&mut entry);

        assert_eq!(entry.trace_id.as_deref(), Some(ctx.trace_id.as_str()));
        assert_eq!(entry.parent_event_id.as_deref(), Some("parent"));
        assert_eq!(entry.depth, 2);
        // Other fields untouched.
        assert_eq!(entry.event_id, original_event_id);
        assert_eq!(entry.action, original_action);
        assert_eq!(entry.ts, original_ts);
    }
}
