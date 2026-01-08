"use client";

import { useMemo } from "react";
import { motion } from "framer-motion";
import { useCacheHitRatios } from "@/stores/eventStore";
import { cn } from "@/lib/utils";
import { Sparkline } from "@/components/charts";

export function CacheHitRatioPanel() {
  const caches = useCacheHitRatios();

  if (caches.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-border p-4 text-center">
        <p className="text-sm text-muted-foreground">
          No cache data yet. Waiting for Cache events...
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {caches.map((cache) => (
        <CacheRow key={cache.name} cache={cache} />
      ))}
    </div>
  );
}

interface CacheRowProps {
  cache: {
    name: string;
    current: number;
    history: Array<{ timestamp: number; value: number }>;
  };
}

function CacheRow({ cache }: CacheRowProps) {
  const avgRatio = useMemo(() => {
    if (cache.history.length === 0) return 0;
    return cache.history.reduce((s, p) => s + p.value, 0) / cache.history.length;
  }, [cache.history]);

  const sparklineData = useMemo(() => {
    return cache.history.map((p) => ({
      timestamp: p.timestamp,
      value: p.value,
    }));
  }, [cache.history]);

  const status: "healthy" | "warning" | "critical" =
    cache.current >= 0.9 ? "healthy" : cache.current >= 0.7 ? "warning" : "critical";

  const statusColors = {
    healthy: {
      badge: "text-status-healthy bg-status-healthy/10",
      line: "hsl(var(--status-healthy))",
    },
    warning: {
      badge: "text-status-warning bg-status-warning/10",
      line: "hsl(var(--status-warning))",
    },
    critical: {
      badge: "text-status-critical bg-status-critical/10",
      line: "hsl(var(--status-critical))",
    },
  };

  // Format cache name for display
  const displayName = cache.name
    .replace(/_/g, " ")
    .replace(/\b\w/g, (c) => c.toUpperCase());

  return (
    <motion.div
      layout
      className="rounded-lg border border-border bg-card p-3"
      initial={{ opacity: 0, y: -5 }}
      animate={{ opacity: 1, y: 0 }}
    >
      <div className="flex items-center justify-between mb-2">
        <div className="flex items-center gap-2">
          <span className="font-medium text-foreground">{displayName}</span>
          <span
            className={cn(
              "rounded px-1.5 py-0.5 text-xs font-medium",
              statusColors[status].badge
            )}
          >
            {(cache.current * 100).toFixed(1)}%
          </span>
        </div>
        <span className="text-xs text-muted-foreground">
          Avg: {(avgRatio * 100).toFixed(1)}%
        </span>
      </div>

      {sparklineData.length > 1 ? (
        <Sparkline
          data={sparklineData}
          width={260}
          height={32}
          color={statusColors[status].line}
          thresholdValue={0.9}
          thresholdColor="hsl(var(--muted-foreground))"
          showArea
        />
      ) : (
        <div className="flex h-8 items-center justify-center rounded bg-muted/30 text-xs text-muted-foreground">
          Collecting data...
        </div>
      )}
    </motion.div>
  );
}
