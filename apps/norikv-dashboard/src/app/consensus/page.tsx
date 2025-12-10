"use client";

import { useState } from "react";
import { AnimatePresence } from "framer-motion";
import { useEventStore } from "@/stores/eventStore";
import {
  ShardLeadershipGrid,
  ElectionTimeline,
  ShardDetailPanel,
} from "@/components/consensus";

export default function ConsensusPage() {
  const [selectedShard, setSelectedShard] = useState<number | null>(null);
  const shards = useEventStore((s) => Array.from(s.shards.values()));
  const nodes = useEventStore((s) => Array.from(s.nodes.values()));

  // Compute stats
  const shardsWithLeader = shards.filter((s) => s.leader !== null).length;
  const shardsWithoutLeader = shards.filter((s) => s.leader === null).length;
  const uniqueLeaders = new Set(shards.map((s) => s.leader).filter((l) => l !== null)).size;
  const avgTerm = shards.length > 0
    ? Math.round(shards.reduce((sum, s) => sum + s.term, 0) / shards.length)
    : 0;

  return (
    <div className="space-y-6">
      {/* Page header */}
      <div>
        <h1 className="text-2xl font-semibold text-foreground">Consensus</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Raft leadership, elections, and shard consensus state
        </p>
      </div>

      {/* Stats row */}
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <StatCard
          title="Total Shards"
          value={shards.length}
          subtitle={`${shardsWithLeader} with leaders`}
        />
        <StatCard
          title="Leader Nodes"
          value={uniqueLeaders}
          subtitle={`of ${nodes.length} total nodes`}
        />
        <StatCard
          title="Leaderless"
          value={shardsWithoutLeader}
          subtitle={shardsWithoutLeader === 0 ? "All healthy" : "Elections needed"}
          warning={shardsWithoutLeader > 0}
        />
        <StatCard
          title="Avg Term"
          value={avgTerm}
          subtitle="Across all shards"
        />
      </div>

      <div className="grid gap-6 lg:grid-cols-3">
        {/* Main content - Leadership Grid */}
        <div className="lg:col-span-2 space-y-6">
          {/* Shard Leadership Grid */}
          <div className="rounded-xl border border-border bg-card p-6">
            <div className="mb-4">
              <h2 className="text-lg font-medium text-foreground">Shard Leadership</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                Click a shard to view details. Colors indicate leader nodes.
              </p>
            </div>
            <ShardLeadershipGrid
              selectedShard={selectedShard}
              onShardSelect={setSelectedShard}
            />
          </div>

          {/* Selected Shard Detail */}
          <AnimatePresence>
            {selectedShard !== null && (
              <ShardDetailPanel
                shardId={selectedShard}
                onClose={() => setSelectedShard(null)}
              />
            )}
          </AnimatePresence>
        </div>

        {/* Sidebar - Election Timeline */}
        <div className="space-y-6">
          <div className="rounded-xl border border-border bg-card p-6">
            <div className="mb-4">
              <h2 className="text-lg font-medium text-foreground">Election Activity</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                {selectedShard !== null
                  ? `Events for Shard ${selectedShard}`
                  : "Recent Raft events"}
              </p>
            </div>
            <ElectionTimeline filterShard={selectedShard} />
          </div>

          {/* Node Leadership Distribution */}
          <div className="rounded-xl border border-border bg-card p-6">
            <h2 className="mb-4 text-lg font-medium text-foreground">Leadership Distribution</h2>
            <LeadershipDistribution />
          </div>
        </div>
      </div>
    </div>
  );
}

function StatCard({
  title,
  value,
  subtitle,
  warning = false,
}: {
  title: string;
  value: number;
  subtitle: string;
  warning?: boolean;
}) {
  return (
    <div
      className={`rounded-xl border bg-card p-4 ${
        warning ? "border-status-warning" : "border-border"
      }`}
    >
      <p className="text-sm text-muted-foreground">{title}</p>
      <p className={`mt-1 text-2xl font-bold ${warning ? "text-status-warning" : "text-foreground"}`}>
        {value}
      </p>
      <p className="mt-1 text-xs text-muted-foreground">{subtitle}</p>
    </div>
  );
}

function LeadershipDistribution() {
  const shards = useEventStore((s) => Array.from(s.shards.values()));
  const nodes = useEventStore((s) => Array.from(s.nodes.values()));

  // Count shards per leader
  const distribution = new Map<number, number>();
  for (const shard of shards) {
    if (shard.leader !== null) {
      const count = distribution.get(shard.leader) || 0;
      distribution.set(shard.leader, count + 1);
    }
  }

  const sortedNodes = Array.from(distribution.entries()).sort((a, b) => b[1] - a[1]);
  const maxCount = Math.max(...sortedNodes.map(([, count]) => count), 1);

  if (sortedNodes.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">No leadership data available</p>
    );
  }

  return (
    <div className="space-y-3">
      {sortedNodes.map(([nodeId, count]) => {
        const node = nodes.find((n) => n.id === nodeId);
        const pct = (count / maxCount) * 100;

        return (
          <div key={nodeId} className="space-y-1">
            <div className="flex items-center justify-between text-sm">
              <div className="flex items-center gap-2">
                <div
                  className={`h-2 w-2 rounded-full ${
                    node?.status === "alive"
                      ? "bg-status-healthy"
                      : node?.status === "suspect"
                      ? "bg-status-warning"
                      : "bg-muted-foreground"
                  }`}
                />
                <span className="font-medium text-foreground">Node {nodeId}</span>
              </div>
              <span className="text-muted-foreground">{count} shards</span>
            </div>
            <div className="h-2 overflow-hidden rounded-full bg-muted">
              <div
                className="h-full bg-primary transition-all duration-300"
                style={{ width: `${pct}%` }}
              />
            </div>
          </div>
        );
      })}

      {/* Ideal distribution note */}
      {shards.length > 0 && sortedNodes.length > 0 && (
        <p className="mt-2 text-xs text-muted-foreground">
          Ideal: ~{Math.round(shards.length / sortedNodes.length)} shards per node
        </p>
      )}
    </div>
  );
}
