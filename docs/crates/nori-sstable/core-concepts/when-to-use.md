# When to Use SSTables

Practical guidance on when SSTables are the right choice for your storage needs.

---

## Decision Matrix

###  Great Fit

| Use Case | Why SSTables Excel |
|----------|-------------------|
| **Write-heavy workloads** | Sequential writes 10-100x faster than B-trees |
| **Time-series data** | Append-only, natural sort order, range scans |
| **Event logging** | Immutability = perfect audit trail |
| **LSM storage engines** | Core building block (LevelDB, RocksDB pattern) |
| **Hot key patterns** | Bloom filters + cache = <10µs reads |
| **Batch writes** | Amortize flush cost over many entries |
| **Large datasets** | Scales to TB without in-memory index |

###  Not the Right Tool

| Use Case | Why Not |
|----------|---------|
| **Ultra-low latency (<1µs)** | Disk I/O inherently slower |
| **Heavy random updates** | Creates many versions, space amplification |
| **Tiny datasets (<1MB)** | Fixed overhead dominates |
| **No compaction budget** | Read amplification grows unbounded |
| **In-memory only** | Better to use HashMap/BTreeMap |

---

## Workload Patterns

### Append-Mostly (Perfect Fit)

```rust
// Event sourcing, logs, time-series
for event in events {
    sst_builder.add(&Entry::put(
        format!("event:{}", event.timestamp).as_bytes(),
        event.data
    )).await?;
}
```

**Why it works:**
- Natural sort order (timestamps)
- Rare updates/deletes
- Sequential writes = maximum throughput
- Range scans over time windows

**Performance:** 100K+ writes/sec

---

### Read-Heavy with Hot Keys (Good Fit)

```rust
// User profiles, product catalog
cache_hit_rate: 80%+
    ↓
p95 latency: <1ms (cache)
p99 latency: ~5ms (disk)
```

**Why it works:**
- Bloom filters skip most disk reads
- LRU cache caches hot blocks
- 80/20 rule: 20% of keys = 80% of reads

**Performance:** 100K+ reads/sec (cached), 10K/sec (disk)

---

### Update-Heavy on Same Keys (Poor Fit)

```rust
// Counter updates, session state
key "counter" updated 1000 times/sec
    ↓
1000 versions on disk (until compaction)
    ↓
Space amplification: 1000x
```

**Problems:**
- Many versions accumulate
- Compaction can't keep up
- Space amplification grows

**Better alternative:** In-memory store with WAL (like Redis)

---

### Random Small Reads (Mixed)

```rust
// Random key lookups
no locality, cache hit rate: 10%
    ↓
p95: 5-10ms (most reads hit disk)
```

**Challenges:**
- Poor cache hit rate
- Bloom filter helps, but still slow
- Multiple levels to check

**Optimization:** Increase cache size, use read-ahead

---

## Use Case Deep Dives

### Use Case 1: Time-Series Database

**Perfect fit for SSTables**

```rust
// Metrics ingestion
struct Metric {
    timestamp: u64,    // Natural sort key
    name: String,
    value: f64,
}

// Write path (batched)
let mut builder = SSTableBuilder::new(config).await?;
for metric in batch.iter().sorted_by_key(|m| m.timestamp) {
    let key = format!("{}:{}", metric.name, metric.timestamp);
    builder.add(&Entry::put(key, metric.value.to_bytes())).await?;
}
```

**Why it works:**
- Timestamp-based keys = sorted by default
- Writes are append-only (past doesn't change)
- Queries are time-range scans
- Compaction merges old data efficiently

**Real-world example:** InfluxDB, Prometheus

---

### Use Case 2: Event Sourcing

**Excellent fit**

```rust
// Append-only event log
struct Event {
    aggregate_id: Uuid,
    sequence: u64,
    event_type: String,
    data: Vec<u8>,
}

// Key: aggregate_id:sequence (sorted)
let key = format!("{}:{:020}", event.aggregate_id, event.sequence);
sstable.add(&Entry::put(key, event.data)).await?;
```

**Benefits:**
- Immutability matches event sourcing semantics
- Natural chronological order
- Replay = scan range
- Snapshots = SSTable references

**Real-world example:** Apache Kafka (log segments similar to SSTables)

---

### Use Case 3: LSM Storage Engine

**Core building block**

```rust
// User-facing key-value store
pub struct LSM {
    memtable: SkipList,
    l0: Vec<SSTable>,      // Recent, may overlap
    l1: Vec<SSTable>,      // 10x size, no overlap
    l2: Vec<SSTable>,      // 100x size
}

impl LSM {
    pub async fn put(&mut self, key: &[u8], value: &[u8]) {
        self.memtable.insert(key, value);
        if self.memtable.size() > THRESHOLD {
            self.flush().await?;  // → New SSTable
        }
    }
}
```

**Why SSTables:**
- Fast writes (memtable + flush)
- Compaction manages multiple levels
- Bloom filters optimize reads
- Battle-tested pattern (RocksDB, LevelDB)

**Real-world example:** NoriKV, CockroachDB, TiKV

---

### Use Case 4: Data Warehouse (Columnar)

**Modified SSTable format**

```
Traditional SSTable:
  Block: [user:1, alice, 25, NY], [user:2, bob, 30, CA]

Columnar SSTable:
  name column: [alice, bob, charlie, ...]
  age column:  [25, 30, 35, ...]
  state column: [NY, CA, TX, ...]
```

**Benefits for analytics:**
- Read only needed columns
- Better compression (similar values)
- SIMD-friendly layout

**Real-world example:** Parquet files (columnar SSTables)

---

## Performance Expectations

### Write Performance

| Workload | Throughput | Latency (p99) |
|----------|------------|---------------|
| **Batch writes** | 100K-500K/sec | 1-5ms (amortized) |
| **Single writes** | 10K-50K/sec | 20-50ms (fsync) |
| **Bulk import** | 1M+ entries/sec | Batch limited |

**Key insight:** Batch writes for best performance

---

### Read Performance

| Scenario | Latency (p95) | Throughput |
|----------|---------------|------------|
| **Cache hit** | <1ms (often <100µs) | 100K+ reads/sec |
| **Bloom filter skip** | ~67ns | Millions/sec |
| **L0 hit (SSD)** | 1-5ms | 10K-50K/sec |
| **L2 hit (SSD)** | 5-10ms | 5K-10K/sec |

**Key insight:** Cache hit rate dominates performance

---

## Configuration Guidelines

### Small Dataset (<100MB)

```rust
SSTableConfig {
    block_size: 4096,
    block_cache_mb: 16,           // Small cache
    compression: Compression::None, // Skip compression
    bloom_bits_per_key: 10,
    ..Default::default()
}
```

**Rationale:** Overhead not worth it for small data

---

### Medium Dataset (100MB-10GB)

```rust
SSTableConfig {
    block_size: 4096,
    block_cache_mb: 256,          // Larger cache
    compression: Compression::Lz4, // Fast compression
    bloom_bits_per_key: 10,
    ..Default::default()
}
```

**Rationale:** Balance compression savings vs CPU

---

### Large Dataset (>10GB)

```rust
SSTableConfig {
    block_size: 16384,            // Larger blocks
    block_cache_mb: 1024,         // GB cache
    compression: Compression::Zstd, // Higher ratio
    bloom_bits_per_key: 12,       // Lower FP rate
    ..Default::default()
}
```

**Rationale:** Maximize compression, larger cache amortizes overhead

---

## Anti-Patterns

###  Using SSTables for Mutable Counters

```rust
// BAD: Frequent updates to same key
for _ in 0..1_000_000 {
    sstable.put(b"counter", current_value.to_bytes()).await?;
}
// Result: 1M versions on disk!
```

**Better:** Use in-memory counter, periodic snapshots to SSTable

---

###  No Compaction Strategy

```rust
// BAD: Write SSTables, never compact
for batch in batches {
    write_sstable(batch).await?;
}
// Result: 1000 files, p99 reads check all of them
```

**Better:** Schedule regular compaction

---

###  Tiny SSTable Files

```rust
// BAD: Flush every 100 entries
if memtable.len() > 100 {
    flush_to_sstable().await?;
}
// Result: Thousands of tiny files, overhead dominates
```

**Better:** Target 1MB-256MB per SSTable

---

## Migration Checklist

Considering SSTables for your project? Check these:

- [ ] Write throughput > 10K/sec required
- [ ] Append-mostly or time-series data
- [ ] OK with eventual consistency for deletes (tombstones)
- [ ] Have background I/O budget for compaction
- [ ] Dataset > 1MB (otherwise overhead not worth it)
- [ ] Can tolerate p99 latency of 5-20ms
- [ ] Cache hit rate expected > 50%

**If all checked:** SSTables likely a good fit!

---

## Next Steps

**Understand the design:**
Read [Design Decisions](../design-decisions/index.md) to see why nori-sstable makes specific choices.

**Learn the internals:**
Check [How It Works](../how-it-works/index.md) for file format and implementation details.

**Optimize performance:**
See [Performance](../performance/index.md) for tuning guides and benchmarks.

**Get started:**
Jump to [Getting Started](../getting-started.md) to build your first SSTable.
