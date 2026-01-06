import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";
import { immer } from "zustand/middleware/immer";
import type {
  EventBatch,
  TimestampedEvent,
  WireRaftEvt,
  WireSwimEvt,
  WireSlotHeatEvt,
  WireLsmEvt,
  WireCompEvt,
  WireWalEvt,
  WireCacheEvt,
  WireShardEvt,
} from "@/types/events";
import {
  TimeSeriesBuffer,
  HistogramBuffer,
  type TimeSeriesPoint,
} from "@/lib/timeSeriesBuffer";

// ============================================================================
// Derived State Types
// ============================================================================

export interface StateTransition {
  state: "alive" | "suspect" | "dead";
  timestamp: number;
}

export interface ClusterNode {
  id: number;
  status: "alive" | "suspect" | "dead";
  lastSeen: number;
  // Enhanced tracking
  stateHistory: StateTransition[];
  suspectedBy: Set<number>;
}

export interface TermHistoryEntry {
  term: number;
  timestamp: number;
  leader: number | null;
}

export interface ShardState {
  id: number;
  leader: number | null;
  term: number;
  replicas: Set<number>;
  // Enhanced tracking
  lastElectionTs: number;
  electionCount: number;
  termHistory: TermHistoryEntry[];
}

export interface SlotState {
  slotId: number;
  heat: number;
  k: number;
  lastUpdate: number;
}

export interface LevelState {
  level: number;
  slots: Map<number, SlotState>;
}

export interface LsmNodeState {
  nodeId: number;
  levels: Map<number, LevelState>;
  writePressure: number;
  writePressureHigh: boolean;
  l0FileCount: number;
  l0Threshold: number;
}

// ============================================================================
// Time-Series History Types
// ============================================================================

export interface WritePressurePoint {
  ratio: number;
  high: boolean;
}

export interface CompactionRecord {
  timestamp: number;
  level: number;
  inBytes: number;
  outBytes: number;
  ratio: number;
}

export interface BanditSelection {
  timestamp: number;
  slotId: number;
  explored: boolean;
  ucbScore: number;
  avgReward: number;
  selectionCount: number;
}

export interface BanditReward {
  timestamp: number;
  slotId: number;
  reward: number;
  bytesWritten: number;
  heatScore: number;
}

export interface BanditState {
  selections: BanditSelection[];
  rewards: BanditReward[];
}

export interface ShardSnapshotState {
  shardId: number;
  snapshotId: string | null;
  status: "idle" | "in_progress" | "completed";
  startedAt: number | null;
  completedAt: number | null;
  bytes: number | null;
}

export interface SwimConnectionEvent {
  from: number;
  to: number;
  kind: "Suspect" | "Confirm" | "Alive";
  timestamp: number;
}

// ============================================================================
// Store State
// ============================================================================

export type WsStatus = "disconnected" | "connecting" | "connected" | "reconnecting";

// Time-series buffers are stored outside Zustand to avoid serialization issues
// They are keyed by identifiers and accessed via selectors
const writePressureHistories = new Map<number, TimeSeriesBuffer<WritePressurePoint>>();
const slotHeatHistories = new Map<string, TimeSeriesBuffer<number>>();
const fsyncLatencyHistories = new Map<number, HistogramBuffer>();
const cacheHitRatioHistories = new Map<string, TimeSeriesBuffer<number>>();

// Fsync latency bucket boundaries in ms
const FSYNC_BUCKETS = [1, 2, 5, 10, 20, 50, 100, 200, 500];

interface EventState {
  // Connection state
  wsStatus: WsStatus;
  serverVersion: string | null;

  // Cluster topology
  nodes: Map<number, ClusterNode>;
  shards: Map<number, ShardState>;

  // Storage state (per-node)
  lsmStates: Map<number, LsmNodeState>;

  // Event buffer for playback (ring buffer)
  eventBuffer: EventBatch[];
  bufferMaxSize: number;

  // Playback state
  isLive: boolean;
  playbackPosition: number;
  playbackSpeed: number;

  // Stats
  totalEventsReceived: number;
  lastEventTimestamp: number;

  // Enhanced tracking
  compactionEfficiency: Map<string, CompactionRecord[]>; // key: `${node}-${level}`
  banditStates: Map<string, BanditState>; // key: `${node}-${level}`
  shardSnapshots: Map<number, ShardSnapshotState>;
  swimConnectionEvents: SwimConnectionEvent[];

  // Actions
  setWsStatus: (status: WsStatus) => void;
  setServerVersion: (version: string) => void;
  ingestBatch: (batch: EventBatch) => void;
  setPlaybackPosition: (posMs: number) => void;
  setPlaybackSpeed: (speed: number) => void;
  toggleLive: () => void;
  reset: () => void;
}

// ============================================================================
// Initial State
// ============================================================================

const BUFFER_MAX_SIZE = 72000; // 60 minutes at 20 batches/sec
const MAX_TERM_HISTORY = 10;
const MAX_STATE_HISTORY = 20;
const MAX_COMPACTION_RECORDS = 100;
const MAX_BANDIT_RECORDS = 50;
const MAX_SWIM_CONNECTION_EVENTS = 50;

const initialState = {
  wsStatus: "disconnected" as WsStatus,
  serverVersion: null,
  nodes: new Map<number, ClusterNode>(),
  shards: new Map<number, ShardState>(),
  lsmStates: new Map<number, LsmNodeState>(),
  eventBuffer: [] as EventBatch[],
  bufferMaxSize: BUFFER_MAX_SIZE,
  isLive: true,
  playbackPosition: Date.now(),
  playbackSpeed: 1,
  totalEventsReceived: 0,
  lastEventTimestamp: 0,
  compactionEfficiency: new Map<string, CompactionRecord[]>(),
  banditStates: new Map<string, BanditState>(),
  shardSnapshots: new Map<number, ShardSnapshotState>(),
  swimConnectionEvents: [] as SwimConnectionEvent[],
};

// ============================================================================
// Store
// ============================================================================

export const useEventStore = create<EventState>()(
  subscribeWithSelector(
    immer((set, get) => ({
      ...initialState,

      setWsStatus: (status) =>
        set((state) => {
          state.wsStatus = status;
        }),

      setServerVersion: (version) =>
        set((state) => {
          state.serverVersion = version;
        }),

      ingestBatch: (batch) =>
        set((state) => {
          // Append to ring buffer
          state.eventBuffer.push(batch);
          if (state.eventBuffer.length > state.bufferMaxSize) {
            state.eventBuffer.shift();
          }

          // Update stats
          state.totalEventsReceived += batch.events.length;
          state.lastEventTimestamp = batch.window_end;

          // Apply events to derive current state
          for (const evt of batch.events) {
            applyEventToState(state, evt);
          }
        }),

      setPlaybackPosition: (posMs) =>
        set((state) => {
          state.playbackPosition = posMs;
          state.isLive = false;
        }),

      setPlaybackSpeed: (speed) =>
        set((state) => {
          state.playbackSpeed = speed;
        }),

      toggleLive: () =>
        set((state) => {
          state.isLive = !state.isLive;
          if (state.isLive) {
            state.playbackPosition = Date.now();
          }
        }),

      reset: () =>
        set((state) => {
          Object.assign(state, initialState);
          // Clear time-series buffers
          writePressureHistories.clear();
          slotHeatHistories.clear();
          fsyncLatencyHistories.clear();
          cacheHitRatioHistories.clear();
        }),
    }))
  )
);

// ============================================================================
// Event Application Logic
// ============================================================================

function applyEventToState(state: EventState, evt: TimestampedEvent) {
  const { event, ts } = evt;

  switch (event.type) {
    case "Swim":
      applySwimEvent(state, event.data, ts);
      break;
    case "Raft":
      applyRaftEvent(state, event.data, ts);
      break;
    case "SlotHeat":
      applySlotHeatEvent(state, event.data, ts);
      break;
    case "Lsm":
      applyLsmEvent(state, event.data, ts);
      break;
    case "Compaction":
      applyCompactionEvent(state, event.data, ts);
      break;
    case "Wal":
      applyWalEvent(state, event.data, ts);
      break;
    case "Cache":
      applyCacheEvent(state, event.data, ts);
      break;
    case "Shard":
      applyShardEvent(state, event.data, ts);
      break;
  }
}

function applySwimEvent(state: EventState, evt: WireSwimEvt, timestamp: number) {
  const existing = state.nodes.get(evt.node);
  const node: ClusterNode = existing || {
    id: evt.node,
    status: "alive",
    lastSeen: timestamp,
    stateHistory: [],
    suspectedBy: new Set(),
  };

  const oldStatus = node.status;
  let newStatus: "alive" | "suspect" | "dead" = node.status;

  switch (evt.kind) {
    case "Alive":
      newStatus = "alive";
      node.lastSeen = timestamp;
      node.suspectedBy.clear();
      break;
    case "Suspect":
      newStatus = "suspect";
      // Track who suspected this node (from evt if available)
      break;
    case "Confirm":
    case "Leave":
      newStatus = "dead";
      break;
  }

  // Record state transition if changed
  if (oldStatus !== newStatus) {
    node.stateHistory.push({ state: newStatus, timestamp });
    if (node.stateHistory.length > MAX_STATE_HISTORY) {
      node.stateHistory.shift();
    }
  }

  node.status = newStatus;
  state.nodes.set(evt.node, node);

  // Track connection events for topology visualization
  if (evt.kind === "Suspect" || evt.kind === "Confirm" || evt.kind === "Alive") {
    state.swimConnectionEvents.push({
      from: 0, // Source node not always available in current event format
      to: evt.node,
      kind: evt.kind as "Suspect" | "Confirm" | "Alive",
      timestamp,
    });
    if (state.swimConnectionEvents.length > MAX_SWIM_CONNECTION_EVENTS) {
      state.swimConnectionEvents.shift();
    }
  }
}

function applyRaftEvent(state: EventState, evt: WireRaftEvt, timestamp: number) {
  const existing = state.shards.get(evt.shard);
  const shard: ShardState = existing || {
    id: evt.shard,
    leader: null,
    term: 0,
    replicas: new Set(),
    lastElectionTs: 0,
    electionCount: 0,
    termHistory: [],
  };

  // Update term
  const termChanged = evt.term > shard.term;
  if (termChanged) {
    shard.term = evt.term;
  }

  // Handle different Raft events
  switch (evt.kind.kind) {
    case "LeaderElected":
      shard.leader = evt.kind.node;
      shard.replicas.add(evt.kind.node);
      shard.lastElectionTs = timestamp;
      shard.electionCount++;
      // Record term history
      shard.termHistory.push({
        term: evt.term,
        timestamp,
        leader: evt.kind.node,
      });
      if (shard.termHistory.length > MAX_TERM_HISTORY) {
        shard.termHistory.shift();
      }
      break;
    case "VoteReq":
    case "VoteGranted":
      shard.replicas.add(evt.kind.from);
      break;
    case "StepDown":
      shard.leader = null;
      // Record step down in term history
      shard.termHistory.push({
        term: evt.term,
        timestamp,
        leader: null,
      });
      if (shard.termHistory.length > MAX_TERM_HISTORY) {
        shard.termHistory.shift();
      }
      break;
  }

  state.shards.set(evt.shard, shard);
}

function applySlotHeatEvent(state: EventState, evt: WireSlotHeatEvt, timestamp: number) {
  let nodeState = state.lsmStates.get(evt.node);
  if (!nodeState) {
    nodeState = {
      nodeId: evt.node,
      levels: new Map(),
      writePressure: 0,
      writePressureHigh: false,
      l0FileCount: 0,
      l0Threshold: 12,
    };
    state.lsmStates.set(evt.node, nodeState);
  }

  let levelState = nodeState.levels.get(evt.level);
  if (!levelState) {
    levelState = {
      level: evt.level,
      slots: new Map(),
    };
    nodeState.levels.set(evt.level, levelState);
  }

  levelState.slots.set(evt.slot, {
    slotId: evt.slot,
    heat: evt.heat,
    k: evt.k,
    lastUpdate: timestamp,
  });

  // Track slot heat history
  const historyKey = `${evt.node}-${evt.level}-${evt.slot}`;
  let heatHistory = slotHeatHistories.get(historyKey);
  if (!heatHistory) {
    heatHistory = new TimeSeriesBuffer<number>({ maxPoints: 120, aggregator: "avg" });
    slotHeatHistories.set(historyKey, heatHistory);
  }
  heatHistory.push(timestamp, evt.heat);
}

function applyLsmEvent(state: EventState, evt: WireLsmEvt, timestamp: number) {
  let nodeState = state.lsmStates.get(evt.node);
  if (!nodeState) {
    nodeState = {
      nodeId: evt.node,
      levels: new Map(),
      writePressure: 0,
      writePressureHigh: false,
      l0FileCount: 0,
      l0Threshold: 12,
    };
    state.lsmStates.set(evt.node, nodeState);
  }

  switch (evt.kind.kind) {
    case "WritePressureUpdate":
      nodeState.writePressure = evt.kind.ratio;
      nodeState.writePressureHigh = evt.kind.high;

      // Track write pressure history
      let pressureHistory = writePressureHistories.get(evt.node);
      if (!pressureHistory) {
        pressureHistory = new TimeSeriesBuffer<WritePressurePoint>({
          maxPoints: 720,
          aggregator: "last",
        });
        writePressureHistories.set(evt.node, pressureHistory);
      }
      pressureHistory.push(timestamp, {
        ratio: evt.kind.ratio,
        high: evt.kind.high,
      });
      break;
    case "L0Stall":
      nodeState.l0FileCount = evt.kind.file_count;
      nodeState.l0Threshold = evt.kind.threshold;
      break;
  }
}

function applyCompactionEvent(state: EventState, evt: WireCompEvt, timestamp: number) {
  const key = `${evt.node}-${evt.level}`;

  switch (evt.kind.kind) {
    case "Finish": {
      // Track compaction efficiency
      let records = state.compactionEfficiency.get(key);
      if (!records) {
        records = [];
        state.compactionEfficiency.set(key, records);
      }
      const ratio = evt.kind.out_bytes > 0 ? evt.kind.in_bytes / evt.kind.out_bytes : 1;
      records.push({
        timestamp,
        level: evt.level,
        inBytes: evt.kind.in_bytes,
        outBytes: evt.kind.out_bytes,
        ratio,
      });
      if (records.length > MAX_COMPACTION_RECORDS) {
        records.shift();
      }
      break;
    }
    case "BanditSelection": {
      let bandit = state.banditStates.get(key);
      if (!bandit) {
        bandit = { selections: [], rewards: [] };
        state.banditStates.set(key, bandit);
      }
      bandit.selections.push({
        timestamp,
        slotId: evt.kind.slot_id,
        explored: evt.kind.explored,
        ucbScore: evt.kind.ucb_score,
        avgReward: evt.kind.avg_reward,
        selectionCount: evt.kind.selection_count,
      });
      if (bandit.selections.length > MAX_BANDIT_RECORDS) {
        bandit.selections.shift();
      }
      break;
    }
    case "BanditReward": {
      let bandit = state.banditStates.get(key);
      if (!bandit) {
        bandit = { selections: [], rewards: [] };
        state.banditStates.set(key, bandit);
      }
      bandit.rewards.push({
        timestamp,
        slotId: evt.kind.slot_id,
        reward: evt.kind.reward,
        bytesWritten: evt.kind.bytes_written,
        heatScore: evt.kind.heat_score,
      });
      if (bandit.rewards.length > MAX_BANDIT_RECORDS) {
        bandit.rewards.shift();
      }
      break;
    }
  }
}

function applyWalEvent(state: EventState, evt: WireWalEvt, timestamp: number) {
  if (evt.kind.kind === "Fsync") {
    // Track fsync latency history
    let fsyncHistory = fsyncLatencyHistories.get(evt.node);
    if (!fsyncHistory) {
      fsyncHistory = new HistogramBuffer(FSYNC_BUCKETS, {
        maxPoints: 720,
        aggregator: "avg",
      });
      fsyncLatencyHistories.set(evt.node, fsyncHistory);
    }
    fsyncHistory.push(timestamp, evt.kind.ms);
  }
}

function applyCacheEvent(state: EventState, evt: WireCacheEvt, timestamp: number) {
  // Track cache hit ratio history
  let cacheHistory = cacheHitRatioHistories.get(evt.name);
  if (!cacheHistory) {
    cacheHistory = new TimeSeriesBuffer<number>({
      maxPoints: 720,
      aggregator: "avg",
    });
    cacheHitRatioHistories.set(evt.name, cacheHistory);
  }
  cacheHistory.push(timestamp, evt.hit_ratio);
}

function applyShardEvent(state: EventState, evt: WireShardEvt, timestamp: number) {
  let snapshot = state.shardSnapshots.get(evt.shard);
  if (!snapshot) {
    snapshot = {
      shardId: evt.shard,
      snapshotId: null,
      status: "idle",
      startedAt: null,
      completedAt: null,
      bytes: null,
    };
  }

  switch (evt.kind) {
    case "SnapshotStart":
      snapshot.status = "in_progress";
      snapshot.startedAt = timestamp;
      snapshot.completedAt = null;
      break;
    case "SnapshotDone":
      snapshot.status = "completed";
      snapshot.completedAt = timestamp;
      break;
    case "Cutover":
      // Reset snapshot state on cutover
      snapshot.status = "idle";
      snapshot.snapshotId = null;
      snapshot.startedAt = null;
      snapshot.completedAt = null;
      break;
  }

  state.shardSnapshots.set(evt.shard, snapshot);
}

// ============================================================================
// Selectors
// ============================================================================

export const useClusterHealth = () =>
  useEventStore((s) => {
    const nodes = Array.from(s.nodes.values());
    const alive = nodes.filter((n) => n.status === "alive").length;
    const suspect = nodes.filter((n) => n.status === "suspect").length;
    const dead = nodes.filter((n) => n.status === "dead").length;
    return {
      total: nodes.length,
      alive,
      suspect,
      dead,
      healthy: dead === 0 && suspect === 0 && nodes.length > 0,
    };
  });

export const useShardLeader = (shardId: number) =>
  useEventStore((s) => s.shards.get(shardId)?.leader ?? null);

export const useNodeStatus = (nodeId: number) =>
  useEventStore((s) => s.nodes.get(nodeId)?.status ?? "alive");

export const useLsmNodeState = (nodeId: number) =>
  useEventStore((s) => s.lsmStates.get(nodeId));

export const useWsStatus = () => useEventStore((s) => s.wsStatus);

export const useIsLive = () => useEventStore((s) => s.isLive);

export const useStats = () =>
  useEventStore((s) => ({
    totalEventsReceived: s.totalEventsReceived,
    lastEventTimestamp: s.lastEventTimestamp,
    bufferSize: s.eventBuffer.length,
  }));

// ============================================================================
// New Time-Series Selectors
// ============================================================================

/**
 * Get write pressure history for a node.
 */
export function getWritePressureHistory(
  nodeId: number,
  count: number = 60
): TimeSeriesPoint<WritePressurePoint>[] {
  const history = writePressureHistories.get(nodeId);
  return history?.getLast(count) ?? [];
}

/**
 * Hook to get write pressure history with reactivity.
 * Note: This reads from the store to trigger re-renders, then fetches from buffer.
 */
export const useWritePressureHistory = (nodeId: number, count: number = 60) => {
  // Subscribe to lastEventTimestamp to trigger re-renders on new events
  useEventStore((s) => s.lastEventTimestamp);
  return getWritePressureHistory(nodeId, count);
};

/**
 * Get slot heat history for a specific slot.
 */
export function getSlotHeatHistory(
  nodeId: number,
  level: number,
  slot: number,
  count: number = 30
): TimeSeriesPoint<number>[] {
  const key = `${nodeId}-${level}-${slot}`;
  const history = slotHeatHistories.get(key);
  return history?.getLast(count) ?? [];
}

export const useSlotHeatHistory = (
  nodeId: number,
  level: number,
  slot: number,
  count: number = 30
) => {
  useEventStore((s) => s.lastEventTimestamp);
  return getSlotHeatHistory(nodeId, level, slot, count);
};

/**
 * Get fsync latency data for a node.
 */
export function getFsyncLatencyData(nodeId: number) {
  const history = fsyncLatencyHistories.get(nodeId);
  if (!history) return null;
  return {
    timeSeries: history.getTimeSeries(),
    buckets: history.getBuckets(),
    totalCount: history.totalCount,
  };
}

export const useFsyncLatencyData = (nodeId: number) => {
  useEventStore((s) => s.lastEventTimestamp);
  return getFsyncLatencyData(nodeId);
};

/**
 * Get all cache hit ratio histories.
 */
export function getCacheHitRatios(): Array<{
  name: string;
  current: number;
  history: TimeSeriesPoint<number>[];
}> {
  const result: Array<{
    name: string;
    current: number;
    history: TimeSeriesPoint<number>[];
  }> = [];

  cacheHitRatioHistories.forEach((history, name) => {
    const latest = history.getLatest();
    result.push({
      name,
      current: latest?.value ?? 0,
      history: history.getLast(60),
    });
  });

  return result;
}

export const useCacheHitRatios = () => {
  useEventStore((s) => s.lastEventTimestamp);
  return getCacheHitRatios();
};

/**
 * Get compaction efficiency for a node/level.
 */
export const useCompactionEfficiency = (nodeId: number, level?: number) =>
  useEventStore((s) => {
    if (level !== undefined) {
      return s.compactionEfficiency.get(`${nodeId}-${level}`) ?? [];
    }
    // Return all levels for this node
    const result: CompactionRecord[] = [];
    s.compactionEfficiency.forEach((records, key) => {
      if (key.startsWith(`${nodeId}-`)) {
        result.push(...records);
      }
    });
    return result.sort((a, b) => b.timestamp - a.timestamp);
  });

/**
 * Get bandit optimization state for a node/level.
 */
export const useBanditState = (nodeId: number, level: number) =>
  useEventStore((s) => s.banditStates.get(`${nodeId}-${level}`));

/**
 * Get shard with enhanced tracking info.
 */
export const useShardState = (shardId: number) =>
  useEventStore((s) => s.shards.get(shardId));

/**
 * Get node with enhanced tracking info.
 */
export const useNodeState = (nodeId: number) =>
  useEventStore((s) => s.nodes.get(nodeId));

/**
 * Get snapshot state for a shard.
 */
export const useShardSnapshot = (shardId: number) =>
  useEventStore((s) => s.shardSnapshots.get(shardId));

/**
 * Get recent SWIM connection events.
 */
export const useSwimConnectionEvents = (count: number = 20) =>
  useEventStore((s) => s.swimConnectionEvents.slice(-count));

/**
 * Get all shards with stability info.
 */
export const useShardsWithStability = () =>
  useEventStore((s) => {
    const now = Date.now();
    return Array.from(s.shards.values()).map((shard) => ({
      ...shard,
      stabilityMs: now - shard.lastElectionTs,
      isStable: now - shard.lastElectionTs > 5 * 60 * 1000, // 5 minutes
      recentElection: now - shard.lastElectionTs < 30 * 1000, // 30 seconds
    }));
  });

/**
 * Get all nodes with state history.
 */
export const useNodesWithHistory = () =>
  useEventStore((s) => Array.from(s.nodes.values()));
