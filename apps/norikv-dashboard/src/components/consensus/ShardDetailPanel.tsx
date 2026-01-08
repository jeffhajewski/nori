"use client";

import { useMemo } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useEventStore, useShardsWithStability, type ShardState, type TermHistoryEntry } from "@/stores/eventStore";
import { cn, formatDuration, formatTimestamp } from "@/lib/utils";

interface ShardDetailPanelProps {
  shardId: number;
  onClose: () => void;
}

// Selector for snapshot state
const useShardSnapshot = (shardId: number) =>
  useEventStore((s) => s.shardSnapshots.get(shardId));

export function ShardDetailPanel({ shardId, onClose }: ShardDetailPanelProps) {
  const shard = useEventStore((s) => s.shards.get(shardId));
  const nodes = useEventStore((s) => s.nodes);
  const snapshot = useShardSnapshot(shardId);

  // Get stability info
  const shardsWithStability = useShardsWithStability();
  const shardStability = useMemo(() => {
    return shardsWithStability.find((s) => s.id === shardId);
  }, [shardsWithStability, shardId]);

  // Simulate replica lag based on node status (real implementation would come from Raft log index)
  const replicaLagData = useMemo(() => {
    if (!shard) return [];

    return Array.from(shard.replicas).map((nodeId) => {
      const node = nodes.get(nodeId);
      const isLeader = nodeId === shard.leader;

      // Simulate lag: leaders have 0 lag, followers have varying lag based on status
      let lagMs = 0;
      let lagStatus: "synced" | "close" | "behind" | "far_behind" = "synced";

      if (!isLeader) {
        if (node?.status === "suspect") {
          lagMs = 5000 + Math.random() * 10000; // 5-15s lag
          lagStatus = "far_behind";
        } else if (node?.status === "dead") {
          lagMs = 30000 + Math.random() * 30000; // 30-60s lag
          lagStatus = "far_behind";
        } else {
          // Simulate small lag for healthy followers
          lagMs = Math.random() * 500;
          lagStatus = lagMs < 100 ? "synced" : lagMs < 500 ? "close" : "behind";
        }
      }

      return {
        nodeId,
        isLeader,
        status: node?.status ?? "unknown",
        lagMs,
        lagStatus,
      };
    }).sort((a, b) => {
      // Leader first, then by node ID
      if (a.isLeader) return -1;
      if (b.isLeader) return 1;
      return a.nodeId - b.nodeId;
    });
  }, [shard, nodes]);

  if (!shard) {
    return (
      <div className="rounded-xl border border-border bg-card p-6">
        <p className="text-sm text-muted-foreground">Shard {shardId} not found</p>
      </div>
    );
  }

  const replicas = Array.from(shard.replicas);
  const leader = shard.leader;

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 20 }}
      className="rounded-xl border border-border bg-card p-6 space-y-4"
    >
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-lg font-semibold text-foreground">Shard {shardId}</h3>
          <p className="text-sm text-muted-foreground">Term {shard.term}</p>
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

      {/* Stability indicator */}
      {shardStability && (
        <div
          className={cn(
            "flex items-center justify-between rounded-lg px-3 py-2 text-sm",
            shardStability.recentElection
              ? "bg-status-critical/10 text-status-critical"
              : shardStability.isStable
              ? "bg-status-healthy/10 text-status-healthy"
              : "bg-status-warning/10 text-status-warning"
          )}
        >
          <span className="font-medium">
            {shardStability.recentElection
              ? "Recent Election"
              : shardStability.isStable
              ? "Stable"
              : "Recovering"}
          </span>
          <span>
            {shardStability.recentElection
              ? `${formatDuration(shardStability.stabilityMs)} ago`
              : `for ${formatDuration(shardStability.stabilityMs)}`}
          </span>
        </div>
      )}

      {/* Leader status */}
      <div className="rounded-lg bg-muted/50 p-4">
        <p className="text-xs font-medium uppercase text-muted-foreground">Current Leader</p>
        {leader !== null ? (
          <div className="mt-2 flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-full bg-status-healthy/20">
              <svg className="h-5 w-5 text-status-healthy" fill="currentColor" viewBox="0 0 20 20">
                <path d="M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z" />
              </svg>
            </div>
            <div>
              <p className="text-lg font-semibold text-foreground">Node {leader}</p>
              <p className="text-sm text-status-healthy">Active Leader</p>
            </div>
          </div>
        ) : (
          <div className="mt-2 flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-full bg-status-warning/20">
              <svg className="h-5 w-5 text-status-warning" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
              </svg>
            </div>
            <div>
              <p className="text-lg font-semibold text-status-warning">No Leader</p>
              <p className="text-sm text-muted-foreground">Election in progress</p>
            </div>
          </div>
        )}
      </div>

      {/* Snapshot progress */}
      <AnimatePresence>
        {snapshot && snapshot.status === "in_progress" && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            className="rounded-lg border border-status-warning/50 bg-status-warning/10 p-4"
          >
            <div className="flex items-center justify-between mb-2">
              <p className="text-sm font-medium text-status-warning">Snapshot in Progress</p>
              {snapshot.startedAt && (
                <span className="text-xs text-muted-foreground">
                  Started {formatDuration(Date.now() - snapshot.startedAt)} ago
                </span>
              )}
            </div>
            <div className="h-2 overflow-hidden rounded-full bg-muted">
              <motion.div
                className="h-full bg-status-warning"
                initial={{ width: 0 }}
                animate={{ width: "100%" }}
                transition={{ duration: 60, ease: "linear", repeat: Infinity }}
              />
            </div>
            {snapshot.snapshotId && (
              <p className="mt-2 text-xs text-muted-foreground font-mono">
                ID: {snapshot.snapshotId}
              </p>
            )}
          </motion.div>
        )}
      </AnimatePresence>

      {/* Replica lag indicators */}
      <div>
        <p className="text-xs font-medium uppercase text-muted-foreground mb-2">
          Replica Set ({replicas.length} nodes)
        </p>
        <div className="space-y-2">
          {replicaLagData.map((replica) => (
            <ReplicaLagRow key={replica.nodeId} replica={replica} />
          ))}
          {replicaLagData.length === 0 && (
            <p className="text-sm text-muted-foreground">No replicas detected yet</p>
          )}
        </div>
      </div>

      {/* Election history */}
      {shard.termHistory.length > 0 && (
        <div>
          <p className="text-xs font-medium uppercase text-muted-foreground mb-2">
            Recent Elections
          </p>
          <div className="space-y-1">
            {shard.termHistory.slice(-5).reverse().map((entry, i) => (
              <div
                key={`${entry.term}-${entry.timestamp}`}
                className="flex items-center justify-between rounded bg-muted/30 px-3 py-1.5 text-xs"
              >
                <div className="flex items-center gap-2">
                  <span className="font-medium text-foreground">Term {entry.term}</span>
                  {entry.leader !== null ? (
                    <span className="text-status-healthy">
                      Node {entry.leader} elected
                    </span>
                  ) : (
                    <span className="text-status-warning">No leader</span>
                  )}
                </div>
                <span className="text-muted-foreground">
                  {formatTimestamp(entry.timestamp)}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Quick stats */}
      <div className="grid grid-cols-3 gap-3">
        <div className="rounded-lg bg-muted/30 p-3 text-center">
          <p className="text-xs text-muted-foreground">Term</p>
          <p className="text-xl font-bold text-foreground">{shard.term}</p>
        </div>
        <div className="rounded-lg bg-muted/30 p-3 text-center">
          <p className="text-xs text-muted-foreground">Replicas</p>
          <p className="text-xl font-bold text-foreground">{replicas.length}</p>
        </div>
        <div className="rounded-lg bg-muted/30 p-3 text-center">
          <p className="text-xs text-muted-foreground">Elections</p>
          <p className="text-xl font-bold text-foreground">
            {shardStability?.electionCount ?? shard.termHistory.length}
          </p>
        </div>
      </div>
    </motion.div>
  );
}

interface ReplicaLagRowProps {
  replica: {
    nodeId: number;
    isLeader: boolean;
    status: string;
    lagMs: number;
    lagStatus: "synced" | "close" | "behind" | "far_behind";
  };
}

function ReplicaLagRow({ replica }: ReplicaLagRowProps) {
  const lagColors = {
    synced: {
      bar: "bg-status-healthy",
      text: "text-status-healthy",
      label: "Synced",
    },
    close: {
      bar: "bg-status-healthy",
      text: "text-status-healthy",
      label: "Close",
    },
    behind: {
      bar: "bg-status-warning",
      text: "text-status-warning",
      label: "Behind",
    },
    far_behind: {
      bar: "bg-status-critical",
      text: "text-status-critical",
      label: "Far Behind",
    },
  };

  const config = lagColors[replica.lagStatus];

  // Calculate bar width (0-100%)
  const maxLag = 30000; // 30 seconds
  const lagPercent = Math.min(100, (replica.lagMs / maxLag) * 100);

  return (
    <div
      className={cn(
        "flex items-center gap-3 rounded-lg border px-3 py-2",
        replica.isLeader
          ? "border-status-healthy bg-status-healthy/10"
          : "border-border bg-muted/30"
      )}
    >
      {/* Status dot */}
      <div
        className={cn(
          "h-2.5 w-2.5 rounded-full",
          replica.status === "alive" && "bg-status-healthy",
          replica.status === "suspect" && "bg-status-warning",
          replica.status === "dead" && "bg-status-critical",
          replica.status === "unknown" && "bg-muted-foreground"
        )}
      />

      {/* Node info */}
      <div className="flex-1">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-foreground">
            Node {replica.nodeId}
          </span>
          {replica.isLeader && (
            <span className="rounded bg-status-healthy/20 px-1.5 py-0.5 text-xs text-status-healthy">
              Leader
            </span>
          )}
        </div>

        {/* Lag bar (only for followers) */}
        {!replica.isLeader && (
          <div className="mt-1 flex items-center gap-2">
            <div className="flex-1 h-1.5 rounded-full bg-muted overflow-hidden">
              <motion.div
                className={cn("h-full", config.bar)}
                initial={{ width: 0 }}
                animate={{ width: `${lagPercent}%` }}
                transition={{ duration: 0.3 }}
              />
            </div>
            <span className={cn("text-xs", config.text)}>
              {replica.lagMs < 1000
                ? `${Math.round(replica.lagMs)}ms`
                : `${(replica.lagMs / 1000).toFixed(1)}s`}
            </span>
          </div>
        )}
      </div>

      {/* Status label */}
      {!replica.isLeader && (
        <span className={cn("text-xs font-medium", config.text)}>
          {config.label}
        </span>
      )}
    </div>
  );
}
