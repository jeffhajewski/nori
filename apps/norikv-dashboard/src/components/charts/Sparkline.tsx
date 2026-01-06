"use client";

import { useMemo, useId } from "react";
import { scaleLinear } from "d3-scale";
import { motion } from "framer-motion";
import { cn } from "@/lib/utils";

export interface SparklineDataPoint {
  timestamp: number;
  value: number;
}

export interface SparklineProps {
  /** Array of data points with timestamp and value */
  data: SparklineDataPoint[];
  /** Width of the sparkline in pixels (default: 120) */
  width?: number;
  /** Height of the sparkline in pixels (default: 32) */
  height?: number;
  /** Line color (default: primary) */
  color?: string;
  /** Optional threshold line value */
  thresholdValue?: number;
  /** Color for threshold line (default: warning) */
  thresholdColor?: string;
  /** Fill area under the curve (default: false) */
  showArea?: boolean;
  /** Animate on data change (default: true) */
  animate?: boolean;
  /** Show dots at data points (default: false) */
  showDots?: boolean;
  /** Additional CSS class */
  className?: string;
  /** Stroke width (default: 1.5) */
  strokeWidth?: number;
  /** Padding inside the SVG (default: 2) */
  padding?: number;
}

/**
 * Lightweight sparkline chart using SVG.
 * Designed to render in <5ms for 20 FPS performance budget.
 */
export function Sparkline({
  data,
  width = 120,
  height = 32,
  color = "hsl(var(--primary))",
  thresholdValue,
  thresholdColor = "hsl(var(--warning))",
  showArea = false,
  animate = true,
  showDots = false,
  className,
  strokeWidth = 1.5,
  padding = 2,
}: SparklineProps) {
  const id = useId();

  const { linePath, areaPath, thresholdY, domainY } = useMemo(() => {
    if (data.length === 0) {
      return { linePath: "", areaPath: "", thresholdY: null, domainY: [0, 1] };
    }

    // Calculate domains
    const timestamps = data.map((d) => d.timestamp);
    const values = data.map((d) => d.value);

    const minTime = Math.min(...timestamps);
    const maxTime = Math.max(...timestamps);
    const minValue = Math.min(...values);
    const maxValue = Math.max(...values);

    // Add some padding to the value range
    const valueRange = maxValue - minValue || 1;
    const valuePadding = valueRange * 0.1;
    const domainY: [number, number] = [
      Math.max(0, minValue - valuePadding),
      maxValue + valuePadding,
    ];

    // Create scales
    const xScale = scaleLinear()
      .domain([minTime, maxTime])
      .range([padding, width - padding]);

    const yScale = scaleLinear()
      .domain(domainY)
      .range([height - padding, padding]);

    // Generate line path
    const points = data.map((d) => ({
      x: xScale(d.timestamp),
      y: yScale(d.value),
    }));

    const linePath = points
      .map((p, i) => `${i === 0 ? "M" : "L"} ${p.x.toFixed(1)} ${p.y.toFixed(1)}`)
      .join(" ");

    // Generate area path (for fill)
    const areaPath = showArea
      ? `${linePath} L ${points[points.length - 1].x.toFixed(1)} ${height - padding} L ${points[0].x.toFixed(1)} ${height - padding} Z`
      : "";

    // Calculate threshold Y position
    const thresholdY =
      thresholdValue !== undefined ? yScale(thresholdValue) : null;

    return { linePath, areaPath, thresholdY, domainY };
  }, [data, width, height, padding, showArea, thresholdValue]);

  if (data.length === 0) {
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

  // Single data point - show as centered dot
  if (data.length === 1) {
    return (
      <svg
        width={width}
        height={height}
        className={className}
        role="img"
        aria-label="Sparkline chart"
      >
        <circle
          cx={width / 2}
          cy={height / 2}
          r={3}
          fill={color}
        />
      </svg>
    );
  }

  return (
    <svg
      width={width}
      height={height}
      className={className}
      role="img"
      aria-label="Sparkline chart"
    >
      {/* Gradient definition for area fill */}
      {showArea && (
        <defs>
          <linearGradient id={`gradient-${id}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={color} stopOpacity={0.3} />
            <stop offset="100%" stopColor={color} stopOpacity={0.05} />
          </linearGradient>
        </defs>
      )}

      {/* Area fill */}
      {showArea && areaPath && (
        <motion.path
          d={areaPath}
          fill={`url(#gradient-${id})`}
          initial={animate ? { opacity: 0 } : undefined}
          animate={{ opacity: 1 }}
          transition={{ duration: 0.3 }}
        />
      )}

      {/* Threshold line */}
      {thresholdY !== null && (
        <line
          x1={padding}
          y1={thresholdY}
          x2={width - padding}
          y2={thresholdY}
          stroke={thresholdColor}
          strokeWidth={1}
          strokeDasharray="3 3"
          opacity={0.7}
        />
      )}

      {/* Main line */}
      <motion.path
        d={linePath}
        fill="none"
        stroke={color}
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
        initial={animate ? { pathLength: 0, opacity: 0 } : undefined}
        animate={{ pathLength: 1, opacity: 1 }}
        transition={{ duration: 0.5, ease: "easeOut" }}
      />

      {/* Data point dots */}
      {showDots &&
        data.map((d, i) => {
          const xScale = scaleLinear()
            .domain([
              Math.min(...data.map((p) => p.timestamp)),
              Math.max(...data.map((p) => p.timestamp)),
            ])
            .range([padding, width - padding]);

          const yScale = scaleLinear()
            .domain(domainY)
            .range([height - padding, padding]);

          return (
            <circle
              key={i}
              cx={xScale(d.timestamp)}
              cy={yScale(d.value)}
              r={2}
              fill={color}
            />
          );
        })}
    </svg>
  );
}

/**
 * Mini sparkline variant with status color based on trend.
 * Automatically colors based on whether values are trending up or down.
 */
export interface TrendSparklineProps extends Omit<SparklineProps, "color"> {
  /** Whether increasing values are good (green) or bad (red) */
  increaseIsGood?: boolean;
}

export function TrendSparkline({
  data,
  increaseIsGood = true,
  ...props
}: TrendSparklineProps) {
  const trendColor = useMemo(() => {
    if (data.length < 2) return "hsl(var(--muted-foreground))";

    const firstHalf = data.slice(0, Math.floor(data.length / 2));
    const secondHalf = data.slice(Math.floor(data.length / 2));

    const avgFirst =
      firstHalf.reduce((a, b) => a + b.value, 0) / firstHalf.length;
    const avgSecond =
      secondHalf.reduce((a, b) => a + b.value, 0) / secondHalf.length;

    const isIncreasing = avgSecond > avgFirst;
    const isGood = increaseIsGood ? isIncreasing : !isIncreasing;

    return isGood ? "hsl(var(--status-healthy))" : "hsl(var(--status-warning))";
  }, [data, increaseIsGood]);

  return <Sparkline data={data} color={trendColor} {...props} />;
}

export default Sparkline;
