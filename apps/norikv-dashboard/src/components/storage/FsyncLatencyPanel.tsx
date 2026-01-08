"use client";

import { useMemo } from "react";
import { useFsyncLatencyData } from "@/stores/eventStore";
import { formatDuration, cn } from "@/lib/utils";
import { Sparkline, MiniHistogram } from "@/components/charts";

interface FsyncLatencyPanelProps {
  nodeId: number;
}

export function FsyncLatencyPanel({ nodeId }: FsyncLatencyPanelProps) {
  const fsyncData = useFsyncLatencyData(nodeId);

  const stats = useMemo(() => {
    if (!fsyncData || fsyncData.timeSeries.length === 0) return null;

    const values = fsyncData.timeSeries.map((p) => p.value);
    const sorted = [...values].sort((a, b) => a - b);

    return {
      count: values.length,
      min: sorted[0],
      max: sorted[sorted.length - 1],
      avg: values.reduce((a, b) => a + b, 0) / values.length,
      p50: sorted[Math.floor(sorted.length * 0.5)],
      p95: sorted[Math.floor(sorted.length * 0.95)],
      p99: sorted[Math.floor(sorted.length * 0.99)],
    };
  }, [fsyncData]);

  const sparklineData = useMemo(() => {
    if (!fsyncData) return [];
    return fsyncData.timeSeries.slice(-60).map((p) => ({
      timestamp: p.timestamp,
      value: p.value,
    }));
  }, [fsyncData]);

  const histogramBuckets = useMemo(() => {
    if (!fsyncData) return [];
    return fsyncData.buckets;
  }, [fsyncData]);

  // Find which bucket contains p95
  const p95BucketIndex = useMemo(() => {
    if (!stats || histogramBuckets.length === 0) return undefined;
    return histogramBuckets.findIndex(
      (b) => stats.p95 >= b.min && stats.p95 < b.max
    );
  }, [stats, histogramBuckets]);

  if (!fsyncData || !stats) {
    return (
      <div className="rounded-lg border border-dashed border-border p-4 text-center">
        <p className="text-sm text-muted-foreground">
          No fsync data for Node {nodeId}. Waiting for WAL events...
        </p>
      </div>
    );
  }

  // SLO thresholds
  const p95Healthy = stats.p95 < 10;
  const p99Healthy = stats.p99 < 20;

  return (
    <div className="space-y-4">
      {/* Latency trend sparkline */}
      <div>
        <div className="flex items-center justify-between mb-2">
          <h4 className="text-sm font-medium text-foreground">Fsync Latency Trend</h4>
          <span className="text-xs text-muted-foreground">{stats.count} samples</span>
        </div>
        {sparklineData.length > 1 ? (
          <Sparkline
            data={sparklineData}
            width={280}
            height={48}
            color="hsl(var(--primary))"
            thresholdValue={10}
            thresholdColor="hsl(var(--warning))"
            showArea
          />
        ) : (
          <div className="flex h-12 items-center justify-center rounded bg-muted/30 text-xs text-muted-foreground">
            Collecting data...
          </div>
        )}
      </div>

      {/* Distribution histogram */}
      {histogramBuckets.length > 0 && (
        <div>
          <h4 className="text-sm font-medium text-foreground mb-2">Distribution</h4>
          <MiniHistogram
            buckets={histogramBuckets}
            width={280}
            height={60}
            highlightBucket={p95BucketIndex}
            color="hsl(var(--primary))"
            highlightColor="hsl(var(--warning))"
          />
        </div>
      )}

      {/* Percentile stats */}
      <div className="grid grid-cols-4 gap-2">
        <StatBox label="Min" value={formatDuration(stats.min)} />
        <StatBox label="p50" value={formatDuration(stats.p50)} />
        <StatBox
          label="p95"
          value={formatDuration(stats.p95)}
          status={p95Healthy ? "healthy" : "warning"}
        />
        <StatBox
          label="p99"
          value={formatDuration(stats.p99)}
          status={p99Healthy ? "healthy" : "critical"}
        />
      </div>

      {/* SLO status */}
      {(!p95Healthy || !p99Healthy) && (
        <div
          className={cn(
            "rounded-lg px-3 py-2 text-xs",
            !p99Healthy
              ? "bg-status-critical/10 text-status-critical"
              : "bg-status-warning/10 text-status-warning"
          )}
        >
          {!p99Healthy
            ? `p99 latency (${formatDuration(stats.p99)}) exceeds 20ms SLO`
            : `p95 latency (${formatDuration(stats.p95)}) exceeds 10ms SLO`}
        </div>
      )}
    </div>
  );
}

function StatBox({
  label,
  value,
  status,
}: {
  label: string;
  value: string;
  status?: "healthy" | "warning" | "critical";
}) {
  const statusColors = {
    healthy: "text-status-healthy",
    warning: "text-status-warning",
    critical: "text-status-critical",
  };

  return (
    <div className="rounded bg-muted/50 px-2 py-1.5 text-center">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p
        className={cn(
          "font-medium",
          status ? statusColors[status] : "text-foreground"
        )}
      >
        {value}
      </p>
    </div>
  );
}
