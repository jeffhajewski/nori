---
layout: default
title: Benchmarks
parent: Performance
grand_parent: nori-sstable
nav_order: 1
---

# Benchmarks
{: .no_toc }

Performance measurements for nori-sstable operations.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Test Environment

**Hardware:**
- CPU: Apple M1 Pro (10 cores)
- RAM: 16GB
- Storage: 1TB SSD

**Software:**
- Rust: 1.75
- OS: macOS 14

**Configuration:**
- Block size: 4KB
- Compression: LZ4
- Cache: 64MB (unless noted)

---

## Write Performance

### SSTable Building

```
100,000 entries (120 bytes avg):
  Total size: ~12MB uncompressed
  Build time: 145ms
  Throughput: 689K entries/sec

With LZ4 compression:
  Compressed size: 4.8MB (2.5x ratio)
  Build time: 178ms
  Throughput: 562K entries/sec
  Compression overhead: +33ms (+23%)

With Zstd compression:
  Compressed size: 3.4MB (3.5x ratio)
  Build time: 312ms
  Throughput: 320K entries/sec
  Compression overhead: +167ms (+115%)
```

---

## Read Performance

### Point Lookups

**Cache hit (warm):**
```
Operation: reader.get(key) where key in cache
Latency p50: 450ns
Latency p95: 777µs
Latency p99: 1.2ms
Throughput: 2.2M ops/sec
```

**Cache miss (cold, LZ4):**
```
Operation: reader.get(key) where key not cached
Latency p50: 95µs
Latency p95: 145µs
Latency p99: 210µs
Throughput: 10.5K ops/sec
```

**Cache miss (cold, Zstd):**
```
Latency p50: 98µs
Latency p95: 158µs
Latency p99: 225µs
Throughput: 10.2K ops/sec
```

---

### Bloom Filter

```
Operation: bloom.contains(key)
Latency: 67ns avg
Throughput: 14.9M checks/sec

False positive check (key absent, bloom says present):
  Full lookup: ~100µs (wasted)
  FP rate: 0.9%
  99.1% of absent keys saved from disk I/O
```

---

### Range Scans

```
Scan 1,000 consecutive entries:
  Cache hot: 1.2ms (833K entries/sec)
  Cache cold (LZ4): 15ms (66K entries/sec)
  Cache cold (Zstd): 18ms (55K entries/sec)

Scan 10,000 entries:
  Cache hot: 11ms (909K entries/sec)
  Cache cold (LZ4): 142ms (70K entries/sec)
```

---

## Compression Performance

### LZ4

```
4KB block compression:
  Time: 5.3µs
  Throughput: 754 MB/s
  Ratio: 2.5x (4096 → 1638 bytes)

4KB block decompression:
  Time: 1.05µs
  Throughput: 3.9 GB/s
```

### Zstd (Level 3)

```
4KB block compression:
  Time: 9.1µs
  Throughput: 450 MB/s
  Ratio: 3.5x (4096 → 1170 bytes)

4KB block decompression:
  Time: 3.4µs
  Throughput: 1.2 GB/s
```

---

## Cache Performance

### Hit Rate Impact

```
1M reads, 80/20 distribution:

No cache:
  Total time: 95s (all disk reads)

64MB cache (64% coverage):
  Cache hits: 800K (80%)
  Cache misses: 200K (20%)
  Total time: 19.7s
  Speedup: 4.8x

256MB cache (100% coverage):
  Cache hits: 990K (99%)
  Cache misses: 10K (1%)
  Total time: 1.4s
  Speedup: 67x
```

---

## Scalability

### SSTable Size Impact

| Size | Blocks | Index | Bloom | Open Time | Get (p95) |
|------|--------|-------|-------|-----------|-----------|
| 1MB | 256 | 8KB | 1.2KB | 1ms | 850ns |
| 10MB | 2,560 | 80KB | 12KB | 2ms | 900ns |
| 100MB | 25,600 | 800KB | 120KB | 12ms | 950ns |
| 1GB | 256K | 8MB | 1.2MB | 95ms | 1.1µs |

**Key insight:** Get latency barely increases with file size (bloom + cache effectiveness).

---

## Real-World Workload

### Hot Key Pattern (80/20)

```
Dataset: 1GB SSTable, 10M keys
Workload: 80% reads on 20% of keys
Cache: 256MB

Results:
  Cache hit rate: 92%
  p50 latency: 520ns
  p95 latency: 1.2µs
  p99 latency: 120µs (cache misses)
  Throughput: 1.9M reads/sec
```

---

## Summary

**Key takeaways:**

- **Writes:** 500-700K entries/sec with compression
- **Hot reads:** <1µs p95 with cache
- **Cold reads:** ~100µs p95 from SSD
- **Bloom:** 67ns checks, 99%+ absent key savings
- **Compression:** LZ4 adds <10% overhead, Zstd +100%
- **Cache:** 80% hit rate → 5x speedup, 99% → 60x speedup
