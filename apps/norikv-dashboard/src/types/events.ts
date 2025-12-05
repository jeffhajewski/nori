/**
 * Wire-format event types mirroring vizd WireEvent.
 * These must stay in sync with apps/norikv-vizd/src/event.rs
 */

export interface TimestampedEvent {
  ts: number;
  node: number;
  event: WireEvent;
}

export type WireEvent =
  | { type: "Wal"; data: WireWalEvt }
  | { type: "Compaction"; data: WireCompEvt }
  | { type: "Lsm"; data: WireLsmEvt }
  | { type: "Raft"; data: WireRaftEvt }
  | { type: "Swim"; data: WireSwimEvt }
  | { type: "Shard"; data: WireShardEvt }
  | { type: "Cache"; data: WireCacheEvt }
  | { type: "SlotHeat"; data: WireSlotHeatEvt };

// WAL Events
export interface WireWalEvt {
  node: number;
  seg: number;
  kind: WireWalKind;
}

export type WireWalKind =
  | { kind: "SegmentRoll"; bytes: number }
  | { kind: "Fsync"; ms: number }
  | { kind: "CorruptionTruncated" }
  | { kind: "SegmentGc" };

// Compaction Events
export interface WireCompEvt {
  node: number;
  level: number;
  kind: WireCompKind;
}

export type WireCompKind =
  | { kind: "Scheduled" }
  | { kind: "Start" }
  | { kind: "Progress"; pct: number }
  | { kind: "Finish"; in_bytes: number; out_bytes: number }
  | {
      kind: "BanditSelection";
      slot_id: number;
      explored: boolean;
      ucb_score: number;
      avg_reward: number;
      selection_count: number;
    }
  | {
      kind: "BanditReward";
      slot_id: number;
      reward: number;
      bytes_written: number;
      heat_score: number;
    }
  | {
      kind: "GuardRebalance";
      old_guard_count: number;
      new_guard_count: number;
      total_files: number;
      imbalance_ratio: number;
    };

// LSM Events
export interface WireLsmEvt {
  node: number;
  kind: WireLsmKind;
}

export type WireLsmKind =
  | { kind: "FlushTriggered"; reason: WireFlushReason; memtable_bytes: number }
  | { kind: "FlushStart"; seqno_min: number; seqno_max: number }
  | { kind: "FlushComplete"; file_number: number; bytes: number; entries: number }
  | { kind: "L0Stall"; file_count: number; threshold: number }
  | { kind: "L0AdmissionStart"; file_number: number }
  | { kind: "L0AdmissionComplete"; file_number: number; target_slot: number }
  | { kind: "GuardAdjustment"; level: number; new_guard_count: number }
  | { kind: "WritePressureUpdate"; ratio: number; high: boolean; threshold: number };

export type WireFlushReason = "MemtableSize" | "WalAge" | "Manual";

// Raft Events
export interface WireRaftEvt {
  shard: number;
  term: number;
  kind: WireRaftKind;
}

export type WireRaftKind =
  | { kind: "VoteReq"; from: number }
  | { kind: "VoteGranted"; from: number }
  | { kind: "LeaderElected"; node: number }
  | { kind: "StepDown" };

// SWIM Events
export interface WireSwimEvt {
  node: number;
  kind: WireSwimKind;
}

export type WireSwimKind = "Alive" | "Suspect" | "Confirm" | "Leave";

// Shard Events
export interface WireShardEvt {
  shard: number;
  kind: WireShardKind;
}

export type WireShardKind = "Plan" | "SnapshotStart" | "SnapshotDone" | "Cutover";

// Cache Events
export interface WireCacheEvt {
  name: string;
  hit_ratio: number;
}

// SlotHeat Events
export interface WireSlotHeatEvt {
  node: number;
  level: number;
  slot: number;
  heat: number;
  k: number;
}

// Batch types
export interface EventBatch {
  window_start: number;
  window_end: number;
  events: TimestampedEvent[];
  summary: BatchSummary;
}

export interface BatchSummary {
  wal_count: number;
  compaction_count: number;
  lsm_count: number;
  raft_count: number;
  swim_count: number;
  shard_count: number;
  cache_count: number;
  heat_count: number;
}

// Server messages
export type ServerMessage =
  | { type: "connected"; version: string; oldest_timestamp_ms: number | null; newest_timestamp_ms: number | null }
  | { type: "batch"; data: EventBatch }
  | { type: "pong"; timestamp_ms: number }
  | { type: "error"; message: string };

// Client messages
export type ClientMessage =
  | { type: "filter"; nodes?: number[]; types?: string[]; shards?: number[] }
  | { type: "pause" }
  | { type: "resume" }
  | { type: "history"; from_ms: number; to_ms: number }
  | { type: "ping" };

// Helper to get event type string
export function getEventType(event: WireEvent): string {
  return event.type.toLowerCase();
}

// Helper to get node from event
export function getEventNode(event: WireEvent): number | null {
  switch (event.type) {
    case "Wal":
      return event.data.node;
    case "Compaction":
      return event.data.node;
    case "Lsm":
      return event.data.node;
    case "Swim":
      return event.data.node;
    case "SlotHeat":
      return event.data.node;
    case "Raft":
      if (event.data.kind.kind === "LeaderElected") {
        return event.data.kind.node;
      }
      return null;
    default:
      return null;
  }
}

// Helper to get shard from event
export function getEventShard(event: WireEvent): number | null {
  switch (event.type) {
    case "Raft":
      return event.data.shard;
    case "Shard":
      return event.data.shard;
    default:
      return null;
  }
}
