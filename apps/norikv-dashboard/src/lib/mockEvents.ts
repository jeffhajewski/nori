/**
 * Mock event generator for development without vizd.
 */

import type { EventBatch, TimestampedEvent, WireEvent } from "@/types/events";
import { useEventStore } from "@/stores/eventStore";

interface MockConfig {
  nodeCount: number;
  shardCount: number;
  eventsPerSecond: number;
  levelCount: number;
  slotsPerLevel: number;
}

const DEFAULT_CONFIG: MockConfig = {
  nodeCount: 3,
  shardCount: 8,
  eventsPerSecond: 50,
  levelCount: 7,
  slotsPerLevel: 16,
};

// Cache names for generating cache events
const CACHE_NAMES = ["block_cache", "index_cache", "filter_cache", "row_cache"];

export class MockEventGenerator {
  private config: MockConfig;
  private intervalId: ReturnType<typeof setInterval> | null = null;
  private running = false;
  private cacheHitRatios: Map<string, number> = new Map();
  private snapshotState: Map<number, { inProgress: boolean; startedAt: number | null }> = new Map();

  constructor(config: Partial<MockConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };

    // Initialize cache hit ratios
    CACHE_NAMES.forEach((name) => {
      this.cacheHitRatios.set(name, 0.9 + Math.random() * 0.08); // 90-98%
    });
  }

  start(): void {
    if (this.running) return;
    this.running = true;

    // Set connected status
    useEventStore.getState().setWsStatus("connected");
    useEventStore.getState().setServerVersion("mock-1.0.0");

    // Generate initial state
    this.generateInitialState();

    // Generate events at 20 FPS (50ms intervals)
    const batchIntervalMs = 50;
    const eventsPerBatch = Math.floor(this.config.eventsPerSecond * 0.05);

    this.intervalId = setInterval(() => {
      const batch = this.generateBatch(eventsPerBatch);
      useEventStore.getState().ingestBatch(batch);
    }, batchIntervalMs);

    console.log("[MockEvents] Started generating events");
  }

  stop(): void {
    if (!this.running) return;
    this.running = false;

    if (this.intervalId) {
      clearInterval(this.intervalId);
      this.intervalId = null;
    }

    useEventStore.getState().setWsStatus("disconnected");
    console.log("[MockEvents] Stopped generating events");
  }

  private generateInitialState(): void {
    const now = Date.now();

    // Generate initial SWIM Alive events for all nodes
    for (let nodeId = 1; nodeId <= this.config.nodeCount; nodeId++) {
      const evt: TimestampedEvent = {
        ts: now,
        node: nodeId,
        event: {
          type: "Swim",
          data: { node: nodeId, kind: "Alive" },
        },
      };
      useEventStore.getState().ingestBatch({
        window_start: now - 50,
        window_end: now,
        events: [evt],
        summary: { wal_count: 0, compaction_count: 0, lsm_count: 0, raft_count: 0, swim_count: 1, shard_count: 0, cache_count: 0, heat_count: 0 },
      });
    }

    // Generate initial leader elections for all shards
    for (let shardId = 1; shardId <= this.config.shardCount; shardId++) {
      const leaderNode = ((shardId - 1) % this.config.nodeCount) + 1;
      const evt: TimestampedEvent = {
        ts: now,
        node: leaderNode,
        event: {
          type: "Raft",
          data: {
            shard: shardId,
            term: 1,
            kind: { kind: "LeaderElected", node: leaderNode },
          },
        },
      };
      useEventStore.getState().ingestBatch({
        window_start: now - 50,
        window_end: now,
        events: [evt],
        summary: { wal_count: 0, compaction_count: 0, lsm_count: 0, raft_count: 1, swim_count: 0, shard_count: 0, cache_count: 0, heat_count: 0 },
      });

      // Initialize snapshot state
      this.snapshotState.set(shardId, { inProgress: false, startedAt: null });
    }

    // Generate initial slot heat for storage visualization
    for (let nodeId = 1; nodeId <= this.config.nodeCount; nodeId++) {
      for (let level = 0; level < this.config.levelCount; level++) {
        const slotCount = Math.pow(2, level + 2); // 4, 8, 16, 32, 64, 128, 256
        for (let slot = 0; slot < Math.min(slotCount, this.config.slotsPerLevel); slot++) {
          const heat = Math.random() * (1 - level * 0.1); // Lower levels cooler
          const evt: TimestampedEvent = {
            ts: now,
            node: nodeId,
            event: {
              type: "SlotHeat",
              data: {
                node: nodeId,
                level,
                slot,
                heat,
                k: level === 0 ? 4 : level < 3 ? 2 : 1,
              },
            },
          };
          useEventStore.getState().ingestBatch({
            window_start: now - 50,
            window_end: now,
            events: [evt],
            summary: { wal_count: 0, compaction_count: 0, lsm_count: 0, raft_count: 0, swim_count: 0, shard_count: 0, cache_count: 0, heat_count: 1 },
          });
        }
      }
    }

    // Generate initial cache hit ratio events
    for (const cacheName of CACHE_NAMES) {
      const evt: TimestampedEvent = {
        ts: now,
        node: 1,
        event: {
          type: "Cache",
          data: {
            name: cacheName,
            hit_ratio: this.cacheHitRatios.get(cacheName) ?? 0.9,
          },
        },
      };
      useEventStore.getState().ingestBatch({
        window_start: now - 50,
        window_end: now,
        events: [evt],
        summary: { wal_count: 0, compaction_count: 0, lsm_count: 0, raft_count: 0, swim_count: 0, shard_count: 0, cache_count: 1, heat_count: 0 },
      });
    }
  }

  private generateBatch(eventCount: number): EventBatch {
    const events: TimestampedEvent[] = [];
    const now = Date.now();

    for (let i = 0; i < eventCount; i++) {
      events.push({
        ts: now,
        node: this.randomNode(),
        event: this.randomEvent(),
      });
    }

    return {
      window_start: now - 50,
      window_end: now,
      events,
      summary: this.computeSummary(events),
    };
  }

  private randomNode(): number {
    return Math.floor(Math.random() * this.config.nodeCount) + 1;
  }

  private randomShard(): number {
    return Math.floor(Math.random() * this.config.shardCount) + 1;
  }

  private randomLevel(): number {
    return Math.floor(Math.random() * this.config.levelCount);
  }

  private randomEvent(): WireEvent {
    const rand = Math.random();

    // Weight distribution with new event types
    if (rand < 0.25) {
      return this.randomSlotHeatEvent();
    } else if (rand < 0.40) {
      return this.randomCompactionEvent();
    } else if (rand < 0.52) {
      return this.randomLsmEvent();
    } else if (rand < 0.62) {
      return this.randomWalEvent();
    } else if (rand < 0.75) {
      return this.randomSwimEvent();
    } else if (rand < 0.85) {
      return this.randomRaftEvent();
    } else if (rand < 0.95) {
      return this.randomCacheEvent();
    } else {
      return this.randomShardEvent();
    }
  }

  private randomSlotHeatEvent(): WireEvent {
    const node = this.randomNode();
    const level = this.randomLevel();
    const slot = Math.floor(Math.random() * this.config.slotsPerLevel);
    const heat = Math.max(0, Math.min(1, Math.random() + (Math.random() - 0.5) * 0.2));

    return {
      type: "SlotHeat",
      data: {
        node,
        level,
        slot,
        heat,
        k: level === 0 ? 4 : level < 3 ? 2 : 1,
      },
    };
  }

  private randomCompactionEvent(): WireEvent {
    const node = this.randomNode();
    const level = this.randomLevel();
    const rand = Math.random();

    // Include bandit events
    if (rand < 0.15) {
      // BanditSelection event
      return {
        type: "Compaction",
        data: {
          node,
          level,
          kind: {
            kind: "BanditSelection",
            slot_id: Math.floor(Math.random() * this.config.slotsPerLevel),
            explored: Math.random() > 0.7,
            ucb_score: Math.random() * 2,
            avg_reward: Math.random() * 0.8 + 0.2,
            selection_count: Math.floor(Math.random() * 100) + 1,
          },
        },
      };
    } else if (rand < 0.30) {
      // BanditReward event
      return {
        type: "Compaction",
        data: {
          node,
          level,
          kind: {
            kind: "BanditReward",
            slot_id: Math.floor(Math.random() * this.config.slotsPerLevel),
            reward: Math.random(),
            bytes_written: Math.floor(Math.random() * 10_000_000),
            heat_score: Math.random(),
          },
        },
      };
    }

    const kinds = [
      { kind: "Scheduled" as const },
      { kind: "Start" as const },
      { kind: "Progress" as const, pct: Math.floor(Math.random() * 100) },
      { kind: "Finish" as const, in_bytes: Math.floor(Math.random() * 100_000_000), out_bytes: Math.floor(Math.random() * 50_000_000) },
    ];

    return {
      type: "Compaction",
      data: {
        node,
        level,
        kind: kinds[Math.floor(Math.random() * kinds.length)],
      },
    };
  }

  private randomLsmEvent(): WireEvent {
    const node = this.randomNode();
    const rand = Math.random();

    if (rand < 0.35) {
      return {
        type: "Lsm",
        data: {
          node,
          kind: {
            kind: "WritePressureUpdate",
            ratio: Math.random() * 0.8,
            high: Math.random() > 0.85,
            threshold: 0.5,
          },
        },
      };
    } else if (rand < 0.60) {
      return {
        type: "Lsm",
        data: {
          node,
          kind: {
            kind: "FlushComplete",
            file_number: Math.floor(Math.random() * 10000),
            bytes: Math.floor(Math.random() * 64_000_000),
            entries: Math.floor(Math.random() * 100_000),
          },
        },
      };
    } else if (rand < 0.80) {
      return {
        type: "Lsm",
        data: {
          node,
          kind: {
            kind: "L0Stall",
            file_count: Math.floor(Math.random() * 8) + 8,
            threshold: 12,
          },
        },
      };
    } else {
      // GuardAdjustment event
      return {
        type: "Lsm",
        data: {
          node,
          kind: {
            kind: "GuardAdjustment",
            level: this.randomLevel(),
            new_guard_count: Math.floor(Math.random() * 4) + 1,
          },
        },
      };
    }
  }

  private randomWalEvent(): WireEvent {
    const node = this.randomNode();
    const kinds = [
      { kind: "SegmentRoll" as const, bytes: Math.floor(Math.random() * 64_000_000) },
      { kind: "Fsync" as const, ms: Math.floor(Math.random() * 30) + Math.floor(Math.random() * Math.random() * 50) },
      { kind: "SegmentGc" as const },
    ];

    return {
      type: "Wal",
      data: {
        node,
        seg: Math.floor(Math.random() * 1000),
        kind: kinds[Math.floor(Math.random() * kinds.length)],
      },
    };
  }

  private randomSwimEvent(): WireEvent {
    const node = this.randomNode();
    const rand = Math.random();

    // Mostly alive, occasionally suspect/confirm/leave
    let kind: "Alive" | "Suspect" | "Confirm" | "Leave";
    if (rand > 0.97) {
      kind = "Confirm";
    } else if (rand > 0.93) {
      kind = "Suspect";
    } else if (rand > 0.99) {
      kind = "Leave";
    } else {
      kind = "Alive";
    }

    return {
      type: "Swim",
      data: { node, kind },
    };
  }

  private randomRaftEvent(): WireEvent {
    const shard = this.randomShard();
    const node = this.randomNode();

    // Occasionally generate election events
    if (Math.random() > 0.9) {
      return {
        type: "Raft",
        data: {
          shard,
          term: Math.floor(Math.random() * 10) + 1,
          kind: { kind: "LeaderElected", node },
        },
      };
    }

    const kinds = [
      { kind: "VoteReq" as const, from: node },
      { kind: "VoteGranted" as const, from: node },
      { kind: "StepDown" as const },
    ];

    return {
      type: "Raft",
      data: {
        shard,
        term: Math.floor(Math.random() * 10) + 1,
        kind: kinds[Math.floor(Math.random() * kinds.length)],
      },
    };
  }

  private randomCacheEvent(): WireEvent {
    // Pick a random cache
    const cacheName = CACHE_NAMES[Math.floor(Math.random() * CACHE_NAMES.length)];

    // Update the cache hit ratio with some drift
    let currentRatio = this.cacheHitRatios.get(cacheName) ?? 0.9;
    currentRatio += (Math.random() - 0.5) * 0.02; // +/- 1%
    currentRatio = Math.max(0.5, Math.min(0.99, currentRatio)); // Clamp 50-99%
    this.cacheHitRatios.set(cacheName, currentRatio);

    return {
      type: "Cache",
      data: {
        name: cacheName,
        hit_ratio: currentRatio,
      },
    };
  }

  private randomShardEvent(): WireEvent {
    const shard = this.randomShard();
    const state = this.snapshotState.get(shard);

    // Simulate snapshot lifecycle
    if (state && state.inProgress) {
      // Complete the snapshot
      this.snapshotState.set(shard, { inProgress: false, startedAt: null });
      return {
        type: "Shard",
        data: {
          shard,
          kind: "SnapshotDone",
        },
      };
    } else if (Math.random() > 0.7) {
      // Start a new snapshot
      this.snapshotState.set(shard, { inProgress: true, startedAt: Date.now() });
      return {
        type: "Shard",
        data: {
          shard,
          kind: "SnapshotStart",
        },
      };
    }

    // Default to Plan event
    return {
      type: "Shard",
      data: {
        shard,
        kind: "Plan",
      },
    };
  }

  private computeSummary(events: TimestampedEvent[]): EventBatch["summary"] {
    const summary = {
      wal_count: 0,
      compaction_count: 0,
      lsm_count: 0,
      raft_count: 0,
      swim_count: 0,
      shard_count: 0,
      cache_count: 0,
      heat_count: 0,
    };

    for (const evt of events) {
      switch (evt.event.type) {
        case "Wal": summary.wal_count++; break;
        case "Compaction": summary.compaction_count++; break;
        case "Lsm": summary.lsm_count++; break;
        case "Raft": summary.raft_count++; break;
        case "Swim": summary.swim_count++; break;
        case "Shard": summary.shard_count++; break;
        case "Cache": summary.cache_count++; break;
        case "SlotHeat": summary.heat_count++; break;
      }
    }

    return summary;
  }
}

// Singleton for easy access
export const mockEventGenerator = new MockEventGenerator();
