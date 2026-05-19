//! 进程内可取消令牌：跨 daemon / consume_im / 未来其他 bot_bridge 子模块的
//! 共享 cancel 信号。提取自原 `daemon.rs::CancelToken`（codex round-8 P1 修复，
//! consume_im 需要接收同一 token 以支持 graceful shutdown）。
//!
//! 设计选择 A（仓库无 `tokio-util::sync::CancellationToken` 依赖，手撸
//! `Arc<AtomicBool>` + `tokio::sync::Notify` 即足）。
//!
//! 用法：
//! ```ignore
//! let cancel = Arc::new(CancelToken::new());
//! let waited = cancel.clone();
//! tokio::select! {
//!     biased;
//!     _ = waited.cancelled() => { /* clean up */ }
//!     /* normal work */
//! }
//! ```

use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

#[derive(Debug, Default)]
pub struct CancelToken {
    flag: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        if !self.flag.swap(true, Ordering::SeqCst) {
            self.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Await until cancel fires. Re-entrant safe; multiple waiters all wake
    /// on a single `cancel()` call.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        loop {
            let waited = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            waited.await;
            if self.is_cancelled() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn cancelled_resolves_after_cancel() {
        let t = Arc::new(CancelToken::new());
        let t2 = t.clone();
        let h = tokio::spawn(async move {
            t2.cancelled().await;
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!t.is_cancelled());
        t.cancel();
        tokio::time::timeout(Duration::from_millis(100), h)
            .await
            .expect("should resolve in <100ms")
            .unwrap();
        assert!(t.is_cancelled());
    }

    #[tokio::test]
    async fn cancelled_returns_immediately_if_already_cancelled() {
        let t = CancelToken::new();
        t.cancel();
        tokio::time::timeout(Duration::from_millis(50), t.cancelled())
            .await
            .expect("immediate return");
    }
}
