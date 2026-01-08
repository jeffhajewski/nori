"use client";

import { useEffect, useState, useMemo } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useEventStore, useNodesWithHistory, useSwimConnectionEvents } from "@/stores/eventStore";
import { formatTimestamp, formatDuration, cn } from "@/lib/utils";

interface SwimEvent {
  id: string;
  timestamp: number;
  node: number;
  kind: string;
}

const MAX_EVENTS = 50;

type ViewMode = "events" | "states" | "metrics";

export function SwimTimeline({ filterNode }: { filterNode?: number | null }) {
  const [events, setEvents] = useState<SwimEvent[]>([]);
  const [viewMode, setViewMode] = useState<ViewMode>("events");
  const nodesWithHistory = useNodesWithHistory();
  const connectionEvents = useSwimConnectionEvents(100);

  // Subscribe to SWIM events
  useEffect(() => {
    const unsub = useEventStore.subscribe(
      (state) => state.eventBuffer,
      (buffer) => {
        if (buffer.length === 0) return;

        const latest = buffer[buffer.length - 1];
        const newEvents: SwimEvent[] = [];

        for (const evt of latest.events) {
          if (evt.event.type === "Swim") {
            const swimEvt = evt.event.data;
            newEvents.push({
              id: `${evt.ts}-${swimEvt.node}-${Math.random()}`,
              timestamp: evt.ts,
              node: swimEvt.node,
              kind: swimEvt.kind,
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

  // Calculate detection metrics
  const detectionMetrics = useMemo(() => {
    let totalSuspects = 0;
    let totalConfirms = 0;
    let falsePositives = 0;
    const suspectToConfirmTimes: number[] = [];

    // Track node states for false positive detection
    const nodeLastSuspect = new Map<number, number>();
    const nodeLastConfirm = new Map<number, number>();

    // Sort events by timestamp
    const sortedEvents = [...events].sort((a, b) => a.timestamp - b.timestamp);

    for (const evt of sortedEvents) {
      if (evt.kind === "Suspect") {
        totalSuspects++;
        nodeLastSuspect.set(evt.node, evt.timestamp);
      } else if (evt.kind === "Confirm") {
        totalConfirms++;
        const suspectTime = nodeLastSuspect.get(evt.node);
        if (suspectTime) {
          suspectToConfirmTimes.push(evt.timestamp - suspectTime);
        }
        nodeLastConfirm.set(evt.node, evt.timestamp);
      } else if (evt.kind === "Alive") {
        // If we see Alive after Suspect (before Confirm), it's a false positive
        const suspectTime = nodeLastSuspect.get(evt.node);
        const confirmTime = nodeLastConfirm.get(evt.node);
        if (suspectTime && (!confirmTime || confirmTime < suspectTime)) {
          falsePositives++;
        }
      }
    }

    const avgSuspectToConfirm = suspectToConfirmTimes.length > 0
      ? suspectToConfirmTimes.reduce((a, b) => a + b, 0) / suspectToConfirmTimes.length
      : 0;

    return {
      totalSuspects,
      totalConfirms,
      falsePositives,
      avgSuspectToConfirm,
      accuracy: totalSuspects > 0 ? (totalConfirms / totalSuspects) * 100 : 100,
    };
  }, [events]);

  const filteredEvents = filterNode !== null && filterNode !== undefined
    ? events.filter((e) => e.node === filterNode)
    : events;

  // Get selected node data for state machine view
  const selectedNodeData = useMemo(() => {
    if (filterNode === null || filterNode === undefined) return null;
    return nodesWithHistory.find((n) => n.id === filterNode) ?? null;
  }, [nodesWithHistory, filterNode]);

  return (
    <div className="space-y-4">
      {/* View mode toggle */}
      <div className="flex items-center justify-between">
        <div className="flex rounded-lg bg-muted p-0.5">
          {(["events", "states", "metrics"] as const).map((mode) => (
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
              {mode === "events" ? "Events" : mode === "states" ? "States" : "Metrics"}
            </button>
          ))}
        </div>
      </div>

      {/* Content based on view mode */}
      {viewMode === "events" && (
        <EventListView events={filteredEvents} />
      )}

      {viewMode === "states" && (
        <StatesMachineView
          selectedNode={filterNode}
          nodeData={selectedNodeData}
          events={filteredEvents}
        />
      )}

      {viewMode === "metrics" && (
        <DetectionMetricsView metrics={detectionMetrics} events={events} />
      )}
    </div>
  );
}

function EventListView({ events }: { events: SwimEvent[] }) {
  if (events.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No SWIM events yet. Waiting for membership updates...
      </p>
    );
  }

  return (
    <div className="max-h-80 space-y-1 overflow-y-auto pr-2">
      <AnimatePresence mode="popLayout">
        {events.map((event) => (
          <SwimEventRow key={event.id} event={event} />
        ))}
      </AnimatePresence>
    </div>
  );
}

interface StatesMachineViewProps {
  selectedNode: number | null | undefined;
  nodeData: {
    id: number;
    status: "alive" | "suspect" | "dead";
    stateHistory: Array<{ state: "alive" | "suspect" | "dead"; timestamp: number }>;
    suspectedBy: Set<number>;
  } | null;
  events: SwimEvent[];
}

function StatesMachineView({ selectedNode, nodeData, events }: StatesMachineViewProps) {
  if (selectedNode === null || selectedNode === undefined) {
    return (
      <div className="flex h-48 items-center justify-center rounded-lg border border-dashed border-border">
        <p className="text-sm text-muted-foreground">
          Select a node to view its state machine
        </p>
      </div>
    );
  }

  // Build state history from events
  const stateHistory = useMemo(() => {
    const nodeEvents = events
      .filter((e) => e.node === selectedNode)
      .sort((a, b) => a.timestamp - b.timestamp);

    return nodeEvents.map((e) => ({
      state: e.kind,
      timestamp: e.timestamp,
    }));
  }, [events, selectedNode]);

  const currentState = nodeData?.status || "unknown";

  return (
    <div className="space-y-4">
      {/* State machine diagram */}
      <div className="rounded-lg border border-border bg-muted/30 p-4">
        <h4 className="text-sm font-medium text-foreground mb-4">
          Node {selectedNode} State Machine
        </h4>
        <StateMachineDiagram currentState={currentState} />
      </div>

      {/* State transition history */}
      {stateHistory.length > 0 && (
        <div className="space-y-2">
          <h4 className="text-sm font-medium text-foreground">State Transitions</h4>
          <div className="max-h-32 overflow-y-auto space-y-1">
            {stateHistory.slice(-10).reverse().map((entry, i) => (
              <div
                key={`${entry.timestamp}-${i}`}
                className="flex items-center justify-between rounded bg-muted/30 px-3 py-1.5 text-xs"
              >
                <div className="flex items-center gap-2">
                  <StateIcon state={entry.state} />
                  <span className="font-medium text-foreground">{entry.state}</span>
                </div>
                <span className="text-muted-foreground">
                  {formatTimestamp(entry.timestamp)}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function StateMachineDiagram({ currentState }: { currentState: string }) {
  const states = ["Alive", "Suspect", "Confirm", "Leave"];

  const statePositions = {
    Alive: { x: 60, y: 60 },
    Suspect: { x: 180, y: 60 },
    Confirm: { x: 180, y: 140 },
    Leave: { x: 60, y: 140 },
  };

  const transitions = [
    { from: "Alive", to: "Suspect", label: "timeout" },
    { from: "Suspect", to: "Alive", label: "ack" },
    { from: "Suspect", to: "Confirm", label: "timeout" },
    { from: "Confirm", to: "Leave", label: "" },
    { from: "Leave", to: "Alive", label: "rejoin" },
  ];

  const stateColors: Record<string, string> = {
    Alive: "#22c55e",
    Suspect: "#f59e0b",
    Confirm: "#ef4444",
    Leave: "#6b7280",
    unknown: "#6b7280",
  };

  return (
    <svg viewBox="0 0 240 180" className="w-full h-40">
      {/* Transitions */}
      {transitions.map(({ from, to, label }) => {
        const fromPos = statePositions[from as keyof typeof statePositions];
        const toPos = statePositions[to as keyof typeof statePositions];
        if (!fromPos || !toPos) return null;

        const midX = (fromPos.x + toPos.x) / 2;
        const midY = (fromPos.y + toPos.y) / 2;

        // Offset for bidirectional arrows
        const dx = toPos.x - fromPos.x;
        const dy = toPos.y - fromPos.y;
        const len = Math.sqrt(dx * dx + dy * dy);
        const offsetX = (dy / len) * 8;
        const offsetY = (-dx / len) * 8;

        return (
          <g key={`${from}-${to}`}>
            <line
              x1={fromPos.x + offsetX}
              y1={fromPos.y + offsetY}
              x2={toPos.x + offsetX}
              y2={toPos.y + offsetY}
              stroke="currentColor"
              strokeWidth="1.5"
              className="text-muted-foreground/50"
              markerEnd="url(#arrowhead)"
            />
            {label && (
              <text
                x={midX + offsetX * 2}
                y={midY + offsetY * 2}
                textAnchor="middle"
                className="fill-muted-foreground text-[8px]"
              >
                {label}
              </text>
            )}
          </g>
        );
      })}

      {/* Arrow marker */}
      <defs>
        <marker
          id="arrowhead"
          markerWidth="6"
          markerHeight="6"
          refX="5"
          refY="3"
          orient="auto"
        >
          <polygon
            points="0 0, 6 3, 0 6"
            className="fill-muted-foreground/50"
          />
        </marker>
      </defs>

      {/* State nodes */}
      {states.map((state) => {
        const pos = statePositions[state as keyof typeof statePositions];
        const isActive = currentState.toLowerCase() === state.toLowerCase();

        return (
          <g key={state}>
            {/* Active ring */}
            {isActive && (
              <motion.circle
                cx={pos.x}
                cy={pos.y}
                r="30"
                fill="none"
                stroke={stateColors[state]}
                strokeWidth="2"
                initial={{ scale: 1, opacity: 0.5 }}
                animate={{ scale: [1, 1.2, 1], opacity: [0.5, 0, 0.5] }}
                transition={{ duration: 2, repeat: Infinity }}
              />
            )}
            <circle
              cx={pos.x}
              cy={pos.y}
              r="24"
              fill={isActive ? stateColors[state] : "transparent"}
              stroke={stateColors[state]}
              strokeWidth="2"
            />
            <text
              x={pos.x}
              y={pos.y + 1}
              textAnchor="middle"
              dominantBaseline="middle"
              className={cn(
                "text-[10px] font-medium",
                isActive ? "fill-white" : "fill-foreground"
              )}
            >
              {state}
            </text>
          </g>
        );
      })}
    </svg>
  );
}

interface DetectionMetricsViewProps {
  metrics: {
    totalSuspects: number;
    totalConfirms: number;
    falsePositives: number;
    avgSuspectToConfirm: number;
    accuracy: number;
  };
  events: SwimEvent[];
}

function DetectionMetricsView({ metrics, events }: DetectionMetricsViewProps) {
  // Count events per type in last 5 minutes
  const recentStats = useMemo(() => {
    const fiveMinutesAgo = Date.now() - 5 * 60 * 1000;
    const recentEvents = events.filter((e) => e.timestamp > fiveMinutesAgo);

    return {
      alive: recentEvents.filter((e) => e.kind === "Alive").length,
      suspect: recentEvents.filter((e) => e.kind === "Suspect").length,
      confirm: recentEvents.filter((e) => e.kind === "Confirm").length,
      leave: recentEvents.filter((e) => e.kind === "Leave").length,
    };
  }, [events]);

  return (
    <div className="space-y-4">
      {/* Detection accuracy */}
      <div className="rounded-lg border border-border bg-muted/30 p-4">
        <h4 className="text-sm font-medium text-foreground mb-3">Detection Accuracy</h4>
        <div className="grid grid-cols-2 gap-3">
          <div className="rounded-lg bg-background p-3 text-center">
            <p className="text-xs text-muted-foreground">Accuracy</p>
            <p
              className={cn(
                "text-2xl font-bold",
                metrics.accuracy >= 90
                  ? "text-status-healthy"
                  : metrics.accuracy >= 70
                  ? "text-status-warning"
                  : "text-status-critical"
              )}
            >
              {metrics.accuracy.toFixed(1)}%
            </p>
          </div>
          <div className="rounded-lg bg-background p-3 text-center">
            <p className="text-xs text-muted-foreground">Avg Detection Time</p>
            <p className="text-2xl font-bold text-foreground">
              {metrics.avgSuspectToConfirm > 0
                ? formatDuration(metrics.avgSuspectToConfirm)
                : "—"}
            </p>
          </div>
        </div>
      </div>

      {/* Counts */}
      <div className="grid grid-cols-4 gap-2">
        <MetricCard
          label="Suspects"
          value={metrics.totalSuspects}
          color="text-status-warning"
        />
        <MetricCard
          label="Confirms"
          value={metrics.totalConfirms}
          color="text-status-critical"
        />
        <MetricCard
          label="False Positives"
          value={metrics.falsePositives}
          color={metrics.falsePositives > 0 ? "text-status-warning" : "text-status-healthy"}
        />
        <MetricCard
          label="Events"
          value={events.length}
          color="text-foreground"
        />
      </div>

      {/* Recent activity (last 5 min) */}
      <div className="rounded-lg border border-border bg-muted/30 p-4">
        <h4 className="text-sm font-medium text-foreground mb-3">Last 5 Minutes</h4>
        <div className="grid grid-cols-4 gap-2">
          <div className="flex items-center gap-2 rounded bg-status-healthy/10 px-2 py-1.5">
            <div className="h-2 w-2 rounded-full bg-status-healthy" />
            <span className="text-xs text-foreground">{recentStats.alive} Alive</span>
          </div>
          <div className="flex items-center gap-2 rounded bg-status-warning/10 px-2 py-1.5">
            <div className="h-2 w-2 rounded-full bg-status-warning" />
            <span className="text-xs text-foreground">{recentStats.suspect} Suspect</span>
          </div>
          <div className="flex items-center gap-2 rounded bg-status-critical/10 px-2 py-1.5">
            <div className="h-2 w-2 rounded-full bg-status-critical" />
            <span className="text-xs text-foreground">{recentStats.confirm} Confirm</span>
          </div>
          <div className="flex items-center gap-2 rounded bg-muted px-2 py-1.5">
            <div className="h-2 w-2 rounded-full bg-muted-foreground" />
            <span className="text-xs text-foreground">{recentStats.leave} Leave</span>
          </div>
        </div>
      </div>
    </div>
  );
}

function MetricCard({
  label,
  value,
  color,
}: {
  label: string;
  value: number;
  color: string;
}) {
  return (
    <div className="rounded-lg bg-muted/50 p-2 text-center">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className={cn("text-lg font-semibold", color)}>{value}</p>
    </div>
  );
}

function StateIcon({ state }: { state: string }) {
  const config = getStateConfig(state);
  return (
    <div className={cn("h-2 w-2 rounded-full", config.bg)} />
  );
}

function getStateConfig(kind: string) {
  switch (kind) {
    case "Alive":
      return { bg: "bg-status-healthy", color: "text-status-healthy" };
    case "Suspect":
      return { bg: "bg-status-warning", color: "text-status-warning" };
    case "Confirm":
      return { bg: "bg-status-critical", color: "text-status-critical" };
    case "Leave":
      return { bg: "bg-muted-foreground", color: "text-muted-foreground" };
    default:
      return { bg: "bg-muted-foreground", color: "text-muted-foreground" };
  }
}

function SwimEventRow({ event }: { event: SwimEvent }) {
  const getEventConfig = (kind: string) => {
    switch (kind) {
      case "Alive":
        return {
          icon: (
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4.318 6.318a4.5 4.5 0 000 6.364L12 20.364l7.682-7.682a4.5 4.5 0 00-6.364-6.364L12 7.636l-1.318-1.318a4.5 4.5 0 00-6.364 0z" />
            </svg>
          ),
          bg: "bg-status-healthy/20",
          color: "text-status-healthy",
          label: "Alive",
        };
      case "Suspect":
        return {
          icon: (
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
            </svg>
          ),
          bg: "bg-status-warning/20",
          color: "text-status-warning",
          label: "Suspect",
        };
      case "Confirm":
        return {
          icon: (
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
          ),
          bg: "bg-status-critical/20",
          color: "text-status-critical",
          label: "Confirmed Dead",
        };
      case "Leave":
        return {
          icon: (
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1" />
            </svg>
          ),
          bg: "bg-muted",
          color: "text-muted-foreground",
          label: "Left Cluster",
        };
      default:
        return {
          icon: (
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
          ),
          bg: "bg-muted",
          color: "text-muted-foreground",
          label: kind,
        };
    }
  };

  const config = getEventConfig(event.kind);

  return (
    <motion.div
      layout
      initial={{ opacity: 0, x: -20 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: 20 }}
      className="flex items-center gap-3 rounded-lg bg-muted/30 px-3 py-2"
    >
      <div className={`flex h-6 w-6 items-center justify-center rounded-full ${config.bg} ${config.color}`}>
        {config.icon}
      </div>
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="font-medium text-foreground">Node {event.node}</span>
          <span className={`text-xs ${config.color}`}>{config.label}</span>
        </div>
      </div>
      <span className="text-xs text-muted-foreground whitespace-nowrap">
        {formatTimestamp(event.timestamp)}
      </span>
    </motion.div>
  );
}
