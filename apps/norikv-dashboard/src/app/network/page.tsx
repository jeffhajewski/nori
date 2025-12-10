"use client";

import { useState } from "react";
import { AnimatePresence } from "framer-motion";
import { useEventStore, useClusterHealth } from "@/stores/eventStore";
import {
  ClusterTopology,
  NodeDetailCard,
  SwimTimeline,
} from "@/components/network";

export default function NetworkPage() {
  const [selectedNode, setSelectedNode] = useState<number | null>(null);
  const health = useClusterHealth();
  const nodes = useEventStore((s) => Array.from(s.nodes.values()));

  return (
    <div className="space-y-6">
      {/* Page header */}
      <div>
        <h1 className="text-2xl font-semibold text-foreground">Network</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          SWIM membership protocol and cluster topology
        </p>
      </div>

      {/* Health summary */}
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <HealthCard
          title="Total Nodes"
          value={health.total}
          status="neutral"
        />
        <HealthCard
          title="Alive"
          value={health.alive}
          status={health.alive === health.total ? "healthy" : "warning"}
        />
        <HealthCard
          title="Suspect"
          value={health.suspect}
          status={health.suspect > 0 ? "warning" : "healthy"}
        />
        <HealthCard
          title="Dead"
          value={health.dead}
          status={health.dead > 0 ? "critical" : "healthy"}
        />
      </div>

      <div className="grid gap-6 lg:grid-cols-3">
        {/* Main content - Topology */}
        <div className="lg:col-span-2 space-y-6">
          {/* Cluster Topology */}
          <div className="rounded-xl border border-border bg-card p-6">
            <div className="mb-4">
              <h2 className="text-lg font-medium text-foreground">Cluster Topology</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                Click a node to view details
              </p>
            </div>
            <ClusterTopology
              selectedNode={selectedNode}
              onNodeSelect={setSelectedNode}
            />
          </div>

          {/* Selected Node Detail */}
          <AnimatePresence>
            {selectedNode !== null && (
              <NodeDetailCard
                nodeId={selectedNode}
                onClose={() => setSelectedNode(null)}
              />
            )}
          </AnimatePresence>

          {/* Node Grid (alternative view) */}
          <div className="rounded-xl border border-border bg-card p-6">
            <h2 className="mb-4 text-lg font-medium text-foreground">Node Status Grid</h2>
            <div className="grid grid-cols-3 gap-3 sm:grid-cols-4 md:grid-cols-6">
              {nodes.map((node) => (
                <NodeGridCard
                  key={node.id}
                  node={node}
                  isSelected={selectedNode === node.id}
                  onClick={() => setSelectedNode(node.id)}
                />
              ))}
              {nodes.length === 0 && (
                <p className="col-span-full text-sm text-muted-foreground">
                  No nodes detected yet
                </p>
              )}
            </div>
          </div>
        </div>

        {/* Sidebar */}
        <div className="space-y-6">
          {/* SWIM Events */}
          <div className="rounded-xl border border-border bg-card p-6">
            <div className="mb-4">
              <h2 className="text-lg font-medium text-foreground">Membership Events</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                {selectedNode !== null
                  ? `Events for Node ${selectedNode}`
                  : "Recent SWIM events"}
              </p>
            </div>
            <SwimTimeline filterNode={selectedNode} />
          </div>

          {/* Cluster Health Status */}
          <div className="rounded-xl border border-border bg-card p-6">
            <h2 className="mb-4 text-lg font-medium text-foreground">Health Status</h2>
            <ClusterHealthIndicator health={health} />
          </div>
        </div>
      </div>
    </div>
  );
}

function HealthCard({
  title,
  value,
  status,
}: {
  title: string;
  value: number;
  status: "healthy" | "warning" | "critical" | "neutral";
}) {
  const statusColors = {
    healthy: "border-l-status-healthy",
    warning: "border-l-status-warning",
    critical: "border-l-status-critical",
    neutral: "border-l-muted-foreground",
  };

  return (
    <div className={`rounded-xl border border-border border-l-4 bg-card p-4 ${statusColors[status]}`}>
      <p className="text-sm text-muted-foreground">{title}</p>
      <p className="mt-1 text-2xl font-bold text-foreground">{value}</p>
    </div>
  );
}

function NodeGridCard({
  node,
  isSelected,
  onClick,
}: {
  node: { id: number; status: "alive" | "suspect" | "dead" };
  isSelected: boolean;
  onClick: () => void;
}) {
  const statusStyles = {
    alive: "border-status-healthy/50 bg-status-healthy/10 text-status-healthy",
    suspect: "border-status-warning/50 bg-status-warning/10 text-status-warning animate-pulse",
    dead: "border-status-critical/50 bg-status-critical/10 text-status-critical",
  };

  return (
    <button
      onClick={onClick}
      className={`rounded-lg border-2 p-3 text-center transition-all hover:scale-105 ${
        statusStyles[node.status]
      } ${isSelected ? "ring-2 ring-primary ring-offset-2 ring-offset-background" : ""}`}
    >
      <p className="text-lg font-bold">N{node.id}</p>
      <p className="text-xs capitalize opacity-70">{node.status}</p>
    </button>
  );
}

function ClusterHealthIndicator({
  health,
}: {
  health: { total: number; alive: number; suspect: number; dead: number; healthy: boolean };
}) {
  const getOverallStatus = () => {
    if (health.dead > 0) return { status: "critical", label: "Degraded", color: "text-status-critical" };
    if (health.suspect > 0) return { status: "warning", label: "Warning", color: "text-status-warning" };
    if (health.total === 0) return { status: "neutral", label: "No Nodes", color: "text-muted-foreground" };
    return { status: "healthy", label: "Healthy", color: "text-status-healthy" };
  };

  const overall = getOverallStatus();
  const alivePercent = health.total > 0 ? (health.alive / health.total) * 100 : 0;

  return (
    <div className="space-y-4">
      {/* Overall status */}
      <div className="flex items-center gap-3">
        <div
          className={`h-4 w-4 rounded-full ${
            overall.status === "healthy"
              ? "bg-status-healthy"
              : overall.status === "warning"
              ? "bg-status-warning animate-pulse"
              : overall.status === "critical"
              ? "bg-status-critical"
              : "bg-muted-foreground"
          }`}
        />
        <span className={`text-lg font-semibold ${overall.color}`}>{overall.label}</span>
      </div>

      {/* Progress bar */}
      {health.total > 0 && (
        <div>
          <div className="flex justify-between text-xs text-muted-foreground">
            <span>Cluster Availability</span>
            <span>{alivePercent.toFixed(0)}%</span>
          </div>
          <div className="mt-1 h-3 overflow-hidden rounded-full bg-muted">
            <div className="flex h-full">
              <div
                className="bg-status-healthy transition-all"
                style={{ width: `${(health.alive / health.total) * 100}%` }}
              />
              <div
                className="bg-status-warning transition-all"
                style={{ width: `${(health.suspect / health.total) * 100}%` }}
              />
              <div
                className="bg-status-critical transition-all"
                style={{ width: `${(health.dead / health.total) * 100}%` }}
              />
            </div>
          </div>
        </div>
      )}

      {/* Breakdown */}
      <div className="space-y-2 text-sm">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <div className="h-2 w-2 rounded-full bg-status-healthy" />
            <span className="text-muted-foreground">Alive</span>
          </div>
          <span className="font-medium text-foreground">{health.alive}</span>
        </div>
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <div className="h-2 w-2 rounded-full bg-status-warning" />
            <span className="text-muted-foreground">Suspect</span>
          </div>
          <span className="font-medium text-foreground">{health.suspect}</span>
        </div>
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <div className="h-2 w-2 rounded-full bg-status-critical" />
            <span className="text-muted-foreground">Dead</span>
          </div>
          <span className="font-medium text-foreground">{health.dead}</span>
        </div>
      </div>
    </div>
  );
}
