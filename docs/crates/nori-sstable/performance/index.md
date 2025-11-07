---
layout: default
title: Performance
nav_order: 5
has_children: true
parent: nori-sstable
grand_parent: Crates
---

# Performance
{: .no_toc }

Benchmarks, tuning guides, and performance analysis for nori-sstable.
{: .fs-6 .fw-300 }

---

## Overview

This section covers performance characteristics, benchmarks, and optimization techniques for nori-sstable.

## Topics

### [Benchmarks](benchmarks)
Comprehensive benchmark results for different workloads and configurations.

### [Tuning Guide](tuning)
How to configure nori-sstable for your specific workload.

### [Compression Ratios](compression-ratios)
Real-world compression ratios for different data types and algorithms.

### [Cache Hit Rates](cache-analysis)
Analysis of cache performance across different workload patterns.

### [Profiling](profiling)
How to profile and optimize nori-sstable performance.

---

## Quick Wins

**Hot workloads:** Increase `block_cache_mb` from 64MB to 256MB+
**Large datasets:** Use `Compression::Zstd` for 3-5x size reduction
**Cold storage:** Disable cache (`block_cache_mb: 0`) to save memory
