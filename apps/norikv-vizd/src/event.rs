//! Wire-format event types for dashboard streaming.
//!
//! These types mirror `nori_observe::VizEvent` but with Serde derives
//! for JSON/MessagePack serialization over WebSocket.

use nori_observe::{
    CacheEvt, CompEvt, CompKind, FlushReason, LsmEvt, LsmKind, RaftEvt, RaftKind, ShardEvt,
    ShardKind, SlotHeatEvt, SwimEvt, SwimKind, VizEvent, WalEvt, WalKind,
};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Timestamped event wrapper for the ring buffer and wire transmission.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimestampedEvent {
    /// Unix timestamp in milliseconds
    pub ts: u64,
    /// Source node ID
    pub node: u32,
    /// The event payload
    pub event: WireEvent,
}

impl TimestampedEvent {
    pub fn new(node: u32, event: WireEvent) -> Self {
        Self {
            ts: current_timestamp_ms(),
            node,
            event,
        }
    }
}

/// Wire format for VizEvent (serializable version).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WireEvent {
    Wal(WireWalEvt),
    Compaction(WireCompEvt),
    Lsm(WireLsmEvt),
    Raft(WireRaftEvt),
    Swim(WireSwimEvt),
    Shard(WireShardEvt),
    Cache(WireCacheEvt),
    SlotHeat(WireSlotHeatEvt),
}

// ============================================================================
// WAL Events
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireWalEvt {
    pub node: u32,
    pub seg: u64,
    pub kind: WireWalKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum WireWalKind {
    SegmentRoll { bytes: u64 },
    Fsync { ms: u32 },
    CorruptionTruncated,
    SegmentGc,
}

// ============================================================================
// Compaction Events
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireCompEvt {
    pub node: u32,
    pub level: u8,
    pub kind: WireCompKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum WireCompKind {
    Scheduled,
    Start,
    Progress { pct: u8 },
    Finish { in_bytes: u64, out_bytes: u64 },
    BanditSelection {
        slot_id: u32,
        explored: bool,
        ucb_score: f64,
        avg_reward: f64,
        selection_count: u64,
    },
    BanditReward {
        slot_id: u32,
        reward: f64,
        bytes_written: u64,
        heat_score: f32,
    },
    GuardRebalance {
        old_guard_count: usize,
        new_guard_count: usize,
        total_files: usize,
        imbalance_ratio: f64,
    },
}

// ============================================================================
// LSM Events
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireLsmEvt {
    pub node: u32,
    pub kind: WireLsmKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum WireLsmKind {
    FlushTriggered {
        reason: WireFlushReason,
        memtable_bytes: usize,
    },
    FlushStart {
        seqno_min: u64,
        seqno_max: u64,
    },
    FlushComplete {
        file_number: u64,
        bytes: u64,
        entries: usize,
    },
    L0Stall {
        file_count: usize,
        threshold: usize,
    },
    L0AdmissionStart {
        file_number: u64,
    },
    L0AdmissionComplete {
        file_number: u64,
        target_slot: u32,
    },
    GuardAdjustment {
        level: u8,
        new_guard_count: usize,
    },
    WritePressureUpdate {
        ratio: f32,
        high: bool,
        threshold: f32,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WireFlushReason {
    MemtableSize,
    WalAge,
    Manual,
}

// ============================================================================
// Raft Events
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireRaftEvt {
    pub shard: u32,
    pub term: u64,
    pub kind: WireRaftKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum WireRaftKind {
    VoteReq { from: u32 },
    VoteGranted { from: u32 },
    LeaderElected { node: u32 },
    StepDown,
}

// ============================================================================
// SWIM Events
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireSwimEvt {
    pub node: u32,
    pub kind: WireSwimKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WireSwimKind {
    Alive,
    Suspect,
    Confirm,
    Leave,
}

// ============================================================================
// Shard Events
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireShardEvt {
    pub shard: u32,
    pub kind: WireShardKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum WireShardKind {
    Plan,
    SnapshotStart,
    SnapshotDone,
    Cutover,
}

// ============================================================================
// Cache Events
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireCacheEvt {
    pub name: String,
    pub hit_ratio: f32,
}

// ============================================================================
// SlotHeat Events
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireSlotHeatEvt {
    pub node: u32,
    pub level: u8,
    pub slot: u32,
    pub heat: f32,
    pub k: u8,
}

// ============================================================================
// Conversions from nori_observe types
// ============================================================================

impl From<VizEvent> for WireEvent {
    fn from(evt: VizEvent) -> Self {
        match evt {
            VizEvent::Wal(w) => WireEvent::Wal(w.into()),
            VizEvent::Compaction(c) => WireEvent::Compaction(c.into()),
            VizEvent::Lsm(l) => WireEvent::Lsm(l.into()),
            VizEvent::Raft(r) => WireEvent::Raft(r.into()),
            VizEvent::Swim(s) => WireEvent::Swim(s.into()),
            VizEvent::Shard(s) => WireEvent::Shard(s.into()),
            VizEvent::Cache(c) => WireEvent::Cache(c.into()),
            VizEvent::SlotHeat(h) => WireEvent::SlotHeat(h.into()),
            // Handle future VizEvent variants (non_exhaustive)
            _ => WireEvent::Cache(WireCacheEvt {
                name: "unknown".to_string(),
                hit_ratio: 0.0,
            }),
        }
    }
}

impl From<WalEvt> for WireWalEvt {
    fn from(evt: WalEvt) -> Self {
        Self {
            node: evt.node,
            seg: evt.seg,
            kind: evt.kind.into(),
        }
    }
}

impl From<WalKind> for WireWalKind {
    fn from(kind: WalKind) -> Self {
        match kind {
            WalKind::SegmentRoll { bytes } => WireWalKind::SegmentRoll { bytes },
            WalKind::Fsync { ms } => WireWalKind::Fsync { ms },
            WalKind::CorruptionTruncated => WireWalKind::CorruptionTruncated,
            WalKind::SegmentGc => WireWalKind::SegmentGc,
        }
    }
}

impl From<CompEvt> for WireCompEvt {
    fn from(evt: CompEvt) -> Self {
        Self {
            node: evt.node,
            level: evt.level,
            kind: evt.kind.into(),
        }
    }
}

impl From<CompKind> for WireCompKind {
    fn from(kind: CompKind) -> Self {
        match kind {
            CompKind::Scheduled => WireCompKind::Scheduled,
            CompKind::Start => WireCompKind::Start,
            CompKind::Progress { pct } => WireCompKind::Progress { pct },
            CompKind::Finish { in_bytes, out_bytes } => {
                WireCompKind::Finish { in_bytes, out_bytes }
            }
            CompKind::BanditSelection {
                slot_id,
                explored,
                ucb_score,
                avg_reward,
                selection_count,
            } => WireCompKind::BanditSelection {
                slot_id,
                explored,
                ucb_score,
                avg_reward,
                selection_count,
            },
            CompKind::BanditReward {
                slot_id,
                reward,
                bytes_written,
                heat_score,
            } => WireCompKind::BanditReward {
                slot_id,
                reward,
                bytes_written,
                heat_score,
            },
            CompKind::GuardRebalance {
                old_guard_count,
                new_guard_count,
                total_files,
                imbalance_ratio,
            } => WireCompKind::GuardRebalance {
                old_guard_count,
                new_guard_count,
                total_files,
                imbalance_ratio,
            },
        }
    }
}

impl From<LsmEvt> for WireLsmEvt {
    fn from(evt: LsmEvt) -> Self {
        Self {
            node: evt.node,
            kind: evt.kind.into(),
        }
    }
}

impl From<LsmKind> for WireLsmKind {
    fn from(kind: LsmKind) -> Self {
        match kind {
            LsmKind::FlushTriggered {
                reason,
                memtable_bytes,
            } => WireLsmKind::FlushTriggered {
                reason: reason.into(),
                memtable_bytes,
            },
            LsmKind::FlushStart {
                seqno_min,
                seqno_max,
            } => WireLsmKind::FlushStart {
                seqno_min,
                seqno_max,
            },
            LsmKind::FlushComplete {
                file_number,
                bytes,
                entries,
            } => WireLsmKind::FlushComplete {
                file_number,
                bytes,
                entries,
            },
            LsmKind::L0Stall {
                file_count,
                threshold,
            } => WireLsmKind::L0Stall {
                file_count,
                threshold,
            },
            LsmKind::L0AdmissionStart { file_number } => {
                WireLsmKind::L0AdmissionStart { file_number }
            }
            LsmKind::L0AdmissionComplete {
                file_number,
                target_slot,
            } => WireLsmKind::L0AdmissionComplete {
                file_number,
                target_slot,
            },
            LsmKind::GuardAdjustment {
                level,
                new_guard_count,
            } => WireLsmKind::GuardAdjustment {
                level,
                new_guard_count,
            },
            LsmKind::WritePressureUpdate {
                ratio,
                high,
                threshold,
            } => WireLsmKind::WritePressureUpdate {
                ratio,
                high,
                threshold,
            },
        }
    }
}

impl From<FlushReason> for WireFlushReason {
    fn from(reason: FlushReason) -> Self {
        match reason {
            FlushReason::MemtableSize => WireFlushReason::MemtableSize,
            FlushReason::WalAge => WireFlushReason::WalAge,
            FlushReason::Manual => WireFlushReason::Manual,
        }
    }
}

impl From<RaftEvt> for WireRaftEvt {
    fn from(evt: RaftEvt) -> Self {
        Self {
            shard: evt.shard,
            term: evt.term,
            kind: evt.kind.into(),
        }
    }
}

impl From<RaftKind> for WireRaftKind {
    fn from(kind: RaftKind) -> Self {
        match kind {
            RaftKind::VoteReq { from } => WireRaftKind::VoteReq { from },
            RaftKind::VoteGranted { from } => WireRaftKind::VoteGranted { from },
            RaftKind::LeaderElected { node } => WireRaftKind::LeaderElected { node },
            RaftKind::StepDown => WireRaftKind::StepDown,
        }
    }
}

impl From<SwimEvt> for WireSwimEvt {
    fn from(evt: SwimEvt) -> Self {
        Self {
            node: evt.node,
            kind: evt.kind.into(),
        }
    }
}

impl From<SwimKind> for WireSwimKind {
    fn from(kind: SwimKind) -> Self {
        match kind {
            SwimKind::Alive => WireSwimKind::Alive,
            SwimKind::Suspect => WireSwimKind::Suspect,
            SwimKind::Confirm => WireSwimKind::Confirm,
            SwimKind::Leave => WireSwimKind::Leave,
        }
    }
}

impl From<ShardEvt> for WireShardEvt {
    fn from(evt: ShardEvt) -> Self {
        Self {
            shard: evt.shard,
            kind: evt.kind.into(),
        }
    }
}

impl From<ShardKind> for WireShardKind {
    fn from(kind: ShardKind) -> Self {
        match kind {
            ShardKind::Plan => WireShardKind::Plan,
            ShardKind::SnapshotStart => WireShardKind::SnapshotStart,
            ShardKind::SnapshotDone => WireShardKind::SnapshotDone,
            ShardKind::Cutover => WireShardKind::Cutover,
        }
    }
}

impl From<CacheEvt> for WireCacheEvt {
    fn from(evt: CacheEvt) -> Self {
        Self {
            name: evt.name.to_string(),
            hit_ratio: evt.hit_ratio,
        }
    }
}

impl From<SlotHeatEvt> for WireSlotHeatEvt {
    fn from(evt: SlotHeatEvt) -> Self {
        Self {
            node: evt.node,
            level: evt.level,
            slot: evt.slot,
            heat: evt.heat,
            k: evt.k,
        }
    }
}

// ============================================================================
// Event Batch (coalesced window)
// ============================================================================

/// A batch of events from a single 50ms coalesce window.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventBatch {
    /// Window start timestamp (ms)
    pub window_start: u64,
    /// Window end timestamp (ms)
    pub window_end: u64,
    /// Events in this window
    pub events: Vec<TimestampedEvent>,
    /// Summary statistics for quick filtering
    pub summary: BatchSummary,
}

impl EventBatch {
    pub fn new(window_start: u64, window_end: u64, events: Vec<TimestampedEvent>) -> Self {
        let summary = BatchSummary::from_events(&events);
        Self {
            window_start,
            window_end,
            events,
            summary,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Summary statistics for efficient client-side filtering.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BatchSummary {
    pub wal_count: u32,
    pub compaction_count: u32,
    pub lsm_count: u32,
    pub raft_count: u32,
    pub swim_count: u32,
    pub shard_count: u32,
    pub cache_count: u32,
    pub heat_count: u32,
}

impl BatchSummary {
    pub fn from_events(events: &[TimestampedEvent]) -> Self {
        let mut summary = Self::default();
        for evt in events {
            match &evt.event {
                WireEvent::Wal(_) => summary.wal_count += 1,
                WireEvent::Compaction(_) => summary.compaction_count += 1,
                WireEvent::Lsm(_) => summary.lsm_count += 1,
                WireEvent::Raft(_) => summary.raft_count += 1,
                WireEvent::Swim(_) => summary.swim_count += 1,
                WireEvent::Shard(_) => summary.shard_count += 1,
                WireEvent::Cache(_) => summary.cache_count += 1,
                WireEvent::SlotHeat(_) => summary.heat_count += 1,
            }
        }
        summary
    }
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Get current timestamp in milliseconds since Unix epoch.
pub fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Get event type as a string for filtering.
impl WireEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            WireEvent::Wal(_) => "wal",
            WireEvent::Compaction(_) => "compaction",
            WireEvent::Lsm(_) => "lsm",
            WireEvent::Raft(_) => "raft",
            WireEvent::Swim(_) => "swim",
            WireEvent::Shard(_) => "shard",
            WireEvent::Cache(_) => "cache",
            WireEvent::SlotHeat(_) => "heat",
        }
    }

    /// Extract shard ID if this event is shard-related.
    pub fn shard_id(&self) -> Option<u32> {
        match self {
            WireEvent::Raft(r) => Some(r.shard),
            WireEvent::Shard(s) => Some(s.shard),
            _ => None,
        }
    }

    /// Extract node ID from event.
    pub fn node_id(&self) -> Option<u32> {
        match self {
            WireEvent::Wal(w) => Some(w.node),
            WireEvent::Compaction(c) => Some(c.node),
            WireEvent::Lsm(l) => Some(l.node),
            WireEvent::Swim(s) => Some(s.node),
            WireEvent::SlotHeat(h) => Some(h.node),
            WireEvent::Raft(r) => match &r.kind {
                WireRaftKind::LeaderElected { node } => Some(*node),
                WireRaftKind::VoteReq { from } | WireRaftKind::VoteGranted { from } => Some(*from),
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wire_event_serialization() {
        let evt = WireEvent::Raft(WireRaftEvt {
            shard: 42,
            term: 5,
            kind: WireRaftKind::LeaderElected { node: 1 },
        });

        let json = serde_json::to_string(&evt).unwrap();
        assert!(json.contains("LeaderElected"));
        assert!(json.contains("\"node\":1"));

        let decoded: WireEvent = serde_json::from_str(&json).unwrap();
        if let WireEvent::Raft(r) = decoded {
            assert_eq!(r.shard, 42);
            assert_eq!(r.term, 5);
        } else {
            panic!("Expected Raft event");
        }
    }

    #[test]
    fn test_batch_summary() {
        let events = vec![
            TimestampedEvent::new(1, WireEvent::Raft(WireRaftEvt {
                shard: 1,
                term: 1,
                kind: WireRaftKind::LeaderElected { node: 1 },
            })),
            TimestampedEvent::new(1, WireEvent::Raft(WireRaftEvt {
                shard: 2,
                term: 1,
                kind: WireRaftKind::VoteReq { from: 2 },
            })),
            TimestampedEvent::new(2, WireEvent::Swim(WireSwimEvt {
                node: 2,
                kind: WireSwimKind::Alive,
            })),
        ];

        let summary = BatchSummary::from_events(&events);
        assert_eq!(summary.raft_count, 2);
        assert_eq!(summary.swim_count, 1);
        assert_eq!(summary.wal_count, 0);
    }
}
