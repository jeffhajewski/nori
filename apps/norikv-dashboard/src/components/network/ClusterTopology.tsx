"use client";

import { useRef, useEffect, useMemo, useState, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useEventStore, useSwimConnectionEvents, type ClusterNode, type SwimConnectionEvent } from "@/stores/eventStore";
import { cn } from "@/lib/utils";

interface ClusterTopologyProps {
  onNodeSelect?: (nodeId: number) => void;
  selectedNode?: number | null;
}

type ConnectionMode = "ring" | "all" | "selected";

export function ClusterTopology({ onNodeSelect, selectedNode }: ClusterTopologyProps) {
  const nodes = useEventStore((s) => Array.from(s.nodes.values()));
  const swimEvents = useSwimConnectionEvents(30);
  const [connectionMode, setConnectionMode] = useState<ConnectionMode>("ring");

  // Calculate positions in a ring layout
  const nodePositions = useMemo(() => {
    const centerX = 150;
    const centerY = 150;
    const radius = nodes.length > 10 ? 110 : 100;

    return nodes.map((node, index) => {
      const angle = (index / nodes.length) * 2 * Math.PI - Math.PI / 2;
      return {
        node,
        x: centerX + radius * Math.cos(angle),
        y: centerY + radius * Math.sin(angle),
        index,
      };
    });
  }, [nodes]);

  // Create position lookup map
  const positionMap = useMemo(() => {
    const map = new Map<number, { x: number; y: number; index: number }>();
    nodePositions.forEach((p) => {
      map.set(p.node.id, { x: p.x, y: p.y, index: p.index });
    });
    return map;
  }, [nodePositions]);

  // Calculate connections based on mode
  const connections = useMemo(() => {
    if (nodes.length === 0) return [];

    const result: Array<{
      from: number;
      to: number;
      quality: "healthy" | "suspect" | "dead";
      hasRecentEvent: boolean;
      eventKind?: "Suspect" | "Confirm" | "Alive";
    }> = [];

    const recentEventPairs = new Set<string>();
    const eventKindMap = new Map<string, "Suspect" | "Confirm" | "Alive">();

    // Track recent events
    swimEvents.forEach((evt) => {
      const key = `${Math.min(evt.from, evt.to)}-${Math.max(evt.from, evt.to)}`;
      recentEventPairs.add(key);
      eventKindMap.set(key, evt.kind);
    });

    if (connectionMode === "ring") {
      // Only show ring neighbors (O(n) instead of O(n²))
      nodePositions.forEach((pos, i) => {
        const nextIndex = (i + 1) % nodePositions.length;
        const nextPos = nodePositions[nextIndex];

        const fromNode = pos.node;
        const toNode = nextPos.node;
        const key = `${Math.min(fromNode.id, toNode.id)}-${Math.max(fromNode.id, toNode.id)}`;

        // Determine connection quality
        let quality: "healthy" | "suspect" | "dead" = "healthy";
        if (fromNode.status === "dead" || toNode.status === "dead") {
          quality = "dead";
        } else if (fromNode.status === "suspect" || toNode.status === "suspect") {
          quality = "suspect";
        }

        result.push({
          from: fromNode.id,
          to: toNode.id,
          quality,
          hasRecentEvent: recentEventPairs.has(key),
          eventKind: eventKindMap.get(key),
        });
      });
    } else if (connectionMode === "selected" && selectedNode !== null && selectedNode !== undefined) {
      // Only show connections from selected node
      const selectedNodeData = nodes.find((n) => n.id === selectedNode);
      if (selectedNodeData) {
        const selected = selectedNode; // Capture for type narrowing
        nodes.forEach((other) => {
          if (other.id === selected) return;

          const key = `${Math.min(selected, other.id)}-${Math.max(selected, other.id)}`;
          let quality: "healthy" | "suspect" | "dead" = "healthy";
          if (selectedNodeData.status === "dead" || other.status === "dead") {
            quality = "dead";
          } else if (selectedNodeData.status === "suspect" || other.status === "suspect") {
            quality = "suspect";
          }

          result.push({
            from: selected,
            to: other.id,
            quality,
            hasRecentEvent: recentEventPairs.has(key),
            eventKind: eventKindMap.get(key),
          });
        });
      }
    } else {
      // All connections (limit for performance)
      const maxConnections = 30;
      let count = 0;

      for (let i = 0; i < nodePositions.length && count < maxConnections; i++) {
        for (let j = i + 1; j < nodePositions.length && count < maxConnections; j++) {
          const fromNode = nodePositions[i].node;
          const toNode = nodePositions[j].node;
          const key = `${Math.min(fromNode.id, toNode.id)}-${Math.max(fromNode.id, toNode.id)}`;

          let quality: "healthy" | "suspect" | "dead" = "healthy";
          if (fromNode.status === "dead" || toNode.status === "dead") {
            quality = "dead";
          } else if (fromNode.status === "suspect" || toNode.status === "suspect") {
            quality = "suspect";
          }

          result.push({
            from: fromNode.id,
            to: toNode.id,
            quality,
            hasRecentEvent: recentEventPairs.has(key),
            eventKind: eventKindMap.get(key),
          });
          count++;
        }
      }
    }

    return result;
  }, [nodes, nodePositions, connectionMode, selectedNode, swimEvents]);

  // Get nodes with suspicion info
  const nodesWithSuspicion = useMemo(() => {
    const suspectedBy = new Map<number, Set<number>>();

    swimEvents.forEach((evt) => {
      if (evt.kind === "Suspect") {
        if (!suspectedBy.has(evt.to)) {
          suspectedBy.set(evt.to, new Set());
        }
        suspectedBy.get(evt.to)!.add(evt.from);
      }
    });

    return nodePositions.map((p) => ({
      ...p,
      suspectedBy: suspectedBy.get(p.node.id) ?? new Set<number>(),
    }));
  }, [nodePositions, swimEvents]);

  if (nodes.length === 0) {
    return (
      <div className="flex h-80 items-center justify-center rounded-lg border border-dashed border-border">
        <p className="text-sm text-muted-foreground">
          No nodes detected yet. Waiting for SWIM events...
        </p>
      </div>
    );
  }

  return (
    <div className="relative">
      {/* Connection mode toggle */}
      <div className="absolute top-0 right-0 z-10 flex rounded-lg bg-muted p-0.5">
        {(["ring", "selected", "all"] as const).map((mode) => (
          <button
            key={mode}
            onClick={() => setConnectionMode(mode)}
            className={cn(
              "rounded-md px-2 py-1 text-xs font-medium transition-colors",
              connectionMode === mode
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground"
            )}
          >
            {mode === "ring" ? "Ring" : mode === "selected" ? "Selected" : "All"}
          </button>
        ))}
      </div>

      <svg viewBox="0 0 300 300" className="h-80 w-full">
        {/* Connection lines */}
        <g>
          {connections.map((conn) => {
            const fromPos = positionMap.get(conn.from);
            const toPos = positionMap.get(conn.to);
            if (!fromPos || !toPos) return null;

            return (
              <ConnectionLine
                key={`${conn.from}-${conn.to}`}
                x1={fromPos.x}
                y1={fromPos.y}
                x2={toPos.x}
                y2={toPos.y}
                quality={conn.quality}
                hasRecentEvent={conn.hasRecentEvent}
                eventKind={conn.eventKind}
              />
            );
          })}
        </g>

        {/* Center indicator */}
        <circle
          cx="150"
          cy="150"
          r="30"
          fill="none"
          stroke="currentColor"
          strokeWidth="1"
          strokeDasharray="4 4"
          className="text-muted-foreground/30"
        />
        <text
          x="150"
          y="155"
          textAnchor="middle"
          className="fill-muted-foreground text-[10px]"
        >
          Cluster
        </text>

        {/* Node circles */}
        {nodesWithSuspicion.map(({ node, x, y, suspectedBy }) => (
          <NodeCircle
            key={node.id}
            node={node}
            x={x}
            y={y}
            isSelected={selectedNode === node.id}
            suspectedByCount={suspectedBy.size}
            onClick={() => onNodeSelect?.(node.id)}
          />
        ))}
      </svg>

      {/* Status legend */}
      <div className="mt-2 flex justify-center gap-4 text-xs">
        <div className="flex items-center gap-1">
          <div className="h-3 w-3 rounded-full bg-status-healthy" />
          <span className="text-muted-foreground">Alive</span>
        </div>
        <div className="flex items-center gap-1">
          <div className="h-3 w-3 rounded-full bg-status-warning" />
          <span className="text-muted-foreground">Suspect</span>
        </div>
        <div className="flex items-center gap-1">
          <div className="h-3 w-3 rounded-full bg-status-critical" />
          <span className="text-muted-foreground">Dead</span>
        </div>
        <div className="flex items-center gap-1">
          <div className="h-0.5 w-6 bg-muted-foreground" style={{ borderStyle: "dashed" }} />
          <span className="text-muted-foreground">Degraded</span>
        </div>
      </div>
    </div>
  );
}

interface ConnectionLineProps {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  quality: "healthy" | "suspect" | "dead";
  hasRecentEvent: boolean;
  eventKind?: "Suspect" | "Confirm" | "Alive";
}

function ConnectionLine({
  x1,
  y1,
  x2,
  y2,
  quality,
  hasRecentEvent,
  eventKind,
}: ConnectionLineProps) {
  const qualityStyles = {
    healthy: { stroke: "rgba(34, 197, 94, 0.4)", strokeDasharray: "none" },
    suspect: { stroke: "rgba(245, 158, 11, 0.6)", strokeDasharray: "4 4" },
    dead: { stroke: "rgba(239, 68, 68, 0.4)", strokeDasharray: "2 4" },
  };

  const style = qualityStyles[quality];

  // Calculate midpoint for event badge
  const midX = (x1 + x2) / 2;
  const midY = (y1 + y2) / 2;

  const eventColors = {
    Suspect: "#f59e0b",
    Confirm: "#ef4444",
    Alive: "#22c55e",
  };

  return (
    <g>
      <line
        x1={x1}
        y1={y1}
        x2={x2}
        y2={y2}
        stroke={style.stroke}
        strokeWidth="2"
        strokeDasharray={style.strokeDasharray}
        className="transition-all duration-300"
      />

      {/* Event badge */}
      <AnimatePresence>
        {hasRecentEvent && eventKind && (
          <motion.g
            initial={{ opacity: 0, scale: 0 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0 }}
          >
            <motion.circle
              cx={midX}
              cy={midY}
              r="6"
              fill={eventColors[eventKind]}
              initial={{ scale: 1 }}
              animate={{ scale: [1, 1.3, 1] }}
              transition={{ duration: 0.5, repeat: 3 }}
            />
            <text
              x={midX}
              y={midY + 1}
              textAnchor="middle"
              dominantBaseline="middle"
              className="fill-white text-[6px] font-bold"
            >
              {eventKind === "Suspect" ? "S" : eventKind === "Confirm" ? "C" : "A"}
            </text>
          </motion.g>
        )}
      </AnimatePresence>
    </g>
  );
}

interface NodeCircleProps {
  node: ClusterNode;
  x: number;
  y: number;
  isSelected: boolean;
  suspectedByCount: number;
  onClick: () => void;
}

function NodeCircle({ node, x, y, isSelected, suspectedByCount, onClick }: NodeCircleProps) {
  const statusColors = {
    alive: { fill: "#22c55e", stroke: "#16a34a", pulse: false },
    suspect: { fill: "#f59e0b", stroke: "#d97706", pulse: true },
    dead: { fill: "#ef4444", stroke: "#dc2626", pulse: false },
  };

  const { fill, stroke, pulse } = statusColors[node.status];

  // Show state history indicator
  const hasRecentStateChange = node.stateHistory && node.stateHistory.length > 1;

  return (
    <g
      className="cursor-pointer transition-transform hover:scale-110"
      onClick={onClick}
    >
      {/* Pulse animation for suspect nodes */}
      {pulse && (
        <motion.circle
          cx={x}
          cy={y}
          r="25"
          fill={fill}
          initial={{ opacity: 0.5, scale: 1 }}
          animate={{ opacity: 0, scale: 1.5 }}
          transition={{ duration: 1.5, repeat: Infinity }}
        />
      )}

      {/* Selection ring */}
      {isSelected && (
        <circle
          cx={x}
          cy={y}
          r="28"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          className="text-primary"
        />
      )}

      {/* Main circle */}
      <circle
        cx={x}
        cy={y}
        r="22"
        fill={fill}
        stroke={stroke}
        strokeWidth="2"
        className="transition-all"
      />

      {/* Node ID */}
      <text
        x={x}
        y={y + 1}
        textAnchor="middle"
        dominantBaseline="middle"
        className="fill-white text-xs font-bold"
      >
        N{node.id}
      </text>

      {/* Status badge */}
      <text
        x={x}
        y={y + 35}
        textAnchor="middle"
        className="fill-muted-foreground text-[9px] capitalize"
      >
        {node.status}
      </text>

      {/* Suspicion count badge */}
      {suspectedByCount > 0 && node.status !== "dead" && (
        <g>
          <circle
            cx={x + 18}
            cy={y - 14}
            r="8"
            fill="#ef4444"
          />
          <text
            x={x + 18}
            y={y - 13}
            textAnchor="middle"
            dominantBaseline="middle"
            className="fill-white text-[8px] font-bold"
          >
            {suspectedByCount}
          </text>
        </g>
      )}

      {/* State change indicator */}
      {hasRecentStateChange && (
        <motion.circle
          cx={x - 18}
          cy={y - 14}
          r="4"
          fill="#3b82f6"
          initial={{ opacity: 0 }}
          animate={{ opacity: [0, 1, 0] }}
          transition={{ duration: 2, repeat: Infinity }}
        />
      )}
    </g>
  );
}
