//! Cross-module integration: trace ↔ journal stamp, budget on-disk
//! round-trip, runaway tracker timeline.

use chrono::NaiveDate;
use roostery::budget::{self, BUDGET_SCHEMA_VERSION, BudgetState};
use roostery::config::BudgetCfg;
use roostery::journal::JournalEntry;
use roostery::runaway::RunawayTracker;
use roostery::trace::{TraceContext, TraceId};
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[test]
fn trace_stamps_journal_entry_for_replay() {
    let mut ctx = TraceContext::new_root(Some("parent_evt".to_string()), 8);
    ctx.depth = 2;

    let mut entry = JournalEntry::new("dispatcher", "runner.invoke");
    let original_event_id = entry.event_id.clone();
    ctx.stamp_journal(&mut entry);

    assert_eq!(entry.trace_id.as_deref(), Some(ctx.trace_id.as_str()));
    assert_eq!(entry.parent_event_id.as_deref(), Some("parent_evt"));
    assert_eq!(entry.depth, 2);
    // event_id stays caller-owned; trace stamping must not touch it.
    assert_eq!(entry.event_id, original_event_id);

    // JSON round-trip preserves trace fields.
    let json = serde_json::to_string(&entry).unwrap();
    let parsed: JournalEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.trace_id, entry.trace_id);
    assert_eq!(parsed.parent_event_id, entry.parent_event_id);
    assert_eq!(parsed.depth, entry.depth);
}

#[test]
fn budget_save_then_load_round_trip_on_real_fs() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("state").join("budget.json");

    let mut cfg = BudgetCfg::default();
    cfg.max_calls = 100;
    cfg.max_cost_usd = 1.0;
    let mut state = BudgetState::from_cfg(&cfg);
    state.check_or_raise(0.0).unwrap();
    state.consume(0.05);
    state.consume(0.10);

    budget::save_to(&state, &path).unwrap();
    assert!(path.exists());

    let loaded = budget::load_from(&path).unwrap();
    assert_eq!(loaded.schema_version, BUDGET_SCHEMA_VERSION);
    assert_eq!(loaded.default.calls, 2);
    assert!((loaded.default.cost_usd - 0.15).abs() < 1e-9);
    assert_eq!(loaded.default.max_calls, 100);
}

#[test]
fn budget_rollover_clears_counters_on_disk_cycle() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("budget.json");

    let mut cfg = BudgetCfg::default();
    cfg.max_calls = 5;
    cfg.max_cost_usd = 1.0;
    let mut state = BudgetState::from_cfg(&cfg);
    state.consume(0.5);
    state.consume(0.4);
    // Force a stale day before persisting.
    state.day = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    budget::save_to(&state, &path).unwrap();

    let mut loaded = budget::load_from(&path).unwrap();
    // First check_or_raise must trigger rollover and let a new dispatch in.
    loaded.check_or_raise(0.5).unwrap();
    assert_eq!(loaded.default.calls, 0);
    assert!(loaded.default.cost_usd.abs() < 1e-9);
}

#[test]
fn runaway_tracker_timeline_with_injected_clock() {
    let base = Instant::now();
    let advance = Arc::new(AtomicU64::new(0));
    let advance_clock = advance.clone();
    let mut tracker = RunawayTracker::with_clock(Duration::from_secs(60), 4, move || {
        base + Duration::from_secs(advance_clock.load(Ordering::SeqCst))
    });
    let tid = TraceId::from_existing("integ-trace");

    // 3 fires close together — under threshold.
    for _ in 0..3 {
        tracker.record(&tid);
    }
    assert_eq!(tracker.check(&tid).unwrap(), 3);

    // 4th fire still inside window → threshold reached.
    advance.store(30, Ordering::SeqCst);
    tracker.record(&tid);
    assert!(tracker.check(&tid).is_err());

    // After advancing past window+offset, all old entries fall out; new fire reads as 1.
    // Earliest survivor was at t=30; window=60s; cutoff at t=100 is t=40 > 30 → all evicted.
    advance.store(100, Ordering::SeqCst);
    assert_eq!(tracker.record(&tid), 1);
    assert_eq!(tracker.check(&tid).unwrap(), 1);
}

#[test]
fn end_to_end_three_gates_chain_for_one_dispatch() {
    // Simulate the Phase 4 dispatcher loop wiring: trace → runaway → budget.
    let mut cfg = BudgetCfg::default();
    cfg.max_calls = 5;
    cfg.max_cost_usd = 1.0;
    let mut budget_state = BudgetState::from_cfg(&cfg);
    let mut tracker = RunawayTracker::with_window_and_threshold(Duration::from_secs(60), 5);

    let parent = TraceContext::new_root(Some("evt_root".to_string()), 3);
    parent.check_depth().unwrap();
    tracker.record(&parent.trace_id);
    tracker.check(&parent.trace_id).unwrap();
    budget_state.check_or_raise(0.001).unwrap();
    budget_state.consume(0.001);

    let child = parent.child(Some("evt_l1".to_string()));
    let grandchild = child.child(Some("evt_l2".to_string()));
    assert_eq!(grandchild.depth, 2);
    grandchild.check_depth().unwrap();

    let great = grandchild.child(Some("evt_l3".to_string()));
    assert_eq!(great.depth, 3);
    // At depth=max_depth=3 → must be rejected.
    assert!(great.check_depth().is_err());
}

#[test]
fn budget_save_writes_pretty_json_with_trailing_newline() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("budget.json");
    let mut cfg = BudgetCfg::default();
    cfg.max_calls = 100;
    cfg.max_cost_usd = 1.0;
    let state = BudgetState::from_cfg(&cfg);
    budget::save_to(&state, &path).unwrap();

    let raw = fs::read(&path).unwrap();
    assert_eq!(*raw.last().unwrap(), b'\n');
    let text = std::str::from_utf8(&raw).unwrap();
    // Pretty-printed JSON contains a newline before keys.
    assert!(text.contains("\n  \"schema_version\""));
}
