"use client";

import { useMemo } from "react";
import { motion } from "framer-motion";
import { useEventStore, type ShardState } from "@/stores/eventStore";
import { cn } from "@/lib/utils";

interface ShardLeadershipGridProps {
  onShardSelect?: (shardId: number) => void;
  selectedShard?: number | null;
}

export function ShardLeadershipGrid({ onShardSelect, selectedShard }: ShardLeadershipGridProps) {
  const shards = useEventStore((s) => Array.from(s.shards.values()));
  const nodes = useEventStore((s) => Array.from(s.nodes.values()));

  const sortedShards = useMemo(() => {
    return [...shards].sort((a, b) => a.id - b.id);
  }, [shards]);

  // Group shards by leader
  const leadershipStats = useMemo(() => {
    const stats = new Map<number | null, number>();
    for (const shard of shards) {
      const count = stats.get(shard.leader) || 0;
      stats.set(shard.leader, count + 1);
    }
    return stats;
  }, [shards]);

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
    const nodeIds = Array.from(new Set(shards.map((s) => s.leader).filter((l): l is number => l !== null)));
    nodeIds.sort((a, b) => a - b).forEach((id, i) => {
      map.set(id, colors[i % colors.length]);
    });
    return map;
  }, [shards]);

  if (sortedShards.length === 0) {
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
        {sortedShards.map((shard) => (
          <ShardCell
            key={shard.id}
            shard={shard}
            color={shard.leader !== null ? nodeColors.get(shard.leader) : undefined}
            isSelected={selectedShard === shard.id}
            onClick={() => onShardSelect?.(shard.id)}
          />
        ))}
      </div>

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
      </div>
    </div>
  );
}

interface ShardCellProps {
  shard: ShardState;
  color?: string;
  isSelected: boolean;
  onClick: () => void;
}

function ShardCell({ shard, color, isSelected, onClick }: ShardCellProps) {
  const hasLeader = shard.leader !== null;

  return (
    <motion.button
      onClick={onClick}
      className={cn(
        "relative flex h-10 w-full flex-col items-center justify-center rounded-lg border-2 transition-all",
        hasLeader
          ? "border-transparent bg-opacity-20 hover:scale-105"
          : "border-status-warning bg-status-warning/10 animate-pulse",
        isSelected && "ring-2 ring-primary ring-offset-2 ring-offset-background"
      )}
      style={{
        backgroundColor: hasLeader && color ? `${color}20` : undefined,
        borderColor: hasLeader && color ? color : undefined,
      }}
      whileHover={{ scale: 1.05 }}
      whileTap={{ scale: 0.95 }}
    >
      <span className="text-xs font-bold text-foreground">S{shard.id}</span>
      <span className="text-[10px] text-muted-foreground">
        {hasLeader ? `N${shard.leader}` : "—"}
      </span>

      {/* Term badge */}
      <div className="absolute -right-1 -top-1 flex h-4 w-4 items-center justify-center rounded-full bg-muted text-[8px] font-medium">
        {shard.term}
      </div>
    </motion.button>
  );
}
