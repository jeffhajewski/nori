"use client";

import { useEffect, useState, useMemo } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useEventStore, useCompactionEfficiency, useBanditState } from "@/stores/eventStore";
import { formatBytes, getLevelColor, cn } from "@/lib/utils";
import type { WireCompEvt } from "@/types/events";

interface CompactionEvent {
  id: string;
  node: number;
  level: number;
  status: "scheduled" | "running" | "finished";
  progress: number;
  inBytes?: number;
  outBytes?: number;
  timestamp: number;
}

export function CompactionPanel({ nodeId }: { nodeId: number }) {
  const [compactions, setCompactions] = useState<Map<string, CompactionEvent>>(new Map());
  const [recentFinished, setRecentFinished] = useState<CompactionEvent[]>([]);
  const efficiencyRecords = useCompactionEfficiency(nodeId);

  // Subscribe to compaction events
  useEffect(() => {
    const unsub = useEventStore.subscribe(
      (state) => state.eventBuffer,
      (buffer) => {
        if (buffer.length === 0) return;

        const latest = buffer[buffer.length - 1];
        for (const evt of latest.events) {
          if (evt.event.type === "Compaction" && evt.event.data.node === nodeId) {
            handleCompactionEvent(evt.event.data, evt.ts);
          }
        }
      }
    );

    return unsub;
  }, [nodeId]);

  const handleCompactionEvent = (evt: WireCompEvt, timestamp: number) => {
    const id = `${evt.node}-${evt.level}`;

    setCompactions((prev) => {
      const next = new Map(prev);
      const existing = next.get(id);

      switch (evt.kind.kind) {
        case "Scheduled":
          next.set(id, {
            id,
            node: evt.node,
            level: evt.level,
            status: "scheduled",
            progress: 0,
            timestamp,
          });
          break;

        case "Start":
          next.set(id, {
            ...existing,
            id,
            node: evt.node,
            level: evt.level,
            status: "running",
            progress: 0,
            timestamp,
          });
          break;

        case "Progress":
          if (existing) {
            next.set(id, {
              ...existing,
              progress: evt.kind.pct,
              timestamp,
            });
          }
          break;

        case "Finish":
          if (existing) {
            const finished: CompactionEvent = {
              ...existing,
              status: "finished",
              progress: 100,
              inBytes: evt.kind.in_bytes,
              outBytes: evt.kind.out_bytes,
              timestamp,
            };
            next.delete(id);

            // Add to recent finished
            setRecentFinished((prev) => [finished, ...prev.slice(0, 4)]);
          }
          break;
      }

      return next;
    });
  };

  const activeCompactions = Array.from(compactions.values());

  // Calculate efficiency metrics
  const efficiencyMetrics = useMemo(() => {
    if (efficiencyRecords.length === 0) return null;

    const totalIn = efficiencyRecords.reduce((sum, r) => sum + r.inBytes, 0);
    const totalOut = efficiencyRecords.reduce((sum, r) => sum + r.outBytes, 0);
    const writeAmplification = totalOut > 0 ? totalIn / totalOut : 1;
    const spaceReclaimed = totalIn - totalOut;

    return {
      writeAmplification,
      spaceReclaimed,
      compactionCount: efficiencyRecords.length,
      avgRatio: efficiencyRecords.reduce((sum, r) => sum + r.ratio, 0) / efficiencyRecords.length,
    };
  }, [efficiencyRecords]);

  return (
    <div className="space-y-4">
      {/* Efficiency metrics */}
      {efficiencyMetrics && (
        <CompactionEfficiencyMetrics metrics={efficiencyMetrics} />
      )}

      {/* Active compactions */}
      <div>
        <h4 className="text-sm font-medium text-foreground">Active Compactions</h4>
        <div className="mt-2 space-y-2">
          <AnimatePresence mode="popLayout">
            {activeCompactions.length > 0 ? (
              activeCompactions.map((comp) => (
                <CompactionRow key={comp.id} compaction={comp} nodeId={nodeId} />
              ))
            ) : (
              <motion.p
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                className="text-sm text-muted-foreground"
              >
                No active compactions
              </motion.p>
            )}
          </AnimatePresence>
        </div>
      </div>

      {/* Conveyor belt visualization */}
      {activeCompactions.length > 0 && (
        <div className="relative h-16 overflow-hidden rounded-lg bg-muted/30">
          <ConveyorBelt compactions={activeCompactions} />
        </div>
      )}

      {/* Recent completed */}
      {recentFinished.length > 0 && (
        <div>
          <h4 className="text-sm font-medium text-foreground">Recently Completed</h4>
          <div className="mt-2 space-y-1">
            {recentFinished.map((comp) => (
              <div
                key={`${comp.id}-${comp.timestamp}`}
                className="flex items-center justify-between rounded bg-muted/30 px-2 py-1 text-xs"
              >
                <span className="font-medium" style={{ color: getLevelColor(comp.level) }}>
                  L{comp.level}
                </span>
                <span className="text-muted-foreground">
                  {comp.inBytes && comp.outBytes && (
                    <>
                      {formatBytes(comp.inBytes)} → {formatBytes(comp.outBytes)}
                      <span className="ml-2 text-status-healthy">
                        ({((1 - comp.outBytes / comp.inBytes) * 100).toFixed(0)}% reduced)
                      </span>
                    </>
                  )}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

interface EfficiencyMetrics {
  writeAmplification: number;
  spaceReclaimed: number;
  compactionCount: number;
  avgRatio: number;
}

function CompactionEfficiencyMetrics({ metrics }: { metrics: EfficiencyMetrics }) {
  return (
    <div className="rounded-lg border border-border bg-muted/30 p-3">
      <h4 className="text-sm font-medium text-foreground mb-2">Efficiency (last hour)</h4>
      <div className="grid grid-cols-3 gap-3">
        <div>
          <p className="text-xs text-muted-foreground">Write Amp</p>
          <p
            className={cn(
              "text-lg font-semibold",
              metrics.writeAmplification > 10 ? "text-status-warning" : "text-foreground"
            )}
          >
            {metrics.writeAmplification.toFixed(1)}x
          </p>
        </div>
        <div>
          <p className="text-xs text-muted-foreground">Space Saved</p>
          <p className="text-lg font-semibold text-status-healthy">
            {formatBytes(metrics.spaceReclaimed)}
          </p>
        </div>
        <div>
          <p className="text-xs text-muted-foreground">Compactions</p>
          <p className="text-lg font-semibold text-foreground">{metrics.compactionCount}</p>
        </div>
      </div>
    </div>
  );
}

function CompactionRow({ compaction, nodeId }: { compaction: CompactionEvent; nodeId: number }) {
  const banditState = useBanditState(nodeId, compaction.level);

  // Calculate bandit stats
  const banditStats = useMemo(() => {
    if (!banditState || banditState.selections.length === 0) return null;

    const recentSelections = banditState.selections.slice(-10);
    const explorationRate = recentSelections.filter((s) => s.explored).length / recentSelections.length;
    const avgUcb = recentSelections.reduce((s, sel) => s + sel.ucbScore, 0) / recentSelections.length;

    const recentRewards = banditState.rewards.slice(-10);
    const avgReward =
      recentRewards.length > 0
        ? recentRewards.reduce((s, r) => s + r.reward, 0) / recentRewards.length
        : 0;

    return { explorationRate, avgUcb, avgReward };
  }, [banditState]);

  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: -10 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, scale: 0.95 }}
      className="rounded-lg border border-border bg-card p-3"
    >
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <div
            className="h-3 w-3 rounded"
            style={{ backgroundColor: getLevelColor(compaction.level) }}
          />
          <span className="font-medium text-foreground">Level {compaction.level}</span>
          <span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
            {compaction.status === "scheduled" ? "Scheduled" : "Running"}
          </span>
        </div>
        <span className="text-sm font-medium text-foreground">{compaction.progress}%</span>
      </div>

      {/* Progress bar */}
      <div className="mt-2 h-2 overflow-hidden rounded-full bg-muted">
        <motion.div
          className="h-full"
          style={{ backgroundColor: getLevelColor(compaction.level) }}
          initial={{ width: 0 }}
          animate={{ width: `${compaction.progress}%` }}
          transition={{ duration: 0.3 }}
        />
      </div>

      {/* Bandit optimization stats */}
      {banditStats && (
        <div className="mt-2 flex items-center gap-4 text-xs text-muted-foreground">
          <div className="flex items-center gap-1">
            <span>Explore:</span>
            <span
              className={cn(
                banditStats.explorationRate > 0.2 ? "text-status-healthy" : "text-muted-foreground"
              )}
            >
              {(banditStats.explorationRate * 100).toFixed(0)}%
            </span>
          </div>
          <div className="flex items-center gap-1">
            <span>UCB:</span>
            <span>{banditStats.avgUcb.toFixed(2)}</span>
          </div>
          <div className="flex items-center gap-1">
            <span>Reward:</span>
            <span className="text-status-healthy">{banditStats.avgReward.toFixed(2)}</span>
          </div>
        </div>
      )}
    </motion.div>
  );
}

function ConveyorBelt({ compactions }: { compactions: CompactionEvent[] }) {
  return (
    <div className="absolute inset-0 flex items-center">
      {/* Belt track */}
      <div className="absolute inset-x-0 top-1/2 h-4 -translate-y-1/2 bg-muted/50">
        {/* Animated belt lines */}
        <motion.div
          className="h-full w-full"
          style={{
            backgroundImage:
              "repeating-linear-gradient(90deg, transparent, transparent 20px, rgba(255,255,255,0.1) 20px, rgba(255,255,255,0.1) 22px)",
          }}
          animate={{ backgroundPositionX: [0, 22] }}
          transition={{ duration: 0.5, repeat: Infinity, ease: "linear" }}
        />
      </div>

      {/* Compaction blocks moving along belt */}
      {compactions.map((comp) => (
        <motion.div
          key={comp.id}
          className="absolute flex h-10 w-16 items-center justify-center rounded border-2 bg-card shadow-lg"
          style={{
            borderColor: getLevelColor(comp.level),
            left: `${10 + (comp.progress / 100) * 70}%`,
          }}
          animate={{
            left: `${10 + (comp.progress / 100) * 70}%`,
            y: [0, -2, 0],
          }}
          transition={{
            left: { duration: 0.3 },
            y: { duration: 0.5, repeat: Infinity },
          }}
        >
          <span className="text-xs font-bold" style={{ color: getLevelColor(comp.level) }}>
            L{comp.level}
          </span>
        </motion.div>
      ))}

      {/* Input funnel */}
      <div className="absolute left-2 top-1/2 -translate-y-1/2 text-xs text-muted-foreground">
        IN
      </div>

      {/* Output */}
      <div className="absolute right-2 top-1/2 -translate-y-1/2 text-xs text-muted-foreground">
        OUT
      </div>
    </div>
  );
}
