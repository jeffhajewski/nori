"use client";

import { useMemo } from "react";
import { scaleLinear, scaleBand } from "d3-scale";
import { motion } from "framer-motion";
import { cn } from "@/lib/utils";

export interface HistogramBucket {
  /** Bucket label (e.g., "0-10ms") */
  label: string;
  /** Lower bound of the bucket */
  min: number;
  /** Upper bound of the bucket */
  max: number;
  /** Count of items in this bucket */
  count: number;
}

export interface MiniHistogramProps {
  /** Array of histogram buckets */
  buckets: HistogramBucket[];
  /** Width of the histogram in pixels (default: 200) */
  width?: number;
  /** Height of the histogram in pixels (default: 60) */
  height?: number;
  /** Bar color (default: primary) */
  color?: string;
  /** Index of bucket to highlight (e.g., p95 bucket) */
  highlightBucket?: number;
  /** Color for highlighted bucket (default: warning) */
  highlightColor?: string;
  /** Show percentile markers (default: false) */
  showPercentiles?: boolean;
  /** Percentile values to show as markers */
  percentiles?: { p50?: number; p95?: number; p99?: number };
  /** Additional CSS class */
  className?: string;
  /** Animate bars on load (default: true) */
  animate?: boolean;
  /** Padding between bars (0-1, default: 0.2) */
  barPadding?: number;
}

/**
 * Mini histogram chart for distribution visualization.
 * Optimized for latency distribution display with percentile highlighting.
 */
export function MiniHistogram({
  buckets,
  width = 200,
  height = 60,
  color = "hsl(var(--primary))",
  highlightBucket,
  highlightColor = "hsl(var(--warning))",
  showPercentiles = false,
  percentiles,
  className,
  animate = true,
  barPadding = 0.2,
}: MiniHistogramProps) {
  const padding = { top: 4, right: 4, bottom: 16, left: 4 };
  const chartWidth = width - padding.left - padding.right;
  const chartHeight = height - padding.top - padding.bottom;

  const { xScale, yScale, maxCount, percentilePositions } = useMemo(() => {
    const labels = buckets.map((b) => b.label);
    const counts = buckets.map((b) => b.count);
    const maxCount = Math.max(...counts, 1);

    const xScale = scaleBand<string>()
      .domain(labels)
      .range([0, chartWidth])
      .padding(barPadding);

    const yScale = scaleLinear()
      .domain([0, maxCount])
      .range([chartHeight, 0]);

    // Calculate percentile marker positions
    let percentilePositions: { value: number; x: number; label: string }[] = [];
    if (showPercentiles && percentiles) {
      const calcPosition = (value: number, label: string) => {
        // Find which bucket this percentile falls into
        const bucketIndex = buckets.findIndex(
          (b) => value >= b.min && value < b.max
        );
        if (bucketIndex !== -1) {
          const bucket = buckets[bucketIndex];
          const bucketX = xScale(bucket.label) || 0;
          const bucketWidth = xScale.bandwidth();
          // Interpolate position within the bucket
          const ratio = (value - bucket.min) / (bucket.max - bucket.min || 1);
          return {
            value,
            x: bucketX + bucketWidth * ratio,
            label,
          };
        }
        return null;
      };

      if (percentiles.p50 !== undefined) {
        const pos = calcPosition(percentiles.p50, "p50");
        if (pos) percentilePositions.push(pos);
      }
      if (percentiles.p95 !== undefined) {
        const pos = calcPosition(percentiles.p95, "p95");
        if (pos) percentilePositions.push(pos);
      }
      if (percentiles.p99 !== undefined) {
        const pos = calcPosition(percentiles.p99, "p99");
        if (pos) percentilePositions.push(pos);
      }
    }

    return { xScale, yScale, maxCount, percentilePositions };
  }, [buckets, chartWidth, chartHeight, barPadding, showPercentiles, percentiles]);

  if (buckets.length === 0) {
    return (
      <div
        className={cn(
          "flex items-center justify-center text-xs text-muted-foreground",
          className
        )}
        style={{ width, height }}
      >
        No data
      </div>
    );
  }

  return (
    <svg
      width={width}
      height={height}
      className={className}
      role="img"
      aria-label="Histogram chart"
    >
      <g transform={`translate(${padding.left}, ${padding.top})`}>
        {/* Bars */}
        {buckets.map((bucket, i) => {
          const barHeight = chartHeight - yScale(bucket.count);
          const isHighlighted = i === highlightBucket;

          return (
            <g key={bucket.label}>
              <motion.rect
                x={xScale(bucket.label)}
                y={yScale(bucket.count)}
                width={xScale.bandwidth()}
                height={barHeight}
                fill={isHighlighted ? highlightColor : color}
                opacity={isHighlighted ? 1 : 0.7}
                rx={1}
                initial={animate ? { height: 0, y: chartHeight } : undefined}
                animate={{ height: barHeight, y: yScale(bucket.count) }}
                transition={{ duration: 0.3, delay: i * 0.03 }}
              />
              {/* Highlight ring for selected bucket */}
              {isHighlighted && (
                <rect
                  x={(xScale(bucket.label) || 0) - 1}
                  y={yScale(bucket.count) - 1}
                  width={xScale.bandwidth() + 2}
                  height={barHeight + 2}
                  fill="none"
                  stroke={highlightColor}
                  strokeWidth={1.5}
                  rx={2}
                />
              )}
            </g>
          );
        })}

        {/* Percentile markers */}
        {percentilePositions.map((p) => (
          <g key={p.label}>
            <line
              x1={p.x}
              y1={0}
              x2={p.x}
              y2={chartHeight}
              stroke="hsl(var(--foreground))"
              strokeWidth={1}
              strokeDasharray="2 2"
              opacity={0.5}
            />
            <text
              x={p.x}
              y={-2}
              textAnchor="middle"
              className="fill-foreground text-[8px]"
            >
              {p.label}
            </text>
          </g>
        ))}

        {/* X-axis labels (simplified - show first, middle, last) */}
        {buckets.length > 0 && (
          <>
            <text
              x={xScale(buckets[0].label)! + xScale.bandwidth() / 2}
              y={chartHeight + 12}
              textAnchor="middle"
              className="fill-muted-foreground text-[9px]"
            >
              {formatBucketLabel(buckets[0])}
            </text>
            {buckets.length > 2 && (
              <text
                x={
                  xScale(buckets[buckets.length - 1].label)! +
                  xScale.bandwidth() / 2
                }
                y={chartHeight + 12}
                textAnchor="middle"
                className="fill-muted-foreground text-[9px]"
              >
                {formatBucketLabel(buckets[buckets.length - 1])}
              </text>
            )}
          </>
        )}
      </g>
    </svg>
  );
}

/**
 * Format bucket label for display.
 */
function formatBucketLabel(bucket: HistogramBucket): string {
  if (bucket.max === Infinity) return `${bucket.min}+`;
  if (bucket.min === 0) return `<${bucket.max}`;
  return `${bucket.min}`;
}

/**
 * Compact histogram variant showing only bars without labels.
 * Useful for inline display in tables or cards.
 */
export interface CompactHistogramProps {
  /** Array of counts for each bucket */
  counts: number[];
  /** Width of the histogram (default: 60) */
  width?: number;
  /** Height of the histogram (default: 20) */
  height?: number;
  /** Bar color (default: primary) */
  color?: string;
  /** Additional CSS class */
  className?: string;
}

export function CompactHistogram({
  counts,
  width = 60,
  height = 20,
  color = "hsl(var(--primary))",
  className,
}: CompactHistogramProps) {
  const maxCount = Math.max(...counts, 1);
  const barWidth = width / counts.length;

  return (
    <svg width={width} height={height} className={className}>
      {counts.map((count, i) => {
        const barHeight = (count / maxCount) * height;
        return (
          <rect
            key={i}
            x={i * barWidth}
            y={height - barHeight}
            width={Math.max(barWidth - 1, 1)}
            height={barHeight}
            fill={color}
            opacity={0.8}
          />
        );
      })}
    </svg>
  );
}

export default MiniHistogram;
