---
layout: default
title: Append-Only Architecture
parent: Design Decisions
nav_order: 1
---

# Append-Only Architecture
{: .no_toc }

Why the WAL never modifies existing data.
{: .fs-6 .fw-300 }

## Table of contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Decision

**nori-wal is strictly append-only.** Once bytes are written to a segment file, they are never modified in-place. All writes go to the end of the current segment.

## Rationale

### 1. Crash Safety

The primary reason for append-only design is crash safety:

```rust
// Append-only (safe)
1. Append record to end of file
2. If crash → Old data intact, new record may be partial
3. Recovery validates CRC, truncates partial record
```

Compare to in-place updates:

```rust
// In-place update (unsafe)
1. Seek to offset
2. Overwrite existing bytes
3. If crash mid-write → Data is corrupted, irrecoverable
```

With append-only, the worst case is losing the last incomplete record. With in-place updates, you can corrupt arbitrary data.

### 2. Simple Recovery

Recovery is straightforward:

```rust
async fn recover(segment: &Path) -> Result<ValidData> {
    let data = read_entire_file(segment).await?;
    let (valid_records, last_valid_offset) = scan_for_valid_records(&data);

    if last_valid_offset < data.len() {
        // Truncate corrupted tail
        truncate_file(segment, last_valid_offset).await?;
    }

    Ok(valid_records)
}
```

This works because:
- Valid records are always at the beginning
- Corruption is always at the end (partial write)
- No need to scan for holes or validate entire file

### 3. Sequential I/O Performance

Appending to a file is fast on all storage devices:

| Storage Type | Sequential Write | Random Write |
|--------------|------------------|--------------|
| NVMe SSD | 3-7 GB/s | 200-500 MB/s |
| SATA SSD | 500 MB/s | 100-200 MB/s |
| HDD | 100-200 MB/s | 1-5 MB/s |

Sequential writes are 10-100x faster than random writes on traditional hard drives.

### 4. Lock-Free Reads

Readers don't need to coordinate with writers:

```rust
// Writer appends to end
wal.append(&record).await?;  // Acquires write lock

// Reader scans from beginning (no lock needed for old data)
reader.read_from(Position { segment_id: 0, offset: 0 }).await?;
```

Since old data never changes, readers can operate without locks on historical segments. Only the active segment needs synchronization.

## What We Gave Up

### No In-Place Updates

Can't modify existing records:

```rust
// Can't do this:
wal.update_at_offset(1024, new_value).await?;  // Not supported!

// Must do this instead:
wal.append(&Record::put(key, new_value)).await?;  // New record
```

This means:
- Multiple versions of the same key exist in the WAL
- Need compaction/garbage collection eventually
- Higher storage usage for frequently updated keys

### No Random Access Optimization

Can't build an index inside the WAL:

```rust
// Can't embed an index in the WAL itself
// Must scan from beginning or maintain external index

let mut reader = wal.read_from(Position::start()).await?;
while let Some((record, _)) = reader.next_record().await? {
    if record.key == target_key {
        return Some(record);  // O(n) scan
    }
}
```

Solution: Build indexes on top (memtable, SSTable, B-tree).

## Alternatives Considered

### Alternative 1: Update-In-Place Log

**Approach:** Allow overwriting records at specific offsets.

**Rejected because:**
- Loses crash safety guarantees
- Recovery becomes complex (need checksums for every record, handle partial overwrites)
- No performance benefit on modern SSDs
- Requires complex locking to coordinate readers/writers

**Where it makes sense:**
- Memory-mapped files with OS-managed durability
- Systems that can afford full file rewrites
- Append-only with compaction (what we do at LSM level)

### Alternative 2: Circular Buffer

**Approach:** Overwrite oldest data when buffer is full.

```rust
struct CircularWAL {
    buffer: Vec<u8>,
    head: usize,  // Write position
    tail: usize,  // Oldest valid position
}
```

**Rejected because:**
- Loses historical data automatically
- Readers must track offsets carefully or lose data
- Recovery is complex (where does valid data start?)
- Can't support multiple readers at different positions

**Where it makes sense:**
- Fixed-size ring buffers for metrics/logs
- Systems where old data is truly disposable
- Memory-constrained embedded systems

### Alternative 3: Copy-on-Write

**Approach:** Write new version to new location, then atomically update pointer.

```rust
// Write new version
let new_offset = write_record(&record, segment);

// Atomic pointer update
index.update(key, new_offset);  // Old version still exists
```

**Why not:**
- Still need garbage collection
- More complex than pure append
- Requires external index (same as our approach)
- No clear advantage over append + compaction

**Where it makes sense:**
- B-tree nodes (btrfs, ZFS)
- Systems with built-in garbage collection
- When you need MVCC (we do this at LSM level)

## Interaction with Other Decisions

### Segments (Bounded Files)

Append-only + segments = bounded recovery time:

```rust
// Each segment is append-only
// When segment reaches 128MB, start new segment
// Recovery only scans last segment for partial writes
```

Without segments, recovery time grows unbounded as WAL grows.

### Compaction (LSM Level)

Append-only WAL + compaction at higher level:

```rust
WAL: [put(k1,v1), put(k2,v2), put(k1,v3), delete(k2)]
     ↓ Flush to memtable
Memtable: {k1: v3}  // Only latest version
     ↓ Compact to SSTable
SSTable: [k1=v3]  // Tombstones removed
```

WAL is append-only (simple, fast).
LSM compaction removes duplicates (space-efficient).

### CRC Checksums

Append-only makes CRC validation simple:

```rust
// CRC protects against partial writes at the end
let crc = compute_crc(&record_bytes);
append(&record_bytes);
append(&crc);  // Last 4 bytes

// Recovery: scan for valid CRCs, stop at first invalid
```

If we allowed in-place updates, every record would need checksums of neighbors to detect corruption.

## Real-World Impact

### SQLite WAL

SQLite uses a similar append-only WAL:

```
WAL format: [record1][record2][record3]...
Checkpoint: Apply WAL to database, then truncate WAL
```

Key difference: SQLite WAL is designed for single-process use. nori-wal supports multiple readers and concurrent access.

### PostgreSQL WAL

PostgreSQL also uses append-only WAL:

```
pg_wal/000000010000000000000001
pg_wal/000000010000000000000002
...
```

Each file is append-only. Archives old WAL files for point-in-time recovery.

### Kafka

Kafka's log is append-only:

```
Segment 0: [msg1][msg2][msg3]...  (1GB)
Segment 1: [msg4][msg5][msg6]...  (1GB, active)
```

Key difference: Kafka is the primary storage. nori-wal is a durability layer for in-memory structures.

## Testing Strategy

We validate append-only semantics with:

**1. Crash simulation tests:**

```rust
#[tokio::test]
async fn test_crash_during_append() {
    // Append 100 records
    for i in 0..100 {
        wal.append(&Record::put(format!("k{}", i), b"value")).await?;
    }

    // Simulate crash (drop WAL without sync)
    drop(wal);

    // Reopen - should recover all synced records
    let (wal2, info) = Wal::open(config).await?;
    assert!(info.valid_records <= 100);  // May lose last few
}
```

**2. Corruption injection:**

```rust
#[tokio::test]
async fn test_tail_corruption() {
    // Write clean data
    wal.append(&record).await?;
    wal.sync().await?;

    // Corrupt tail of segment
    corrupt_last_bytes(segment_path, 100);

    // Recovery should truncate corruption
    let (wal2, info) = Wal::open(config).await?;
    assert!(info.corruption_detected);
    assert_eq!(info.bytes_truncated, 100);
}
```

**3. Concurrent access:**

```rust
#[tokio::test]
async fn test_concurrent_readers() {
    // Write 1000 records
    // Spawn 10 readers from different positions
    // All should read consistent data (append-only guarantee)
}
```

## Conclusion

Append-only architecture is fundamental to nori-wal's design. It provides:

- **Crash safety** - Partial writes can't corrupt old data
- **Simple recovery** - Truncate at first corruption
- **Performance** - Sequential I/O is fast
- **Concurrency** - Lock-free reads of historical data

The trade-off (no in-place updates) is acceptable because:
- WAL is a durability layer, not primary storage
- LSM compaction handles deduplication
- Sequential writes are faster than random updates anyway

This decision has proven stable since v0.1 and is unlikely to change.
