"use client";

import { useMemo } from "react";
import { motion } from "framer-motion";
import { useEventStore, type ShardState } from "@/stores/eventStore";
import { cn } from "@/lib/utils";

interface ShardDetailPanelProps {
  shardId: number;
  onClose: () => void;
}

export function ShardDetailPanel({ shardId, onClose }: ShardDetailPanelProps) {
  const shard = useEventStore((s) => s.shards.get(shardId));
  const nodes = useEventStore((s) => s.nodes);

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
      className="rounded-xl border border-border bg-card p-6"
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

      {/* Leader status */}
      <div className="mt-4 rounded-lg bg-muted/50 p-4">
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

      {/* Replica set */}
      <div className="mt-4">
        <p className="text-xs font-medium uppercase text-muted-foreground">
          Replica Set ({replicas.length} nodes)
        </p>
        <div className="mt-2 flex flex-wrap gap-2">
          {replicas.length > 0 ? (
            replicas.sort((a, b) => a - b).map((nodeId) => {
              const node = nodes.get(nodeId);
              const isLeader = nodeId === leader;

              return (
                <div
                  key={nodeId}
                  className={cn(
                    "flex items-center gap-2 rounded-lg border px-3 py-2",
                    isLeader
                      ? "border-status-healthy bg-status-healthy/10"
                      : "border-border bg-muted/30"
                  )}
                >
                  <div
                    className={cn(
                      "h-2 w-2 rounded-full",
                      node?.status === "alive" && "bg-status-healthy",
                      node?.status === "suspect" && "bg-status-warning",
                      node?.status === "dead" && "bg-status-critical",
                      !node && "bg-muted-foreground"
                    )}
                  />
                  <span className="text-sm font-medium text-foreground">
                    Node {nodeId}
                  </span>
                  {isLeader && (
                    <span className="rounded bg-status-healthy/20 px-1.5 py-0.5 text-xs text-status-healthy">
                      Leader
                    </span>
                  )}
                </div>
              );
            })
          ) : (
            <p className="text-sm text-muted-foreground">No replicas detected yet</p>
          )}
        </div>
      </div>

      {/* Quick stats */}
      <div className="mt-4 grid grid-cols-2 gap-3">
        <div className="rounded-lg bg-muted/30 p-3">
          <p className="text-xs text-muted-foreground">Current Term</p>
          <p className="text-xl font-bold text-foreground">{shard.term}</p>
        </div>
        <div className="rounded-lg bg-muted/30 p-3">
          <p className="text-xs text-muted-foreground">Replicas</p>
          <p className="text-xl font-bold text-foreground">{replicas.length}</p>
        </div>
      </div>
    </motion.div>
  );
}
