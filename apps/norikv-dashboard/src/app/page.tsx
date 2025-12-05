"use client";

import { useEventStore, useClusterHealth, useStats } from "@/stores/eventStore";
import { formatNumber, formatBytes } from "@/lib/utils";

export default function OverviewPage() {
  const health = useClusterHealth();
  const stats = useStats();
  const nodes = useEventStore((s) => Array.from(s.nodes.values()));
  const shards = useEventStore((s) => Array.from(s.shards.values()));
  const lsmStates = useEventStore((s) => Array.from(s.lsmStates.values()));

  return (
    <div className="space-y-6">
      {/* Page header */}
      <div>
        <h1 className="text-2xl font-semibold text-foreground">Cluster Overview</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Real-time health and performance metrics for your NoriKV cluster
        </p>
      </div>

      {/* Stats cards */}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <StatCard
          title="Nodes"
          value={health.total}
          subtitle={
            health.healthy
              ? "All healthy"
              : `${health.alive} alive, ${health.suspect} suspect, ${health.dead} dead`
          }
          status={health.healthy ? "healthy" : health.dead > 0 ? "critical" : "warning"}
        />
        <StatCard
          title="Shards"
          value={shards.length}
          subtitle={`${shards.filter((s) => s.leader !== null).length} with leaders`}
          status={shards.every((s) => s.leader !== null) ? "healthy" : "warning"}
        />
        <StatCard
          title="Events Received"
          value={formatNumber(stats.totalEventsReceived)}
          subtitle={`${formatNumber(stats.bufferSize)} in buffer`}
          status="neutral"
        />
        <StatCard
          title="Storage Nodes"
          value={lsmStates.length}
          subtitle={`${lsmStates.filter((s) => s.writePressureHigh).length} under pressure`}
          status={lsmStates.some((s) => s.writePressureHigh) ? "warning" : "healthy"}
        />
      </div>

      {/* Node grid */}
      <div className="rounded-xl border border-border bg-card p-6">
        <h2 className="text-lg font-medium text-foreground">Node Status</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Membership status from SWIM protocol
        </p>

        <div className="mt-4 grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6">
          {nodes.length > 0 ? (
            nodes.map((node) => (
              <NodeCard key={node.id} node={node} />
            ))
          ) : (
            <p className="col-span-full text-sm text-muted-foreground">
              No nodes detected yet. Waiting for events...
            </p>
          )}
        </div>
      </div>

      {/* Shard leaders */}
      <div className="rounded-xl border border-border bg-card p-6">
        <h2 className="text-lg font-medium text-foreground">Shard Leadership</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Raft consensus leadership across shards
        </p>

        <div className="mt-4 grid grid-cols-4 gap-3 sm:grid-cols-6 md:grid-cols-8 lg:grid-cols-12">
          {shards.length > 0 ? (
            shards.map((shard) => (
              <ShardCell key={shard.id} shard={shard} />
            ))
          ) : (
            <p className="col-span-full text-sm text-muted-foreground">
              No shards detected yet. Waiting for Raft events...
            </p>
          )}
        </div>
      </div>

      {/* Write pressure */}
      <div className="rounded-xl border border-border bg-card p-6">
        <h2 className="text-lg font-medium text-foreground">Write Pressure</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          LSM engine write throttling status per node
        </p>

        <div className="mt-4 space-y-3">
          {lsmStates.length > 0 ? (
            lsmStates.map((state) => (
              <WritePressureBar key={state.nodeId} state={state} />
            ))
          ) : (
            <p className="text-sm text-muted-foreground">
              No LSM state detected yet. Waiting for write pressure events...
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

interface StatCardProps {
  title: string;
  value: string | number;
  subtitle: string;
  status: "healthy" | "warning" | "critical" | "neutral";
}

function StatCard({ title, value, subtitle, status }: StatCardProps) {
  const statusColors = {
    healthy: "border-l-status-healthy",
    warning: "border-l-status-warning",
    critical: "border-l-status-critical",
    neutral: "border-l-muted-foreground",
  };

  return (
    <div
      className={`rounded-xl border border-border border-l-4 bg-card p-4 ${statusColors[status]}`}
    >
      <p className="text-sm font-medium text-muted-foreground">{title}</p>
      <p className="mt-1 text-2xl font-semibold text-foreground">{value}</p>
      <p className="mt-1 text-xs text-muted-foreground">{subtitle}</p>
    </div>
  );
}

interface NodeCardProps {
  node: { id: number; status: "alive" | "suspect" | "dead"; lastSeen: number };
}

function NodeCard({ node }: NodeCardProps) {
  const statusStyles = {
    alive: "bg-status-healthy/10 border-status-healthy/30 text-status-healthy",
    suspect: "bg-status-warning/10 border-status-warning/30 text-status-warning",
    dead: "bg-status-critical/10 border-status-critical/30 text-status-critical",
  };

  return (
    <div
      className={`rounded-lg border p-3 text-center transition-all ${statusStyles[node.status]}`}
    >
      <p className="text-lg font-semibold">Node {node.id}</p>
      <p className="mt-1 text-xs capitalize opacity-80">{node.status}</p>
    </div>
  );
}

interface ShardCellProps {
  shard: { id: number; leader: number | null; term: number };
}

function ShardCell({ shard }: ShardCellProps) {
  const hasLeader = shard.leader !== null;

  return (
    <div
      className={`rounded-lg border p-2 text-center text-xs transition-all ${
        hasLeader
          ? "border-primary/30 bg-primary/10"
          : "border-status-warning/30 bg-status-warning/10"
      }`}
      title={`Shard ${shard.id}: Leader=${shard.leader ?? "none"}, Term=${shard.term}`}
    >
      <p className="font-medium text-foreground">S{shard.id}</p>
      <p className="text-muted-foreground">
        {hasLeader ? `N${shard.leader}` : "—"}
      </p>
    </div>
  );
}

interface WritePressureBarProps {
  state: {
    nodeId: number;
    writePressure: number;
    writePressureHigh: boolean;
    l0FileCount: number;
    l0Threshold: number;
  };
}

function WritePressureBar({ state }: WritePressureBarProps) {
  const pct = Math.min(state.writePressure * 100, 100);
  const barColor = state.writePressureHigh
    ? "bg-status-critical"
    : pct > 50
    ? "bg-status-warning"
    : "bg-status-healthy";

  return (
    <div className="flex items-center gap-4">
      <span className="w-16 text-sm font-medium text-foreground">Node {state.nodeId}</span>
      <div className="flex-1">
        <div className="h-3 overflow-hidden rounded-full bg-muted">
          <div
            className={`h-full transition-all duration-300 ${barColor}`}
            style={{ width: `${pct}%` }}
          />
        </div>
      </div>
      <span className="w-16 text-right text-sm text-muted-foreground">
        {pct.toFixed(0)}%
      </span>
      <span className="w-24 text-right text-xs text-muted-foreground">
        L0: {state.l0FileCount}/{state.l0Threshold}
      </span>
    </div>
  );
}
