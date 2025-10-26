---
layout: default
title: Block Caching
parent: nori-sstable
nav_order: 6
---

# LRU Block Cache
{: .no_toc }

Deep dive into the LRU block cache and achieving 18x performance improvements.
{: .fs-6 .fw-300 }

---

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Overview

nori-sstable includes a built-in **LRU (Least Recently Used) block cache** that dramatically improves read performance for hot key workloads. The cache stores **decompressed blocks** in memory, eliminating repeated disk I/O and decompression overhead.

{: .highlight }
> **NEW Feature:** The LRU block cache was added in version 0.1 and delivers **18x performance improvements** for hot key workloads (14ms → 777µs for 1000 operations).

### Why Cache?

**The Problem:**
- Every SSTable read requires disk I/O (~100-1000µs latency)
- With compression, every read also requires decompression (~10µs)
- Hot keys (frequently accessed) pay this cost repeatedly
- 80/20 workloads (80% of requests hit 20% of keys) suffer most

**The Solution:**
- Cache decompressed blocks in memory (one-time cost)
- Serve subsequent reads from RAM (~100ns access time)
- **Result:** 10-100x faster reads for hot data

---

## Performance Impact

### Benchmark Results

From `benches/read_sstable.rs` hot key pattern:

| Configuration | Time (1000 ops) | Improvement |
|---------------|-----------------|-------------|
| **No cache** | 14.0 ms | Baseline |
| **64MB cache** | **777 µs** | **18x faster!** |
| **256MB cache** | 650 µs | 21.5x faster |

**Point lookup improvements:**

| Entry Count | No Cache | With Cache | Improvement |
|-------------|----------|------------|-------------|
| 100 entries | 13.3 µs | 486 ns | **27x faster** |
| 1K entries | 14.2 µs | 609 ns | **23x faster** |
| 10K entries | 15.1 µs | 877 ns | **17x faster** |

{: .note }
> **Why the improvement?** Cache hits avoid **two expensive operations**: (1) Disk I/O (~100-1000µs), and (2) Decompression (~10µs). RAM access is ~100ns, giving 10-100x speedups.

---

## How It Works

### Cache Architecture

```
User Request (key)
    ↓
Bloom filter check (key might exist?)
    ↓
Index lookup (which block contains key?)
    ↓
read_block(offset, size)
    ↓
┌─────────────────────────────────┐
│  Check LRU Cache                │
│  Key: block offset (u64)        │
│  Value: decompressed Block      │
└─────────────────────────────────┘
    ↓                      ↓
Cache HIT           Cache MISS
    ↓                      ↓
Return cached       Read from disk
block (~100ns)          ↓
                   Decompress block
                        ↓
                   Add to cache (evict LRU if full)
                        ↓
                   Return block
```

**Key insights:**
1. **Cache key = block offset:** Each 4KB block is cached by its file offset
2. **Cache value = decompressed block:** Avoid repeated decompression
3. **Thread-safe:** Uses `Mutex<LruCache>` for concurrent access
4. **LRU eviction:** Automatically evicts least recently used blocks when full

### Data Flow

```rust
// In SSTableReader::read_block()
pub(crate) async fn read_block(&self, offset: u64, size: u32) -> Result<Block> {
    // 1. Check cache first
    if let Some(ref cache) = self.block_cache {
        let mut cache_lock = cache.lock().await;
        if let Some(block) = cache_lock.get(&offset) {
            // Cache hit! Return cloned block
            self.meter.counter("sstable_block_cache_hits", &[]).inc(1);
            return Ok(block.clone());
        }
        // Cache miss
        self.meter.counter("sstable_block_cache_misses", &[]).inc(1);
    }

    // 2. Read compressed block from disk
    let mut file = self.file.lock().await;
    file.seek(SeekFrom::Start(offset)).await?;
    let mut compressed_bytes = vec![0u8; size as usize];
    file.read_exact(&mut compressed_bytes).await?;

    // 3. Decompress block
    let decompressed_bytes = compress::decompress(&compressed_bytes, self.footer.compression)?;

    // 4. Decode block structure
    let block = Block::decode(Bytes::from(decompressed_bytes))?;

    // 5. Cache the decompressed block
    if let Some(ref cache) = self.block_cache {
        let mut cache_lock = cache.lock().await;
        cache_lock.put(offset, block.clone());  // LRU eviction happens here
    }

    Ok(block)
}
```

---

## Configuration

### Default Configuration (64MB)

```rust
use nori_sstable::{SSTableConfig, SSTableReader};

// Option 1: Use default (64MB cache)
let reader = SSTableReader::open("data.sst".into()).await?;

// Option 2: Explicit default in config
let config = SSTableConfig {
    path: "data.sst".into(),
    block_cache_mb: 64,  // Default
    ..Default::default()
};
```

**Memory usage:** 64MB ≈ 16,384 blocks (at 4KB per block)

---

### Custom Cache Size

```rust
use nori_sstable::SSTableReader;
use nori_observe::NoopMeter;
use std::sync::Arc;

// Hot workload: Increase cache to 256MB
let reader = SSTableReader::open_with_config(
    "hot_data.sst".into(),
    Arc::new(NoopMeter),
    256  // 256MB cache
).await?;
```

**Memory usage:** 256MB ≈ 65,536 blocks

---

### Disable Cache (Cold Storage)

```rust
let config = SSTableConfig {
    path: "archive.sst".into(),
    block_cache_mb: 0,  // Disable caching
    ..Default::default()
};
```

**Use case:** Cold storage where reads are infrequent and memory is limited

---

## Cache Sizing Guide

### How Much Memory Does the Cache Use?

**Formula:**
```
Cache Memory = block_cache_mb * 1024 * 1024 bytes
Number of Blocks = Cache Memory / block_size (default 4KB)
```

**Examples:**

| Config | Memory | Blocks Cached | Use Case |
|--------|--------|---------------|----------|
| `block_cache_mb: 16` | 16 MB | ~4,096 | Development, testing |
| `block_cache_mb: 64` | 64 MB | ~16,384 | **Default** (good balance) |
| `block_cache_mb: 256` | 256 MB | ~65,536 | Hot workloads |
| `block_cache_mb: 1024` | 1 GB | ~262,144 | Very large hot datasets |
| `block_cache_mb: 0` | 0 | 0 | Cold storage |

---

### Sizing for Your Workload

#### 80/20 Hot Key Pattern (Common)

**Scenario:** 80% of requests hit 20% of your data

**Calculation:**
1. Total SSTable size: 1 GB
2. Hot data (20%): 200 MB
3. Number of blocks: 200 MB / 4 KB = 51,200 blocks
4. **Recommended cache:** 256 MB (to fit all hot blocks)

**Expected hit rate:** 80-90%

---

#### Uniform Access Pattern (Rare)

**Scenario:** All keys accessed equally (cache less effective)

**Calculation:**
1. Total SSTable size: 10 GB
2. Cache can only hold fraction of data
3. **Recommended:** 64-128 MB (default is fine)

**Expected hit rate:** 1-5% (proportional to cache/data ratio)

---

#### Very Hot Keys (e.g., Metadata, Counters)

**Scenario:** 95% of requests hit 5% of data

**Calculation:**
1. Total SSTable size: 500 MB
2. Hot data (5%): 25 MB
3. Number of blocks: 25 MB / 4 KB = 6,400 blocks
4. **Recommended cache:** 64 MB (default is sufficient!)

**Expected hit rate:** 95%+

---

## Monitoring Cache Performance

### Key Metrics

nori-sstable emits metrics via the `Meter` trait:

```rust
// Cache hits (good!)
sstable_block_cache_hits{} = 8500

// Cache misses (first-time reads or evictions)
sstable_block_cache_misses{} = 1500

// Calculate hit rate
hit_rate = hits / (hits + misses) = 8500 / 10000 = 85%
```

### Interpreting Metrics

| Hit Rate | Status | Action |
|----------|--------|--------|
| **>80%** | 🟢 Excellent | Cache is sized well |
| **50-80%** | 🟡 Good | Consider increasing cache |
| **20-50%** | 🟠 Fair | Increase cache or optimize access pattern |
| **<20%** | 🔴 Poor | Cache too small or uniform access pattern |

### Example: Monitoring in Production

```rust
use nori_sstable::{SSTableBuilder, SSTableConfig};
use nori_observe_prom::PrometheusMeter;  // Prometheus exporter

// Create custom meter
let meter = Box::new(PrometheusMeter::new());

let config = SSTableConfig {
    path: "data.sst".into(),
    block_cache_mb: 256,
    ..Default::default()
};

let mut builder = SSTableBuilder::new_with_meter(config, meter).await?;

// Metrics are now exported to Prometheus:
// sstable_block_cache_hits
// sstable_block_cache_misses
// sstable_block_reads (total disk reads)
```

Query in Prometheus:
```promql
# Cache hit rate
rate(sstable_block_cache_hits[5m]) /
(rate(sstable_block_cache_hits[5m]) + rate(sstable_block_cache_misses[5m]))
```

---

## Cache + Compression Synergy

### The Winning Combination

**Configuration:**
```rust
let config = SSTableConfig {
    compression: Compression::Lz4,  // 2-3x storage savings
    block_cache_mb: 64,              // Cache decompressed blocks
    ..Default::default()
};
```

**What happens:**

```
┌─────────────────────────────────────────────┐
│ Disk: Compressed blocks (2-3x smaller)     │
│ [...LZ4 compressed data...]                │
└─────────────────────────────────────────────┘
            ↓ Read compressed (fast due to smaller size)
            ↓ Decompress (one-time cost: ~10µs)
┌─────────────────────────────────────────────┐
│ Cache: Decompressed blocks (fast access)   │
│ [Block1][Block2][Block3]...[BlockN]       │
│ (RAM: ~100ns access)                       │
└─────────────────────────────────────────────┘
            ↓ Serve from cache (no decompression!)
         User Request (~5µs total)
```

**Benefits:**
1. **Storage:** Save 50-70% disk space (LZ4 compression)
2. **First read:** Pay decompression cost once (~10µs)
3. **Cached reads:** Zero decompression cost (cached = decompressed)
4. **Network:** Faster backups/replication (smaller files)

**Result:** Best of both worlds! 🎉

---

## Implementation Details

### LRU Algorithm

nori-sstable uses the [`lru` crate](https://crates.io/crates/lru) for O(1) cache operations:

```rust
use lru::LruCache;
use std::num::NonZeroUsize;

// In SSTableReader
pub struct SSTableReader {
    // ...
    block_cache: Option<Mutex<LruCache<u64, Block>>>,
    // ...
}

// Cache initialization
let blocks_in_cache = (block_cache_mb * 1024 * 1024) / 4096;
let cache = LruCache::new(
    NonZeroUsize::new(blocks_in_cache.max(1)).unwrap()
);
```

**Operations:**
- `get(&key)` - O(1) - Returns value and marks as recently used
- `put(key, value)` - O(1) - Inserts and evicts LRU if full
- Thread-safe with `Mutex`

### Memory Management

**Block structure:**
```rust
pub struct Block {
    data: Bytes,              // Block data (4KB typical)
    restart_offsets: Vec<u32>, // Restart point offsets
    // ...
}
```

**Memory per cached block:** ~4KB + ~64 bytes overhead = ~4100 bytes

**Total cache memory:**
```
Memory = (blocks_in_cache * 4100 bytes) ≈ block_cache_mb * 1024 * 1024
```

### Thread Safety

The cache is protected by a Tokio async `Mutex`:

```rust
// Multiple readers can safely access cache concurrently
let reader = Arc::new(SSTableReader::open("data.sst").await?);

for _ in 0..4 {
    let reader_clone = reader.clone();
    tokio::spawn(async move {
        // Each task safely accesses shared cache
        reader_clone.get(b"key").await?;
    });
}
```

**Lock contention:**
- Cache locks are held briefly (<1µs typically)
- Read-heavy workloads scale well (cache hits don't modify LRU state)
- Multiple readers can queue on cache lock (fair scheduling)

---

## Best Practices

### ✅ Do

- **Enable caching by default** (64MB is reasonable for most workloads)
- **Monitor hit rates** to validate cache effectiveness
- **Increase cache for hot workloads** (80/20 patterns benefit most)
- **Combine with compression** (store compressed, cache decompressed)
- **Use `Arc<SSTableReader>`** to share cache across threads
- **Size based on hot data** (not total data size)

### ❌ Don't

- Don't disable cache unless memory is extremely limited
- Don't over-provision cache (diminishing returns beyond hot set size)
- Don't forget to monitor `sstable_block_cache_*` metrics
- Don't use tiny caches (<16MB) - overhead not worth it
- Don't expect miracles on uniform access patterns (cache helps less)

---

## Troubleshooting

### "Cache hit rate is low (<20%)"

**Possible causes:**
1. Cache too small for working set
2. Uniform access pattern (all keys accessed equally)
3. Working set larger than total data (shouldn't happen)

**Solutions:**
- Increase `block_cache_mb` to 256 or 512 MB
- Check access pattern (is data actually hot?)
- Monitor `sstable_block_reads` to see if disk I/O is high

---

### "High memory usage"

**Possible causes:**
1. Cache configured too large
2. Multiple SSTableReaders with separate caches

**Solutions:**
- Reduce `block_cache_mb` to 64 or 32 MB
- Share `SSTableReader` instances via `Arc` (shares cache)
- Use `block_cache_mb: 0` for cold data SSTables

---

### "Cache not improving performance"

**Possible causes:**
1. Data access is write-heavy (cache only helps reads)
2. First-time reads (cache miss is expected)
3. Access pattern is sequential scan (cache less helpful)

**Solutions:**
- Verify workload is read-heavy
- Allow warm-up period (first reads populate cache)
- For sequential scans, cache is less beneficial (still helps for iterators)

---

## Advanced Topics

### Cache Warming

For predictable workloads, pre-warm the cache:

```rust
// Pre-load hot keys into cache
let hot_keys = vec![b"key1", b"key2", b"key3"];

for key in hot_keys {
    let _ = reader.get(key).await?;  // Populate cache
}

// Now subsequent reads are fast
let value = reader.get(b"key1").await?;  // Cache hit!
```

### Multiple Readers, Shared Cache

```rust
// Share reader (and cache) across threads
let reader = Arc::new(SSTableReader::open("data.sst").await?);

let mut handles = vec![];
for i in 0..10 {
    let reader_clone = reader.clone();  // Shares cache!
    let handle = tokio::spawn(async move {
        reader_clone.get(format!("key{}", i).as_bytes()).await
    });
    handles.push(handle);
}

// All tasks share the same cache
for handle in handles {
    handle.await??;
}
```

### Separate Caches for Different SSTables

```rust
// Each reader gets its own cache
let reader1 = Arc::new(SSTableReader::open("hot.sst").await?);
let reader2 = Arc::new(SSTableReader::open("cold.sst").await?);

// hot.sst has 256MB cache
// cold.sst has 16MB cache (or 0 to disable)
```

**Use case:** Allocate more cache to hot SSTables, less to cold ones.

---

## Performance Tuning Workflow

1. **Start with defaults:** `block_cache_mb: 64`
2. **Deploy and monitor:**
   ```promql
   sstable_block_cache_hits / (sstable_block_cache_hits + sstable_block_cache_misses)
   ```
3. **Analyze hit rate:**
   - **>80%:** Cache is sized well ✅
   - **50-80%:** Consider increasing cache
   - **<50%:** Increase cache or investigate access pattern
4. **Tune cache size:**
   - Increase by 2x increments (64 → 128 → 256)
   - Monitor hit rate improvement
   - Stop when hit rate plateaus (diminishing returns)
5. **Balance memory:** Ensure total cache usage across all readers fits in RAM

---

## Related Documentation

- **[Compression Guide](compression)** - Learn how compression works with caching
- **[Performance Benchmarks](performance/benchmarks)** - See cache benchmark results
- **[API Reference](api-reference/reader)** - SSTableReader cache methods
- **[Tuning Guide](performance/tuning)** - General performance tuning

---

## Summary

**LRU Block Cache in nori-sstable:**
- ✅ **18x faster reads** for hot key workloads
- ✅ Caches **decompressed blocks** (avoid repeated decompression)
- ✅ **LRU eviction** - automatic management
- ✅ **Thread-safe** - share via `Arc`
- ✅ **Configurable** - default 64MB, tune to workload
- ✅ **Works with compression** - best of both worlds

**Recommended configuration:**
```rust
SSTableConfig {
    compression: Compression::Lz4,  // Storage savings
    block_cache_mb: 64,              // Good default (tune as needed)
    ..Default::default()
}
```

**Monitor this metric:**
```
cache_hit_rate = sstable_block_cache_hits / (hits + misses)
```

🚀 **Result:** Dramatically faster reads for hot data + storage savings!
