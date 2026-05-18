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
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
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
    ///
    /// Defense-in-depth against non-finite state: NaN propagates through
    /// arithmetic but all comparisons against NaN return false, so the
    /// naive `total > max` check would fail-open if `self.cost_usd` ever
    /// became NaN (e.g. corrupted on-disk state, hand-edited budget.json).
    /// Explicit non-finite checks below short-circuit to "exceeded" so the
    /// caller can fail-closed and let the operator investigate.
    pub fn would_exceed(&self, calls: u32, cost_usd: f64) -> Option<String> {
        if self.calls.saturating_add(calls) > self.max_calls {
            return Some(format!(
                "calls {} > max_calls {}",
                self.calls.saturating_add(calls),
                self.max_calls
            ));
        }
        if !self.cost_usd.is_finite() || !cost_usd.is_finite() {
            return Some(format!(
                "cost_usd non-finite (self={}, incoming={}); refuse to admit",
                self.cost_usd, cost_usd
            ));
        }
        let total = self.cost_usd + cost_usd;
        if total > self.max_cost_usd {
            return Some(format!(
                "cost_usd {total:.6} > max_cost_usd {:.6}",
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

// --- BudgetGuard: cross-process atomic RMW (codex round-4 P1) -------------
//
// **问题**：原 dispatcher::fire 走 `load() → 多次 mutate → save()`，两条
// hook 进程并发时都读到旧 state、各自通过 gate、最后互覆 rename，导致
// `max_calls` / `max_cost_usd` 被绕过。
//
// **修复**：引入 BudgetGuard，开 file 立刻 acquire exclusive flock，整个
// fire() 期间持锁，commit / drop 时释放。其他 fire 进程在 lock_exclusive()
// 阻塞，串行化跨进程 RMW。锁基于 advisory `flock(2)`（Rust 1.89+ stdlib），
// drop 自动释放。
//
// **死锁**：单 fire 内只 acquire 一把锁，无嵌套；超时未集成（hook 本身有
// timeout 守 agent runtime）。

/// Lock guard 文件路径：`{budget_state_path}.lock`。专用锁文件（不锁
/// budget.json 本身）让 rename(tmp, budget.json) 不被锁文件 inode 干扰。
fn lock_path(state_path: &Path) -> PathBuf {
    state_path.with_extension("json.lock")
}

/// 持锁的 BudgetState handle。`open` acquire 锁、读 + 初始化 state；调用方
/// 在 lifetime 内 mutate state；`commit` 原子写回 + 释放锁；drop 也释放锁
/// 但不 commit（panic 安全：进程崩溃 state 不被半写）。
pub struct BudgetGuard {
    state: BudgetState,
    state_path: PathBuf,
    _lock_file: File, // 持有 = 锁存在；drop 释放
}

impl BudgetGuard {
    /// 打开 budget state + acquire exclusive flock。NotFound 视为首次运行，
    /// 用 `cfg` 构造 fresh state。其他 load 错误（parse / schema mismatch /
    /// IO）传给 caller —— 由调用方决定是 log + fresh 还是 bail。
    pub fn open(cfg: &BudgetCfg) -> Result<Self, BudgetError> {
        Self::open_at(cfg, &paths::budget_state_path())
    }

    /// 测试可注入的变体——指定 state 文件路径。
    pub fn open_at(cfg: &BudgetCfg, state_path: &Path) -> Result<Self, BudgetError> {
        // 1. 确保目录存在
        if let Some(parent) = state_path.parent() {
            fs::create_dir_all(parent).map_err(|source| BudgetError::SaveFailed {
                path: state_path.to_path_buf(),
                source,
            })?;
        }
        // 2. 打开 / 创建锁文件 + 阻塞式 acquire exclusive flock
        let lock_path = lock_path(state_path);
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| BudgetError::SaveFailed {
                path: lock_path.clone(),
                source,
            })?;
        lock_file.lock().map_err(|source| BudgetError::SaveFailed {
            path: lock_path.clone(),
            source,
        })?;
        // 3. 锁内读 state
        let state = match File::open(state_path) {
            Ok(mut f) => {
                let mut buf = Vec::new();
                f.read_to_end(&mut buf)
                    .map_err(|source| BudgetError::LoadFailed {
                        path: state_path.to_path_buf(),
                        source,
                    })?;
                let parsed: BudgetState =
                    serde_json::from_slice(&buf).map_err(|source| BudgetError::ParseFailed {
                        path: state_path.to_path_buf(),
                        source,
                    })?;
                if parsed.schema_version != BUDGET_SCHEMA_VERSION {
                    return Err(BudgetError::SchemaVersionMismatch {
                        found: parsed.schema_version,
                        expected: BUDGET_SCHEMA_VERSION,
                    });
                }
                parsed
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => BudgetState::from_cfg(cfg),
            Err(source) => {
                return Err(BudgetError::LoadFailed {
                    path: state_path.to_path_buf(),
                    source,
                });
            }
        };
        Ok(Self {
            state,
            state_path: state_path.to_path_buf(),
            _lock_file: lock_file,
        })
    }

    pub fn state(&self) -> &BudgetState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut BudgetState {
        &mut self.state
    }

    /// 原子写回 state（atomic temp + rename）+ 释放锁（_lock_file drop）。
    pub fn commit(self) -> Result<PathBuf, BudgetError> {
        save_to(&self.state, &self.state_path)
        // _lock_file 在此处 drop，flock 释放
    }
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

    // --- non-finite defense ----------------------------------------------

    #[test]
    fn would_exceed_fails_closed_on_nan_self_cost_usd() {
        let mut b = Bucket::from_cfg(&small_cfg());
        b.cost_usd = f64::NAN;
        let reason = b
            .would_exceed(1, 0.0)
            .expect("NaN self.cost_usd must short-circuit to exceeded");
        assert!(reason.contains("non-finite"), "got: {reason}");
    }

    #[test]
    fn would_exceed_fails_closed_on_nan_incoming_cost_usd() {
        let b = Bucket::from_cfg(&small_cfg());
        let reason = b
            .would_exceed(1, f64::NAN)
            .expect("NaN incoming cost_usd must short-circuit to exceeded");
        assert!(reason.contains("non-finite"), "got: {reason}");
    }

    #[test]
    fn would_exceed_fails_closed_on_infinity_self() {
        let mut b = Bucket::from_cfg(&small_cfg());
        b.cost_usd = f64::INFINITY;
        let reason = b.would_exceed(1, 0.0).expect("Inf must short-circuit");
        assert!(reason.contains("non-finite"), "got: {reason}");
    }

    // --- codex round-4 P1: BudgetGuard 跨进程 RMW serialization ------------

    #[test]
    fn budget_guard_open_creates_fresh_state_on_first_run() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("budget.json");
        let cfg = small_cfg();
        let guard = BudgetGuard::open_at(&cfg, &path).unwrap();
        assert_eq!(guard.state().default.calls, 0);
        assert_eq!(guard.state().default.max_calls, cfg.max_calls);
    }

    #[test]
    fn budget_guard_commit_persists_then_reload_sees_state() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("budget.json");
        let cfg = small_cfg();
        // 第一次打开 + consume 2 次 + commit
        let mut guard = BudgetGuard::open_at(&cfg, &path).unwrap();
        guard.state_mut().consume(0.1);
        guard.state_mut().consume(0.2);
        guard.commit().unwrap();
        // 第二次打开应见持久状态
        let guard2 = BudgetGuard::open_at(&cfg, &path).unwrap();
        assert_eq!(guard2.state().default.calls, 2);
        assert!((guard2.state().default.cost_usd - 0.3).abs() < 1e-9);
    }

    #[test]
    fn budget_guard_creates_lock_file_with_json_lock_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("budget.json");
        let cfg = small_cfg();
        let _guard = BudgetGuard::open_at(&cfg, &state_path).unwrap();
        let expected_lock = tmp.path().join("budget.json.lock");
        assert!(expected_lock.exists(), "lock file should be created");
    }

    #[test]
    fn budget_guard_second_open_in_same_thread_blocks_until_first_drops() {
        // 注意：本测试 sanity check 同进程 fd 上的 advisory flock 是否串行化。
        // POSIX flock 在 Linux 是 per-file（inode）锁；不同 fd 也会阻塞。
        // 我们用非阻塞 try_lock 验证：第二把锁应失败而不是阻塞死。
        let tmp = tempfile::tempdir().unwrap();
        let state_path = tmp.path().join("budget.json");
        let cfg = small_cfg();
        let _g1 = BudgetGuard::open_at(&cfg, &state_path).unwrap();
        // 手动打开 lock 文件 + try_lock
        let lock_path = state_path.with_extension("json.lock");
        let f2 = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        // try_lock 不阻塞；持锁中应失败
        let res = f2.try_lock();
        assert!(
            res.is_err(),
            "second try_lock should fail while first guard holds the lock: {res:?}"
        );
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
