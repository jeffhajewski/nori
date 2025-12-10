"use client";

import { useState, useEffect } from "react";
import { useEventStore, useLsmNodeState } from "@/stores/eventStore";
import {
  LsmHeatmap,
  WritePressureGauge,
  CompactionPanel,
  NodeSelector,
} from "@/components/storage";

export default function StoragePage() {
  const [selectedNode, setSelectedNode] = useState<number | null>(null);
  const lsmStates = useEventStore((s) => s.lsmStates);

  // Auto-select first node when data arrives
  useEffect(() => {
    if (selectedNode === null && lsmStates.size > 0) {
      const firstNode = Array.from(lsmStates.keys()).sort((a, b) => a - b)[0];
      setSelectedNode(firstNode);
    }
  }, [selectedNode, lsmStates.size]);

  const nodeState = selectedNode !== null ? lsmStates.get(selectedNode) : undefined;

  return (
    <div className="space-y-6">
      {/* Page header */}
      <div>
        <h1 className="text-2xl font-semibold text-foreground">Storage</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          LSM tree visualization, compaction activity, and write pressure monitoring
        </p>
      </div>

      {/* Node selector */}
      <div className="rounded-xl border border-border bg-card p-4">
        <h2 className="mb-3 text-sm font-medium text-muted-foreground">Select Node</h2>
        <NodeSelector selectedNode={selectedNode} onSelectNode={setSelectedNode} />
      </div>

      {selectedNode !== null && nodeState ? (
        <div className="grid gap-6 lg:grid-cols-3">
          {/* Main content - LSM Heatmap */}
          <div className="lg:col-span-2 space-y-6">
            {/* LSM Heatmap */}
            <div className="rounded-xl border border-border bg-card p-6">
              <div className="mb-4">
                <h2 className="text-lg font-medium text-foreground">
                  LSM Slot Heat - Node {selectedNode}
                </h2>
                <p className="mt-1 text-sm text-muted-foreground">
                  Heat distribution across LSM levels. Hotter slots indicate more frequent access.
                </p>
              </div>
              <LsmHeatmap
                nodeState={nodeState}
                onSlotHover={(level, slot, heat) => {
                  // Could show tooltip or update info panel
                }}
              />
            </div>

            {/* Compaction Panel */}
            <div className="rounded-xl border border-border bg-card p-6">
              <div className="mb-4">
                <h2 className="text-lg font-medium text-foreground">Compaction Activity</h2>
                <p className="mt-1 text-sm text-muted-foreground">
                  Active compactions and recently completed operations
                </p>
              </div>
              <CompactionPanel nodeId={selectedNode} />
            </div>
          </div>

          {/* Sidebar - Write Pressure */}
          <div className="space-y-6">
            {/* Write Pressure Gauge */}
            <div className="rounded-xl border border-border bg-card p-6">
              <h2 className="mb-4 text-lg font-medium text-foreground">Write Pressure</h2>
              <WritePressureGauge
                pressure={nodeState.writePressure}
                isHigh={nodeState.writePressureHigh}
                threshold={0.5}
                l0FileCount={nodeState.l0FileCount}
                l0Threshold={nodeState.l0Threshold}
              />
            </div>

            {/* Level Stats */}
            <div className="rounded-xl border border-border bg-card p-6">
              <h2 className="mb-4 text-lg font-medium text-foreground">Level Statistics</h2>
              <LevelStats nodeState={nodeState} />
            </div>
          </div>
        </div>
      ) : (
        <div className="flex h-64 items-center justify-center rounded-xl border border-dashed border-border">
          <div className="text-center">
            <p className="text-lg font-medium text-muted-foreground">No Node Selected</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Select a node above to view its storage state
            </p>
          </div>
        </div>
      )}
    </div>
  );
}

function LevelStats({
  nodeState,
}: {
  nodeState: {
    levels: Map<number, { level: number; slots: Map<number, { heat: number }> }>;
  };
}) {
  const levels = Array.from(nodeState.levels.entries())
    .sort(([a], [b]) => a - b)
    .map(([level, state]) => {
      const slots = Array.from(state.slots.values());
      const avgHeat = slots.length > 0
        ? slots.reduce((sum, s) => sum + s.heat, 0) / slots.length
        : 0;
      const maxHeat = slots.length > 0
        ? Math.max(...slots.map((s) => s.heat))
        : 0;
      const hotSlots = slots.filter((s) => s.heat > 0.7).length;

      return { level, slotCount: slots.length, avgHeat, maxHeat, hotSlots };
    });

  const levelColors = [
    "bg-red-500",
    "bg-orange-500",
    "bg-amber-500",
    "bg-yellow-500",
    "bg-green-500",
    "bg-teal-500",
    "bg-blue-500",
  ];

  if (levels.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">No level data available</p>
    );
  }

  return (
    <div className="space-y-3">
      {levels.map(({ level, slotCount, avgHeat, maxHeat, hotSlots }) => (
        <div key={level} className="space-y-1">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <div className={`h-3 w-3 rounded ${levelColors[Math.min(level, 6)]}`} />
              <span className="text-sm font-medium text-foreground">L{level}</span>
            </div>
            <span className="text-xs text-muted-foreground">{slotCount} slots</span>
          </div>
          <div className="grid grid-cols-3 gap-2 text-xs">
            <div className="rounded bg-muted/50 px-2 py-1">
              <p className="text-muted-foreground">Avg</p>
              <p className="font-medium text-foreground">{(avgHeat * 100).toFixed(0)}%</p>
            </div>
            <div className="rounded bg-muted/50 px-2 py-1">
              <p className="text-muted-foreground">Max</p>
              <p className="font-medium text-foreground">{(maxHeat * 100).toFixed(0)}%</p>
            </div>
            <div className="rounded bg-muted/50 px-2 py-1">
              <p className="text-muted-foreground">Hot</p>
              <p className="font-medium text-foreground">{hotSlots}</p>
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}
