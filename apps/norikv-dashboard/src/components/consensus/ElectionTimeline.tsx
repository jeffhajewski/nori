"use client";

import { useEffect, useState, useMemo } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useEventStore, useShardsWithStability } from "@/stores/eventStore";
import { formatTimestamp, cn } from "@/lib/utils";
import {
  BarChart,
  Bar,
  LineChart,
  Line,
  XAxis,
  YAxis,
  Tooltip,
  ResponsiveContainer,
  CartesianGrid,
  Legend,
} from "recharts";

interface RaftEvent {
  id: string;
  timestamp: number;
  shard: number;
  term: number;
  kind: string;
  node?: number;
  from?: number;
}

const MAX_EVENTS = 100;

type ViewMode = "events" | "frequency" | "terms";

export function ElectionTimeline({ filterShard }: { filterShard?: number | null }) {
  const [events, setEvents] = useState<RaftEvent[]>([]);
  const [viewMode, setViewMode] = useState<ViewMode>("events");
  const shards = useShardsWithStability();

  // Subscribe to Raft events
  useEffect(() => {
    const unsub = useEventStore.subscribe(
      (state) => state.eventBuffer,
      (buffer) => {
        if (buffer.length === 0) return;

        const latest = buffer[buffer.length - 1];
        const newEvents: RaftEvent[] = [];

        for (const evt of latest.events) {
          if (evt.event.type === "Raft") {
            const raftEvt = evt.event.data;
            newEvents.push({
              id: `${evt.ts}-${raftEvt.shard}-${raftEvt.term}-${Math.random()}`,
              timestamp: evt.ts,
              shard: raftEvt.shard,
              term: raftEvt.term,
              kind: raftEvt.kind.kind,
              node: raftEvt.kind.kind === "LeaderElected" ? raftEvt.kind.node : undefined,
              from: raftEvt.kind.kind === "VoteReq" || raftEvt.kind.kind === "VoteGranted"
                ? raftEvt.kind.from
                : undefined,
            });
          }
        }

        if (newEvents.length > 0) {
          setEvents((prev) => [...newEvents, ...prev].slice(0, MAX_EVENTS));
        }
      }
    );

    return unsub;
  }, []);

  const filteredEvents = filterShard !== null && filterShard !== undefined
    ? events.filter((e) => e.shard === filterShard)
    : events;

  // Compute election frequency data (elections per minute)
  const frequencyData = useMemo(() => {
    const electionEvents = events.filter((e) => e.kind === "LeaderElected");
    if (electionEvents.length === 0) return [];

    // Group by minute
    const buckets = new Map<number, number>();
    const now = Date.now();

    // Create 10 minute buckets
    for (let i = 0; i < 10; i++) {
      const bucketTime = Math.floor((now - i * 60000) / 60000) * 60000;
      buckets.set(bucketTime, 0);
    }

    for (const evt of electionEvents) {
      const bucketTime = Math.floor(evt.timestamp / 60000) * 60000;
      if (buckets.has(bucketTime)) {
        buckets.set(bucketTime, (buckets.get(bucketTime) || 0) + 1);
      }
    }

    return Array.from(buckets.entries())
      .map(([timestamp, count]) => ({
        timestamp,
        time: new Date(timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }),
        elections: count,
      }))
      .sort((a, b) => a.timestamp - b.timestamp);
  }, [events]);

  // Compute term progression data per shard
  const termProgressionData = useMemo(() => {
    // Get unique shards
    const shardIds = Array.from(new Set(events.map((e) => e.shard))).sort((a, b) => a - b).slice(0, 8);
    if (shardIds.length === 0) return { data: [], shardIds: [] };

    // Create time buckets (last 10 minutes, 1 minute intervals)
    const now = Date.now();
    const bucketCount = 10;
    const data: Array<{ time: string; timestamp: number; [key: string]: number | string }> = [];

    for (let i = bucketCount - 1; i >= 0; i--) {
      const bucketTime = Math.floor((now - i * 60000) / 60000) * 60000;
      const point: { time: string; timestamp: number; [key: string]: number | string } = {
        timestamp: bucketTime,
        time: new Date(bucketTime).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }),
      };

      // For each shard, find the most recent term at that time
      for (const shardId of shardIds) {
        const shardEvents = events
          .filter((e) => e.shard === shardId && e.timestamp <= bucketTime + 60000)
          .sort((a, b) => b.timestamp - a.timestamp);

        const latestTerm = shardEvents[0]?.term ?? 0;
        point[`S${shardId}`] = latestTerm;
      }

      data.push(point);
    }

    return { data, shardIds };
  }, [events]);

  // Detect term storms (rapid term increases)
  const termStormWarning = useMemo(() => {
    const recentElections = events
      .filter((e) => e.kind === "LeaderElected" && Date.now() - e.timestamp < 60000);

    // Group by shard
    const shardCounts = new Map<number, number>();
    for (const evt of recentElections) {
      shardCounts.set(evt.shard, (shardCounts.get(evt.shard) || 0) + 1);
    }

    // Find any shard with > 3 elections in last minute
    const stormyShards = Array.from(shardCounts.entries())
      .filter(([, count]) => count > 3)
      .map(([shardId]) => shardId);

    return stormyShards;
  }, [events]);

  // Shard colors for charts
  const shardColors = [
    "hsl(210, 100%, 60%)",
    "hsl(150, 80%, 50%)",
    "hsl(280, 80%, 60%)",
    "hsl(30, 100%, 55%)",
    "hsl(340, 80%, 60%)",
    "hsl(180, 70%, 50%)",
    "hsl(60, 90%, 50%)",
    "hsl(0, 80%, 60%)",
  ];

  return (
    <div className="space-y-4">
      {/* View mode toggle */}
      <div className="flex items-center justify-between">
        <div className="flex rounded-lg bg-muted p-0.5">
          {(["events", "frequency", "terms"] as const).map((mode) => (
            <button
              key={mode}
              onClick={() => setViewMode(mode)}
              className={cn(
                "rounded-md px-3 py-1.5 text-xs font-medium transition-colors",
                viewMode === mode
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground"
              )}
            >
              {mode === "events" ? "Events" : mode === "frequency" ? "Frequency" : "Terms"}
            </button>
          ))}
        </div>

        {/* Term storm warning */}
        {termStormWarning.length > 0 && (
          <motion.div
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            className="flex items-center gap-2 rounded-lg bg-status-critical/10 px-3 py-1.5 text-xs font-medium text-status-critical"
          >
            <span className="h-2 w-2 animate-pulse rounded-full bg-status-critical" />
            Term storm: Shard {termStormWarning.join(", ")}
          </motion.div>
        )}
      </div>

      {/* Content based on view mode */}
      {viewMode === "events" && (
        <EventListView events={filteredEvents} />
      )}

      {viewMode === "frequency" && (
        <FrequencyChartView data={frequencyData} />
      )}

      {viewMode === "terms" && (
        <TermProgressionView
          data={termProgressionData.data}
          shardIds={termProgressionData.shardIds}
          colors={shardColors}
        />
      )}
    </div>
  );
}

function EventListView({ events }: { events: RaftEvent[] }) {
  if (events.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No Raft events yet. Waiting for elections...
      </p>
    );
  }

  return (
    <div className="max-h-96 space-y-1 overflow-y-auto pr-2">
      <AnimatePresence mode="popLayout">
        {events.map((event) => (
          <EventRow key={event.id} event={event} />
        ))}
      </AnimatePresence>
    </div>
  );
}

function FrequencyChartView({ data }: { data: Array<{ time: string; elections: number }> }) {
  if (data.length === 0) {
    return (
      <div className="flex h-48 items-center justify-center rounded-lg border border-dashed border-border">
        <p className="text-sm text-muted-foreground">
          No election data yet
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <h4 className="text-sm font-medium text-foreground">Elections per Minute</h4>
        <span className="text-xs text-muted-foreground">Last 10 minutes</span>
      </div>
      <div className="h-48">
        <ResponsiveContainer width="100%" height="100%">
          <BarChart data={data} margin={{ top: 5, right: 5, left: -20, bottom: 5 }}>
            <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
            <XAxis
              dataKey="time"
              tick={{ fontSize: 10 }}
              className="text-muted-foreground"
            />
            <YAxis
              allowDecimals={false}
              tick={{ fontSize: 10 }}
              className="text-muted-foreground"
            />
            <Tooltip
              contentStyle={{
                backgroundColor: "hsl(var(--card))",
                border: "1px solid hsl(var(--border))",
                borderRadius: "8px",
                fontSize: "12px",
              }}
              labelStyle={{ color: "hsl(var(--foreground))" }}
            />
            <Bar
              dataKey="elections"
              fill="hsl(var(--primary))"
              radius={[4, 4, 0, 0]}
              name="Elections"
            />
          </BarChart>
        </ResponsiveContainer>
      </div>

      {/* Summary stats */}
      <div className="grid grid-cols-3 gap-2">
        <div className="rounded-lg bg-muted/50 p-2 text-center">
          <p className="text-xs text-muted-foreground">Total</p>
          <p className="text-lg font-semibold text-foreground">
            {data.reduce((sum, d) => sum + d.elections, 0)}
          </p>
        </div>
        <div className="rounded-lg bg-muted/50 p-2 text-center">
          <p className="text-xs text-muted-foreground">Avg/min</p>
          <p className="text-lg font-semibold text-foreground">
            {(data.reduce((sum, d) => sum + d.elections, 0) / Math.max(data.length, 1)).toFixed(1)}
          </p>
        </div>
        <div className="rounded-lg bg-muted/50 p-2 text-center">
          <p className="text-xs text-muted-foreground">Peak</p>
          <p className="text-lg font-semibold text-foreground">
            {Math.max(...data.map((d) => d.elections), 0)}
          </p>
        </div>
      </div>
    </div>
  );
}

function TermProgressionView({
  data,
  shardIds,
  colors,
}: {
  data: Array<{ time: string; [key: string]: number | string }>;
  shardIds: number[];
  colors: string[];
}) {
  if (data.length === 0 || shardIds.length === 0) {
    return (
      <div className="flex h-48 items-center justify-center rounded-lg border border-dashed border-border">
        <p className="text-sm text-muted-foreground">
          No term progression data yet
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <h4 className="text-sm font-medium text-foreground">Term Progression by Shard</h4>
        <span className="text-xs text-muted-foreground">
          Rapid increases indicate instability
        </span>
      </div>
      <div className="h-48">
        <ResponsiveContainer width="100%" height="100%">
          <LineChart data={data} margin={{ top: 5, right: 5, left: -20, bottom: 5 }}>
            <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
            <XAxis
              dataKey="time"
              tick={{ fontSize: 10 }}
              className="text-muted-foreground"
            />
            <YAxis
              allowDecimals={false}
              tick={{ fontSize: 10 }}
              className="text-muted-foreground"
              label={{
                value: "Term",
                angle: -90,
                position: "insideLeft",
                style: { fontSize: 10 },
              }}
            />
            <Tooltip
              contentStyle={{
                backgroundColor: "hsl(var(--card))",
                border: "1px solid hsl(var(--border))",
                borderRadius: "8px",
                fontSize: "12px",
              }}
              labelStyle={{ color: "hsl(var(--foreground))" }}
            />
            <Legend
              wrapperStyle={{ fontSize: "10px" }}
              iconType="line"
            />
            {shardIds.map((shardId, i) => (
              <Line
                key={shardId}
                type="stepAfter"
                dataKey={`S${shardId}`}
                stroke={colors[i % colors.length]}
                strokeWidth={2}
                dot={false}
                name={`Shard ${shardId}`}
              />
            ))}
          </LineChart>
        </ResponsiveContainer>
      </div>

      {/* Shard term summary */}
      <div className="flex flex-wrap gap-2">
        {shardIds.map((shardId, i) => {
          const latestTerm = data[data.length - 1]?.[`S${shardId}`] as number | undefined;
          return (
            <div
              key={shardId}
              className="flex items-center gap-1.5 rounded-lg bg-muted/50 px-2 py-1"
            >
              <div
                className="h-2 w-2 rounded-full"
                style={{ backgroundColor: colors[i % colors.length] }}
              />
              <span className="text-xs font-medium text-foreground">S{shardId}</span>
              <span className="text-xs text-muted-foreground">
                T{latestTerm ?? "?"}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function EventRow({ event }: { event: RaftEvent }) {
  const getEventIcon = (kind: string) => {
    switch (kind) {
      case "LeaderElected":
        return (
          <div className="flex h-6 w-6 items-center justify-center rounded-full bg-status-healthy/20 text-status-healthy">
            <svg className="h-3.5 w-3.5" fill="currentColor" viewBox="0 0 20 20">
              <path d="M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z" />
            </svg>
          </div>
        );
      case "VoteReq":
        return (
          <div className="flex h-6 w-6 items-center justify-center rounded-full bg-blue-500/20 text-blue-500">
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8.228 9c.549-1.165 2.03-2 3.772-2 2.21 0 4 1.343 4 3 0 1.4-1.278 2.575-3.006 2.907-.542.104-.994.54-.994 1.093m0 3h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
          </div>
        );
      case "VoteGranted":
        return (
          <div className="flex h-6 w-6 items-center justify-center rounded-full bg-green-500/20 text-green-500">
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
            </svg>
          </div>
        );
      case "StepDown":
        return (
          <div className="flex h-6 w-6 items-center justify-center rounded-full bg-status-warning/20 text-status-warning">
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 14l-7 7m0 0l-7-7m7 7V3" />
            </svg>
          </div>
        );
      default:
        return (
          <div className="flex h-6 w-6 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
          </div>
        );
    }
  };

  const getEventDescription = (event: RaftEvent) => {
    switch (event.kind) {
      case "LeaderElected":
        return (
          <span>
            <strong className="text-status-healthy">Node {event.node}</strong> elected leader
          </span>
        );
      case "VoteReq":
        return (
          <span>
            Vote requested from <strong>Node {event.from}</strong>
          </span>
        );
      case "VoteGranted":
        return (
          <span>
            Vote granted to <strong>Node {event.from}</strong>
          </span>
        );
      case "StepDown":
        return <span>Leader stepped down</span>;
      default:
        return <span>{event.kind}</span>;
    }
  };

  return (
    <motion.div
      layout
      initial={{ opacity: 0, x: -20 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: 20 }}
      className="flex items-center gap-3 rounded-lg bg-muted/30 px-3 py-2"
    >
      {getEventIcon(event.kind)}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="rounded bg-primary/20 px-1.5 py-0.5 text-xs font-medium text-primary">
            Shard {event.shard}
          </span>
          <span className="text-xs text-muted-foreground">Term {event.term}</span>
        </div>
        <p className="mt-0.5 text-sm text-foreground truncate">
          {getEventDescription(event)}
        </p>
      </div>
      <span className="text-xs text-muted-foreground whitespace-nowrap">
        {formatTimestamp(event.timestamp)}
      </span>
    </motion.div>
  );
}
