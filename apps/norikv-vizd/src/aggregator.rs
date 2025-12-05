//! Event aggregator with 50ms coalesce window.
//!
//! Receives VizEvents from nodes, batches them in 50ms windows,
//! stores in ring buffer, and broadcasts to WebSocket clients.

use crate::event::{current_timestamp_ms, EventBatch, TimestampedEvent, WireEvent};
use crate::ringbuffer::EventRingBuffer;
use nori_observe::VizEvent;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::time::{interval, Duration, Instant};
use tracing::{debug, info, warn};

/// Coalesce window duration in milliseconds.
const COALESCE_WINDOW_MS: u64 = 50;

/// Broadcast channel capacity for real-time updates.
const BROADCAST_CHANNEL_SIZE: usize = 1024;

/// Event aggregator that batches events and broadcasts to clients.
pub struct EventAggregator {
    /// Per-node event accumulators for current window.
    current_window: RwLock<HashMap<u32, Vec<TimestampedEvent>>>,
    /// Window start time.
    window_start_ms: RwLock<u64>,
    /// Ring buffer for 60-minute playback.
    ring_buffer: Arc<EventRingBuffer>,
    /// Broadcast channel for real-time updates.
    broadcast_tx: broadcast::Sender<EventBatch>,
    /// Metrics
    total_events_received: RwLock<u64>,
    total_batches_sent: RwLock<u64>,
}

impl EventAggregator {
    /// Create a new event aggregator.
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(BROADCAST_CHANNEL_SIZE);
        Arc::new(Self {
            current_window: RwLock::new(HashMap::new()),
            window_start_ms: RwLock::new(current_timestamp_ms()),
            ring_buffer: Arc::new(EventRingBuffer::new()),
            broadcast_tx: tx,
            total_events_received: RwLock::new(0),
            total_batches_sent: RwLock::new(0),
        })
    }

    /// Subscribe to real-time event batches.
    pub fn subscribe(&self) -> broadcast::Receiver<EventBatch> {
        self.broadcast_tx.subscribe()
    }

    /// Get a reference to the ring buffer for history queries.
    pub fn ring_buffer(&self) -> &Arc<EventRingBuffer> {
        &self.ring_buffer
    }

    /// Ingest a VizEvent from a node.
    pub fn ingest(&self, node_id: u32, event: VizEvent) {
        let wire_event: WireEvent = event.into();
        let timestamped = TimestampedEvent::new(node_id, wire_event);

        let mut window = self.current_window.write();
        window.entry(node_id).or_default().push(timestamped);

        *self.total_events_received.write() += 1;
    }

    /// Ingest a pre-converted WireEvent (for testing or direct injection).
    pub fn ingest_wire(&self, node_id: u32, event: WireEvent) {
        let timestamped = TimestampedEvent::new(node_id, event);

        let mut window = self.current_window.write();
        window.entry(node_id).or_default().push(timestamped);

        *self.total_events_received.write() += 1;
    }

    /// Flush the current window and broadcast to clients.
    fn flush_window(&self) {
        let now = current_timestamp_ms();
        let window_start = *self.window_start_ms.read();

        // Collect all events from current window
        let events: Vec<TimestampedEvent> = {
            let mut window = self.current_window.write();
            window.drain().flat_map(|(_, v)| v).collect()
        };

        // Update window start for next batch
        *self.window_start_ms.write() = now;

        // Skip empty windows
        if events.is_empty() {
            return;
        }

        let batch = EventBatch::new(window_start, now, events);
        let event_count = batch.events.len();

        // Store in ring buffer for playback
        self.ring_buffer.push(batch.clone());

        // Broadcast to connected clients
        let receiver_count = self.broadcast_tx.receiver_count();
        if receiver_count > 0 {
            match self.broadcast_tx.send(batch) {
                Ok(_) => {
                    debug!(
                        events = event_count,
                        receivers = receiver_count,
                        "Broadcast batch"
                    );
                }
                Err(e) => {
                    warn!("Failed to broadcast batch: {}", e);
                }
            }
        }

        *self.total_batches_sent.write() += 1;
    }

    /// Start the coalescing ticker (run in background task).
    ///
    /// This should be spawned as a tokio task.
    pub async fn run_coalesce_loop(self: Arc<Self>) {
        info!(
            window_ms = COALESCE_WINDOW_MS,
            "Starting event coalesce loop"
        );

        let mut ticker = interval(Duration::from_millis(COALESCE_WINDOW_MS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            self.flush_window();
        }
    }

    /// Get aggregator statistics.
    pub fn stats(&self) -> AggregatorStats {
        AggregatorStats {
            total_events_received: *self.total_events_received.read(),
            total_batches_sent: *self.total_batches_sent.read(),
            buffer_size: self.ring_buffer.len(),
            buffer_events: self.ring_buffer.total_events(),
            connected_clients: self.broadcast_tx.receiver_count(),
            oldest_event_ms: self.ring_buffer.oldest_timestamp(),
            newest_event_ms: self.ring_buffer.newest_timestamp(),
        }
    }
}

impl Default for EventAggregator {
    fn default() -> Self {
        Arc::try_unwrap(Self::new()).unwrap_or_else(|arc| (*arc).clone())
    }
}

impl Clone for EventAggregator {
    fn clone(&self) -> Self {
        Self {
            current_window: RwLock::new(self.current_window.read().clone()),
            window_start_ms: RwLock::new(*self.window_start_ms.read()),
            ring_buffer: Arc::clone(&self.ring_buffer),
            broadcast_tx: self.broadcast_tx.clone(),
            total_events_received: RwLock::new(*self.total_events_received.read()),
            total_batches_sent: RwLock::new(*self.total_batches_sent.read()),
        }
    }
}

/// Statistics about the aggregator state.
#[derive(Clone, Debug, serde::Serialize)]
pub struct AggregatorStats {
    pub total_events_received: u64,
    pub total_batches_sent: u64,
    pub buffer_size: usize,
    pub buffer_events: usize,
    pub connected_clients: usize,
    pub oldest_event_ms: Option<u64>,
    pub newest_event_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{WireRaftEvt, WireRaftKind};
    use std::time::Duration;

    #[tokio::test]
    async fn test_ingest_and_flush() {
        let agg = EventAggregator::new();
        let mut rx = agg.subscribe();

        // Ingest some events
        agg.ingest_wire(
            1,
            WireEvent::Raft(WireRaftEvt {
                shard: 1,
                term: 1,
                kind: WireRaftKind::LeaderElected { node: 1 },
            }),
        );
        agg.ingest_wire(
            2,
            WireEvent::Raft(WireRaftEvt {
                shard: 1,
                term: 1,
                kind: WireRaftKind::VoteGranted { from: 2 },
            }),
        );

        // Manually flush
        agg.flush_window();

        // Should receive a batch
        let batch = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("Timeout waiting for batch")
            .expect("Channel closed");

        assert_eq!(batch.events.len(), 2);
        assert_eq!(batch.summary.raft_count, 2);
    }

    #[tokio::test]
    async fn test_ring_buffer_storage() {
        let agg = EventAggregator::new();

        // Ingest and flush
        agg.ingest_wire(
            1,
            WireEvent::Raft(WireRaftEvt {
                shard: 1,
                term: 1,
                kind: WireRaftKind::LeaderElected { node: 1 },
            }),
        );
        agg.flush_window();

        // Check ring buffer
        let buffer = agg.ring_buffer();
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.total_events(), 1);
    }

    #[tokio::test]
    async fn test_stats() {
        let agg = EventAggregator::new();
        let _rx = agg.subscribe();

        agg.ingest_wire(
            1,
            WireEvent::Raft(WireRaftEvt {
                shard: 1,
                term: 1,
                kind: WireRaftKind::LeaderElected { node: 1 },
            }),
        );

        let stats = agg.stats();
        assert_eq!(stats.total_events_received, 1);
        assert_eq!(stats.connected_clients, 1);
    }
}
