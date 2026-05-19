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
/// Every N `record()` calls, do a full sweep removing buckets whose all
/// entries are outside the window. Bounds memory at O(active_in_window +
/// PRUNE_EVERY) instead of unbounded growth as unique trace_ids accumulate.
/// Fixes issue 2026-05-19-runaway-tracker-empty-bucket-leak.
const PRUNE_EVERY: u32 = 256;

type Clock = Box<dyn Fn() -> Instant + Send + Sync>;

pub struct RunawayTracker {
    window: Duration,
    threshold: u32,
    fires: BTreeMap<TraceId, Vec<Instant>>,
    clock: Clock,
    record_count_since_prune: u32,
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
            record_count_since_prune: 0,
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
            record_count_since_prune: 0,
        }
    }

    /// Register one dispatch; returns the number of fires within the current
    /// window after this one is added. Lazily evicts older entries.
    pub fn record(&mut self, trace_id: &TraceId) -> u32 {
        let now = (self.clock)();
        // codex round-8 P2 fix: `now.checked_sub(window)` 在 daemon 启动后
        // 早期（now < window，常见于刚启动后几分钟）会下溢——之前 fallback
        // 用 `now` 当 cutoff 会**误清掉**所有更早的合法 entry。改为
        // Option<Instant>：None = "窗口未满，什么都不过期"，跳过 retain。
        let cutoff = now.checked_sub(self.window);
        // 周期性扫除"完全过期 bucket"——见 PRUNE_EVERY 文档。
        self.record_count_since_prune = self.record_count_since_prune.saturating_add(1);
        if self.record_count_since_prune >= PRUNE_EVERY {
            self.prune_expired(cutoff);
            self.record_count_since_prune = 0;
        }
        let bucket = self.fires.entry(trace_id.clone()).or_default();
        if let Some(c) = cutoff {
            bucket.retain(|ts| *ts >= c);
        }
        bucket.push(now);
        bucket.len() as u32
    }

    /// 手动触发一次过期 bucket 清扫。Daemon 主循环可在 idle 时调用，或单纯
    /// 依赖 `record()` 内的周期清扫。返清扫掉的 bucket 数（含部分清掉的）。
    pub fn prune(&mut self) -> usize {
        let now = (self.clock)();
        let cutoff = now.checked_sub(self.window);
        self.prune_expired(cutoff)
    }

    /// 内部清扫：从每个 bucket 内剔除过期 Instant，整个 bucket 都过期则
    /// 整条移除。返清扫的 bucket 数。`cutoff = None` 表示窗口未满，直接
    /// noop（不清任何 entry）。
    fn prune_expired(&mut self, cutoff: Option<Instant>) -> usize {
        let Some(cutoff) = cutoff else {
            return 0;
        };
        let before = self.fires.len();
        self.fires.retain(|_, v| {
            v.retain(|ts| *ts >= cutoff);
            !v.is_empty()
        });
        before - self.fires.len()
    }

    /// 当前驻留的 bucket 数（测试 / 监控用）。
    pub fn bucket_count(&self) -> usize {
        self.fires.len()
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

    // --- issue 2026-05-19-runaway-tracker-empty-bucket-leak 回归 ---

    #[test]
    fn manual_prune_removes_fully_expired_buckets() {
        let base = Instant::now();
        let advance = Arc::new(AtomicU64::new(0));
        let advance_clock = advance.clone();
        let mut t = RunawayTracker::with_clock(Duration::from_secs(10), 5, move || {
            base + Duration::from_secs(advance_clock.load(Ordering::SeqCst))
        });

        // 10 个 trace_id 各 record 一次，bucket_count = 10
        for i in 0..10 {
            t.record(&tid(&format!("trace-{i}")));
        }
        assert_eq!(t.bucket_count(), 10);

        // 时钟跳过窗口 → 所有 entries 过期 → prune 清扫整张表
        advance.store(11, Ordering::SeqCst);
        let removed = t.prune();
        assert_eq!(removed, 10);
        assert_eq!(t.bucket_count(), 0);
    }

    #[test]
    fn periodic_auto_prune_keeps_memory_bounded() {
        // 模拟 daemon 长跑：很多独立 trace_id 各 fire 一次后再不出现；prune
        // 阈值（PRUNE_EVERY=256）触发后过期 bucket 被清扫。
        let base = Instant::now();
        let advance = Arc::new(AtomicU64::new(0));
        let advance_clock = advance.clone();
        let mut t = RunawayTracker::with_clock(Duration::from_secs(10), 5, move || {
            base + Duration::from_secs(advance_clock.load(Ordering::SeqCst))
        });

        // 第一波 200 条独立 trace_id（在窗口内）→ 全部驻留
        for i in 0..200 {
            t.record(&tid(&format!("first-{i}")));
        }
        assert_eq!(t.bucket_count(), 200);

        // 时钟跳过窗口
        advance.store(11, Ordering::SeqCst);

        // 第二波 60 条新 trace_id（在 PRUNE_EVERY 边界附近）；超过 256-200=56
        // 时触发 record 内的 prune_expired，清掉 first-* 那批
        for i in 0..60 {
            t.record(&tid(&format!("second-{i}")));
        }

        // first-* 应被清扫（已过期），second-* 仍驻留
        assert!(
            t.bucket_count() <= 60,
            "expected ≤60 buckets after auto-prune, got {}",
            t.bucket_count()
        );
    }
}
