/**
 * Time-series ring buffer with configurable retention and downsampling.
 * Used throughout the dashboard to track metric history for sparklines and trends.
 *
 * Memory budget: ~50KB per buffer at default settings (720 points × ~70 bytes each)
 */

export interface TimeSeriesPoint<T> {
  timestamp: number;
  value: T;
}

export type AggregationMode = "last" | "avg" | "max" | "min" | "sum";

export interface TimeSeriesConfig {
  /** Maximum number of points to retain (default: 720 for 60 mins at 12 points/min) */
  maxPoints: number;
  /** Merge points within this window in ms (default: 5000ms) */
  downsampleIntervalMs: number;
  /** How to aggregate values within a downsample window */
  aggregator: AggregationMode;
}

const DEFAULT_CONFIG: TimeSeriesConfig = {
  maxPoints: 720,
  downsampleIntervalMs: 5000,
  aggregator: "last",
};

/**
 * Aggregate numeric values based on the aggregation mode.
 */
function aggregateNumbers(values: number[], mode: AggregationMode): number {
  if (values.length === 0) return 0;

  switch (mode) {
    case "last":
      return values[values.length - 1];
    case "avg":
      return values.reduce((a, b) => a + b, 0) / values.length;
    case "max":
      return Math.max(...values);
    case "min":
      return Math.min(...values);
    case "sum":
      return values.reduce((a, b) => a + b, 0);
  }
}

/**
 * Generic ring buffer for time-series data with automatic downsampling.
 * Supports both primitive numbers and complex objects.
 */
export class TimeSeriesBuffer<T> {
  private buffer: TimeSeriesPoint<T>[] = [];
  private config: TimeSeriesConfig;
  private pendingWindow: { start: number; values: T[] } | null = null;

  constructor(config: Partial<TimeSeriesConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /**
   * Add a new data point. Points within the downsample window are aggregated.
   */
  push(timestamp: number, value: T): void {
    const windowStart = Math.floor(timestamp / this.config.downsampleIntervalMs) * this.config.downsampleIntervalMs;

    // If we have a pending window and this point is in a new window, flush the old one
    if (this.pendingWindow && this.pendingWindow.start !== windowStart) {
      this.flushPendingWindow();
    }

    // Initialize pending window if needed
    if (!this.pendingWindow) {
      this.pendingWindow = { start: windowStart, values: [] };
    }

    this.pendingWindow.values.push(value);
  }

  /**
   * Flush any pending values to the buffer.
   */
  private flushPendingWindow(): void {
    if (!this.pendingWindow || this.pendingWindow.values.length === 0) return;

    const aggregatedValue = this.aggregateValues(this.pendingWindow.values);
    this.buffer.push({
      timestamp: this.pendingWindow.start + this.config.downsampleIntervalMs / 2,
      value: aggregatedValue,
    });

    // Enforce max size
    while (this.buffer.length > this.config.maxPoints) {
      this.buffer.shift();
    }

    this.pendingWindow = null;
  }

  /**
   * Aggregate values based on their type and the configured mode.
   */
  private aggregateValues(values: T[]): T {
    if (values.length === 0) {
      throw new Error("Cannot aggregate empty values array");
    }

    // For primitive numbers, use numeric aggregation
    if (typeof values[0] === "number") {
      return aggregateNumbers(values as number[], this.config.aggregator) as T;
    }

    // For objects, use the last value (or could implement custom aggregation)
    // This works well for complex state objects where we want the latest snapshot
    return values[values.length - 1];
  }

  /**
   * Get points within a time range (inclusive).
   */
  getRange(startMs: number, endMs: number): TimeSeriesPoint<T>[] {
    this.flushPendingWindow();
    return this.buffer.filter((p) => p.timestamp >= startMs && p.timestamp <= endMs);
  }

  /**
   * Get the most recent N points.
   */
  getLast(count: number): TimeSeriesPoint<T>[] {
    this.flushPendingWindow();
    return this.buffer.slice(-count);
  }

  /**
   * Get the latest point, or null if buffer is empty.
   */
  getLatest(): TimeSeriesPoint<T> | null {
    this.flushPendingWindow();
    return this.buffer.length > 0 ? this.buffer[this.buffer.length - 1] : null;
  }

  /**
   * Get all points in the buffer.
   */
  getAll(): TimeSeriesPoint<T>[] {
    this.flushPendingWindow();
    return [...this.buffer];
  }

  /**
   * Get the current buffer length.
   */
  get length(): number {
    return this.buffer.length + (this.pendingWindow ? 1 : 0);
  }

  /**
   * Check if the buffer is empty.
   */
  get isEmpty(): boolean {
    return this.buffer.length === 0 && !this.pendingWindow;
  }

  /**
   * Clear all data from the buffer.
   */
  clear(): void {
    this.buffer = [];
    this.pendingWindow = null;
  }

  /**
   * Get statistics about numeric values in the buffer.
   * Only works when T is number.
   */
  getStats(): { min: number; max: number; avg: number; count: number } | null {
    this.flushPendingWindow();
    if (this.buffer.length === 0) return null;

    const values = this.buffer.map((p) => p.value);
    if (typeof values[0] !== "number") return null;

    const nums = values as number[];
    return {
      min: Math.min(...nums),
      max: Math.max(...nums),
      avg: nums.reduce((a, b) => a + b, 0) / nums.length,
      count: nums.length,
    };
  }
}

/**
 * Specialized buffer for tracking numeric metrics with pre-computed percentiles.
 * Useful for latency tracking where p50/p95/p99 are important.
 */
export class PercentilesBuffer extends TimeSeriesBuffer<number> {
  private sortedCache: number[] | null = null;
  private lastCacheLength = 0;

  constructor(config: Partial<TimeSeriesConfig> = {}) {
    super({ ...config, aggregator: "avg" });
  }

  /**
   * Get percentile values from the buffer.
   */
  getPercentiles(): { p50: number; p95: number; p99: number; min: number; max: number } | null {
    const points = this.getAll();
    if (points.length === 0) return null;

    // Invalidate cache if length changed
    if (points.length !== this.lastCacheLength) {
      this.sortedCache = null;
    }

    if (!this.sortedCache) {
      this.sortedCache = points.map((p) => p.value).sort((a, b) => a - b);
      this.lastCacheLength = this.sortedCache.length;
    }

    const sorted = this.sortedCache;
    return {
      p50: sorted[Math.floor(sorted.length * 0.5)],
      p95: sorted[Math.floor(sorted.length * 0.95)],
      p99: sorted[Math.floor(sorted.length * 0.99)],
      min: sorted[0],
      max: sorted[sorted.length - 1],
    };
  }

  override push(timestamp: number, value: number): void {
    this.sortedCache = null; // Invalidate cache on new data
    super.push(timestamp, value);
  }

  override clear(): void {
    this.sortedCache = null;
    this.lastCacheLength = 0;
    super.clear();
  }
}

/**
 * Buffer for tracking histogram buckets (e.g., latency distribution).
 * Maintains both time-series data and histogram buckets.
 */
export class HistogramBuffer {
  private timeSeries: TimeSeriesBuffer<number>;
  private buckets: Map<string, number> = new Map();
  private bucketBoundaries: number[];

  constructor(bucketBoundaries: number[], config: Partial<TimeSeriesConfig> = {}) {
    this.bucketBoundaries = bucketBoundaries.sort((a, b) => a - b);
    this.timeSeries = new TimeSeriesBuffer<number>(config);
  }

  /**
   * Add a value to both the time-series and histogram.
   */
  push(timestamp: number, value: number): void {
    this.timeSeries.push(timestamp, value);

    // Find the appropriate bucket
    let bucketIdx = this.bucketBoundaries.findIndex((b) => value < b);
    if (bucketIdx === -1) bucketIdx = this.bucketBoundaries.length;

    const bucketKey =
      bucketIdx === 0
        ? `<${this.bucketBoundaries[0]}`
        : bucketIdx === this.bucketBoundaries.length
          ? `>=${this.bucketBoundaries[bucketIdx - 1]}`
          : `${this.bucketBoundaries[bucketIdx - 1]}-${this.bucketBoundaries[bucketIdx]}`;

    this.buckets.set(bucketKey, (this.buckets.get(bucketKey) || 0) + 1);
  }

  /**
   * Get histogram bucket counts.
   */
  getBuckets(): Array<{ label: string; min: number; max: number; count: number }> {
    const result: Array<{ label: string; min: number; max: number; count: number }> = [];

    // First bucket: [0, boundaries[0])
    const firstKey = `<${this.bucketBoundaries[0]}`;
    result.push({
      label: firstKey,
      min: 0,
      max: this.bucketBoundaries[0],
      count: this.buckets.get(firstKey) || 0,
    });

    // Middle buckets
    for (let i = 0; i < this.bucketBoundaries.length - 1; i++) {
      const key = `${this.bucketBoundaries[i]}-${this.bucketBoundaries[i + 1]}`;
      result.push({
        label: key,
        min: this.bucketBoundaries[i],
        max: this.bucketBoundaries[i + 1],
        count: this.buckets.get(key) || 0,
      });
    }

    // Last bucket: [boundaries[n-1], infinity)
    const lastBoundary = this.bucketBoundaries[this.bucketBoundaries.length - 1];
    const lastKey = `>=${lastBoundary}`;
    result.push({
      label: lastKey,
      min: lastBoundary,
      max: Infinity,
      count: this.buckets.get(lastKey) || 0,
    });

    return result;
  }

  /**
   * Get the time-series data.
   */
  getTimeSeries(): TimeSeriesPoint<number>[] {
    return this.timeSeries.getAll();
  }

  /**
   * Get the last N time-series points.
   */
  getLast(count: number): TimeSeriesPoint<number>[] {
    return this.timeSeries.getLast(count);
  }

  /**
   * Get total sample count.
   */
  get totalCount(): number {
    let total = 0;
    this.buckets.forEach((count) => (total += count));
    return total;
  }

  /**
   * Clear all data.
   */
  clear(): void {
    this.timeSeries.clear();
    this.buckets.clear();
  }
}
