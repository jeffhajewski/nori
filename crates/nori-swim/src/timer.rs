//! Timer abstractions for SWIM protocol.
//!
//! Provides resettable, shutdown-aware timers for:
//! - Probe intervals (periodic)
//! - Ping timeouts (one-shot)

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::sleep;

/// Periodic timer that fires at regular intervals.
///
/// Can be reset or shut down. Used for probe intervals.
pub struct PeriodicTimer {
    /// Interval between ticks
    interval: Duration,

    /// Notifier for shutdown
    shutdown: Arc<Notify>,
}

impl PeriodicTimer {
    /// Create a new periodic timer.
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            shutdown: Arc::new(Notify::new()),
        }
    }

    /// Get a handle to shut down this timer.
    pub fn shutdown_handle(&self) -> Arc<Notify> {
        self.shutdown.clone()
    }

    /// Request shutdown.
    pub fn shutdown(&self) {
        self.shutdown.notify_one();
    }

    /// Run the timer, calling `on_tick` at each interval.
    ///
    /// Runs until shutdown is requested.
    pub async fn run<F>(self, mut on_tick: F)
    where
        F: FnMut(),
    {
        loop {
            tokio::select! {
                _ = sleep(self.interval) => {
                    on_tick();
                }
                _ = self.shutdown.notified() => {
                    break;
                }
            }
        }
    }

    /// Run the timer, calling an async `on_tick` at each interval.
    pub async fn run_async<F, Fut>(self, mut on_tick: F)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        loop {
            tokio::select! {
                _ = sleep(self.interval) => {
                    on_tick().await;
                }
                _ = self.shutdown.notified() => {
                    break;
                }
            }
        }
    }
}

/// One-shot timeout that can be cancelled.
pub struct Timeout {
    duration: Duration,
    cancel: Arc<Notify>,
}

/// Result of waiting on a timeout.
#[derive(Debug, PartialEq, Eq)]
pub enum TimeoutResult {
    /// Timeout elapsed
    Elapsed,
    /// Timeout was cancelled
    Cancelled,
}

impl Timeout {
    /// Create a new timeout.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            cancel: Arc::new(Notify::new()),
        }
    }

    /// Get a handle to cancel this timeout.
    pub fn cancel_handle(&self) -> Arc<Notify> {
        self.cancel.clone()
    }

    /// Cancel the timeout.
    pub fn cancel(&self) {
        self.cancel.notify_one();
    }

    /// Wait for the timeout or cancellation.
    pub async fn wait(self) -> TimeoutResult {
        tokio::select! {
            _ = sleep(self.duration) => TimeoutResult::Elapsed,
            _ = self.cancel.notified() => TimeoutResult::Cancelled,
        }
    }

    /// Wait for the timeout, or return early if the future completes.
    ///
    /// Returns `Ok(T)` if the future completes, `Err(())` if timeout elapsed.
    pub async fn race<T, F: std::future::Future<Output = T>>(
        duration: Duration,
        future: F,
    ) -> Result<T, ()> {
        tokio::select! {
            result = future => Ok(result),
            _ = sleep(duration) => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_periodic_timer_fires() {
        let count = Arc::new(AtomicU32::new(0));
        let count_clone = count.clone();

        let timer = PeriodicTimer::new(Duration::from_millis(10));
        let shutdown = timer.shutdown_handle();

        let handle = tokio::spawn(async move {
            timer.run(move || {
                count_clone.fetch_add(1, Ordering::SeqCst);
            }).await;
        });

        // Wait for a few ticks
        sleep(Duration::from_millis(55)).await;
        shutdown.notify_one();

        let _ = timeout(Duration::from_millis(100), handle).await;

        let final_count = count.load(Ordering::SeqCst);
        assert!(final_count >= 4, "Expected at least 4 ticks, got {}", final_count);
    }

    #[tokio::test]
    async fn test_periodic_timer_shutdown() {
        let timer = PeriodicTimer::new(Duration::from_secs(10)); // Long interval
        let shutdown = timer.shutdown_handle();

        // Shutdown immediately
        shutdown.notify_one();

        // Timer should exit quickly
        let result = timeout(Duration::from_millis(100), timer.run(|| {})).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_timeout_elapsed() {
        let t = Timeout::new(Duration::from_millis(10));
        let result = t.wait().await;
        assert_eq!(result, TimeoutResult::Elapsed);
    }

    #[tokio::test]
    async fn test_timeout_cancelled() {
        let t = Timeout::new(Duration::from_secs(10)); // Long timeout
        let cancel = t.cancel_handle();

        // Cancel after a short delay
        tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            cancel.notify_one();
        });

        let result = t.wait().await;
        assert_eq!(result, TimeoutResult::Cancelled);
    }

    #[tokio::test]
    async fn test_timeout_race_completes() {
        let result = Timeout::race(
            Duration::from_secs(10),
            async { 42 }
        ).await;
        assert_eq!(result, Ok(42));
    }

    #[tokio::test]
    async fn test_timeout_race_elapsed() {
        let result = Timeout::race(
            Duration::from_millis(10),
            async {
                sleep(Duration::from_secs(10)).await;
                42
            }
        ).await;
        assert_eq!(result, Err(()));
    }
}
