//! Election timer for Raft.
//!
//! Per Raft §5.2, election timeout is randomized to prevent split votes.
//! When timeout fires, follower/candidate should start a new election.
//!
//! Key properties:
//! - Randomized timeout from config (default: 300-600ms)
//! - Resettable when heartbeat received
//! - Uses tokio sleep for async timing
//! - Can be gracefully shut down

use crate::config::RaftConfig;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tokio::time::sleep;

/// Election timer.
///
/// Fires when the election timeout elapses without being reset.
/// Used by followers and candidates to detect leader failure.
///
/// # Example
///
/// ```ignore
/// let timer = ElectionTimer::new(config.clone());
/// let timeout_rx = timer.timeout_channel();
///
/// tokio::spawn(async move {
///     timer.run().await;
/// });
///
/// // Wait for timeout
/// timeout_rx.recv().await;
/// println!("Election timeout fired!");
///
/// // Reset timer on heartbeat
/// timer.reset();
/// ```
pub struct ElectionTimer {
    /// Raft configuration (for random timeout)
    config: RaftConfig,

    /// Notify channel for resets
    reset_notify: Arc<Notify>,

    /// Notify channel for shutdown
    shutdown_notify: Arc<Notify>,

    /// Channel to send timeout notifications
    timeout_tx: tokio::sync::mpsc::Sender<()>,

    /// Receiver for timeout notifications (cloneable)
    timeout_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<()>>>,
}

impl ElectionTimer {
    /// Create a new election timer.
    pub fn new(config: RaftConfig) -> Self {
        let (timeout_tx, timeout_rx) = tokio::sync::mpsc::channel(1);

        Self {
            config,
            reset_notify: Arc::new(Notify::new()),
            shutdown_notify: Arc::new(Notify::new()),
            timeout_tx,
            timeout_rx: Arc::new(tokio::sync::Mutex::new(timeout_rx)),
        }
    }

    /// Get a receiver for timeout notifications.
    ///
    /// Returns a channel that receives a message when the election timeout fires.
    pub fn timeout_receiver(&self) -> tokio::sync::mpsc::Receiver<()> {
        let (tx, rx) = tokio::sync::mpsc::channel(10);

        // Forward from internal channel to new receiver
        let timeout_rx = self.timeout_rx.clone();
        let shutdown_notify = self.shutdown_notify.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(_) = async {
                        let mut rx = timeout_rx.lock().await;
                        rx.recv().await
                    } => {
                        if tx.send(()).await.is_err() {
                            break; // Receiver dropped
                        }
                    }
                    _ = shutdown_notify.notified() => {
                        break;
                    }
                }
            }
        });

        rx
    }

    /// Subscribe to timeout notifications (alias for timeout_receiver).
    pub fn subscribe(&self) -> tokio::sync::mpsc::Receiver<()> {
        self.timeout_receiver()
    }

    /// Reset the election timer.
    ///
    /// Called when:
    /// - Follower receives AppendEntries from leader
    /// - Follower grants vote to candidate
    /// - Node becomes leader (disables timer)
    pub fn reset(&self) {
        self.reset_notify.notify_one();
    }

    /// Shutdown the timer.
    pub fn shutdown(&self) {
        self.shutdown_notify.notify_one();
    }

    /// Run the timer loop (async task).
    ///
    /// Spawns a task that:
    /// 1. Sleeps for random election timeout
    /// 2. Sends timeout notification if not reset
    /// 3. Repeats until shutdown
    ///
    /// This should be spawned as a background task.
    pub async fn run(self: Arc<Self>) {
        loop {
            // Generate random timeout
            let timeout = self.config.random_election_timeout();
            let deadline = Instant::now() + timeout;

            // Sleep until deadline or reset
            tokio::select! {
                _ = sleep_until(deadline) => {
                    // Timeout fired - send notification
                    if self.timeout_tx.send(()).await.is_err() {
                        // Receiver dropped, exit
                        break;
                    }
                }
                _ = self.reset_notify.notified() => {
                    // Timer reset, restart loop with new timeout
                    continue;
                }
                _ = self.shutdown_notify.notified() => {
                    // Shutdown requested
                    break;
                }
            }
        }
    }
}

/// Sleep until an instant (wrapper for tokio::time::sleep_until).
///
/// Handles the case where deadline is in the past (returns immediately).
async fn sleep_until(deadline: Instant) {
    let now = Instant::now();
    if deadline > now {
        sleep(deadline - now).await;
    }
}

/// Simple resettable timer (alternative simpler API).
///
/// Provides a oneshot timer that can be reset.
/// Useful for testing or simpler use cases.
pub struct SimpleTimer {
    deadline: tokio::sync::RwLock<Instant>,
    reset_notify: Arc<Notify>,
}

impl SimpleTimer {
    /// Create a timer that fires after `duration`.
    pub fn new(duration: Duration) -> Self {
        Self {
            deadline: tokio::sync::RwLock::new(Instant::now() + duration),
            reset_notify: Arc::new(Notify::new()),
        }
    }

    /// Wait for the timer to fire.
    ///
    /// Returns when the deadline is reached.
    /// If timer is reset, waits for new deadline.
    pub async fn wait(&self) {
        loop {
            let deadline = *self.deadline.read().await;

            tokio::select! {
                _ = sleep_until(deadline) => {
                    return;
                }
                _ = self.reset_notify.notified() => {
                    // Timer reset, check new deadline
                    continue;
                }
            }
        }
    }

    /// Reset the timer with a new duration.
    pub async fn reset(&self, duration: Duration) {
        let mut deadline = self.deadline.write().await;
        *deadline = Instant::now() + duration;
        self.reset_notify.notify_waiters();
    }

    /// Check if timer has expired (non-blocking).
    pub async fn is_expired(&self) -> bool {
        let deadline = *self.deadline.read().await;
        Instant::now() >= deadline
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_election_timer_fires() {
        let mut config = RaftConfig::default();
        config.election_timeout_min = Duration::from_millis(50);
        config.election_timeout_max = Duration::from_millis(100);

        let timer = Arc::new(ElectionTimer::new(config));
        let mut timeout_rx = timer.timeout_receiver();

        // Spawn timer task
        let timer_clone = timer.clone();
        tokio::spawn(async move {
            timer_clone.run().await;
        });

        // Wait for timeout (should fire within 100ms)
        let result = timeout(Duration::from_millis(200), timeout_rx.recv()).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_election_timer_reset() {
        let mut config = RaftConfig::default();
        config.election_timeout_min = Duration::from_millis(100);
        config.election_timeout_max = Duration::from_millis(200);

        let timer = Arc::new(ElectionTimer::new(config));
        let mut timeout_rx = timer.timeout_receiver();

        // Spawn timer task
        let timer_clone = timer.clone();
        tokio::spawn(async move {
            timer_clone.run().await;
        });

        // Reset timer every 50ms, prevent timeout
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            timer.reset();
        }

        // Should not have timed out yet
        let result = timeout(Duration::from_millis(10), timeout_rx.recv()).await;
        assert!(result.is_err()); // Timeout (no message received)

        // Now let it fire
        let result = timeout(Duration::from_millis(250), timeout_rx.recv()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_simple_timer() {
        let timer = SimpleTimer::new(Duration::from_millis(50));

        let start = Instant::now();
        timer.wait().await;
        let elapsed = start.elapsed();

        assert!(elapsed >= Duration::from_millis(50));
        assert!(elapsed < Duration::from_millis(100)); // Should fire quickly
    }

    #[tokio::test]
    async fn test_simple_timer_reset() {
        let timer = SimpleTimer::new(Duration::from_millis(100));

        // Wait a bit, then reset with new timeout
        tokio::time::sleep(Duration::from_millis(50)).await;
        timer.reset(Duration::from_millis(100)).await;

        let start = Instant::now();
        timer.wait().await;
        let elapsed = start.elapsed();

        // Should have waited ~100ms from reset (not from initial creation)
        assert!(elapsed >= Duration::from_millis(90));
        assert!(elapsed < Duration::from_millis(150));
    }

    #[tokio::test]
    async fn test_simple_timer_is_expired() {
        let timer = SimpleTimer::new(Duration::from_millis(50));

        assert!(!timer.is_expired().await);

        tokio::time::sleep(Duration::from_millis(60)).await;

        assert!(timer.is_expired().await);
    }
}
