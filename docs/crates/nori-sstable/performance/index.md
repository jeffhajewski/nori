# Performance

Benchmarks, tuning guides, and performance analysis for nori-sstable.

---

## Overview

This section covers performance characteristics, benchmarks, and optimization techniques for nori-sstable.

## Topics

### [Benchmarks](benchmarks.md)
Comprehensive benchmark results for different workloads and configurations.

### [Tuning Guide](tuning.md)
How to configure nori-sstable for your specific workload.

### [Compression Ratios](compression-ratios.md)
Real-world compression ratios for different data types and algorithms.

### [Cache Hit Rates](cache-analysis.md)
Analysis of cache performance across different workload patterns.

### [Profiling](profiling.md)
How to profile and optimize nori-sstable performance.

---

## Quick Wins

**Hot workloads:** Increase `block_cache_mb` from 64MB to 256MB+
**Large datasets:** Use `Compression::Zstd` for 3-5x size reduction
**Cold storage:** Disable cache (`block_cache_mb: 0`) to save memory
