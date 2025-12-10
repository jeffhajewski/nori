"use client";

import { useEffect, useState, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useEventStore, useStats } from "@/stores/eventStore";
import { formatTimestamp, formatBytes, formatNumber } from "@/lib/utils";
import type { TimestampedEvent, WireEvent } from "@/types/events";
import { cn } from "@/lib/utils";

const MAX_EVENTS = 200;

type EventFilter = "all" | "wal" | "compaction" | "lsm" | "raft" | "swim" | "shard" | "cache" | "heat";

export default function EventsPage() {
  const [events, setEvents] = useState<TimestampedEvent[]>([]);
  const [filter, setFilter] = useState<EventFilter>("all");
  const [paused, setPaused] = useState(false);
  const stats = useStats();

  // Subscribe to events
  useEffect(() => {
    if (paused) return;

    const unsub = useEventStore.subscribe(
      (state) => state.eventBuffer,
      (buffer) => {
        if (buffer.length === 0) return;
        const latest = buffer[buffer.length - 1];
        setEvents((prev) => [...latest.events, ...prev].slice(0, MAX_EVENTS));
      }
    );

    return unsub;
  }, [paused]);

  const filteredEvents = filter === "all"
    ? events
    : events.filter((e) => e.event.type.toLowerCase() === filter);

  const clearEvents = useCallback(() => setEvents([]), []);

  return (
    <div className="space-y-6">
      {/* Page header */}
      <div className="flex items-start justify-between">
        <div>
          <h1 className="text-2xl font-semibold text-foreground">Events</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Real-time event stream from the cluster
          </p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => setPaused(!paused)}
            className={cn(
              "flex items-center gap-2 rounded-lg px-3 py-2 text-sm font-medium transition-colors",
              paused
                ? "bg-status-warning/20 text-status-warning"
                : "bg-primary/20 text-primary"
            )}
          >
            {paused ? (
              <>
                <svg className="h-4 w-4" fill="currentColor" viewBox="0 0 20 20">
                  <path d="M6.3 2.841A1.5 1.5 0 004 4.11V15.89a1.5 1.5 0 002.3 1.269l9.344-5.89a1.5 1.5 0 000-2.538L6.3 2.84z" />
                </svg>
                Resume
              </>
            ) : (
              <>
                <svg className="h-4 w-4" fill="currentColor" viewBox="0 0 20 20">
                  <path d="M5.75 3a.75.75 0 00-.75.75v12.5c0 .414.336.75.75.75h1.5a.75.75 0 00.75-.75V3.75A.75.75 0 007.25 3h-1.5zM12.75 3a.75.75 0 00-.75.75v12.5c0 .414.336.75.75.75h1.5a.75.75 0 00.75-.75V3.75a.75.75 0 00-.75-.75h-1.5z" />
                </svg>
                Pause
              </>
            )}
          </button>
          <button
            onClick={clearEvents}
            className="rounded-lg bg-muted px-3 py-2 text-sm font-medium text-muted-foreground hover:bg-muted/80 hover:text-foreground"
          >
            Clear
          </button>
        </div>
      </div>

      {/* Stats bar */}
      <div className="flex flex-wrap gap-4 rounded-xl border border-border bg-card p-4">
        <Stat label="Total Received" value={formatNumber(stats.totalEventsReceived)} />
        <Stat label="Buffer Size" value={`${formatNumber(stats.bufferSize)} batches`} />
        <Stat label="Displaying" value={`${filteredEvents.length} events`} />
        <Stat label="Last Event" value={stats.lastEventTimestamp > 0 ? formatTimestamp(stats.lastEventTimestamp) : "—"} />
      </div>

      {/* Filter tabs */}
      <div className="flex flex-wrap gap-2">
        {(["all", "wal", "compaction", "lsm", "raft", "swim", "shard", "heat"] as EventFilter[]).map((f) => (
          <button
            key={f}
            onClick={() => setFilter(f)}
            className={cn(
              "rounded-lg px-3 py-1.5 text-sm font-medium capitalize transition-colors",
              filter === f
                ? "bg-primary text-primary-foreground"
                : "bg-muted text-muted-foreground hover:bg-muted/80 hover:text-foreground"
            )}
          >
            {f}
          </button>
        ))}
      </div>

      {/* Event stream */}
      <div className="rounded-xl border border-border bg-card">
        <div className="max-h-[600px] overflow-y-auto">
          {filteredEvents.length === 0 ? (
            <div className="flex h-40 items-center justify-center">
              <p className="text-sm text-muted-foreground">
                {paused ? "Stream paused" : "Waiting for events..."}
              </p>
            </div>
          ) : (
            <div className="divide-y divide-border">
              <AnimatePresence mode="popLayout">
                {filteredEvents.map((evt, i) => (
                  <EventRow key={`${evt.ts}-${i}`} event={evt} />
                ))}
              </AnimatePresence>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="text-xs text-muted-foreground">{label}:</span>
      <span className="text-sm font-medium text-foreground">{value}</span>
    </div>
  );
}

function EventRow({ event }: { event: TimestampedEvent }) {
  const typeConfig = getEventTypeConfig(event.event);

  return (
    <motion.div
      layout
      initial={{ opacity: 0, height: 0 }}
      animate={{ opacity: 1, height: "auto" }}
      exit={{ opacity: 0, height: 0 }}
      className="flex items-start gap-3 px-4 py-3"
    >
      {/* Type badge */}
      <div className={cn("flex h-6 w-6 shrink-0 items-center justify-center rounded", typeConfig.bg)}>
        <span className={cn("text-xs font-bold", typeConfig.color)}>{typeConfig.abbr}</span>
      </div>

      {/* Content */}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2 flex-wrap">
          <span className={cn("text-sm font-medium", typeConfig.color)}>{event.event.type}</span>
          <span className="text-xs text-muted-foreground">Node {event.node}</span>
        </div>
        <p className="mt-0.5 text-sm text-muted-foreground truncate">
          {getEventDescription(event.event)}
        </p>
      </div>

      {/* Timestamp */}
      <span className="shrink-0 text-xs text-muted-foreground">
        {formatTimestamp(event.ts)}
      </span>
    </motion.div>
  );
}

function getEventTypeConfig(event: WireEvent): { abbr: string; color: string; bg: string } {
  switch (event.type) {
    case "Wal": return { abbr: "W", color: "text-blue-500", bg: "bg-blue-500/20" };
    case "Compaction": return { abbr: "C", color: "text-purple-500", bg: "bg-purple-500/20" };
    case "Lsm": return { abbr: "L", color: "text-amber-500", bg: "bg-amber-500/20" };
    case "Raft": return { abbr: "R", color: "text-green-500", bg: "bg-green-500/20" };
    case "Swim": return { abbr: "S", color: "text-cyan-500", bg: "bg-cyan-500/20" };
    case "Shard": return { abbr: "H", color: "text-pink-500", bg: "bg-pink-500/20" };
    case "Cache": return { abbr: "A", color: "text-orange-500", bg: "bg-orange-500/20" };
    case "SlotHeat": return { abbr: "T", color: "text-red-500", bg: "bg-red-500/20" };
    default: return { abbr: "?", color: "text-muted-foreground", bg: "bg-muted" };
  }
}

function getEventDescription(event: WireEvent): string {
  switch (event.type) {
    case "Wal":
      switch (event.data.kind.kind) {
        case "SegmentRoll": return `Segment roll: ${formatBytes(event.data.kind.bytes)}`;
        case "Fsync": return `Fsync: ${event.data.kind.ms}ms`;
        case "SegmentGc": return "Segment garbage collected";
        case "CorruptionTruncated": return "Corruption truncated";
        default: return `WAL event on segment ${event.data.seg}`;
      }
    case "Compaction":
      switch (event.data.kind.kind) {
        case "Scheduled": return `Level ${event.data.level} compaction scheduled`;
        case "Start": return `Level ${event.data.level} compaction started`;
        case "Progress": return `Level ${event.data.level} compaction: ${event.data.kind.pct}%`;
        case "Finish": return `Level ${event.data.level} done: ${formatBytes(event.data.kind.in_bytes)} → ${formatBytes(event.data.kind.out_bytes)}`;
        default: return `Compaction on level ${event.data.level}`;
      }
    case "Lsm":
      switch (event.data.kind.kind) {
        case "WritePressureUpdate": return `Write pressure: ${(event.data.kind.ratio * 100).toFixed(0)}%${event.data.kind.high ? " (HIGH)" : ""}`;
        case "FlushComplete": return `Flush complete: ${formatBytes(event.data.kind.bytes)}, ${formatNumber(event.data.kind.entries)} entries`;
        case "L0Stall": return `L0 stall: ${event.data.kind.file_count}/${event.data.kind.threshold} files`;
        default: return "LSM event";
      }
    case "Raft":
      switch (event.data.kind.kind) {
        case "LeaderElected": return `Shard ${event.data.shard}: Node ${event.data.kind.node} elected (term ${event.data.term})`;
        case "VoteGranted": return `Shard ${event.data.shard}: Vote granted to Node ${event.data.kind.from}`;
        case "VoteReq": return `Shard ${event.data.shard}: Vote requested by Node ${event.data.kind.from}`;
        case "StepDown": return `Shard ${event.data.shard}: Leader stepped down`;
        default: return `Raft event on shard ${event.data.shard}`;
      }
    case "Swim":
      return `Node ${event.data.node}: ${event.data.kind}`;
    case "Shard":
      return `Shard ${event.data.shard}: ${event.data.kind}`;
    case "Cache":
      return `${event.data.name}: ${(event.data.hit_ratio * 100).toFixed(1)}% hit ratio`;
    case "SlotHeat":
      return `L${event.data.level} slot ${event.data.slot}: heat ${(event.data.heat * 100).toFixed(0)}%`;
    default:
      return "Unknown event";
  }
}
