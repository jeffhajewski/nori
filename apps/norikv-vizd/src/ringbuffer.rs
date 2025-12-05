//! Ring buffer for 60-minute event history with efficient range queries.

use crate::event::EventBatch;
use parking_lot::RwLock;
use std::collections::VecDeque;

/// 50ms coalesce windows = 20 batches/second = 72,000 batches for 60 minutes.
const BATCHES_PER_SECOND: usize = 20;
const SECONDS_PER_MINUTE: usize = 60;
const BUFFER_DURATION_MINS: usize = 60;
const DEFAULT_CAPACITY: usize = BATCHES_PER_SECOND * SECONDS_PER_MINUTE * BUFFER_DURATION_MINS;

/// Thread-safe ring buffer for storing event batches.
///
/// Automatically evicts old entries when capacity is reached or when
/// entries exceed the configured time window.
pub struct EventRingBuffer {
    buffer: RwLock<VecDeque<EventBatch>>,
    capacity: usize,
    max_duration_ms: u64,
}

impl EventRingBuffer {
    /// Create a new ring buffer with default 60-minute capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY, 60)
    }

    /// Create a ring buffer with custom capacity and duration.
    pub fn with_capacity(capacity: usize, max_duration_mins: u64) -> Self {
        Self {
            buffer: RwLock::new(VecDeque::with_capacity(capacity)),
            capacity,
            max_duration_ms: max_duration_mins * 60 * 1000,
        }
    }

    /// Push a new batch to the buffer, evicting old entries as needed.
    pub fn push(&self, batch: EventBatch) {
        let mut buf = self.buffer.write();

        // Evict entries that exceed the time window
        let cutoff_ms = batch.window_end.saturating_sub(self.max_duration_ms);
        while let Some(front) = buf.front() {
            if front.window_end < cutoff_ms {
                buf.pop_front();
            } else {
                break;
            }
        }

        // Evict if at capacity
        if buf.len() >= self.capacity {
            buf.pop_front();
        }

        buf.push_back(batch);
    }

    /// Get all batches in a time range (inclusive).
    pub fn range(&self, from_ms: u64, to_ms: u64) -> Vec<EventBatch> {
        let buf = self.buffer.read();
        buf.iter()
            .filter(|b| b.window_start >= from_ms && b.window_end <= to_ms)
            .cloned()
            .collect()
    }

    /// Get all batches from a timestamp to now.
    pub fn since(&self, from_ms: u64) -> Vec<EventBatch> {
        let buf = self.buffer.read();
        buf.iter()
            .filter(|b| b.window_start >= from_ms)
            .cloned()
            .collect()
    }

    /// Get the most recent N batches.
    pub fn recent(&self, count: usize) -> Vec<EventBatch> {
        let buf = self.buffer.read();
        buf.iter().rev().take(count).cloned().collect()
    }

    /// Get the oldest timestamp in the buffer, if any.
    pub fn oldest_timestamp(&self) -> Option<u64> {
        self.buffer.read().front().map(|b| b.window_start)
    }

    /// Get the newest timestamp in the buffer, if any.
    pub fn newest_timestamp(&self) -> Option<u64> {
        self.buffer.read().back().map(|b| b.window_end)
    }

    /// Get the current number of batches in the buffer.
    pub fn len(&self) -> usize {
        self.buffer.read().len()
    }

    /// Check if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.read().is_empty()
    }

    /// Clear all entries from the buffer.
    pub fn clear(&self) {
        self.buffer.write().clear();
    }

    /// Get total event count across all batches.
    pub fn total_events(&self) -> usize {
        self.buffer.read().iter().map(|b| b.events.len()).sum()
    }
}

impl Default for EventRingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{BatchSummary, TimestampedEvent, WireEvent, WireSwimEvt, WireSwimKind};

    fn make_batch(start: u64, end: u64, event_count: usize) -> EventBatch {
        let events: Vec<_> = (0..event_count)
            .map(|i| TimestampedEvent {
                ts: start + i as u64,
                node: 1,
                event: WireEvent::Swim(WireSwimEvt {
                    node: 1,
                    kind: WireSwimKind::Alive,
                }),
            })
            .collect();
        EventBatch {
            window_start: start,
            window_end: end,
            summary: BatchSummary::from_events(&events),
            events,
        }
    }

    #[test]
    fn test_push_and_len() {
        let buf = EventRingBuffer::with_capacity(10, 60);
        assert!(buf.is_empty());

        buf.push(make_batch(0, 50, 5));
        assert_eq!(buf.len(), 1);

        buf.push(make_batch(50, 100, 3));
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn test_capacity_eviction() {
        let buf = EventRingBuffer::with_capacity(3, 60);

        buf.push(make_batch(0, 50, 1));
        buf.push(make_batch(50, 100, 1));
        buf.push(make_batch(100, 150, 1));
        assert_eq!(buf.len(), 3);

        buf.push(make_batch(150, 200, 1));
        assert_eq!(buf.len(), 3);

        // Oldest should have been evicted
        assert_eq!(buf.oldest_timestamp(), Some(50));
    }

    #[test]
    fn test_time_based_eviction() {
        // 1 minute max duration
        let buf = EventRingBuffer::with_capacity(100, 1);

        // Push batch at time 0
        buf.push(make_batch(0, 50, 1));
        assert_eq!(buf.len(), 1);

        // Push batch at time 61 seconds (61000ms) - should evict first
        buf.push(make_batch(61000, 61050, 1));
        assert_eq!(buf.len(), 1);
        assert_eq!(buf.oldest_timestamp(), Some(61000));
    }

    #[test]
    fn test_range_query() {
        let buf = EventRingBuffer::with_capacity(10, 60);

        buf.push(make_batch(0, 50, 1));
        buf.push(make_batch(50, 100, 1));
        buf.push(make_batch(100, 150, 1));
        buf.push(make_batch(150, 200, 1));

        let result = buf.range(50, 150);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].window_start, 50);
        assert_eq!(result[1].window_start, 100);
    }

    #[test]
    fn test_since_query() {
        let buf = EventRingBuffer::with_capacity(10, 60);

        buf.push(make_batch(0, 50, 1));
        buf.push(make_batch(50, 100, 1));
        buf.push(make_batch(100, 150, 1));

        let result = buf.since(50);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_recent() {
        let buf = EventRingBuffer::with_capacity(10, 60);

        buf.push(make_batch(0, 50, 1));
        buf.push(make_batch(50, 100, 1));
        buf.push(make_batch(100, 150, 1));

        let result = buf.recent(2);
        assert_eq!(result.len(), 2);
        // Recent returns in reverse order (newest first)
        assert_eq!(result[0].window_start, 100);
        assert_eq!(result[1].window_start, 50);
    }

    #[test]
    fn test_total_events() {
        let buf = EventRingBuffer::with_capacity(10, 60);

        buf.push(make_batch(0, 50, 5));
        buf.push(make_batch(50, 100, 3));
        buf.push(make_batch(100, 150, 2));

        assert_eq!(buf.total_events(), 10);
    }
}
