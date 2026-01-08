"use client";

import { useMemo, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useShardsWithStability } from "@/stores/eventStore";
import { cn, formatDuration } from "@/lib/utils";

interface ShardLeadershipGridProps {
  onShardSelect?: (shardId: number) => void;
  selectedShard?: number | null;
}

type StabilityFilter = "all" | "stable" | "recent";

export function ShardLeadershipGrid({ onShardSelect, selectedShard }: ShardLeadershipGridProps) {
  const shardsWithStability = useShardsWithStability();
  const [filter, setFilter] = useState<StabilityFilter>("all");
  const [hoveredShard, setHoveredShard] = useState<number | null>(null);

  const sortedShards = useMemo(() => {
    let shards = [...shardsWithStability].sort((a, b) => a.id - b.id);

    if (filter === "stable") {
      shards = shards.filter((s) => s.isStable);
    } else if (filter === "recent") {
      shards = shards.filter((s) => s.recentElection);
    }

    return shards;
  }, [shardsWithStability, filter]);

  // Group shards by leader
  const leadershipStats = useMemo(() => {
    const stats = new Map<number | null, number>();
    for (const shard of shardsWithStability) {
      const count = stats.get(shard.leader) || 0;
      stats.set(shard.leader, count + 1);
    }
    return stats;
  }, [shardsWithStability]);

  // Stability stats
  const stabilityStats = useMemo(() => {
    const stable = shardsWithStability.filter((s) => s.isStable).length;
    const recent = shardsWithStability.filter((s) => s.recentElection).length;
    const warning = shardsWithStability.filter(
      (s) => !s.isStable && !s.recentElection && s.stabilityMs < 2 * 60 * 1000
    ).length;
    return { stable, recent, warning, total: shardsWithStability.length };
  }, [shardsWithStability]);

  // Get node colors for consistency
  const nodeColors = useMemo(() => {
    const colors = [
      "hsl(210, 100%, 60%)", // Blue
      "hsl(150, 80%, 50%)",  // Green
      "hsl(280, 80%, 60%)",  // Purple
      "hsl(30, 100%, 55%)",  // Orange
      "hsl(340, 80%, 60%)",  // Pink
      "hsl(180, 70%, 50%)",  // Cyan
      "hsl(60, 90%, 50%)",   // Yellow
    ];
    const map = new Map<number, string>();
    const nodeIds = Array.from(
      new Set(shardsWithStability.map((s) => s.leader).filter((l): l is number => l !== null))
    );
    nodeIds.sort((a, b) => a - b).forEach((id, i) => {
      map.set(id, colors[i % colors.length]);
    });
    return map;
  }, [shardsWithStability]);

  if (shardsWithStability.length === 0) {
    return (
      <div className="flex h-48 items-center justify-center rounded-lg border border-dashed border-border">
        <p className="text-sm text-muted-foreground">
          No shard data yet. Waiting for Raft events...
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {/* Stability summary bar */}
      <div className="flex items-center justify-between rounded-lg bg-muted/30 p-3">
        <div className="flex items-center gap-4">
          <div className="flex items-center gap-2">
            <div className="h-2.5 w-2.5 rounded-full bg-status-healthy" />
            <span className="text-sm text-foreground">
              {stabilityStats.stable} stable
            </span>
          </div>
          {stabilityStats.warning > 0 && (
            <div className="flex items-center gap-2">
              <div className="h-2.5 w-2.5 rounded-full bg-status-warning" />
              <span className="text-sm text-foreground">
                {stabilityStats.warning} recovering
              </span>
            </div>
          )}
          {stabilityStats.recent > 0 && (
            <div className="flex items-center gap-2">
              <div className="h-2.5 w-2.5 rounded-full bg-status-critical animate-pulse" />
              <span className="text-sm text-foreground">
                {stabilityStats.recent} recent election
              </span>
            </div>
          )}
        </div>

        {/* Filter controls */}
        <div className="flex rounded-lg bg-background p-0.5">
          {(["all", "stable", "recent"] as const).map((f) => (
            <button
              key={f}
              onClick={() => setFilter(f)}
              className={cn(
                "rounded-md px-3 py-1 text-xs font-medium transition-colors",
                filter === f
                  ? "bg-primary text-primary-foreground"
                  : "text-muted-foreground hover:text-foreground"
              )}
            >
              {f === "all" ? "All" : f === "stable" ? "Stable" : "Recent"}
            </button>
          ))}
        </div>
      </div>

      {/* Leadership summary */}
      <div className="flex flex-wrap gap-3">
        {Array.from(leadershipStats.entries())
          .sort(([a], [b]) => (a ?? 999) - (b ?? 999))
          .map(([leader, count]) => (
            <div
              key={leader ?? "none"}
              className="flex items-center gap-2 rounded-lg bg-muted/50 px-3 py-1.5"
            >
              <div
                className="h-3 w-3 rounded-full"
                style={{
                  backgroundColor: leader !== null ? nodeColors.get(leader) : "#6b7280",
                }}
              />
              <span className="text-sm font-medium text-foreground">
                {leader !== null ? `Node ${leader}` : "No leader"}
              </span>
              <span className="rounded bg-background px-1.5 py-0.5 text-xs text-muted-foreground">
                {count} shard{count !== 1 ? "s" : ""}
              </span>
            </div>
          ))}
      </div>

      {/* Shard grid */}
      <div className="grid grid-cols-8 gap-2 sm:grid-cols-12 md:grid-cols-16 lg:grid-cols-20">
        <AnimatePresence mode="popLayout">
          {sortedShards.map((shard) => (
            <ShardCell
              key={shard.id}
              shard={shard}
              color={shard.leader !== null ? nodeColors.get(shard.leader) : undefined}
              isSelected={selectedShard === shard.id}
              isHovered={hoveredShard === shard.id}
              onClick={() => onShardSelect?.(shard.id)}
              onHover={(hovered) => setHoveredShard(hovered ? shard.id : null)}
            />
          ))}
        </AnimatePresence>
      </div>

      {/* Hovered shard tooltip */}
      <AnimatePresence>
        {hoveredShard !== null && (
          <HoveredShardTooltip
            shard={sortedShards.find((s) => s.id === hoveredShard)}
          />
        )}
      </AnimatePresence>

      {/* Legend */}
      <div className="flex items-center gap-4 text-xs text-muted-foreground">
        <div className="flex items-center gap-1">
          <div className="h-3 w-3 rounded border-2 border-primary" />
          <span>Has leader</span>
        </div>
        <div className="flex items-center gap-1">
          <div className="h-3 w-3 rounded border-2 border-status-warning bg-status-warning/20" />
          <span>No leader</span>
        </div>
        <div className="flex items-center gap-1">
          <StabilityRingIcon status="stable" />
          <span>&gt;5 min stable</span>
        </div>
        <div className="flex items-center gap-1">
          <StabilityRingIcon status="warning" />
          <span>&lt;2 min</span>
        </div>
        <div className="flex items-center gap-1">
          <StabilityRingIcon status="critical" />
          <span>&lt;30s</span>
        </div>
      </div>
    </div>
  );
}

interface ShardWithStability {
  id: number;
  term: number;
  leader: number | null;
  stabilityMs: number;
  isStable: boolean;
  recentElection: boolean;
  lastElectionTs: number;
  electionCount: number;
}

interface ShardCellProps {
  shard: ShardWithStability;
  color?: string;
  isSelected: boolean;
  isHovered: boolean;
  onClick: () => void;
  onHover: (hovered: boolean) => void;
}

function ShardCell({ shard, color, isSelected, isHovered, onClick, onHover }: ShardCellProps) {
  const hasLeader = shard.leader !== null;

  // Calculate stability status
  const stabilityStatus: "stable" | "warning" | "critical" = shard.recentElection
    ? "critical"
    : shard.isStable
    ? "stable"
    : shard.stabilityMs < 2 * 60 * 1000
    ? "warning"
    : "stable";

  // Calculate ring fill percentage (0-100) based on time since election
  // Full ring at 5 minutes
  const ringFillPercent = Math.min(100, (shard.stabilityMs / (5 * 60 * 1000)) * 100);

  return (
    <motion.button
      layout
      onClick={onClick}
      onMouseEnter={() => onHover(true)}
      onMouseLeave={() => onHover(false)}
      className={cn(
        "relative flex h-12 w-full flex-col items-center justify-center rounded-lg border-2 transition-all",
        hasLeader
          ? "border-transparent bg-opacity-20"
          : "border-status-warning bg-status-warning/10 animate-pulse",
        isSelected && "ring-2 ring-primary ring-offset-2 ring-offset-background"
      )}
      style={{
        backgroundColor: hasLeader && color ? `${color}20` : undefined,
        borderColor: hasLeader && color ? color : undefined,
      }}
      initial={{ opacity: 0, scale: 0.8 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.8 }}
      whileHover={{ scale: 1.05 }}
      whileTap={{ scale: 0.95 }}
    >
      {/* Stability ring */}
      <StabilityRing fillPercent={ringFillPercent} status={stabilityStatus} />

      {/* Election flash effect */}
      {shard.recentElection && (
        <motion.div
          className="absolute inset-0 rounded-lg bg-status-critical/30"
          initial={{ opacity: 0.8 }}
          animate={{ opacity: [0.8, 0, 0.8] }}
          transition={{ duration: 1, repeat: Infinity }}
        />
      )}

      <span className="text-xs font-bold text-foreground z-10">S{shard.id}</span>
      <span className="text-[10px] text-muted-foreground z-10">
        {hasLeader ? `N${shard.leader}` : "—"}
      </span>

      {/* Term badge */}
      <div className="absolute -right-1 -top-1 flex h-4 w-4 items-center justify-center rounded-full bg-muted text-[8px] font-medium z-10">
        {shard.term}
      </div>

      {/* Election count badge (if multiple elections) */}
      {shard.electionCount > 1 && (
        <div className="absolute -left-1 -top-1 flex h-4 w-4 items-center justify-center rounded-full bg-status-warning text-[8px] font-bold text-white z-10">
          {shard.electionCount}
        </div>
      )}
    </motion.button>
  );
}

interface StabilityRingProps {
  fillPercent: number;
  status: "stable" | "warning" | "critical";
}

function StabilityRing({ fillPercent, status }: StabilityRingProps) {
  const radius = 20;
  const strokeWidth = 2;
  const circumference = 2 * Math.PI * radius;
  const strokeDashoffset = circumference - (fillPercent / 100) * circumference;

  const colors = {
    stable: "stroke-status-healthy",
    warning: "stroke-status-warning",
    critical: "stroke-status-critical",
  };

  return (
    <svg
      className="absolute inset-0 w-full h-full -rotate-90"
      viewBox="0 0 48 48"
      fill="none"
    >
      {/* Background ring */}
      <circle
        cx="24"
        cy="24"
        r={radius}
        className="stroke-muted"
        strokeWidth={strokeWidth}
        fill="none"
      />
      {/* Progress ring */}
      <motion.circle
        cx="24"
        cy="24"
        r={radius}
        className={colors[status]}
        strokeWidth={strokeWidth}
        fill="none"
        strokeLinecap="round"
        strokeDasharray={circumference}
        initial={{ strokeDashoffset: circumference }}
        animate={{ strokeDashoffset }}
        transition={{ duration: 0.5 }}
      />
    </svg>
  );
}

function StabilityRingIcon({ status }: { status: "stable" | "warning" | "critical" }) {
  const colors = {
    stable: "stroke-status-healthy",
    warning: "stroke-status-warning",
    critical: "stroke-status-critical",
  };

  return (
    <svg width="14" height="14" viewBox="0 0 14 14" className="-rotate-90">
      <circle
        cx="7"
        cy="7"
        r="5"
        className="stroke-muted"
        strokeWidth="2"
        fill="none"
      />
      <circle
        cx="7"
        cy="7"
        r="5"
        className={colors[status]}
        strokeWidth="2"
        fill="none"
        strokeDasharray={2 * Math.PI * 5}
        strokeDashoffset={status === "stable" ? 0 : status === "warning" ? 15 : 25}
      />
    </svg>
  );
}

function HoveredShardTooltip({ shard }: { shard?: ShardWithStability }) {
  if (!shard) return null;

  const stabilityText = shard.recentElection
    ? `Election ${formatDuration(shard.stabilityMs)} ago`
    : shard.isStable
    ? `Stable for ${formatDuration(shard.stabilityMs)}`
    : `Recovering for ${formatDuration(shard.stabilityMs)}`;

  return (
    <motion.div
      initial={{ opacity: 0, y: 5 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 5 }}
      className="rounded-lg border border-border bg-card p-3 shadow-lg"
    >
      <div className="flex items-center gap-3">
        <div>
          <p className="font-medium text-foreground">Shard {shard.id}</p>
          <p className="text-sm text-muted-foreground">
            {shard.leader !== null ? `Leader: Node ${shard.leader}` : "No leader"} • Term {shard.term}
          </p>
        </div>
        <div className="text-right">
          <p
            className={cn(
              "text-sm font-medium",
              shard.recentElection
                ? "text-status-critical"
                : shard.isStable
                ? "text-status-healthy"
                : "text-status-warning"
            )}
          >
            {stabilityText}
          </p>
          <p className="text-xs text-muted-foreground">
            {shard.electionCount} election{shard.electionCount !== 1 ? "s" : ""} total
          </p>
        </div>
      </div>
    </motion.div>
  );
}
