# Performance Tuning Guide

How to optimize nori-wal for your specific workload.

## Table of contents

---

## Quick Wins

Before diving into complex tuning, try these common optimizations:

### 1. Use Batched Fsync

**Problem:** `FsyncPolicy::Always` gives you 400 writes/sec maximum.

**Solution:**

```rust
let config = WalConfig {
    fsync_policy: FsyncPolicy::Batch(Duration::from_millis(5)),
    ..Default::default()
};
```

**Impact:** 200x throughput improvement (400 → 86,000 writes/sec)

**Trade-off:** Up to 5ms of data loss on crash (uncommitted writes in buffer).

### 2. Batch Your Writes

**Problem:** Individual `append()` calls are fast, but each one waits for the next fsync window.

**Solution:**

```rust
// Bad: One at a time
for record in records {
    wal.append(&record).await?;
    wal.sync().await?;  // 2-3ms each
}

// Good: Batch them
for record in records {
    wal.append(&record).await?;
}
wal.sync().await?;  // Single 2-3ms for all
```

**Impact:** 100-1000x throughput improvement depending on batch size.

### 3. Increase Segment Size

**Problem:** Frequent segment rotation (every 30 seconds) adds latency spikes.

**Solution:**

```rust
let config = WalConfig {
    max_segment_size: 512 * 1024 * 1024,  // 512 MB (default is 128 MB)
    ..Default::default()
};
```

**Impact:** Reduces rotation frequency 4x (every 2 minutes instead of 30 seconds).

**Trade-off:** Larger segments mean longer recovery time and more disk space.

### 4. Use Compression for Large Records

**Problem:** Writing large text/JSON records saturates disk I/O.

**Solution:**

```rust
let record = Record::put(key, value)
    .with_compression(Compression::Lz4);

wal.append(&record).await?;
```

**Impact:** 10x storage savings, 1.2x slower writes (still net win for I/O bound workloads).

**Trade-off:** CPU overhead (2µs per write for LZ4).

## Workload-Specific Tuning

### High-Throughput Writes

**Goal:** Maximize writes/sec.

**Configuration:**

```rust
let config = WalConfig {
    max_segment_size: 512 * 1024 * 1024,       // Large segments
    fsync_policy: FsyncPolicy::Batch(
        Duration::from_millis(10)              // Longer batch window
    ),
    preallocate: true,                         // Avoid allocation pauses
    node_id: 0,
};
```

**Code Pattern:**

```rust
// Batch writes before syncing
let mut batch = Vec::new();
for i in 0..1000 {
    batch.push(Record::put(&format!("key{}", i), b"value"));
}

for record in batch {
    wal.append(&record).await?;
}
wal.sync().await?;  // Single fsync for 1000 records
```

**Expected Performance:**
- 100K+ writes/sec
- p99 latency: 10ms

### Low-Latency Reads

**Goal:** Minimize read latency.

**Strategy:**

WAL is optimized for writes, not reads. For fast reads:

1. Build an in-memory index (see [Key-Value Store recipe](../recipes/key-value-store.md))
2. Use snapshots to avoid scanning entire WAL
3. Keep frequently accessed data in memory

**Example:**

```rust
pub struct FastKvStore {
    data: HashMap<Bytes, Bytes>,  // In-memory for O(1) reads
    wal: Wal,                      // For durability
}

impl FastKvStore {
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        // O(1) read from memory
        self.data.get(key).map(|v| v.as_ref())
    }

    pub async fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        // Write to WAL first
        self.wal.append(&Record::put(key, value)).await?;

        // Then update in-memory
        self.data.insert(key.into(), value.into());

        Ok(())
    }
}
```

**Expected Performance:**
- Read latency: <1µs (memory lookup)
- Write latency: 10ms (WAL write + fsync)

### Durability-Critical Workloads

**Goal:** Never lose data, even on crash.

**Configuration:**

```rust
let config = WalConfig {
    fsync_policy: FsyncPolicy::Always,         // Fsync every write
    max_segment_size: 128 * 1024 * 1024,       // Smaller segments
    preallocate: true,                         // Detect disk full early
    node_id: 0,
};
```

**Code Pattern:**

```rust
// Sync after every write
wal.append(&record).await?;
wal.sync().await?;  // Ensure on disk before proceeding
```

**Expected Performance:**
- 400 writes/sec (disk bound)
- p99 latency: 3ms

**Optimization:** Use faster storage (NVMe SSD, Optane).

### Large Record Workloads

**Goal:** Store large values (>100 KB) efficiently.

**Configuration:**

```rust
let config = WalConfig {
    max_segment_size: 1024 * 1024 * 1024,      // 1 GB segments
    fsync_policy: FsyncPolicy::Batch(
        Duration::from_millis(5)
    ),
    preallocate: false,                        // Don't preallocate (wastes space)
    node_id: 0,
};
```

**Code Pattern:**

```rust
// Compress large records
let large_value = vec![0u8; 1_000_000];  // 1 MB

let record = Record::put(b"key", &large_value)
    .with_compression(Compression::Zstd);

wal.append(&record).await?;
```

**Expected Performance:**
- 50 MB/sec throughput
- 16x compression ratio for text

## Configuration Parameters

### `max_segment_size`

**What it does:** Maximum size of a single segment file before rotation.

**Default:** 128 MB

**Tuning:**

| Workload | Recommended Size | Reason |
|----------|-----------------|--------|
| High write rate | 512 MB - 1 GB | Reduce rotation overhead |
| Low write rate | 64 MB - 128 MB | Faster recovery, less space waste |
| Large records | 1 GB+ | Avoid frequent rotation |

**Trade-offs:**

- **Larger:** Less rotation, but longer recovery time
- **Smaller:** Faster recovery, but more rotation overhead

### `fsync_policy`

**What it does:** Controls when data is fsynced to disk.

**Options:**

| Policy | Use Case | Throughput | Durability |
|--------|----------|-----------|------------|
| `Always` | Financial transactions, critical data | 400 writes/sec | Maximum |
| `Batch(1ms)` | Conservative durability | 55K writes/sec | 1ms loss window |
| `Batch(5ms)` | Balanced (recommended) | 86K writes/sec | 5ms loss window |
| `Batch(10ms)` | High throughput | 100K writes/sec | 10ms loss window |
| `Os` | Maximum speed, no guarantees | 110K writes/sec | None |

**Recommendation:** Start with `Batch(5ms)` and adjust based on needs.

### `preallocate`

**What it does:** Pre-allocates segment file space on creation.

**Default:** `true`

**Trade-offs:**

| Setting | Pros | Cons |
|---------|------|------|
| `true` | Early disk-full detection, less fragmentation | Slower segment creation (12ms) |
| `false` | Fast segment creation (2ms) | Disk full errors during writes |

**Recommendation:** Keep `true` unless you have very frequent rotation (<1 second segments).

## Advanced Techniques

### Custom Batching Strategy

Implement application-level batching:

```rust
pub struct BatchingWal {
    wal: Wal,
    buffer: Vec<Record>,
    batch_size: usize,
}

impl BatchingWal {
    pub async fn append(&mut self, record: Record) -> Result<()> {
        self.buffer.push(record);

        if self.buffer.len() >= self.batch_size {
            self.flush().await?;
        }

        Ok(())
    }

    pub async fn flush(&mut self) -> Result<()> {
        for record in self.buffer.drain(..) {
            self.wal.append(&record).await?;
        }
        self.wal.sync().await?;

        Ok(())
    }
}
```

**Benefit:** Amortize fsync cost across many writes.

### Parallel Segment Readers

For replay/recovery, read segments in parallel:

```rust
pub async fn parallel_replay(wal: &Wal) -> Result<Vec<Record>> {
    let segments = wal.list_segments().await?;

    let handles: Vec<_> = segments.into_iter()
        .map(|segment_id| {
            let wal = wal.clone();
            tokio::spawn(async move {
                let mut records = Vec::new();
                let mut reader = wal.read_segment(segment_id).await?;

                while let Some((record, _)) = reader.next_record().await? {
                    records.push(record);
                }

                Ok::<_, anyhow::Error>(records)
            })
        })
        .collect();

    let mut all_records = Vec::new();
    for handle in handles {
        all_records.extend(handle.await??);
    }

    Ok(all_records)
}
```

**Benefit:** 4-8x faster recovery on multi-core systems.

### Compression Selection

Choose compression based on data:

```rust
fn choose_compression(value: &[u8]) -> Compression {
    if value.len() < 1024 {
        Compression::None  // Small values: overhead not worth it
    } else if is_text_or_json(value) {
        Compression::Lz4  // Fast compression for text
    } else {
        Compression::None  // Binary data often incompressible
    }
}
```

**Benefit:** Only compress when it helps.

## Monitoring and Tuning Workflow

### 1. Establish Baseline

Measure current performance:

```rust
let start = Instant::now();

for i in 0..10_000 {
    wal.append(&Record::put(&format!("key{}", i), b"value")).await?;
}
wal.sync().await?;

let elapsed = start.elapsed();
println!("Throughput: {} writes/sec", 10_000.0 / elapsed.as_secs_f64());
```

### 2. Identify Bottleneck

Check metrics:

```rust
// Write latency
let start = Instant::now();
wal.append(&record).await?;
let write_latency = start.elapsed();

// Fsync latency
let start = Instant::now();
wal.sync().await?;
let fsync_latency = start.elapsed();

println!("Write: {:?}, Fsync: {:?}", write_latency, fsync_latency);
```

**Common bottlenecks:**
- High fsync latency → Use batching or faster disk
- High write latency → Reduce record size or use compression
- Frequent rotation → Increase segment size

### 3. Apply Tuning

Make one change at a time:

1. Change `fsync_policy` to `Batch(5ms)`
2. Measure improvement
3. If not enough, try batching writes
4. If still not enough, increase segment size
5. If still not enough, upgrade hardware

### 4. Verify

Run for extended period (24+ hours):

```rust
// Track p99 latency
let mut latencies = Vec::new();

for _ in 0..100_000 {
    let start = Instant::now();
    wal.append(&record).await?;
    latencies.push(start.elapsed());
}

latencies.sort();
let p99 = latencies[(latencies.len() * 99) / 100];
println!("p99 latency: {:?}", p99);
```

## Common Pitfalls

### 1. Not Batching Syncs

**Problem:**

```rust
// Bad: Sync after every write
for record in records {
    wal.append(&record).await?;
    wal.sync().await?;  // 2-3ms penalty each time
}
```

**Solution:** Batch syncs (see [Batched Writes benchmark](benchmarks.md#batched-writes)).

### 2. Segments Too Small

**Problem:** `max_segment_size: 1024 * 1024` (1 MB) causes rotation every second.

**Impact:** 15ms pause every second (1.5% overhead).

**Solution:** Use at least 64 MB segments.

### 3. Ignoring Disk Speed

**Problem:** HDD gives 100 IOPS, NVMe gives 500K IOPS.

**Impact:** 5000x performance difference.

**Solution:** Profile disk speed:

```bash
# Linux
sudo fio --name=random-write --ioengine=libaio --rw=randwrite --bs=4k --size=1G --numjobs=1 --runtime=60 --direct=1 --filename=/path/to/wal/testfile

# macOS
dd if=/dev/zero of=/path/to/wal/testfile bs=4k count=250000
```

If disk is slow, WAL performance will be limited.

### 4. Not Pre-allocating

**Problem:** `preallocate: false` + full disk = corruption.

**Solution:** Keep `preallocate: true` unless you have good monitoring.

## Performance Checklist

Before deploying to production:

- [ ] Measured baseline performance on target hardware
- [ ] Chose appropriate `fsync_policy` for durability requirements
- [ ] Implemented write batching (if needed)
- [ ] Set `max_segment_size` based on write rate
- [ ] Enabled compression for large text/JSON records
- [ ] Verified disk is fast enough (SSD minimum, NVMe recommended)
- [ ] Tested recovery time with realistic data size
- [ ] Monitored p99 latency under load
- [ ] Set up alerts for slow writes/rotation

## Conclusion

Performance tuning is iterative:

1. **Measure** current performance
2. **Identify** bottleneck
3. **Apply** one change
4. **Verify** improvement
5. **Repeat**

Most workloads get good performance with:

```rust
let config = WalConfig {
    max_segment_size: 256 * 1024 * 1024,
    fsync_policy: FsyncPolicy::Batch(Duration::from_millis(5)),
    preallocate: true,
    node_id: 0,
};
```

Adjust from there based on your specific needs.
