# Write Path

How writes flow through nori-lsm from `put()` call to persistent storage, including WAL durability, memtable buffering, L0 flushes, and compaction.

---

## Overview

**Write path stages**:
1. **WAL append** → Durability (crash recovery)
2. **Memtable write** → In-memory buffer (fast lookups)
3. **Memtable flush** → Convert to L0 SSTable (persistent)
4. **L0 compaction** → Merge into slots (reduce read amplification)
5. **Slot compaction** → Maintain K-way fanout (ATLL optimization)

**Latency breakdown** (typical values):
```
put() call:
  ├─ WAL append:        1-2ms   (fsync to disk)
  ├─ Memtable insert:   50ns    (skiplist insert)
  └─ Total:             1-2ms   (dominated by WAL)

Background (async):
  ├─ Memtable flush:    50-200ms  (write SSTable)
  ├─ L0 compaction:     200-500ms (merge to slots)
  └─ Slot compaction:   500-2000ms (slot-local tiering)
```

**Key invariant**: Every write is durable **before** returning to caller (via WAL).

---

## Stage 1: WAL Append

### Purpose

**Durability**: Ensure writes survive crashes/power loss

**Mechanism**:
```rust
async fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
    // 1. Append to WAL (synchronous fsync)
    self.wal.append(&key, &value).await?;

    // 2. Write to memtable (fast, in-memory)
    self.memtable.insert(key, value);

    Ok(())
}
```

**WAL Entry Format**:
```
┌────────────────────────────────────────┐
│ Header (12 bytes)                      │
│  ├─ Length: u32      (4 bytes)         │
│  ├─ CRC32:  u32      (4 bytes)         │
│  └─ Type:   u8       (1 byte: Put/Del) │
├────────────────────────────────────────┤
│ Key Length: u32      (4 bytes)         │
├────────────────────────────────────────┤
│ Key Data:   [u8; key_len]              │
├────────────────────────────────────────┤
│ Value Length: u32    (4 bytes)         │
├────────────────────────────────────────┤
│ Value Data: [u8; val_len]              │
└────────────────────────────────────────┘
```

**Durability Guarantee**:
```rust
// WAL append includes fsync before returning
pub async fn append(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
    let entry = encode_entry(key, value);
    self.file.write_all(&entry).await?;
    self.file.sync_all().await?;  // ← Blocks until kernel writes to disk
    Ok(())
}
```

**Crash Recovery**:
```rust
// On startup, replay WAL to reconstruct memtable
pub async fn recover(&mut self) -> Result<()> {
    for entry in self.wal.read_entries().await? {
        match entry.op_type {
            OpType::Put => self.memtable.insert(entry.key, entry.value),
            OpType::Delete => self.memtable.delete(entry.key),
        }
    }
    Ok(())
}
```

**Performance**:
- **Latency**: 1-2ms (dominated by `fsync` system call)
- **Throughput**: ~10,000 writes/sec (single WAL, sequential)
- **Bottleneck**: Disk write latency (SSD: ~100µs, HDD: ~10ms)

---

## Stage 2: Memtable Insert

### Purpose

**Fast reads**: Recent writes stay in memory (no disk I/O)

**Data Structure**: Skip list (lock-free, concurrent)
```
┌────────────────────────────────────────┐
│  Skip List (in-memory, sorted by key)  │
│                                        │
│  Level 3:  [a] ─────────────> [z]     │
│  Level 2:  [a] ──> [m] ──────> [z]    │
│  Level 1:  [a] ──> [g] ──> [m] ──> [z]│
│  Level 0:  [a][b][c][d][e][f][g]...    │
└────────────────────────────────────────┘
```

**Insert Operation**:
```rust
pub fn insert(&self, key: Vec<u8>, value: Vec<u8>) {
    // Skiplist insert is lock-free (uses atomic CAS)
    self.skiplist.insert(key, value);

    // Update memory usage (for flush triggering)
    self.memory_usage.fetch_add(
        key.len() + value.len(),
        Ordering::Relaxed
    );
}
```

**Lookup (for reads)**:
```rust
pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
    // O(log N) expected, lock-free
    self.skiplist.get(key)
}
```

**Performance**:
- **Latency**: 50-100ns (in-memory, cache-friendly)
- **Throughput**: Millions of ops/sec (concurrent reads/writes)
- **Memory**: ~24 bytes overhead per key (skip list pointers)

---

## Stage 3: Memtable Flush

### Flush Trigger

**Condition**: Memtable size exceeds threshold
```rust
if memtable.memory_usage() >= config.memtable_size_threshold {
    flush_memtable_to_l0().await?;
}
```

**Default Threshold**: 64 MB (configurable via `ATLLConfig.memtables_mib`)

**Backpressure** (if flush can't keep up):
```rust
// Memory pressure system applies adaptive delays
if memory_pressure == MemoryPressure::Moderate {
    // Yellow zone: Soft throttle (20ms base delay)
    tokio::time::sleep(Duration::from_millis(20)).await;
} else if memory_pressure == MemoryPressure::High {
    // Orange zone: Heavy throttle (200ms)
    tokio::time::sleep(Duration::from_millis(200)).await;
} else if memory_pressure == MemoryPressure::Critical {
    // Red zone: Hard stall (reject writes)
    return Err(Error::SystemPressure);
}
```

### Flush Algorithm

**Steps**:
1. **Freeze memtable**: Mark as read-only, create new active memtable
2. **Iterate keys**: Scan skiplist in sorted order
3. **Write SSTable**: Serialize to L0 file
4. **Update manifest**: Record new L0 file
5. **Clear WAL**: Truncate WAL (data now in SSTable)

**Pseudocode**:
```rust
async fn flush_memtable(&mut self) -> Result<()> {
    // 1. Freeze current memtable
    let frozen = self.memtable.freeze();
    self.memtable = Memtable::new();

    // 2. Create L0 SSTable file
    let sst_path = format!("L0-{}.sst", self.next_file_id);
    let mut builder = SSTableBuilder::new(sst_path);

    // 3. Iterate memtable in sorted order
    for (key, value) in frozen.iter() {
        builder.add(key, value)?;
    }

    // 4. Finalize SSTable (write bloom, index, footer)
    let sst_metadata = builder.finish().await?;

    // 5. Update manifest
    self.manifest.add_l0_file(sst_metadata).await?;

    // 6. Truncate WAL (data now durable in SSTable)
    self.wal.truncate().await?;

    Ok(())
}
```

**SSTable Structure** (refresher):
```
┌────────────────────────────────────────┐
│  Data Blocks (4KB each)                │
│   ├─ Block 0: [a..f]                   │
│   ├─ Block 1: [g..m]                   │
│   └─ Block 2: [n..z]                   │
├────────────────────────────────────────┤
│  Block Index                           │
│   ├─ Block 0: offset=0, first_key="a"  │
│   ├─ Block 1: offset=4096, first_key="g"│
│   └─ Block 2: offset=8192, first_key="n"│
├────────────────────────────────────────┤
│  Bloom Filter (10 bits/key)            │
├────────────────────────────────────────┤
│  Footer (metadata, offsets)            │
└────────────────────────────────────────┘
```

**Performance**:
- **Latency**: 50-200ms (64 MB memtable → ~64 MB SSTable)
- **Throughput**: ~320 MB/s (SSD write bandwidth)
- **I/O Pattern**: Sequential writes (efficient)

---

## Stage 4: L0 Compaction

### Problem: L0 Overlaps

**L0 files overlap** (contain arbitrary key ranges):
```
L0-001.sst: [a..z]  (entire key space)
L0-002.sst: [b..y]  (overlaps with L0-001)
L0-003.sst: [m..t]  (overlaps with both)
```

**Read Amplification**:
```
get("m"):
  Check L0-003 Yes (might contain "m")
  Check L0-002 Yes (might contain "m")
  Check L0-001 Yes (might contain "m")
  → Read 3 files for 1 key
```

**Solution**: Compact L0 into range-partitioned slots (L1+)

### Compaction Trigger

**Condition**: L0 file count exceeds threshold
```rust
if l0_file_count >= config.l0.max_files {
    // Critical: Hard stall (reject writes)
    return Err(Error::L0Overflow);
} else if l0_file_count >= config.l0.soft_throttle_threshold {
    // Moderate: Soft throttle (apply backpressure)
    schedule_l0_compaction();
}
```

**Default Thresholds**:
- **Soft throttle**: 6 files (50% of max)
- **Hard stall**: 12 files (100% of max)

### L0 → Slot Compaction

**Algorithm**: Merge L0 files into appropriate slots based on guard keys

**Pseudocode**:
```rust
async fn compact_l0_to_slots(&mut self) -> Result<()> {
    // 1. Select all L0 files
    let l0_files = self.manifest.l0_files();

    // 2. Create iterators for each L0 file
    let iterators = l0_files.iter()
        .map(|f| SSTableIterator::new(f))
        .collect();

    // 3. Merge-sort all L0 keys
    let mut merger = KWayMerge::new(iterators);

    // 4. Route keys to appropriate slots based on guard keys
    let mut slot_builders = HashMap::new();  // slot_id -> SSTableBuilder

    while let Some((key, value)) = merger.next().await? {
        // Find slot for this key
        let slot_id = self.find_slot_for_key(&key);

        // Add to slot's builder
        let builder = slot_builders.entry(slot_id)
            .or_insert_with(|| SSTableBuilder::new(slot_id));
        builder.add(key, value)?;
    }

    // 5. Finalize all slot SSTables
    for (slot_id, builder) in slot_builders {
        let sst_metadata = builder.finish().await?;
        self.manifest.add_slot_file(slot_id, sst_metadata).await?;
    }

    // 6. Delete L0 files (data now in slots)
    for file in l0_files {
        self.delete_sst(file).await?;
    }

    Ok(())
}
```

**Visual Example**:
```
Before:
  L0-001: [a, b, m, n, z]
  L0-002: [c, d, p, q]

  Slots (guard keys):
    Slot 0: [a..m)
    Slot 1: [m..z)

After:
  Slot 0: [a, b, c, d]  (new SSTable)
  Slot 1: [m, n, p, q, z] (new SSTable)

  L0: (empty)
```

**Performance**:
- **Latency**: 200-500ms (merge 6-12 files, ~384-768 MB)
- **Throughput**: ~1.5 GB/s (read + write)
- **I/O Pattern**: Sequential reads + writes (efficient)

**Read Amplification Improvement**:
```
Before L0 compaction (6 L0 files):
  get("m"): Check 6 L0 files

After L0 compaction:
  get("m"): Check Slot 1 only (1 file)
  → 6x reduction in reads
```

---

## Stage 5: Slot Compaction

### Purpose

**Maintain K-way fanout**: Ensure each slot has ≤ k_max sorted runs

**Trigger**: Slot exceeds k_max runs
```rust
if slot.runs.len() > slot.k_max {
    schedule_slot_compaction(slot.slot_id);
}
```

**Slot-Local Tiering**: Merge oldest runs within slot
```rust
async fn compact_slot(&mut self, slot_id: u32) -> Result<()> {
    let slot = &self.slots[slot_id];

    // 1. Select oldest K runs to merge (size-tiered)
    let runs_to_merge = slot.runs
        .iter()
        .rev()  // Oldest first
        .take(slot.k_max)
        .cloned()
        .collect();

    // 2. Merge runs into single new run
    let merged_run = merge_runs(runs_to_merge).await?;

    // 3. Update slot (replace old runs with merged run)
    slot.runs.retain(|r| !runs_to_merge.contains(r));
    slot.runs.push(merged_run);

    Ok(())
}
```

**Adaptive K-Max** (ATLL innovation):
```
Hot slot (heat_score = 0.9):
  k_max = 1  → Compact frequently (maintain leveled structure)

Cold slot (heat_score = 0.1):
  k_max = 4  → Compact rarely (allow tiered structure)
```

**Bandit Scheduler**: Chooses which slot to compact
```rust
fn select_slot_for_compaction(&self) -> u32 {
    if random() < epsilon {
        // Exploration (10%)
        random_slot()
    } else {
        // Exploitation (90%)
        argmax(slot.ucb_score)
    }
}
```

**Performance**:
- **Latency**: 500-2000ms (depends on slot size)
- **Frequency**: Hot slots → frequent, cold slots → rare
- **I/O Pattern**: Sequential (merge multiple SSTables)

---

## Write Amplification Analysis

### Formula

**Write Amplification (WA)** = Physical bytes written / Logical bytes written

### Per-Stage WA

**1. WAL Append**:
```
WA_wal = 1.0x (write once to WAL)
```

**2. Memtable Flush**:
```
WA_flush = 1.0x (write once to L0 SSTable)
```

**3. L0 → Slot Compaction**:
```
WA_l0 = 1.0x (write once to slot)
```

**4. Slot Compaction** (varies by heat):
```
Hot slot (k_max=1, compacted every 10 writes):
  WA_slot = 10x (rewrite 10 times)

Cold slot (k_max=4, compacted every 50 writes):
  WA_slot = 2x (rewrite 2 times)
```

### Total WA

**Formula**:
```
WA_total = WA_wal + WA_flush + WA_l0 + WA_slot
         = 1 + 1 + 1 + WA_slot
         = 3 + WA_slot
```

**Example** (80/20 hot/cold workload):
```
80% writes to hot slots (WA_slot = 10x):
  WA_hot = 3 + 10 = 13x

20% writes to cold slots (WA_slot = 2x):
  WA_cold = 3 + 2 = 5x

Weighted average:
  WA_total = 0.8 × 13 + 0.2 × 5
           = 10.4 + 1.0
           = 11.4x
```

**Comparison**:
- **Pure Leveled**: 40-100x (compact everything frequently)
- **ATLL**: 8-20x (compact hot data frequently, cold data rarely)
- **Pure Tiered**: 6-8x (but high read amplification)

---

## Backpressure and Flow Control

### Memory Pressure System

**Composite Pressure Score**:
```
composite_score = 0.4 × l0_pressure + 0.4 × memory_pressure + 0.2 × memtable_pressure

Levels:
  None:     < 0.5  (Green zone)
  Low:      0.5-0.75 (Green zone)
  Moderate: 0.75-0.9 (Yellow zone)
  High:     0.9-1.1 (Orange zone)
  Critical: ≥ 1.1   (Red zone)
```

### Adaptive Backpressure

**Green Zone** (None, Low):
```rust
// No throttling
put(key, value).await?;  // ~1-2ms latency
```

**Yellow Zone** (Moderate):
```rust
// Soft throttle
let delay = base_delay_ms × l0_excess;  // 20ms × 3 = 60ms
tokio::time::sleep(Duration::from_millis(delay)).await;
put(key, value).await?;  // ~60ms total latency
```

**Orange Zone** (High):
```rust
// Heavy throttle
let delay = base_delay_ms × l0_excess × 2;  // 20ms × 5 × 2 = 200ms
tokio::time::sleep(Duration::from_millis(delay)).await;
put(key, value).await?;  // ~200ms total latency
```

**Red Zone** (Critical):
```rust
// Hard stall (reject writes)
return Err(Error::SystemPressure);
// Caller must retry with exponential backoff
```

### Exponential Backoff Retry

**Client-side retry strategy**:
```rust
let mut retries = 0;
loop {
    match lsm.put(key, value).await {
        Ok(()) => break,
        Err(Error::SystemPressure) if retries < max_retries => {
            let delay = base_delay_ms × 2u64.pow(retries);
            tokio::time::sleep(Duration::from_millis(delay)).await;
            retries += 1;
        }
        Err(e) => return Err(e),
    }
}
```

**Example**:
```
Retry 0: Wait 5ms
Retry 1: Wait 10ms
Retry 2: Wait 20ms
Retry 3: Wait 40ms
Retry 4: Wait 80ms
Max retries: 5 (total ~155ms)
```

---

## Failure Recovery

### WAL-Based Crash Recovery

**On startup**:
```rust
pub async fn recover(wal_path: &Path) -> Result<Memtable> {
    let mut memtable = Memtable::new();
    let entries = WAL::read_all_entries(wal_path).await?;

    for entry in entries {
        match entry.op_type {
            OpType::Put => memtable.insert(entry.key, entry.value),
            OpType::Delete => memtable.delete(entry.key),
        }
    }

    Ok(memtable)
}
```

**WAL Truncation Strategy**:
```
Scenario 1: Crash during memtable flush
  ├─ Partial SSTable written (incomplete)
  └─ Recovery: Replay WAL to rebuild memtable, retry flush

Scenario 2: Crash after flush, before WAL truncate
  ├─ SSTable complete, WAL still has data
  └─ Recovery: Detect duplicate data, skip WAL entries already in SSTable

Scenario 3: Crash during WAL write
  ├─ Partial WAL entry (corrupt CRC)
  └─ Recovery: Truncate at last valid entry, discard partial write
```

**Idempotency**:
```rust
// Recovery is idempotent (safe to replay multiple times)
for entry in wal_entries {
    // Last-write-wins semantics
    memtable.insert(entry.key, entry.value);
}
```

---

## Performance Tuning

### Memtable Size

**Trade-off**:
- **Larger** (128 MB, 256 MB):
  -  Fewer L0 flushes (lower WA)
  -  Better sequential write batching
  -  Higher memory usage
  -  Slower crash recovery (larger WAL replay)

- **Smaller** (32 MB, 64 MB):
  -  Lower memory footprint
  -  Faster recovery
  -  More L0 flushes (higher WA)

**Recommendation**: 64 MB (default)

### L0 Thresholds

**Trade-off**:
- **Higher max_files** (16, 20):
  -  Less write throttling
  -  Higher sustained write throughput
  -  Higher read amplification
  -  Longer L0 compaction times

- **Lower max_files** (8, 10):
  -  Lower read amplification
  -  Faster L0 compactions
  -  More frequent write throttling

**Recommendation**: 12 files (default)

### WAL Fsync Strategy

**Options**:
```rust
// 1. Sync per write (default, most durable)
wal.append(key, value).await?;
wal.sync_all().await?;  // Every write

// 2. Sync every N writes (higher throughput)
wal.append(key, value).await?;
if write_count % N == 0 {
    wal.sync_all().await?;
}

// 3. Group commit (batched sync)
batch.push((key, value));
if batch.len() >= batch_size {
    wal.append_batch(&batch).await?;
    wal.sync_all().await?;
    batch.clear();
}
```

**Trade-off**:
- **Sync per write**: 1-2ms latency, 100% durable
- **Sync every 100 writes**: 50µs latency, risk losing last 99 writes
- **Group commit**: 200µs latency, no data loss (batch all synced together)

**Recommendation**: Sync per write (safety first)

---

## Summary

**Write Path Stages**:
1. **WAL append** (1-2ms) → Durability
2. **Memtable insert** (50ns) → Fast lookups
3. **Memtable flush** (50-200ms, async) → L0 SSTable
4. **L0 compaction** (200-500ms, async) → Slot SSTables
5. **Slot compaction** (500-2000ms, async) → Maintain K-way fanout

**Write Amplification**:
- **ATLL**: 8-20x (adaptive per slot)
- **Leveled**: 40-100x (global leveling)
- **Tiered**: 6-8x (but high RA)

**Backpressure**:
- **Green**: No throttling
- **Yellow**: Soft delays (60ms)
- **Orange**: Heavy delays (200ms)
- **Red**: Hard stall (reject writes)

**Durability**:
- WAL fsync before `put()` returns
- Crash recovery via WAL replay
- Idempotent recovery (safe to replay)

**Next**: Read [Read Path](read-path.md) to understand how queries work.

---

*Last Updated: 2025-10-31*
*See Also: [ATLL Architecture](atll-architecture.md), [Memory Pressure](../MEMORY_PRESSURE.md)*
