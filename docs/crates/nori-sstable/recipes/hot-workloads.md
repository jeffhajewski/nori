# Hot Workloads

Optimizing for high-throughput, cache-friendly read workloads.

---

## Scenario

**Workload characteristics:**
- 1M+ reads/sec
- 80/20 distribution (20% of keys = 80% of reads)
- Low latency required (p95 < 1ms, p99 < 5ms)
- Dataset: 10GB across multiple SSTables

---

## Optimal Configuration

```rust
use nori_sstable::{SSTableConfig, Compression};

let config = SSTableConfig {
    path: PathBuf::from("hot_data.sst"),
    estimated_entries: 1_000_000,

    // Fast decompression
    compression: Compression::Lz4,

    // Large cache for hot keys
    block_cache_mb: 512,

    // Lower false positive rate
    bloom_bits_per_key: 12,

    ..Default::default()
};
```

---

## Cache Sizing

**Calculate working set:**
```
Hot keys: 20% of 10M = 2M keys
Entries per block: ~50
Hot blocks: 2M / 50 = 40K blocks
Working set: 40K * 4KB = 160MB
```

**Recommended cache:**
```rust
// 1.5-2x working set
block_cache_mb: 256  // Good
block_cache_mb: 512  // Better (accounts for variance)
```

---

## Reader Setup

```rust
use nori_observe::NoopMeter;
use std::sync::Arc;

// Share reader across threads
let reader = Arc::new(
    SSTableReader::open_with_config(
        PathBuf::from("hot_data.sst"),
        Arc::new(NoopMeter),
        512  // 512MB cache
    ).await?
);

// Spawn worker threads
for i in 0..num_cpus::get() {
    let r = reader.clone();
    tokio::spawn(async move {
        handle_requests(r).await
    });
}
```

---

## Concurrent Access Pattern

```rust
async fn handle_requests(reader: Arc<SSTableReader>) -> Result<()> {
    loop {
        let key = receive_request().await?;

        // Fast path: bloom + cache
        if let Some(entry) = reader.get(&key).await? {
            send_response(entry.value).await?;
        } else {
            send_not_found().await?;
        }
    }
}
```

**Performance:**
```
Cache hits (80%): <1µs p95
Cache misses (20%): ~100µs p95
Overall p95: ~20µs
```

---

## Pre-warming Cache

```rust
// Load hot keys into cache at startup
async fn prewarm_cache(
    reader: Arc<SSTableReader>,
    hot_keys: Vec<Bytes>
) -> Result<()> {
    println!("Pre-warming cache with {} keys...", hot_keys.len());

    for key in hot_keys {
        reader.get(&key).await?;
    }

    println!("Cache warmed!");
    Ok(())
}
```

---

## Monitoring

```rust
struct CacheStats {
    hits: AtomicU64,
    misses: AtomicU64,
}

let stats = Arc::new(CacheStats::default());

// Report periodically
tokio::spawn({
    let s = stats.clone();
    async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let hits = s.hits.load(Ordering::Relaxed);
            let misses = s.misses.load(Ordering::Relaxed);
            let rate = hits as f64 / (hits + misses) as f64;
            println!("Cache hit rate: {:.1}%", rate * 100.0);
        }
    }
});
```

---

## Expected Performance

```
Configuration:
  - 512MB cache
  - LZ4 compression
  - 12 bits/key bloom

Results:
  - Throughput: 1.5M reads/sec
  - p50 latency: 450ns
  - p95 latency: 950ns
  - p99 latency: 85µs
  - Cache hit rate: 92%
```
