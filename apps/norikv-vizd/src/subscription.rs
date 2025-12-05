//! Subscription filters for WebSocket clients.
//!
//! Clients can filter events by node, event type, and shard.

use crate::event::{EventBatch, TimestampedEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Filter criteria for event subscriptions.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SubscriptionFilter {
    /// Filter by specific node IDs (empty = all nodes).
    #[serde(default)]
    pub nodes: HashSet<u32>,
    /// Filter by event types (empty = all types).
    /// Valid types: wal, compaction, lsm, raft, swim, shard, cache, heat
    #[serde(default)]
    pub types: HashSet<String>,
    /// Filter by specific shard IDs (empty = all shards).
    #[serde(default)]
    pub shards: HashSet<u32>,
}

impl SubscriptionFilter {
    /// Create a filter that accepts all events.
    pub fn all() -> Self {
        Self::default()
    }

    /// Create a filter for specific nodes.
    pub fn nodes(nodes: impl IntoIterator<Item = u32>) -> Self {
        Self {
            nodes: nodes.into_iter().collect(),
            ..Default::default()
        }
    }

    /// Create a filter for specific event types.
    pub fn types(types: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            types: types.into_iter().map(Into::into).collect(),
            ..Default::default()
        }
    }

    /// Create a filter for specific shards.
    pub fn shards(shards: impl IntoIterator<Item = u32>) -> Self {
        Self {
            shards: shards.into_iter().collect(),
            ..Default::default()
        }
    }

    /// Parse filter from query parameters.
    pub fn from_query_params(
        nodes: Option<&str>,
        types: Option<&str>,
        shards: Option<&str>,
    ) -> Self {
        Self {
            nodes: parse_u32_list(nodes),
            types: parse_string_list(types),
            shards: parse_u32_list(shards),
        }
    }

    /// Check if this filter accepts all events.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.types.is_empty() && self.shards.is_empty()
    }

    /// Check if a single event matches this filter.
    pub fn matches_event(&self, event: &TimestampedEvent) -> bool {
        // Check node filter
        if !self.nodes.is_empty() {
            let event_node = event.event.node_id().unwrap_or(event.node);
            if !self.nodes.contains(&event_node) {
                return false;
            }
        }

        // Check type filter
        if !self.types.is_empty() {
            if !self.types.contains(event.event.event_type()) {
                return false;
            }
        }

        // Check shard filter
        if !self.shards.is_empty() {
            if let Some(shard_id) = event.event.shard_id() {
                if !self.shards.contains(&shard_id) {
                    return false;
                }
            }
            // Events without shard ID pass through if shard filter is set
            // (they might be relevant, like node-level events)
        }

        true
    }

    /// Apply filter to a batch, returning a filtered batch.
    ///
    /// If no events match, returns None.
    pub fn apply(&self, batch: &EventBatch) -> Option<EventBatch> {
        // Fast path: no filters
        if self.is_empty() {
            return Some(batch.clone());
        }

        let filtered_events: Vec<_> = batch
            .events
            .iter()
            .filter(|e| self.matches_event(e))
            .cloned()
            .collect();

        if filtered_events.is_empty() {
            return None;
        }

        Some(EventBatch::new(
            batch.window_start,
            batch.window_end,
            filtered_events,
        ))
    }

    /// Merge another filter into this one (union of constraints).
    pub fn merge(&mut self, other: &SubscriptionFilter) {
        self.nodes.extend(&other.nodes);
        self.types.extend(other.types.iter().cloned());
        self.shards.extend(&other.shards);
    }
}

/// Parse comma-separated u32 list.
fn parse_u32_list(input: Option<&str>) -> HashSet<u32> {
    input
        .map(|s| {
            s.split(',')
                .filter_map(|p| p.trim().parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Parse comma-separated string list.
fn parse_string_list(input: Option<&str>) -> HashSet<String> {
    input
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Control message from WebSocket client.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// Update subscription filter.
    #[serde(rename = "filter")]
    Filter(SubscriptionFilter),
    /// Pause event streaming.
    #[serde(rename = "pause")]
    Pause,
    /// Resume event streaming.
    #[serde(rename = "resume")]
    Resume,
    /// Request history playback.
    #[serde(rename = "history")]
    History { from_ms: u64, to_ms: u64 },
    /// Ping for keepalive.
    #[serde(rename = "ping")]
    Ping,
}

/// Message to WebSocket client.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// Initial connection state.
    #[serde(rename = "connected")]
    Connected {
        /// Server version.
        version: String,
        /// Oldest available timestamp.
        oldest_timestamp_ms: Option<u64>,
        /// Newest timestamp.
        newest_timestamp_ms: Option<u64>,
    },
    /// Event batch (real-time or playback).
    #[serde(rename = "batch")]
    Batch(EventBatch),
    /// Pong response.
    #[serde(rename = "pong")]
    Pong { timestamp_ms: u64 },
    /// Error message.
    #[serde(rename = "error")]
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{WireEvent, WireRaftEvt, WireRaftKind, WireSwimEvt, WireSwimKind};

    fn make_raft_event(node: u32, shard: u32) -> TimestampedEvent {
        TimestampedEvent {
            ts: 1000,
            node,
            event: WireEvent::Raft(WireRaftEvt {
                shard,
                term: 1,
                kind: WireRaftKind::LeaderElected { node },
            }),
        }
    }

    fn make_swim_event(node: u32) -> TimestampedEvent {
        TimestampedEvent {
            ts: 1000,
            node,
            event: WireEvent::Swim(WireSwimEvt {
                node,
                kind: WireSwimKind::Alive,
            }),
        }
    }

    #[test]
    fn test_empty_filter_matches_all() {
        let filter = SubscriptionFilter::all();
        assert!(filter.matches_event(&make_raft_event(1, 1)));
        assert!(filter.matches_event(&make_swim_event(2)));
    }

    #[test]
    fn test_node_filter() {
        let filter = SubscriptionFilter::nodes([1, 2]);
        assert!(filter.matches_event(&make_raft_event(1, 1)));
        assert!(filter.matches_event(&make_raft_event(2, 1)));
        assert!(!filter.matches_event(&make_raft_event(3, 1)));
    }

    #[test]
    fn test_type_filter() {
        let filter = SubscriptionFilter::types(["raft"]);
        assert!(filter.matches_event(&make_raft_event(1, 1)));
        assert!(!filter.matches_event(&make_swim_event(1)));
    }

    #[test]
    fn test_shard_filter() {
        let filter = SubscriptionFilter::shards([1, 2]);
        assert!(filter.matches_event(&make_raft_event(1, 1)));
        assert!(filter.matches_event(&make_raft_event(1, 2)));
        assert!(!filter.matches_event(&make_raft_event(1, 3)));
        // SWIM events don't have shard, should pass through
        assert!(filter.matches_event(&make_swim_event(1)));
    }

    #[test]
    fn test_combined_filters() {
        let filter = SubscriptionFilter {
            nodes: [1].into_iter().collect(),
            types: ["raft"].into_iter().map(String::from).collect(),
            shards: [1].into_iter().collect(),
        };

        assert!(filter.matches_event(&make_raft_event(1, 1)));
        assert!(!filter.matches_event(&make_raft_event(2, 1))); // wrong node
        assert!(!filter.matches_event(&make_raft_event(1, 2))); // wrong shard
        assert!(!filter.matches_event(&make_swim_event(1))); // wrong type
    }

    #[test]
    fn test_parse_query_params() {
        let filter = SubscriptionFilter::from_query_params(
            Some("1,2,3"),
            Some("raft,swim"),
            Some("10,20"),
        );

        assert_eq!(filter.nodes, [1, 2, 3].into_iter().collect());
        assert_eq!(
            filter.types,
            ["raft", "swim"].into_iter().map(String::from).collect()
        );
        assert_eq!(filter.shards, [10, 20].into_iter().collect());
    }

    #[test]
    fn test_apply_filter() {
        let batch = EventBatch::new(
            0,
            50,
            vec![
                make_raft_event(1, 1),
                make_raft_event(2, 1),
                make_swim_event(1),
            ],
        );

        let filter = SubscriptionFilter::types(["raft"]);
        let filtered = filter.apply(&batch).unwrap();

        assert_eq!(filtered.events.len(), 2);
        assert_eq!(filtered.summary.raft_count, 2);
        assert_eq!(filtered.summary.swim_count, 0);
    }
}
