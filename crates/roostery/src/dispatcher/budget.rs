//! Budget gate (roadmap §4.6 default bucket).
//!
//! Phase 4 Module E gate persisted at `paths::budget_state_path()`. Linear
//! flow: `load` → `roll_over_if_needed` (per-call, so tail-running daemons
//! correctly cross midnight) → `check_or_raise` → `consume` → `save`. Only
//! the `default` bucket is supported in this feature; per-runner /
//! per-rule granularity would extend the on-disk schema and is deferred to
//! a future `cs-roadmap update`.
//!
//! `schema_version = 1` is a public contract: bumps require
//! `cs-roadmap update` + backward-compatible deserialize for the old form.
//!
//! See `.codestable/features/2026-05-18-dispatcher-trace-budget/dispatcher-trace-budget-design.md`
//! §2.1.2.

use crate::config::BudgetCfg;
use crate::paths;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const BUDGET_SCHEMA_VERSION: u32 = 1;
const DEFAULT_BUCKET_KIND: &str = "default";

/// Running counters + per-bucket caps. `max_*` mirror the values from
/// `Config.budgets.default` at the time `from_cfg` is called.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Bucket {
    pub calls: u32,
    pub cost_usd: f64,
    pub max_calls: u32,
    pub max_cost_usd: f64,
}

impl Bucket {
    pub fn from_cfg(cfg: &BudgetCfg) -> Self {
        Self {
            calls: 0,
            cost_usd: 0.0,
            max_calls: cfg.max_calls,
            max_cost_usd: cfg.max_cost_usd,
        }
    }

    /// Returns `Some(reason)` when consuming `calls` / `cost_usd` more
    /// would exceed a cap; `None` when it fits.
    pub fn would_exceed(&self, calls: u32, cost_usd: f64) -> Option<String> {
        if self.calls.saturating_add(calls) > self.max_calls {
            return Some(format!(
                "calls {} > max_calls {}",
                self.calls.saturating_add(calls),
                self.max_calls
            ));
        }
        if self.cost_usd + cost_usd > self.max_cost_usd {
            return Some(format!(
                "cost_usd {:.6} > max_cost_usd {:.6}",
                self.cost_usd + cost_usd,
                self.max_cost_usd
            ));
        }
        None
    }

    pub fn consume(&mut self, calls: u32, cost_usd: f64) {
        self.calls = self.calls.saturating_add(calls);
        self.cost_usd += cost_usd;
    }
}

/// On-disk state. Field set defined by `BUDGET_SCHEMA_VERSION`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BudgetState {
    pub schema_version: u32,
    pub day: NaiveDate,
    pub default: Bucket,
}

impl BudgetState {
    pub fn from_cfg(cfg: &BudgetCfg) -> Self {
        Self {
            schema_version: BUDGET_SCHEMA_VERSION,
            day: today_utc(),
            default: Bucket::from_cfg(cfg),
        }
    }

    /// Reset counters when `state.day` no longer matches today. Returns
    /// `true` when reset happened. Caller normally invokes this before
    /// every `check_or_raise` so tail-running daemons cross midnight
    /// correctly.
    pub fn roll_over_if_needed(&mut self) -> bool {
        let today = today_utc();
        if self.day == today {
            return false;
        }
        self.day = today;
        self.default.calls = 0;
        self.default.cost_usd = 0.0;
        true
    }

    /// Pre-flight check: would consuming one more call (+ `cost_usd`)
    /// exceed any cap? Calls `roll_over_if_needed` first.
    pub fn check_or_raise(&mut self, cost_usd: f64) -> Result<(), BudgetError> {
        self.roll_over_if_needed();
        if let Some(reason) = self.default.would_exceed(1, cost_usd) {
            return Err(BudgetError::Exceeded {
                kind: DEFAULT_BUCKET_KIND.to_string(),
                reason,
            });
        }
        Ok(())
    }

    /// Record one successful runner invocation. Calls `roll_over_if_needed`
    /// first to handle midnight crossings between check and consume.
    pub fn consume(&mut self, cost_usd: f64) {
        self.roll_over_if_needed();
        self.default.consume(1, cost_usd);
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BudgetError {
    #[error("failed to read budget state {path}: {source}")]
    LoadFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse budget state {path}: {source}")]
    ParseFailed {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write budget state {path}: {source}")]
    SaveFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("budget bucket {kind}: {reason}")]
    Exceeded { kind: String, reason: String },
    #[error("budget schema version {found} not supported (expected {expected})")]
    SchemaVersionMismatch { found: u32, expected: u32 },
}

pub fn load() -> Result<BudgetState, BudgetError> {
    load_from(&paths::budget_state_path())
}

pub fn load_from(path: &Path) -> Result<BudgetState, BudgetError> {
    let bytes = fs::read(path).map_err(|source| BudgetError::LoadFailed {
        path: path.to_path_buf(),
        source,
    })?;
    let state: BudgetState =
        serde_json::from_slice(&bytes).map_err(|source| BudgetError::ParseFailed {
            path: path.to_path_buf(),
            source,
        })?;
    if state.schema_version != BUDGET_SCHEMA_VERSION {
        return Err(BudgetError::SchemaVersionMismatch {
            found: state.schema_version,
            expected: BUDGET_SCHEMA_VERSION,
        });
    }
    Ok(state)
}

pub fn save(state: &BudgetState) -> Result<PathBuf, BudgetError> {
    save_to(state, &paths::budget_state_path())
}

pub fn save_to(state: &BudgetState, path: &Path) -> Result<PathBuf, BudgetError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| BudgetError::SaveFailed {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let mut bytes =
        serde_json::to_vec_pretty(state).map_err(|source| BudgetError::ParseFailed {
            path: path.to_path_buf(),
            source,
        })?;
    bytes.push(b'\n');
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &bytes).map_err(|source| BudgetError::SaveFailed {
        path: path.to_path_buf(),
        source,
    })?;
    fs::rename(&tmp, path).map_err(|source| BudgetError::SaveFailed {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(path.to_path_buf())
}

fn today_utc() -> NaiveDate {
    chrono::Utc::now().date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BudgetCfg;

    fn small_cfg() -> BudgetCfg {
        BudgetCfg {
            max_calls: 3,
            max_cost_usd: 1.0,
        }
    }

    // --- S3 schema tests ------------------------------------------------------

    #[test]
    fn schema_version_const_is_one() {
        assert_eq!(BUDGET_SCHEMA_VERSION, 1);
    }

    #[test]
    fn bucket_would_exceed_calls_only() {
        let mut b = Bucket {
            calls: 2,
            cost_usd: 0.0,
            max_calls: 3,
            max_cost_usd: 1.0,
        };
        assert!(b.would_exceed(1, 0.0).is_none()); // 3 ≤ 3 ok
        assert!(b.would_exceed(2, 0.0).is_some()); // 4 > 3
        b.consume(1, 0.0);
        assert!(b.would_exceed(1, 0.0).is_some()); // calls 3+1>3
    }

    #[test]
    fn bucket_would_exceed_cost_only() {
        let b = Bucket {
            calls: 0,
            cost_usd: 0.7,
            max_calls: 100,
            max_cost_usd: 1.0,
        };
        assert!(b.would_exceed(1, 0.2).is_none());
        assert!(b.would_exceed(1, 0.5).is_some());
    }

    #[test]
    fn bucket_would_exceed_both_fit() {
        let b = Bucket {
            calls: 0,
            cost_usd: 0.0,
            max_calls: 100,
            max_cost_usd: 1.0,
        };
        assert!(b.would_exceed(1, 0.001).is_none());
    }

    // --- S4 calculation + persistence tests -----------------------------------

    #[test]
    fn from_cfg_zero_init_with_cfg_caps() {
        let cfg = small_cfg();
        let state = BudgetState::from_cfg(&cfg);
        assert_eq!(state.schema_version, BUDGET_SCHEMA_VERSION);
        assert_eq!(state.default.calls, 0);
        assert_eq!(state.default.cost_usd, 0.0);
        assert_eq!(state.default.max_calls, 3);
        assert_eq!(state.default.max_cost_usd, 1.0);
    }

    #[test]
    fn rollover_same_day_noop() {
        let cfg = small_cfg();
        let mut state = BudgetState::from_cfg(&cfg);
        state.default.calls = 2;
        state.default.cost_usd = 0.5;
        assert!(!state.roll_over_if_needed());
        assert_eq!(state.default.calls, 2);
        assert_eq!(state.default.cost_usd, 0.5);
    }

    #[test]
    fn rollover_different_day_resets() {
        let cfg = small_cfg();
        let mut state = BudgetState::from_cfg(&cfg);
        state.default.calls = 5;
        state.default.cost_usd = 0.9;
        state.day = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        assert!(state.roll_over_if_needed());
        assert_eq!(state.default.calls, 0);
        assert_eq!(state.default.cost_usd, 0.0);
        assert_eq!(state.day, today_utc());
        // Caps preserved.
        assert_eq!(state.default.max_calls, 3);
    }

    #[test]
    fn check_or_raise_passes_when_under_cap() {
        let cfg = small_cfg();
        let mut state = BudgetState::from_cfg(&cfg);
        state.check_or_raise(0.001).unwrap();
    }

    #[test]
    fn check_or_raise_fails_when_calls_exceeded() {
        let cfg = small_cfg();
        let mut state = BudgetState::from_cfg(&cfg);
        state.default.calls = 3; // already at cap
        match state.check_or_raise(0.0) {
            Err(BudgetError::Exceeded { kind, reason }) => {
                assert_eq!(kind, "default");
                assert!(reason.contains("calls"));
            }
            other => panic!("expected Exceeded, got {other:?}"),
        }
    }

    #[test]
    fn check_or_raise_fails_when_cost_exceeded() {
        let cfg = small_cfg();
        let mut state = BudgetState::from_cfg(&cfg);
        state.default.cost_usd = 0.9;
        match state.check_or_raise(0.2) {
            Err(BudgetError::Exceeded { reason, .. }) => {
                assert!(reason.contains("cost_usd"));
            }
            other => panic!("expected Exceeded, got {other:?}"),
        }
    }

    #[test]
    fn check_or_raise_triggers_rollover() {
        let cfg = small_cfg();
        let mut state = BudgetState::from_cfg(&cfg);
        state.default.calls = 3;
        state.day = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        // Despite calls=3, rollover during check should reset and allow.
        state.check_or_raise(0.001).unwrap();
        assert_eq!(state.default.calls, 0);
    }

    #[test]
    fn consume_increments_calls_and_cost() {
        let cfg = small_cfg();
        let mut state = BudgetState::from_cfg(&cfg);
        state.consume(0.25);
        assert_eq!(state.default.calls, 1);
        assert!((state.default.cost_usd - 0.25).abs() < 1e-9);
    }

    #[test]
    fn save_then_load_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("budget.json");
        let cfg = small_cfg();
        let mut state = BudgetState::from_cfg(&cfg);
        state.default.calls = 2;
        state.default.cost_usd = 0.42;

        save_to(&state, &path).unwrap();
        assert!(path.exists());
        // .tmp must not linger.
        assert!(!path.with_extension("json.tmp").exists());

        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, state);
    }

    #[test]
    fn save_to_creates_parent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("deep/sub/budget.json");
        let cfg = small_cfg();
        let state = BudgetState::from_cfg(&cfg);
        save_to(&state, &nested).unwrap();
        assert!(nested.exists());
    }

    #[test]
    fn load_missing_file_returns_load_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nope.json");
        match load_from(&path) {
            Err(BudgetError::LoadFailed { .. }) => {}
            other => panic!("expected LoadFailed, got {other:?}"),
        }
    }

    #[test]
    fn load_invalid_json_returns_parse_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bad.json");
        fs::write(&path, b"not json").unwrap();
        match load_from(&path) {
            Err(BudgetError::ParseFailed { .. }) => {}
            other => panic!("expected ParseFailed, got {other:?}"),
        }
    }

    #[test]
    fn load_wrong_schema_version_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("v2.json");
        let body = r#"{"schema_version":2,"day":"2026-05-18","default":{"calls":0,"cost_usd":0.0,"max_calls":100,"max_cost_usd":1.0}}"#;
        fs::write(&path, body).unwrap();
        match load_from(&path) {
            Err(BudgetError::SchemaVersionMismatch { found, expected }) => {
                assert_eq!(found, 2);
                assert_eq!(expected, 1);
            }
            other => panic!("expected SchemaVersionMismatch, got {other:?}"),
        }
    }
}
