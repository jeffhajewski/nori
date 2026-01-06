"use client";

import { useMemo } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { cn } from "@/lib/utils";
import { formatDuration, formatTimestamp } from "@/lib/utils";
import { useWritePressureHistory } from "@/stores/eventStore";
import { Sparkline } from "@/components/charts";

interface WritePressureGaugeProps {
  nodeId: number;
  pressure: number; // 0-1
  isHigh: boolean;
  threshold: number;
  l0FileCount: number;
  l0Threshold: number;
}

export function WritePressureGauge({
  nodeId,
  pressure,
  isHigh,
  threshold,
  l0FileCount,
  l0Threshold,
}: WritePressureGaugeProps) {
  const pressureHistory = useWritePressureHistory(nodeId, 60);
  const pct = Math.min(pressure * 100, 100);
  const angle = (pct / 100) * 180 - 90; // -90 to 90 degrees

  // Find the last breach
  const lastBreach = useMemo(() => {
    const breaches = pressureHistory.filter((p) => p.value.high);
    if (breaches.length === 0) return null;
    return breaches[breaches.length - 1];
  }, [pressureHistory]);

  const timeSinceLastBreach = lastBreach ? Date.now() - lastBreach.timestamp : null;

  // Prepare sparkline data
  const sparklineData = useMemo(() => {
    return pressureHistory.map((p) => ({
      timestamp: p.timestamp,
      value: p.value.ratio,
    }));
  }, [pressureHistory]);

  // Determine color based on pressure
  const getColor = (p: number) => {
    if (p < 0.3) return "text-status-healthy";
    if (p < 0.6) return "text-status-warning";
    return "text-status-critical";
  };

  const color = getColor(pressure);
  const strokeColor = isHigh ? "#ef4444" : pressure > 0.5 ? "#f59e0b" : "#22c55e";

  return (
    <div className="flex flex-col items-center">
      {/* Gauge */}
      <div className="relative h-32 w-48">
        {/* Pulse animation when high */}
        <AnimatePresence>
          {isHigh && (
            <motion.div
              className="absolute inset-0 rounded-full border-4 border-status-critical"
              initial={{ opacity: 0.5, scale: 0.9 }}
              animate={{ opacity: 0, scale: 1.1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 1, repeat: Infinity }}
            />
          )}
        </AnimatePresence>

        {/* Background arc */}
        <svg className="absolute inset-0 h-full w-full" viewBox="0 0 100 60">
          <path
            d="M 10 55 A 40 40 0 0 1 90 55"
            fill="none"
            stroke="currentColor"
            strokeWidth="8"
            strokeLinecap="round"
            className="text-muted/30"
          />
          {/* Threshold marker */}
          <circle
            cx={50 + 40 * Math.cos((threshold * 180 - 90) * (Math.PI / 180))}
            cy={55 + 40 * Math.sin((threshold * 180 - 90) * (Math.PI / 180))}
            r="3"
            fill="#f59e0b"
          />
          {/* Value arc */}
          <motion.path
            d="M 10 55 A 40 40 0 0 1 90 55"
            fill="none"
            stroke={strokeColor}
            strokeWidth="8"
            strokeLinecap="round"
            strokeDasharray="126"
            initial={{ strokeDashoffset: 126 }}
            animate={{ strokeDashoffset: 126 - (pct / 100) * 126 }}
            transition={{ duration: 0.5, ease: "easeOut" }}
          />
        </svg>

        {/* Needle */}
        <motion.div
          className="absolute left-1/2 top-[55px] h-8 w-1 origin-bottom -translate-x-1/2"
          style={{ transformOrigin: "bottom center" }}
          initial={{ rotate: -90 }}
          animate={{ rotate: angle }}
          transition={{ duration: 0.5, ease: "easeOut" }}
        >
          <div className="h-full w-full rounded-full bg-foreground" />
          <div className="absolute -bottom-1 left-1/2 h-3 w-3 -translate-x-1/2 rounded-full bg-foreground" />
        </motion.div>

        {/* Center value */}
        <div className="absolute bottom-0 left-1/2 -translate-x-1/2 text-center">
          <p className={cn("text-2xl font-bold", color)}>{pct.toFixed(0)}%</p>
          <p className="text-xs text-muted-foreground">Write Pressure</p>
        </div>
      </div>

      {/* Status indicator */}
      <div
        className={cn(
          "mt-2 rounded-full px-3 py-1 text-xs font-medium",
          isHigh
            ? "bg-status-critical/20 text-status-critical"
            : pressure > threshold
            ? "bg-status-warning/20 text-status-warning"
            : "bg-status-healthy/20 text-status-healthy"
        )}
      >
        {isHigh ? "THROTTLING" : pressure > threshold ? "ELEVATED" : "NORMAL"}
      </div>

      {/* Pressure trend sparkline */}
      <div className="mt-4 w-full">
        <div className="flex items-center justify-between text-xs text-muted-foreground mb-1">
          <span>Pressure Trend (1h)</span>
          {timeSinceLastBreach !== null && timeSinceLastBreach < 3600000 && (
            <span className="text-status-warning">
              Last breach: {formatDuration(timeSinceLastBreach)} ago
            </span>
          )}
        </div>
        {sparklineData.length > 1 ? (
          <Sparkline
            data={sparklineData}
            width={192}
            height={40}
            color={strokeColor}
            thresholdValue={threshold}
            thresholdColor="hsl(var(--warning))"
            showArea
            className="w-full"
          />
        ) : (
          <div className="flex h-10 items-center justify-center rounded bg-muted/30 text-xs text-muted-foreground">
            Collecting data...
          </div>
        )}
      </div>

      {/* L0 file count */}
      <div className="mt-4 w-full">
        <div className="flex items-center justify-between text-xs">
          <span className="text-muted-foreground">L0 Files</span>
          <span className={cn(
            l0FileCount >= l0Threshold ? "text-status-critical font-medium" : "text-foreground"
          )}>
            {l0FileCount} / {l0Threshold}
          </span>
        </div>
        <div className="mt-1 h-2 overflow-hidden rounded-full bg-muted">
          <motion.div
            className={cn(
              "h-full",
              l0FileCount >= l0Threshold ? "bg-status-critical" : "bg-primary"
            )}
            initial={{ width: 0 }}
            animate={{ width: `${Math.min((l0FileCount / l0Threshold) * 100, 100)}%` }}
            transition={{ duration: 0.3 }}
          />
        </div>
        {l0FileCount >= l0Threshold && (
          <p className="mt-1 text-xs text-status-critical">
            L0 stall triggered - compaction backlog
          </p>
        )}
      </div>
    </div>
  );
}
