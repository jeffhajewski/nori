"use client";

import { cn } from "@/lib/utils";
import { useEventStore } from "@/stores/eventStore";

interface NodeSelectorProps {
  selectedNode: number | null;
  onSelectNode: (nodeId: number) => void;
}

export function NodeSelector({ selectedNode, onSelectNode }: NodeSelectorProps) {
  const nodes = useEventStore((s) => Array.from(s.nodes.values()));
  const lsmStates = useEventStore((s) => s.lsmStates);

  // Get all nodes that have LSM data
  const nodesWithLsm = Array.from(lsmStates.keys());

  // Combine both sources
  const allNodeIds = Array.from(
    new Set([...nodes.map((n) => n.id), ...nodesWithLsm])
  ).sort((a, b) => a - b);

  if (allNodeIds.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-border p-4 text-center text-sm text-muted-foreground">
        No nodes detected yet
      </div>
    );
  }

  return (
    <div className="flex flex-wrap gap-2">
      {allNodeIds.map((nodeId) => {
        const node = nodes.find((n) => n.id === nodeId);
        const hasLsm = lsmStates.has(nodeId);
        const lsmState = lsmStates.get(nodeId);
        const isSelected = selectedNode === nodeId;

        return (
          <button
            key={nodeId}
            onClick={() => onSelectNode(nodeId)}
            className={cn(
              "relative flex flex-col items-center rounded-lg border px-4 py-2 transition-all",
              isSelected
                ? "border-primary bg-primary/10 ring-2 ring-primary/30"
                : "border-border bg-card hover:border-primary/50"
            )}
          >
            <span className="text-lg font-semibold text-foreground">Node {nodeId}</span>
            <div className="mt-1 flex items-center gap-2">
              {/* Node status */}
              {node && (
                <span
                  className={cn(
                    "h-2 w-2 rounded-full",
                    node.status === "alive" && "bg-status-healthy",
                    node.status === "suspect" && "bg-status-warning",
                    node.status === "dead" && "bg-status-critical"
                  )}
                  title={`Status: ${node.status}`}
                />
              )}
              {/* Write pressure indicator */}
              {lsmState && (
                <span
                  className={cn(
                    "text-xs",
                    lsmState.writePressureHigh
                      ? "text-status-critical"
                      : lsmState.writePressure > 0.5
                      ? "text-status-warning"
                      : "text-muted-foreground"
                  )}
                >
                  {(lsmState.writePressure * 100).toFixed(0)}% pressure
                </span>
              )}
            </div>
          </button>
        );
      })}
    </div>
  );
}
