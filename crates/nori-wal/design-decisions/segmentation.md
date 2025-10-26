---
layout: default
title: Segment-Based Storage
parent: Design Decisions
nav_order: 3
---

# Segment-Based Storage
{: .no_toc }

Why the WAL is split into multiple files.
{: .fs-6 .fw-300 }

## Table of contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Decision

**nori-wal splits the log into multiple segment files** of 128 MB each (configurable), rather than using a single unbounded file.

## What This Means

```
wal/
  000000.wal  (128 MB, complete)
  000001.wal  (128 MB, complete)
  000002.wal  (75 MB, active)
```

Each segment is a separate file. When a segment reaches 128 MB, a new segment is created.

## Rationale

### 1. Bounded Recovery Time

Recovery time is proportional to data size. With segments:

```rust
// Single file (bad)
Recovery: Scan 10 GB file = 3 seconds

// Segments (good)
Recovery: Scan last 128 MB segment = 40 ms
```

Only the last (incomplete) segment needs validation. Complete segments were already validated when closed.

### 2. Garbage Collection

Can delete old segments safely:

```rust
// Segments
wal.delete_segments_before(checkpoint_position).await?;
// Deletes: 000000.wal, 000001.wal
// Keeps: 000002.wal (still needed)

// Single file (can't do this)
// Would need to copy entire file to remove old data
```

After compaction/checkpointing, old WAL data is no longer needed. With segments, just delete old files.

### 3. Filesystem Limits

Many filesystems have performance cliffs at large file sizes:

| Filesystem | Performance Issue | Size Threshold |
|------------|-------------------|----------------|
| ext4 | Indirect blocks | > 2 TB |
| XFS | Extent fragmentation | > 100 GB |
| NTFS | MFT size | > 256 GB |

Keeping files < 1 GB avoids these issues.

### 4. Operational Safety

Easier to manage multiple small files:

```bash
# Backup
rsync wal/000000.wal backup/  # Fast, can be done incrementally

# Corruption analysis
hexdump -C wal/000042.wal | less  # Manageable size

# Space reclamation
rm wal/000000.wal  # Immediate space recovery
```

Compare to a single 100 GB file - harder to copy, analyze, or clean up.

## What We Gave Up

### More File Handles

```rust
// With segments (many file handles)
let mut segments = vec![];
for id in 0..100 {
    segments.push(File::open(segment_path(id)).await?);
}

// Single file (one file handle)
let file = File::open("wal.log").await?;
```

Operating systems limit open file handles (typically 1024-65536). With many segments, could hit limits.

**Mitigation:** Only keep active segment open for writing. Open others on-demand for reading.

### Rotation Overhead

Creating new segments has overhead:

```rust
// Every 128 MB
1. Finalize current segment (truncate to exact size)
2. Sync current segment
3. Create new segment file
4. Pre-allocate new segment (fallocate)
5. Emit rotation event

// Cost: ~10-50ms depending on filesystem
```

**Mitigation:** 128 MB threshold means rotation every ~10-60 seconds for typical workloads.

### Complexity in Position Tracking

Positions now have two components:

```rust
struct Position {
    segment_id: u64,  // Which file
    offset: u64,      // Where in file
}

// vs single file
struct Position {
    offset: u64,  // Just offset
}
```

**Mitigation:** Helper methods make this transparent to users.

## Alternatives Considered

### Alternative 1: Single Unbounded File

**Approach:** One file that grows forever.

```rust
// wal.log grows to 10 GB, 100 GB, 1 TB...
```

**Rejected because:**

1. **Recovery time unbounded:** Need to scan entire file
2. **No garbage collection:** Can't remove old data without rewriting entire file
3. **Filesystem issues:** Large files hit performance cliffs
4. **Operational pain:** Hard to backup, analyze, or manage

**When it makes sense:**
- Embedded systems with bounded workloads
- When recovery time doesn't matter
- Short-lived processes that don't accumulate much data

### Alternative 2: Time-Based Segments

**Approach:** Rotate segments based on time instead of size.

```rust
// Rotate every hour
let segment_name = format!("{}.wal", chrono::Utc::now().timestamp());
```

**Rejected because:**

1. **Unpredictable size:** High-traffic hours create huge segments
2. **Unpredictable recovery time:** Large segments take longer to recover
3. **Wasted space:** Low-traffic hours create tiny segments

**When it makes sense:**
- Log rotation for human-readable logs
- When workload is predictable and uniform
- When time-based organization helps debugging

### Alternative 3: Record-Count Segments

**Approach:** Rotate after N records instead of N bytes.

```rust
// Rotate after 1 million records
if record_count >= 1_000_000 {
    rotate_segment();
}
```

**Rejected because:**

1. **Variable size:** 1M small records ≠ 1M large records
2. **Can't predict disk usage:** Users don't know how much space needed
3. **Unpredictable recovery time:** Large records = longer recovery

**When it makes sense:**
- When records are uniform size
- When counting records is more intuitive than bytes
- Fixed-size record systems

## Size Selection (128 MB)

Why 128 MB specifically?

**Too Small (< 16 MB):**
- Frequent rotations (overhead)
- Many file handles
- Fragmented storage

**Too Large (> 512 MB):**
- Longer recovery time
- More data loss on corruption
- Harder to manage files

**128 MB is sweet spot:**
- Recovery: 40ms on NVMe
- Rotation: ~30-60 seconds on typical workload
- Manageable file size for operations
- Fits in Linux page cache

Users can configure based on workload:

```rust
let config = WalConfig {
    max_segment_size: 64 * 1024 * 1024,  // 64 MB for faster recovery
    // or
    max_segment_size: 256 * 1024 * 1024,  // 256 MB for fewer rotations
    ..Default::default()
};
```

## Interaction with Other Decisions

### Prefix-Valid Recovery + Segments

Segments bound the data loss from corruption:

```rust
Corruption in segment 5:
- Segments 0-4: Unaffected (complete)
- Segment 5: Truncated at corruption
- Segments 6+: Don't exist yet

Maximum loss: One segment worth of data (128 MB)
```

Without segments, corruption anywhere could affect entire log.

### Garbage Collection

Segments enable simple GC:

```rust
// After LSM compaction, old WAL data is in SSTables
let checkpoint = last_flushed_position;
wal.delete_segments_before(checkpoint).await?;

// Deletes old files immediately
// No need to rewrite or compact WAL itself
```

### File Pre-allocation

Each segment can be pre-allocated:

```rust
// When creating segment 5:
let file = create_file("000005.wal");
fallocate(file, 128 * 1024 * 1024);  // Reserve space

// Benefits:
// - Early "disk full" detection
// - Reduced fragmentation
// - Better filesystem locality
```

Pre-allocation works best with fixed-size segments.

## Real-World Examples

### Kafka

Kafka uses segments (default 1 GB):

```
/var/lib/kafka/topic-0/
  00000000000000000000.log  (1 GB)
  00000000000000123456.log  (1 GB)
  00000000000000456789.log  (500 MB, active)
  00000000000000000000.index
  00000000000000123456.index
```

Segments enable:
- Log retention (delete old segments)
- Replication (send segments to followers)
- Compaction (rebuild segments without deleted keys)

### PostgreSQL

PostgreSQL WAL segments (default 16 MB):

```
pg_wal/
  000000010000000000000001  (16 MB)
  000000010000000000000002  (16 MB)
  ...
```

Smaller segments because:
- PostgreSQL has smaller working set
- Frequent checkpointing
- Need fast recovery for ACID guarantees

### RocksDB

RocksDB WAL rotates with memtable flush:

```
WAL tied to memtable lifecycle:
- Memtable 1 → WAL 000001.log
- Memtable 2 → WAL 000002.log
- When memtable flushed → delete corresponding WAL
```

Different approach: Logical rotation (per memtable) vs physical rotation (size-based).

## Testing Strategy

**1. Rotation behavior:**

```rust
#[tokio::test]
async fn test_rotates_at_threshold() {
    let config = WalConfig {
        max_segment_size: 1024 * 1024,  // 1 MB
        ..Default::default()
    };

    let (wal, _) = Wal::open(config).await?;

    // Write 2 MB of data
    for _ in 0..2000 {
        wal.append(&Record::put(b"k", &[0u8; 1024])).await?;
    }

    // Should have created 2+ segments
    let segments = list_segments(&config.dir)?;
    assert!(segments.len() >= 2);
}
```

**2. Garbage collection:**

```rust
#[tokio::test]
async fn test_deletes_old_segments() {
    // Create 5 segments
    // ...

    // Delete first 3
    let checkpoint = Position { segment_id: 3, offset: 0 };
    let deleted = wal.delete_segments_before(checkpoint).await?;

    assert_eq!(deleted, 3);
    assert!(!segment_exists("000000.wal"));
    assert!(!segment_exists("000001.wal"));
    assert!(!segment_exists("000002.wal"));
    assert!(segment_exists("000003.wal"));
}
```

**3. Recovery across segments:**

```rust
#[tokio::test]
async fn test_recovers_across_segments() {
    // Write data spanning 3 segments
    // Close WAL
    // Reopen

    let (wal, info) = Wal::open(config).await?;
    assert_eq!(info.segments_scanned, 3);
    assert_eq!(info.valid_records, expected_count);
}
```

## Monitoring

Users should monitor segment metrics:

```rust
// Observe via nori-observe
metrics.gauge("wal.active_segment_id", segment_id);
metrics.gauge("wal.active_segment_bytes", bytes_written);
metrics.counter("wal.segments_rotated", 1);
metrics.counter("wal.segments_deleted", deleted_count);
```

Alerts:
- Too many segments → Might need larger segment size or GC
- Frequent rotations → Might need tuning
- No rotations → Workload might be too light

## Conclusion

Segment-based storage is essential for nori-wal because:

- **Bounded recovery time** - Only validate last segment
- **Garbage collection** - Delete old segments easily
- **Operational simplicity** - Manageable file sizes
- **Filesystem compatibility** - Avoid large file issues

The trade-offs (file handles, rotation overhead) are acceptable because:
- Keep only active segment open
- Rotation is infrequent (every 30-60s)
- Benefits far outweigh costs

This decision has proven stable since v0.1 and is unlikely to change.
