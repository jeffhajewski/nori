import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";
import { immer } from "zustand/middleware/immer";
import type {
  EventBatch,
  TimestampedEvent,
  WireEvent,
  WireRaftEvt,
  WireSwimEvt,
  WireSlotHeatEvt,
  WireLsmEvt,
} from "@/types/events";

// ============================================================================
// Derived State Types
// ============================================================================

export interface ClusterNode {
  id: number;
  status: "alive" | "suspect" | "dead";
  lastSeen: number;
}

export interface ShardState {
  id: number;
  leader: number | null;
  term: number;
  replicas: Set<number>;
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
// Store State
// ============================================================================

export type WsStatus = "disconnected" | "connecting" | "connected" | "reconnecting";

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
        }),
    }))
  )
);

// ============================================================================
// Event Application Logic
// ============================================================================

function applyEventToState(state: EventState, evt: TimestampedEvent) {
  const { event } = evt;

  switch (event.type) {
    case "Swim":
      applySwimEvent(state, event.data);
      break;
    case "Raft":
      applyRaftEvent(state, event.data);
      break;
    case "SlotHeat":
      applySlotHeatEvent(state, event.data);
      break;
    case "Lsm":
      applyLsmEvent(state, event.data, evt.ts);
      break;
    // Other event types don't affect derived state (yet)
  }
}

function applySwimEvent(state: EventState, evt: WireSwimEvt) {
  const node = state.nodes.get(evt.node) || {
    id: evt.node,
    status: "alive" as const,
    lastSeen: Date.now(),
  };

  switch (evt.kind) {
    case "Alive":
      node.status = "alive";
      node.lastSeen = Date.now();
      break;
    case "Suspect":
      node.status = "suspect";
      break;
    case "Confirm":
    case "Leave":
      node.status = "dead";
      break;
  }

  state.nodes.set(evt.node, node);
}

function applyRaftEvent(state: EventState, evt: WireRaftEvt) {
  const shard = state.shards.get(evt.shard) || {
    id: evt.shard,
    leader: null,
    term: 0,
    replicas: new Set<number>(),
  };

  // Update term
  if (evt.term > shard.term) {
    shard.term = evt.term;
  }

  // Handle different Raft events
  switch (evt.kind.kind) {
    case "LeaderElected":
      shard.leader = evt.kind.node;
      shard.replicas.add(evt.kind.node);
      break;
    case "VoteReq":
    case "VoteGranted":
      shard.replicas.add(evt.kind.from);
      break;
    case "StepDown":
      shard.leader = null;
      break;
  }

  state.shards.set(evt.shard, shard);
}

function applySlotHeatEvent(state: EventState, evt: WireSlotHeatEvt) {
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
    lastUpdate: Date.now(),
  });
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
      break;
    case "L0Stall":
      nodeState.l0FileCount = evt.kind.file_count;
      nodeState.l0Threshold = evt.kind.threshold;
      break;
  }
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
