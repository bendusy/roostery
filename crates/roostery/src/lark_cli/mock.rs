//! `MockLarkRunner` — test utility, production code should not depend on this.
//!
//! Self-built FIFO queue + call recording. Not behind `cfg(test)` so that
//! sibling test modules across the crate (and future task_writer / dispatcher
//! tests) can use it; LTO drops unused mock code from release builds.

use crate::lark_cli::error::LarkError;
use crate::lark_cli::runner::{LarkRunner, RunOptions};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

struct MockState {
    queue: VecDeque<Result<Value, LarkError>>,
    calls: Vec<Vec<String>>,
}

/// Test double for [`LarkRunner`]. Pre-load responses with `enqueue_ok` /
/// `enqueue_err` (fluent `&Self` chain); inspect with `calls()`; assert
/// "consumed all" with `assert_no_unconsumed`.
pub struct MockLarkRunner {
    inner: Arc<Mutex<MockState>>,
}

impl MockLarkRunner {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MockState {
                queue: VecDeque::new(),
                calls: Vec::new(),
            })),
        }
    }

    pub fn enqueue_ok(&self, value: Value) -> &Self {
        self.inner.lock().unwrap().queue.push_back(Ok(value));
        self
    }

    pub fn enqueue_err(&self, err: LarkError) -> &Self {
        self.inner.lock().unwrap().queue.push_back(Err(err));
        self
    }

    /// Snapshot of recorded calls (owned args per call, in chronological order).
    pub fn calls(&self) -> Vec<Vec<String>> {
        self.inner.lock().unwrap().calls.clone()
    }

    /// Panic if the response queue still has unconsumed entries.
    pub fn assert_no_unconsumed(&self) {
        let queue_len = self.inner.lock().unwrap().queue.len();
        assert_eq!(
            queue_len, 0,
            "MockLarkRunner: {queue_len} unconsumed response(s) in queue"
        );
    }
}

impl Default for MockLarkRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MockLarkRunner {
    fn drop(&mut self) {
        // Inner Arc may have other strong refs (e.g. when wrapped in Journaled);
        // only warn from the last drop.
        if Arc::strong_count(&self.inner) > 1 {
            return;
        }
        if let Ok(state) = self.inner.lock()
            && !state.queue.is_empty()
        {
            tracing::warn!(
                unconsumed = state.queue.len(),
                "MockLarkRunner dropped with unconsumed response queue"
            );
        }
    }
}

#[async_trait]
impl LarkRunner for MockLarkRunner {
    async fn run_with_options(&self, args: &[&str], _opts: RunOptions) -> Result<Value, LarkError> {
        let mut state = self.inner.lock().unwrap();
        state
            .calls
            .push(args.iter().map(|s| s.to_string()).collect());
        match state.queue.pop_front() {
            Some(r) => r,
            None => panic!(
                "MockLarkRunner: queue exhausted on call #{} with args {:?}",
                state.calls.len(),
                args
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[tokio::test]
    async fn s4_1_enqueue_ok_then_call() {
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!({"foo":"bar"}));
        let v = mock.run(&["x"]).await.unwrap();
        assert_eq!(v, json!({"foo":"bar"}));
        assert_eq!(mock.calls(), vec![vec!["x".to_string()]]);
    }

    #[tokio::test]
    async fn s4_2_enqueue_err_then_call() {
        let mock = MockLarkRunner::new();
        mock.enqueue_err(LarkError::Timeout { timeout_ms: 5 });
        let err = mock.run(&["x"]).await.unwrap_err();
        match err {
            LarkError::Timeout { timeout_ms } => assert_eq!(timeout_ms, 5),
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn s4_3_fifo_order() {
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!(1))
            .enqueue_ok(json!(2))
            .enqueue_ok(json!(3));
        assert_eq!(mock.run(&["a"]).await.unwrap(), json!(1));
        assert_eq!(mock.run(&["b"]).await.unwrap(), json!(2));
        assert_eq!(mock.run(&["c"]).await.unwrap(), json!(3));
    }

    #[tokio::test]
    async fn s4_3b_fluent_chain_with_mixed_ok_err() {
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!("v1"))
            .enqueue_err(LarkError::Timeout { timeout_ms: 1 })
            .enqueue_ok(json!("v2"));
        assert_eq!(mock.run(&["a"]).await.unwrap(), json!("v1"));
        assert!(matches!(
            mock.run(&["b"]).await.unwrap_err(),
            LarkError::Timeout { .. }
        ));
        assert_eq!(mock.run(&["c"]).await.unwrap(), json!("v2"));
    }

    #[tokio::test]
    #[should_panic(expected = "queue exhausted")]
    async fn s4_4_empty_queue_panics() {
        let mock = MockLarkRunner::new();
        let _ = mock.run(&["x"]).await;
    }

    #[tokio::test]
    async fn s4_5_assert_no_unconsumed() {
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!(1));
        mock.enqueue_ok(json!(2));
        mock.run(&["a"]).await.unwrap();
        mock.run(&["b"]).await.unwrap();
        mock.assert_no_unconsumed(); // consumed exactly, no panic
    }

    #[tokio::test]
    #[should_panic(expected = "unconsumed response")]
    async fn s4_5b_assert_no_unconsumed_panics_on_leftover() {
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!(1));
        mock.enqueue_ok(json!(2));
        mock.run(&["a"]).await.unwrap();
        mock.assert_no_unconsumed();
    }

    #[tokio::test]
    async fn s4_6_calls_order_matches_invocation() {
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!(0)).enqueue_ok(json!(0));
        mock.run(&["first", "--flag"]).await.unwrap();
        mock.run(&["second"]).await.unwrap();
        let calls = mock.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], vec!["first", "--flag"]);
        assert_eq!(calls[1], vec!["second"]);
    }

    #[tokio::test]
    async fn s1_2_dyn_compatible_via_box_dyn() {
        let r: Box<dyn LarkRunner> = Box::new(MockLarkRunner::new());
        // Box<dyn LarkRunner> coerces to &dyn LarkRunner for the call.
        // We need a pre-loaded response — Box can't access enqueue_*. So
        // we build the mock first, enqueue, then box it.
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!("via-box"));
        let r2: Box<dyn LarkRunner> = Box::new(mock);
        let v = r2
            .run_with_options(&["x"], RunOptions::new())
            .await
            .unwrap();
        assert_eq!(v, json!("via-box"));
        // (The first `r` is dropped silently — its empty queue is fine.)
        drop(r);
    }

    #[tokio::test]
    async fn s1_1_trait_default_method_delegates() {
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!("default-path"));
        // Calling `run` exercises the default method, which delegates
        // to run_with_options(args, RunOptions::default()).
        let v = mock.run(&["x"]).await.unwrap();
        assert_eq!(v, json!("default-path"));
    }

    #[tokio::test]
    async fn run_options_are_ignored_by_mock() {
        // Mock doesn't honor RunOptions; just records args. This is by design
        // — tests are about response/argv interactions, not subprocess
        // configuration.
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!("ok"));
        let opts = RunOptions::new()
            .with_timeout(Duration::from_secs(1))
            .with_stdin("ignored")
            .with_profile("ignored");
        let v = mock.run_with_options(&["x"], opts).await.unwrap();
        assert_eq!(v, json!("ok"));
    }
}
