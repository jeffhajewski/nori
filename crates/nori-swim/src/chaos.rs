//! Chaos testing utilities for SWIM protocol.
//!
//! Provides a chaos transport wrapper that can simulate:
//! - Packet loss
//! - Message delays
//! - Message reordering
//!
//! Useful for testing protocol resilience under adverse network conditions.

use crate::message::SwimMessage;
use crate::transport::SwimTransport;
use crate::SwimError;
use async_trait::async_trait;
use rand::Rng;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Configuration for chaos injection.
#[derive(Debug, Clone)]
pub struct ChaosConfig {
    /// Probability of dropping a message (0.0 - 1.0)
    pub drop_rate: f64,

    /// Probability of delaying a message (0.0 - 1.0)
    pub delay_rate: f64,

    /// Min delay when a message is delayed
    pub delay_min: Duration,

    /// Max delay when a message is delayed
    pub delay_max: Duration,

    /// Probability of reordering (0.0 - 1.0)
    /// When triggered, message goes to reorder queue instead of immediate send
    pub reorder_rate: f64,

    /// Max messages to hold in reorder queue before flushing
    pub reorder_queue_size: usize,
}

impl Default for ChaosConfig {
    fn default() -> Self {
        Self {
            drop_rate: 0.0,
            delay_rate: 0.0,
            delay_min: Duration::from_millis(10),
            delay_max: Duration::from_millis(100),
            reorder_rate: 0.0,
            reorder_queue_size: 5,
        }
    }
}

impl ChaosConfig {
    /// Create config with specified drop rate.
    pub fn with_drop_rate(mut self, rate: f64) -> Self {
        self.drop_rate = rate.clamp(0.0, 1.0);
        self
    }

    /// Create config with specified delay settings.
    pub fn with_delay(mut self, rate: f64, min: Duration, max: Duration) -> Self {
        self.delay_rate = rate.clamp(0.0, 1.0);
        self.delay_min = min;
        self.delay_max = max;
        self
    }

    /// Create config with specified reorder settings.
    pub fn with_reorder(mut self, rate: f64, queue_size: usize) -> Self {
        self.reorder_rate = rate.clamp(0.0, 1.0);
        self.reorder_queue_size = queue_size;
        self
    }

    /// Create a "lossy network" preset (10% loss).
    pub fn lossy() -> Self {
        Self::default().with_drop_rate(0.1)
    }

    /// Create a "high latency" preset (50% delay, 50-200ms).
    pub fn high_latency() -> Self {
        Self::default().with_delay(0.5, Duration::from_millis(50), Duration::from_millis(200))
    }

    /// Create a "chaotic network" preset (5% loss, 20% delay, 10% reorder).
    pub fn chaotic() -> Self {
        Self::default()
            .with_drop_rate(0.05)
            .with_delay(0.2, Duration::from_millis(20), Duration::from_millis(100))
            .with_reorder(0.1, 3)
    }
}

/// A pending reordered message.
struct ReorderedMessage {
    target: SocketAddr,
    msg: SwimMessage,
}

/// Transport wrapper that injects chaos (packet loss, delays, reordering).
pub struct ChaosTransport<T: SwimTransport> {
    /// Underlying transport
    inner: Arc<T>,

    /// Chaos configuration
    config: ChaosConfig,

    /// Queue for reordered messages
    reorder_queue: Mutex<VecDeque<ReorderedMessage>>,

    /// Statistics
    stats: Mutex<ChaosStats>,
}

/// Statistics about chaos injection.
#[derive(Debug, Clone, Default)]
pub struct ChaosStats {
    /// Total messages sent
    pub messages_sent: u64,

    /// Messages dropped
    pub messages_dropped: u64,

    /// Messages delayed
    pub messages_delayed: u64,

    /// Messages reordered
    pub messages_reordered: u64,
}

impl<T: SwimTransport> ChaosTransport<T> {
    /// Create a new chaos transport wrapping the given transport.
    pub fn new(inner: Arc<T>, config: ChaosConfig) -> Self {
        Self {
            inner,
            config,
            reorder_queue: Mutex::new(VecDeque::new()),
            stats: Mutex::new(ChaosStats::default()),
        }
    }

    /// Get current statistics.
    pub async fn stats(&self) -> ChaosStats {
        self.stats.lock().await.clone()
    }

    /// Reset statistics.
    pub async fn reset_stats(&self) {
        *self.stats.lock().await = ChaosStats::default();
    }

    /// Flush any reordered messages.
    pub async fn flush_reorder_queue(&self) -> Result<(), SwimError> {
        let mut queue = self.reorder_queue.lock().await;
        while let Some(msg) = queue.pop_front() {
            self.inner.send(msg.target, msg.msg).await?;
        }
        Ok(())
    }

    /// Check if we should drop this message.
    fn should_drop(&self) -> bool {
        if self.config.drop_rate <= 0.0 {
            return false;
        }
        rand::thread_rng().gen::<f64>() < self.config.drop_rate
    }

    /// Check if we should delay this message.
    fn should_delay(&self) -> bool {
        if self.config.delay_rate <= 0.0 {
            return false;
        }
        rand::thread_rng().gen::<f64>() < self.config.delay_rate
    }

    /// Get a random delay duration.
    fn random_delay(&self) -> Duration {
        let mut rng = rand::thread_rng();
        let min_ms = self.config.delay_min.as_millis() as u64;
        let max_ms = self.config.delay_max.as_millis() as u64;
        Duration::from_millis(rng.gen_range(min_ms..=max_ms))
    }

    /// Check if we should reorder this message.
    fn should_reorder(&self) -> bool {
        if self.config.reorder_rate <= 0.0 {
            return false;
        }
        rand::thread_rng().gen::<f64>() < self.config.reorder_rate
    }
}

#[async_trait]
impl<T: SwimTransport> SwimTransport for ChaosTransport<T> {
    async fn send(&self, target: SocketAddr, msg: SwimMessage) -> Result<(), SwimError> {
        // Update stats
        {
            let mut stats = self.stats.lock().await;
            stats.messages_sent += 1;
        }

        // Check for drop
        if self.should_drop() {
            let mut stats = self.stats.lock().await;
            stats.messages_dropped += 1;
            return Ok(()); // Silently drop
        }

        // Check for reorder
        if self.should_reorder() {
            let mut queue = self.reorder_queue.lock().await;
            let mut stats = self.stats.lock().await;
            stats.messages_reordered += 1;

            queue.push_back(ReorderedMessage {
                target,
                msg: msg.clone(),
            });

            // If queue is full, flush oldest
            if queue.len() >= self.config.reorder_queue_size {
                if let Some(old) = queue.pop_front() {
                    drop(queue); // Release lock before send
                    drop(stats);
                    self.inner.send(old.target, old.msg).await?;
                }
            }

            return Ok(());
        }

        // Check for delay
        if self.should_delay() {
            {
                let mut stats = self.stats.lock().await;
                stats.messages_delayed += 1;
            }
            let delay = self.random_delay();
            tokio::time::sleep(delay).await;
        }

        // First, flush any reordered messages (FIFO)
        self.flush_reorder_queue().await?;

        // Send the message
        self.inner.send(target, msg).await
    }

    async fn recv(&self) -> Result<(SocketAddr, SwimMessage), SwimError> {
        // Receive passes through directly
        self.inner.recv().await
    }

    fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{create_transport_mesh, InMemoryTransport};

    fn test_addr(port: u16) -> SocketAddr {
        format!("127.0.0.1:{}", port).parse().unwrap()
    }

    #[tokio::test]
    async fn test_chaos_no_effects() {
        let addr1 = test_addr(8001);
        let addr2 = test_addr(8002);

        let mesh = create_transport_mesh(vec![addr1, addr2]);
        let t1 = mesh.get(&addr1).unwrap().clone();
        let t2 = mesh.get(&addr2).unwrap().clone();

        // No chaos - all messages should pass through
        let chaos = Arc::new(ChaosTransport::new(t1, ChaosConfig::default()));

        let msg = SwimMessage::Ping {
            seq: 1,
            from_id: "node1".to_string(),
            from_addr: addr1,
            gossip: vec![],
        };

        chaos.send(addr2, msg.clone()).await.unwrap();

        let (from, received) = t2.recv().await.unwrap();
        assert_eq!(from, addr1);
        assert_eq!(received, msg);

        let stats = chaos.stats().await;
        assert_eq!(stats.messages_sent, 1);
        assert_eq!(stats.messages_dropped, 0);
    }

    #[tokio::test]
    async fn test_chaos_drop() {
        let addr1 = test_addr(8001);
        let addr2 = test_addr(8002);

        let mesh = create_transport_mesh(vec![addr1, addr2]);
        let t1 = mesh.get(&addr1).unwrap().clone();
        let t2 = mesh.get(&addr2).unwrap().clone();

        // 100% drop rate
        let config = ChaosConfig::default().with_drop_rate(1.0);
        let chaos = Arc::new(ChaosTransport::new(t1, config));

        let msg = SwimMessage::Ping {
            seq: 1,
            from_id: "node1".to_string(),
            from_addr: addr1,
            gossip: vec![],
        };

        chaos.send(addr2, msg).await.unwrap();

        // Message should be dropped - recv should timeout
        let result = tokio::time::timeout(Duration::from_millis(50), t2.recv()).await;
        assert!(result.is_err(), "Message should have been dropped");

        let stats = chaos.stats().await;
        assert_eq!(stats.messages_sent, 1);
        assert_eq!(stats.messages_dropped, 1);
    }

    #[tokio::test]
    async fn test_chaos_delay() {
        let addr1 = test_addr(8001);
        let addr2 = test_addr(8002);

        let mesh = create_transport_mesh(vec![addr1, addr2]);
        let t1 = mesh.get(&addr1).unwrap().clone();
        let t2 = mesh.get(&addr2).unwrap().clone();

        // 100% delay with 50ms
        let config = ChaosConfig::default()
            .with_delay(1.0, Duration::from_millis(50), Duration::from_millis(50));
        let chaos = Arc::new(ChaosTransport::new(t1, config));

        let msg = SwimMessage::Ping {
            seq: 1,
            from_id: "node1".to_string(),
            from_addr: addr1,
            gossip: vec![],
        };

        let start = std::time::Instant::now();
        chaos.send(addr2, msg).await.unwrap();
        let elapsed = start.elapsed();

        // Should have been delayed ~50ms
        assert!(elapsed >= Duration::from_millis(45), "Message should have been delayed");

        // Message should still arrive
        let (from, _) = t2.recv().await.unwrap();
        assert_eq!(from, addr1);

        let stats = chaos.stats().await;
        assert_eq!(stats.messages_delayed, 1);
    }

    #[tokio::test]
    async fn test_chaos_presets() {
        // Test that presets have expected values
        let lossy = ChaosConfig::lossy();
        assert!((lossy.drop_rate - 0.1).abs() < 0.001);

        let high_latency = ChaosConfig::high_latency();
        assert!((high_latency.delay_rate - 0.5).abs() < 0.001);

        let chaotic = ChaosConfig::chaotic();
        assert!((chaotic.drop_rate - 0.05).abs() < 0.001);
        assert!((chaotic.delay_rate - 0.2).abs() < 0.001);
        assert!((chaotic.reorder_rate - 0.1).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_chaos_stats_reset() {
        let addr = test_addr(8001);
        let (transport, _) = InMemoryTransport::new(addr);

        let chaos = ChaosTransport::new(
            Arc::new(transport),
            ChaosConfig::default().with_drop_rate(1.0),
        );

        // Send a message (will be dropped)
        let msg = SwimMessage::Ping {
            seq: 1,
            from_id: "node1".to_string(),
            from_addr: addr,
            gossip: vec![],
        };
        let _ = chaos.send(addr, msg).await;

        let stats = chaos.stats().await;
        assert_eq!(stats.messages_sent, 1);

        // Reset stats
        chaos.reset_stats().await;

        let stats = chaos.stats().await;
        assert_eq!(stats.messages_sent, 0);
        assert_eq!(stats.messages_dropped, 0);
    }
}
