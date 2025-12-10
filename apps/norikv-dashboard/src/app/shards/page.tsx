"use client";

import { useState, useMemo } from "react";
import { motion } from "framer-motion";
import { useEventStore } from "@/stores/eventStore";
import { cn } from "@/lib/utils";

export default function ShardsPage() {
  const [selectedShard, setSelectedShard] = useState<number | null>(null);
  const [viewMode, setViewMode] = useState<"grid" | "list">("grid");
  const shards = useEventStore((s) => Array.from(s.shards.values()));
  const nodes = useEventStore((s) => s.nodes);

  const sortedShards = useMemo(() => {
    return [...shards].sort((a, b) => a.id - b.id);
  }, [shards]);

  // Group by leader
  const shardsByLeader = useMemo(() => {
    const map = new Map<number | null, typeof shards>();
    for (const shard of shards) {
      const arr = map.get(shard.leader) || [];
      arr.push(shard);
      map.set(shard.leader, arr);
    }
    return map;
  }, [shards]);

  return (
    <div className="space-y-6">
      {/* Page header */}
      <div className="flex items-start justify-between">
        <div>
          <h1 className="text-2xl font-semibold text-foreground">Shards</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Shard placement and replica distribution
          </p>
        </div>
        <div className="flex gap-1 rounded-lg bg-muted p-1">
          <button
            onClick={() => setViewMode("grid")}
            className={cn(
              "rounded px-3 py-1 text-sm transition-colors",
              viewMode === "grid" ? "bg-background text-foreground shadow" : "text-muted-foreground"
            )}
          >
            Grid
          </button>
          <button
            onClick={() => setViewMode("list")}
            className={cn(
              "rounded px-3 py-1 text-sm transition-colors",
              viewMode === "list" ? "bg-background text-foreground shadow" : "text-muted-foreground"
            )}
          >
            List
          </button>
        </div>
      </div>

      {/* Stats */}
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <StatCard title="Total Shards" value={shards.length} />
        <StatCard title="With Leader" value={shards.filter((s) => s.leader !== null).length} />
        <StatCard title="Leaderless" value={shards.filter((s) => s.leader === null).length} warning />
        <StatCard title="Avg Replicas" value={shards.length > 0 ? (shards.reduce((sum, s) => sum + s.replicas.size, 0) / shards.length).toFixed(1) : "0"} />
      </div>

      {/* Shards by leader view */}
      <div className="rounded-xl border border-border bg-card p-6">
        <h2 className="mb-4 text-lg font-medium text-foreground">Shards by Leader</h2>
        <div className="space-y-4">
          {Array.from(shardsByLeader.entries())
            .sort(([a], [b]) => (a ?? 999) - (b ?? 999))
            .map(([leader, leaderShards]) => (
              <div key={leader ?? "none"} className="rounded-lg border border-border p-4">
                <div className="flex items-center justify-between mb-3">
                  <div className="flex items-center gap-2">
                    {leader !== null ? (
                      <>
                        <div className={cn(
                          "h-3 w-3 rounded-full",
                          nodes.get(leader)?.status === "alive" ? "bg-status-healthy" :
                          nodes.get(leader)?.status === "suspect" ? "bg-status-warning" : "bg-status-critical"
                        )} />
                        <span className="font-medium text-foreground">Node {leader}</span>
                      </>
                    ) : (
                      <>
                        <div className="h-3 w-3 rounded-full bg-status-warning animate-pulse" />
                        <span className="font-medium text-status-warning">No Leader</span>
                      </>
                    )}
                  </div>
                  <span className="text-sm text-muted-foreground">
                    {leaderShards.length} shard{leaderShards.length !== 1 ? "s" : ""}
                  </span>
                </div>
                <div className="flex flex-wrap gap-2">
                  {leaderShards.sort((a, b) => a.id - b.id).map((shard) => (
                    <button
                      key={shard.id}
                      onClick={() => setSelectedShard(shard.id === selectedShard ? null : shard.id)}
                      className={cn(
                        "rounded-lg border px-3 py-1.5 text-sm font-medium transition-all",
                        selectedShard === shard.id
                          ? "border-primary bg-primary/10 text-primary"
                          : "border-border bg-muted/30 text-foreground hover:border-primary/50"
                      )}
                    >
                      S{shard.id}
                    </button>
                  ))}
                </div>
              </div>
            ))}
        </div>
      </div>

      {/* Selected shard detail */}
      {selectedShard !== null && (
        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          className="rounded-xl border border-border bg-card p-6"
        >
          <div className="flex items-start justify-between">
            <h2 className="text-lg font-medium text-foreground">Shard {selectedShard} Details</h2>
            <button
              onClick={() => setSelectedShard(null)}
              className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
            >
              <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>
          <ShardDetail shardId={selectedShard} />
        </motion.div>
      )}

      {/* Full grid view */}
      {viewMode === "grid" && (
        <div className="rounded-xl border border-border bg-card p-6">
          <h2 className="mb-4 text-lg font-medium text-foreground">All Shards</h2>
          <div className="grid grid-cols-8 gap-2 sm:grid-cols-12 md:grid-cols-16 lg:grid-cols-20">
            {sortedShards.map((shard) => (
              <button
                key={shard.id}
                onClick={() => setSelectedShard(shard.id)}
                className={cn(
                  "aspect-square rounded-lg border-2 p-1 text-center text-xs font-medium transition-all hover:scale-105",
                  shard.leader !== null
                    ? "border-primary/30 bg-primary/10 text-foreground"
                    : "border-status-warning/30 bg-status-warning/10 text-status-warning animate-pulse",
                  selectedShard === shard.id && "ring-2 ring-primary"
                )}
              >
                {shard.id}
              </button>
            ))}
            {sortedShards.length === 0 && (
              <p className="col-span-full text-sm text-muted-foreground">No shards detected yet</p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function StatCard({ title, value, warning = false }: { title: string; value: number | string; warning?: boolean }) {
  return (
    <div className={cn(
      "rounded-xl border bg-card p-4",
      warning && Number(value) > 0 ? "border-status-warning" : "border-border"
    )}>
      <p className="text-sm text-muted-foreground">{title}</p>
      <p className={cn(
        "mt-1 text-2xl font-bold",
        warning && Number(value) > 0 ? "text-status-warning" : "text-foreground"
      )}>
        {value}
      </p>
    </div>
  );
}

function ShardDetail({ shardId }: { shardId: number }) {
  const shard = useEventStore((s) => s.shards.get(shardId));
  const nodes = useEventStore((s) => s.nodes);

  if (!shard) {
    return <p className="text-sm text-muted-foreground">Shard not found</p>;
  }

  const replicas = Array.from(shard.replicas).sort((a, b) => a - b);

  return (
    <div className="mt-4 space-y-4">
      <div className="grid grid-cols-3 gap-4">
        <div className="rounded-lg bg-muted/30 p-3">
          <p className="text-xs text-muted-foreground">Leader</p>
          <p className="text-lg font-bold text-foreground">
            {shard.leader !== null ? `Node ${shard.leader}` : "None"}
          </p>
        </div>
        <div className="rounded-lg bg-muted/30 p-3">
          <p className="text-xs text-muted-foreground">Term</p>
          <p className="text-lg font-bold text-foreground">{shard.term}</p>
        </div>
        <div className="rounded-lg bg-muted/30 p-3">
          <p className="text-xs text-muted-foreground">Replicas</p>
          <p className="text-lg font-bold text-foreground">{replicas.length}</p>
        </div>
      </div>

      <div>
        <p className="text-xs font-medium uppercase text-muted-foreground mb-2">Replica Set</p>
        <div className="flex flex-wrap gap-2">
          {replicas.map((nodeId) => {
            const node = nodes.get(nodeId);
            const isLeader = nodeId === shard.leader;
            return (
              <div
                key={nodeId}
                className={cn(
                  "flex items-center gap-2 rounded-lg border px-3 py-2",
                  isLeader ? "border-primary bg-primary/10" : "border-border bg-muted/30"
                )}
              >
                <div className={cn(
                  "h-2 w-2 rounded-full",
                  node?.status === "alive" ? "bg-status-healthy" :
                  node?.status === "suspect" ? "bg-status-warning" : "bg-status-critical"
                )} />
                <span className="text-sm font-medium">Node {nodeId}</span>
                {isLeader && (
                  <span className="rounded bg-primary/20 px-1.5 py-0.5 text-xs text-primary">Leader</span>
                )}
              </div>
            );
          })}
          {replicas.length === 0 && (
            <p className="text-sm text-muted-foreground">No replicas detected</p>
          )}
        </div>
      </div>
    </div>
  );
}
