"use client";

import { useRef, useEffect, useMemo } from "react";
import { motion } from "framer-motion";
import { useEventStore, type ClusterNode } from "@/stores/eventStore";
import { cn } from "@/lib/utils";

interface ClusterTopologyProps {
  onNodeSelect?: (nodeId: number) => void;
  selectedNode?: number | null;
}

export function ClusterTopology({ onNodeSelect, selectedNode }: ClusterTopologyProps) {
  const nodes = useEventStore((s) => Array.from(s.nodes.values()));

  // Calculate positions in a ring layout
  const nodePositions = useMemo(() => {
    const centerX = 150;
    const centerY = 150;
    const radius = 100;

    return nodes.map((node, index) => {
      const angle = (index / nodes.length) * 2 * Math.PI - Math.PI / 2;
      return {
        node,
        x: centerX + radius * Math.cos(angle),
        y: centerY + radius * Math.sin(angle),
      };
    });
  }, [nodes]);

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
      <svg viewBox="0 0 300 300" className="h-80 w-full">
        {/* Connection lines between all nodes */}
        <g className="opacity-20">
          {nodePositions.map((pos1, i) =>
            nodePositions.slice(i + 1).map((pos2, j) => (
              <line
                key={`${pos1.node.id}-${pos2.node.id}`}
                x1={pos1.x}
                y1={pos1.y}
                x2={pos2.x}
                y2={pos2.y}
                stroke="currentColor"
                strokeWidth="1"
                className="text-muted-foreground"
              />
            ))
          )}
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
        {nodePositions.map(({ node, x, y }) => (
          <NodeCircle
            key={node.id}
            node={node}
            x={x}
            y={y}
            isSelected={selectedNode === node.id}
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
      </div>
    </div>
  );
}

interface NodeCircleProps {
  node: ClusterNode;
  x: number;
  y: number;
  isSelected: boolean;
  onClick: () => void;
}

function NodeCircle({ node, x, y, isSelected, onClick }: NodeCircleProps) {
  const statusColors = {
    alive: { fill: "#22c55e", stroke: "#16a34a", pulse: false },
    suspect: { fill: "#f59e0b", stroke: "#d97706", pulse: true },
    dead: { fill: "#ef4444", stroke: "#dc2626", pulse: false },
  };

  const { fill, stroke, pulse } = statusColors[node.status];

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
    </g>
  );
}
