"use client";

import { useMemo } from "react";
import { motion } from "framer-motion";
import { useEventStore, type ClusterNode } from "@/stores/eventStore";
import { formatTimestamp } from "@/lib/utils";
import { cn } from "@/lib/utils";

interface NodeDetailCardProps {
  nodeId: number;
  onClose: () => void;
}

export function NodeDetailCard({ nodeId, onClose }: NodeDetailCardProps) {
  const node = useEventStore((s) => s.nodes.get(nodeId));
  const shards = useEventStore((s) => Array.from(s.shards.values()));
  const lsmState = useEventStore((s) => s.lsmStates.get(nodeId));

  // Get shards this node is leader of
  const leaderShards = useMemo(() => {
    return shards.filter((s) => s.leader === nodeId);
  }, [shards, nodeId]);

  // Get shards this node is a replica of
  const replicaShards = useMemo(() => {
    return shards.filter((s) => s.replicas.has(nodeId) && s.leader !== nodeId);
  }, [shards, nodeId]);

  if (!node) {
    return (
      <div className="rounded-xl border border-border bg-card p-6">
        <p className="text-sm text-muted-foreground">Node {nodeId} not found</p>
      </div>
    );
  }

  const statusConfig = {
    alive: {
      color: "text-status-healthy",
      bg: "bg-status-healthy/10",
      border: "border-status-healthy/30",
      label: "Healthy",
      icon: (
        <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
        </svg>
      ),
    },
    suspect: {
      color: "text-status-warning",
      bg: "bg-status-warning/10",
      border: "border-status-warning/30",
      label: "Suspect",
      icon: (
        <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
        </svg>
      ),
    },
    dead: {
      color: "text-status-critical",
      bg: "bg-status-critical/10",
      border: "border-status-critical/30",
      label: "Dead",
      icon: (
        <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
        </svg>
      ),
    },
  };

  const status = statusConfig[node.status];

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 20 }}
      className="rounded-xl border border-border bg-card p-6"
    >
      {/* Header */}
      <div className="flex items-start justify-between">
        <div className="flex items-center gap-3">
          <div className={cn("flex h-12 w-12 items-center justify-center rounded-full", status.bg)}>
            <span className={cn("text-lg font-bold", status.color)}>N{nodeId}</span>
          </div>
          <div>
            <h3 className="text-lg font-semibold text-foreground">Node {nodeId}</h3>
            <div className={cn("flex items-center gap-1", status.color)}>
              {status.icon}
              <span className="text-sm font-medium">{status.label}</span>
            </div>
          </div>
        </div>
        <button
          onClick={onClose}
          className="rounded-lg p-2 text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      {/* Last seen */}
      <div className="mt-4 rounded-lg bg-muted/30 p-3">
        <p className="text-xs text-muted-foreground">Last Seen</p>
        <p className="text-sm font-medium text-foreground">
          {formatTimestamp(node.lastSeen)}
        </p>
      </div>

      {/* Shard assignments */}
      <div className="mt-4 grid grid-cols-2 gap-4">
        <div className="rounded-lg bg-muted/30 p-3">
          <p className="text-xs text-muted-foreground">Leader Of</p>
          <p className="text-2xl font-bold text-foreground">{leaderShards.length}</p>
          <p className="text-xs text-muted-foreground">shards</p>
        </div>
        <div className="rounded-lg bg-muted/30 p-3">
          <p className="text-xs text-muted-foreground">Replica Of</p>
          <p className="text-2xl font-bold text-foreground">{replicaShards.length}</p>
          <p className="text-xs text-muted-foreground">shards</p>
        </div>
      </div>

      {/* Leader shards list */}
      {leaderShards.length > 0 && (
        <div className="mt-4">
          <p className="text-xs font-medium uppercase text-muted-foreground">Leading Shards</p>
          <div className="mt-2 flex flex-wrap gap-1">
            {leaderShards.slice(0, 20).map((shard) => (
              <span
                key={shard.id}
                className="rounded bg-primary/20 px-2 py-0.5 text-xs font-medium text-primary"
              >
                S{shard.id}
              </span>
            ))}
            {leaderShards.length > 20 && (
              <span className="text-xs text-muted-foreground">
                +{leaderShards.length - 20} more
              </span>
            )}
          </div>
        </div>
      )}

      {/* Write pressure (if LSM data available) */}
      {lsmState && (
        <div className="mt-4 rounded-lg border border-border p-3">
          <div className="flex items-center justify-between">
            <p className="text-xs text-muted-foreground">Write Pressure</p>
            <span
              className={cn(
                "text-sm font-medium",
                lsmState.writePressureHigh
                  ? "text-status-critical"
                  : lsmState.writePressure > 0.5
                  ? "text-status-warning"
                  : "text-status-healthy"
              )}
            >
              {(lsmState.writePressure * 100).toFixed(0)}%
            </span>
          </div>
          <div className="mt-2 h-2 overflow-hidden rounded-full bg-muted">
            <div
              className={cn(
                "h-full transition-all",
                lsmState.writePressureHigh
                  ? "bg-status-critical"
                  : lsmState.writePressure > 0.5
                  ? "bg-status-warning"
                  : "bg-status-healthy"
              )}
              style={{ width: `${Math.min(lsmState.writePressure * 100, 100)}%` }}
            />
          </div>
        </div>
      )}
    </motion.div>
  );
}
