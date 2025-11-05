package com.norikv.client.benchmark;

import java.util.*;
import java.util.concurrent.TimeUnit;

/**
 * Benchmark runner for measuring performance metrics.
 *
 * <p>Collects latency measurements and computes percentiles, mean, and throughput.
 */
public class BenchmarkRunner {
    private final String name;
    private final List<Long> latenciesNs;
    private final long startTimeNs;
    private long endTimeNs;

    public BenchmarkRunner(String name) {
        this.name = name;
        this.latenciesNs = new ArrayList<>();
        this.startTimeNs = System.nanoTime();
    }

    /**
     * Records a single operation latency.
     *
     * @param latencyNs latency in nanoseconds
     */
    public void recordLatency(long latencyNs) {
        latenciesNs.add(latencyNs);
    }

    /**
     * Completes the benchmark and computes results.
     *
     * @return benchmark results
     */
    public BenchmarkResult complete() {
        endTimeNs = System.nanoTime();
        return new BenchmarkResult(name, latenciesNs, startTimeNs, endTimeNs);
    }

    /**
     * Results from a benchmark run.
     */
    public static class BenchmarkResult {
        private final String name;
        private final int totalOps;
        private final double durationSeconds;
        private final double throughputOpsPerSec;
        private final LatencyStats latencyStats;

        BenchmarkResult(String name, List<Long> latenciesNs, long startTimeNs, long endTimeNs) {
            this.name = name;
            this.totalOps = latenciesNs.size();
            this.durationSeconds = (endTimeNs - startTimeNs) / 1_000_000_000.0;
            this.throughputOpsPerSec = totalOps / durationSeconds;
            this.latencyStats = computeStats(latenciesNs);
        }

        private LatencyStats computeStats(List<Long> latenciesNs) {
            if (latenciesNs.isEmpty()) {
                return new LatencyStats(0, 0, 0, 0, 0, 0, 0);
            }

            // Sort for percentile calculation
            List<Long> sorted = new ArrayList<>(latenciesNs);
            Collections.sort(sorted);

            long min = sorted.get(0);
            long max = sorted.get(sorted.size() - 1);

            // Mean
            long sum = 0;
            for (long l : sorted) {
                sum += l;
            }
            double mean = sum / (double) sorted.size();

            // Percentiles
            double p50 = percentile(sorted, 0.50);
            double p95 = percentile(sorted, 0.95);
            double p99 = percentile(sorted, 0.99);
            double p999 = percentile(sorted, 0.999);

            return new LatencyStats(min, max, mean, p50, p95, p99, p999);
        }

        private double percentile(List<Long> sorted, double percentile) {
            if (sorted.isEmpty()) {
                return 0;
            }
            int index = (int) Math.ceil(percentile * sorted.size()) - 1;
            index = Math.max(0, Math.min(index, sorted.size() - 1));
            return sorted.get(index);
        }

        public String getName() {
            return name;
        }

        public int getTotalOps() {
            return totalOps;
        }

        public double getDurationSeconds() {
            return durationSeconds;
        }

        public double getThroughputOpsPerSec() {
            return throughputOpsPerSec;
        }

        public LatencyStats getLatencyStats() {
            return latencyStats;
        }

        /**
         * Prints a formatted report of the benchmark results.
         */
        public void printReport() {
            System.out.println("========================================");
            System.out.println("Benchmark: " + name);
            System.out.println("========================================");
            System.out.printf("Total Operations: %,d%n", totalOps);
            System.out.printf("Duration: %.2f seconds%n", durationSeconds);
            System.out.printf("Throughput: %,.0f ops/sec%n", throughputOpsPerSec);
            System.out.println();
            System.out.println("Latency Statistics:");
            System.out.printf("  Min:    %,d ns (%.3f ms)%n",
                latencyStats.min, latencyStats.min / 1_000_000.0);
            System.out.printf("  Mean:   %,.0f ns (%.3f ms)%n",
                latencyStats.mean, latencyStats.mean / 1_000_000.0);
            System.out.printf("  p50:    %,.0f ns (%.3f ms)%n",
                latencyStats.p50, latencyStats.p50 / 1_000_000.0);
            System.out.printf("  p95:    %,.0f ns (%.3f ms)%n",
                latencyStats.p95, latencyStats.p95 / 1_000_000.0);
            System.out.printf("  p99:    %,.0f ns (%.3f ms)%n",
                latencyStats.p99, latencyStats.p99 / 1_000_000.0);
            System.out.printf("  p99.9:  %,.0f ns (%.3f ms)%n",
                latencyStats.p999, latencyStats.p999 / 1_000_000.0);
            System.out.printf("  Max:    %,d ns (%.3f ms)%n",
                latencyStats.max, latencyStats.max / 1_000_000.0);
            System.out.println();

            // SLO validation
            double p95Ms = latencyStats.p95 / 1_000_000.0;
            System.out.println("SLO Validation:");
            if (name.contains("GET")) {
                boolean pass = p95Ms <= 10.0;
                System.out.printf("  GET p95 target: 10ms, actual: %.3f ms [%s]%n",
                    p95Ms, pass ? "PASS ✓" : "FAIL ✗");
            } else if (name.contains("PUT")) {
                boolean pass = p95Ms <= 20.0;
                System.out.printf("  PUT p95 target: 20ms, actual: %.3f ms [%s]%n",
                    p95Ms, pass ? "PASS ✓" : "FAIL ✗");
            }
            System.out.println("========================================");
            System.out.println();
        }

        /**
         * Returns a CSV header for benchmark results.
         */
        public static String csvHeader() {
            return "Benchmark,TotalOps,DurationSec,ThroughputOpsPerSec," +
                   "MinNs,MeanNs,P50Ns,P95Ns,P99Ns,P999Ns,MaxNs," +
                   "MinMs,MeanMs,P50Ms,P95Ms,P99Ms,P999Ms,MaxMs";
        }

        /**
         * Returns benchmark results as CSV row.
         */
        public String toCsv() {
            return String.format("%s,%d,%.3f,%.0f,%d,%.0f,%.0f,%.0f,%.0f,%.0f,%d," +
                               "%.3f,%.3f,%.3f,%.3f,%.3f,%.3f,%.3f",
                name, totalOps, durationSeconds, throughputOpsPerSec,
                latencyStats.min, latencyStats.mean, latencyStats.p50,
                latencyStats.p95, latencyStats.p99, latencyStats.p999, latencyStats.max,
                latencyStats.min / 1_000_000.0, latencyStats.mean / 1_000_000.0,
                latencyStats.p50 / 1_000_000.0, latencyStats.p95 / 1_000_000.0,
                latencyStats.p99 / 1_000_000.0, latencyStats.p999 / 1_000_000.0,
                latencyStats.max / 1_000_000.0);
        }
    }

    /**
     * Latency statistics.
     */
    public static class LatencyStats {
        private final long min;
        private final long max;
        private final double mean;
        private final double p50;
        private final double p95;
        private final double p99;
        private final double p999;

        LatencyStats(long min, long max, double mean, double p50, double p95, double p99, double p999) {
            this.min = min;
            this.max = max;
            this.mean = mean;
            this.p50 = p50;
            this.p95 = p95;
            this.p99 = p99;
            this.p999 = p999;
        }

        public long getMin() {
            return min;
        }

        public long getMax() {
            return max;
        }

        public double getMean() {
            return mean;
        }

        public double getP50() {
            return p50;
        }

        public double getP95() {
            return p95;
        }

        public double getP99() {
            return p99;
        }

        public double getP999() {
            return p999;
        }

        public double getP95Ms() {
            return p95 / 1_000_000.0;
        }

        public double getP99Ms() {
            return p99 / 1_000_000.0;
        }
    }

    /**
     * Utility to measure operation latency.
     */
    public static long measureNs(Runnable operation) {
        long start = System.nanoTime();
        operation.run();
        return System.nanoTime() - start;
    }

    /**
     * Utility to measure operation latency with return value.
     */
    public static <T> MeasuredResult<T> measure(java.util.function.Supplier<T> operation) {
        long start = System.nanoTime();
        T result = operation.get();
        long latency = System.nanoTime() - start;
        return new MeasuredResult<>(result, latency);
    }

    /**
     * Result with measured latency.
     */
    public static class MeasuredResult<T> {
        private final T result;
        private final long latencyNs;

        MeasuredResult(T result, long latencyNs) {
            this.result = result;
            this.latencyNs = latencyNs;
        }

        public T getResult() {
            return result;
        }

        public long getLatencyNs() {
            return latencyNs;
        }
    }
}
