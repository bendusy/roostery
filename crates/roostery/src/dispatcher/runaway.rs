//! Sliding-window runaway detector — defense layer on top of `trace::check_depth`.
//!
//! `RunawayTracker` is memory-only (no persistence): a `dispatcher` process
//! instance counts dispatches per `TraceId` within a rolling window; if a
//! trace fires more than `threshold` times in `window`, the next `check`
//! returns `RunawayError::Detected`. Per-process scope is sufficient for
//! single-daemon use; cross-process tracking would require a separate
//! persistence layer (see roadmap §7).
//!
//! Each `record` lazily evicts entries that fell outside the window — no
//! background thread.
//!
//! See `.codestable/features/2026-05-18-dispatcher-trace-budget/dispatcher-trace-budget-design.md`
//! §2.1.3.

use super::trace::TraceId;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use thiserror::Error;

pub const DEFAULT_WINDOW_SECS: u64 = 300;
pub const DEFAULT_THRESHOLD: u32 = 10;

type Clock = Box<dyn Fn() -> Instant + Send + Sync>;

pub struct RunawayTracker {
    window: Duration,
    threshold: u32,
    fires: BTreeMap<TraceId, Vec<Instant>>,
    clock: Clock,
}

impl RunawayTracker {
    pub fn new() -> Self {
        Self::with_window_and_threshold(Duration::from_secs(DEFAULT_WINDOW_SECS), DEFAULT_THRESHOLD)
    }

    pub fn with_window_and_threshold(window: Duration, threshold: u32) -> Self {
        Self {
            window,
            threshold,
            fires: BTreeMap::new(),
            clock: Box::new(Instant::now),
        }
    }

    pub fn with_clock(
        window: Duration,
        threshold: u32,
        clock: impl Fn() -> Instant + Send + Sync + 'static,
    ) -> Self {
        Self {
            window,
            threshold,
            fires: BTreeMap::new(),
            clock: Box::new(clock),
        }
    }

    /// Register one dispatch; returns the number of fires within the current
    /// window after this one is added. Lazily evicts older entries.
    pub fn record(&mut self, trace_id: &TraceId) -> u32 {
        let now = (self.clock)();
        let cutoff = now.checked_sub(self.window).unwrap_or(now);
        let bucket = self.fires.entry(trace_id.clone()).or_default();
        bucket.retain(|ts| *ts >= cutoff);
        bucket.push(now);
        bucket.len() as u32
    }

    /// Return current window count or `Err(Detected)` if count reaches
    /// threshold. Does not evict — uses last recorded state.
    pub fn check(&self, trace_id: &TraceId) -> Result<u32, RunawayError> {
        let count = self
            .fires
            .get(trace_id)
            .map(|v| v.len() as u32)
            .unwrap_or(0);
        if count >= self.threshold {
            return Err(RunawayError::Detected {
                trace_id: trace_id.clone(),
                count,
                window_secs: self.window.as_secs(),
                threshold: self.threshold,
            });
        }
        Ok(count)
    }
}

impl Default for RunawayTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RunawayError {
    #[error("trace {trace_id} fired {count} dispatches in {window_secs}s (threshold {threshold})")]
    Detected {
        trace_id: TraceId,
        count: u32,
        window_secs: u64,
        threshold: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tid(s: &str) -> TraceId {
        TraceId::from_existing(s)
    }

    #[test]
    fn defaults_are_300s_and_10() {
        assert_eq!(DEFAULT_WINDOW_SECS, 300);
        assert_eq!(DEFAULT_THRESHOLD, 10);
    }

    #[test]
    fn runaway_error_display_contains_trace_id() {
        let err = RunawayError::Detected {
            trace_id: tid("abc"),
            count: 11,
            window_secs: 300,
            threshold: 10,
        };
        let msg = err.to_string();
        assert!(msg.contains("abc"));
        assert!(msg.contains("11"));
        assert!(msg.contains("300"));
    }

    #[test]
    fn single_record_returns_one() {
        let mut t = RunawayTracker::new();
        assert_eq!(t.record(&tid("a")), 1);
    }

    #[test]
    fn record_within_window_accumulates() {
        let mut t = RunawayTracker::with_window_and_threshold(Duration::from_secs(60), 5);
        for i in 1..=4 {
            assert_eq!(t.record(&tid("a")), i);
        }
        assert_eq!(t.check(&tid("a")).unwrap(), 4);
    }

    #[test]
    fn check_at_threshold_returns_err() {
        let mut t = RunawayTracker::with_window_and_threshold(Duration::from_secs(60), 3);
        for _ in 0..3 {
            t.record(&tid("a"));
        }
        match t.check(&tid("a")) {
            Err(RunawayError::Detected {
                count,
                threshold,
                window_secs,
                ..
            }) => {
                assert_eq!(count, 3);
                assert_eq!(threshold, 3);
                assert_eq!(window_secs, 60);
            }
            other => panic!("expected Detected, got {other:?}"),
        }
    }

    #[test]
    fn record_evicts_entries_outside_window() {
        let base = Instant::now();
        let advance = Arc::new(AtomicU64::new(0));
        let advance_clock = advance.clone();
        let mut t = RunawayTracker::with_clock(Duration::from_secs(10), 5, move || {
            base + Duration::from_secs(advance_clock.load(Ordering::SeqCst))
        });

        for _ in 0..3 {
            t.record(&tid("a"));
        }
        assert_eq!(t.check(&tid("a")).unwrap(), 3);

        // Advance past the window (cutoff at t=11 > t=0 → 3 old entries evicted).
        advance.store(11, Ordering::SeqCst);
        assert_eq!(t.record(&tid("a")), 1);
    }

    #[test]
    fn different_trace_ids_are_independent() {
        let mut t = RunawayTracker::with_window_and_threshold(Duration::from_secs(60), 5);
        for _ in 0..3 {
            t.record(&tid("a"));
        }
        t.record(&tid("b"));
        assert_eq!(t.check(&tid("a")).unwrap(), 3);
        assert_eq!(t.check(&tid("b")).unwrap(), 1);
    }

    #[test]
    fn check_on_unknown_trace_returns_zero() {
        let t = RunawayTracker::new();
        assert_eq!(t.check(&tid("never-recorded")).unwrap(), 0);
    }
}
