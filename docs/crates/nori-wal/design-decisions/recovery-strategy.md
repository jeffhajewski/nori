# Prefix-Valid Recovery Strategy

Why we truncate at the first corruption.

## Table of contents

---

## Decision

**nori-wal uses prefix-valid recovery:** When a corrupt or incomplete record is detected during recovery, we truncate the segment at that point and discard all data after it.

## What This Means

```
Segment file:
[valid record 1][valid record 2][corrupt record][valid record 3][valid record 4]
                                      ↑
                                   truncate here

After recovery:
[valid record 1][valid record 2]
```

Records 3 and 4 are lost, even though they may be valid, because they come after corruption.

## Rationale

### 1. Append-Only Semantics

Because the WAL is strictly append-only, we know:

- Records are written in order
- Newer records come after older records
- Corruption at offset N means writes stopped at N

If we find corruption at byte 1000, we know:
- Bytes 0-999 were written before the crash
- Bytes 1000+ were written during or after the crash
- We can't trust anything after byte 1000

### 2. Simplicity

Prefix-valid recovery is simple to implement and reason about:

```rust
fn recover_segment(data: &[u8]) -> (Vec<Record>, usize) {
    let mut records = vec![];
    let mut offset = 0;

    loop {
        match Record::decode(&data[offset..]) {
            Ok((record, size)) => {
                records.push(record);
                offset += size;
            }
            Err(_) => {
                // Stop at first error
                return (records, offset);
            }
        }
    }
}
```

No complex logic to skip corruption or search for valid records.

### 3. No False Positives

Alternative strategies risk accepting invalid data:

```rust
// Bad: Skip corruption and continue
match Record::decode(&data[offset..]) {
    Ok((record, size)) => { /* use record */ }
    Err(_) => {
        offset += 1;  // Skip one byte
        continue;     // Try again
    }
}
```

Problems:
- May interpret garbage as valid records (false positive)
- CRC might accidentally match for random data
- Could return records that were never actually written

### 4. Predictable Behavior

Users know exactly what to expect:

**Guarantee:** If recovery returns N records, those are the first N records that were successfully synced.

**No surprises:**
- No skipping corrupted records
- No "best effort" recovery
- No heuristics that might change behavior

## What We Gave Up

### Can't Recover Data After Corruption

```
Scenario: Power loss during write

Before crash:
[R1][R2][R3][R4][R5][partial R6]

After recovery (prefix-valid):
[R1][R2][R3][R4][R5]
         ↓
    R6 lost permanently

Alternative (scan-and-skip):
[R1][R2][R3][R4][R5][skip partial R6][R7][R8]
                                       ↑
                                  recovered!
```

We accept losing R7 and R8 to maintain simplicity and correctness guarantees.

### Higher Data Loss in Some Cases

If corruption happens early in a large segment:

```
Segment (100 MB):
[5 MB valid][1 KB corrupt][94 MB valid]
            ↑
        truncate here → lose 94 MB
```

We lose 94 MB of potentially valid data.

## Alternatives Considered

### Alternative 1: Scan-and-Skip Recovery

**Approach:** Continue scanning after corruption, try to recover more records.

```rust
fn scan_and_skip_recovery(data: &[u8]) -> Vec<Record> {
    let mut records = vec![];
    let mut offset = 0;

    while offset < data.len() {
        match Record::decode(&data[offset..]) {
            Ok((record, size)) => {
                records.push(record);
                offset += size;
            }
            Err(_) => {
                // Skip one byte, try again
                offset += 1;
            }
        }
    }

    records
}
```

**Rejected because:**

1. **False positives:** Random bytes might have valid CRC by chance (1 in 4 billion)
2. **Undefined behavior:** What if "recovered" record was never actually written?
3. **Complexity:** Need heuristics to detect false positives
4. **User confusion:** Sometimes recovers more, sometimes doesn't

**When it makes sense:**
- Interactive recovery tools where user can inspect results
- Systems where false positives are acceptable
- When data loss is catastrophic and corruption is localized

### Alternative 2: Checkpointing with Skip-List

**Approach:** Write periodic checkpoints to help recovery skip over corruption.

```rust
Segment format:
[checkpoint 1][records...][checkpoint 2][records...][checkpoint 3][records...]
                                            ↑
                                         corrupt
Recovery:
- Find last valid checkpoint
- Continue from there
```

**Rejected because:**

1. **Complexity:** Need to maintain checkpoint format, verify checkpoint integrity
2. **Write amplification:** Checkpoints add overhead to every segment
3. **Limited benefit:** Corruption usually at tail (partial write), not middle
4. **Still can't recover after tail corruption**

**When it makes sense:**
- Very large segments (>1 GB) where tail corruption wastes much space
- Systems with frequent mid-segment corruption (hardware issues)
- When checkpoint overhead is acceptable

### Alternative 3: Redundant Copies

**Approach:** Write each record twice to different locations.

```rust
Segment format:
[R1 primary][R1 secondary][R2 primary][R2 secondary]...

Recovery:
- If primary corrupt, use secondary
- If secondary corrupt, use primary
```

**Rejected because:**

1. **Storage overhead:** 2x disk usage
2. **Write amplification:** 2x writes
3. **Doesn't help with common case:** Both copies corrupt if crash during write
4. **Complex recovery:** Need to reconcile primary vs secondary

**When it makes sense:**
- Critical systems where data loss is unacceptable
- When storage is cheap and speed is not critical
- Distributed systems with replication (different approach)

## Interaction with Other Decisions

### Segments (Bounded Loss)

Prefix-valid + segments = bounded data loss:

```rust
Scenario: Corruption in segment 5

Segments:
0.wal (128 MB, complete)
1.wal (128 MB, complete)
2.wal (128 MB, complete)
3.wal (128 MB, complete)
4.wal (128 MB, complete)
5.wal (50 MB written, corrupt at 10 MB)
       ↑
    truncate to 10 MB

Maximum loss: 40 MB (segment 5 only)
Segments 0-4 unaffected
```

Without segments, corruption could affect entire WAL.

### CRC Checksums

CRC + prefix-valid = reliable detection:

```rust
// CRC catches corruption immediately
match Record::decode(data) {
    Ok(_) => { /* valid */ }
    Err(RecordError::CrcMismatch { .. }) => {
        // Stop here, truncate
    }
}
```

Without CRC, we might not detect corruption until much later (or never).

### Fsync Policies

Prefix-valid + fsync = predictable loss window:

```rust
FsyncPolicy::Batch(5ms) means:
- Crash within 5ms of write → may lose last writes
- Crash after successful sync → no loss
- Recovery always recovers up to last valid sync
```

Users know maximum data loss: batching window duration.

## Real-World Examples

### PostgreSQL

PostgreSQL uses prefix-valid recovery for WAL:

```
Recovery process:
1. Read WAL segments in order
2. Apply each record to database
3. Stop at first invalid record
4. Truncate WAL at that point
```

Source: PostgreSQL src/backend/access/transam/xlog.c

### RocksDB

RocksDB WAL also uses prefix-valid:

```cpp
// From RocksDB log_reader.cc
Status ReadRecord(...) {
  while (true) {
    ...
    if (crc_check && !CheckCrc(...)) {
      // Corruption detected - stop reading
      return Status::Corruption("CRC mismatch");
    }
  }
}
```

### Kafka

Kafka uses prefix-valid for each segment:

```
// From Kafka Log.scala
def recoverSegment(segment: LogSegment): Int = {
  var validBytes = 0
  while (...) {
    if (record.isValid()) {
      validBytes += record.size
    } else {
      segment.truncateTo(validBytes)
      return validBytes
    }
  }
}
```

## Testing Strategy

We validate prefix-valid recovery with:

**1. Corruption injection tests:**

```rust
#[tokio::test]
async fn test_truncates_at_first_corruption() {
    // Write 100 valid records
    for i in 0..100 {
        wal.append(&Record::put(format!("k{}", i), b"v")).await?;
    }
    wal.sync().await?;

    // Corrupt record 50
    corrupt_record_at_offset(segment_path, 50);

    // Recovery should stop at record 50
    let (wal2, info) = Wal::open(config).await?;
    assert_eq!(info.valid_records, 49);  // 0-48 recovered, 49+ lost
}
```

**2. Partial write tests:**

```rust
#[tokio::test]
async fn test_truncates_partial_tail() {
    wal.append(&record1).await?;
    wal.sync().await?;

    // Simulate crash mid-append (write partial record)
    write_partial_record(segment_path, &record2);

    // Recovery should truncate partial record
    let (wal2, info) = Wal::open(config).await?;
    assert_eq!(info.valid_records, 1);
    assert!(info.bytes_truncated > 0);
}
```

**3. Multi-segment recovery:**

```rust
#[tokio::test]
async fn test_corruption_only_affects_one_segment() {
    // Write to segment 0 (full)
    // Write to segment 1 (full)
    // Write to segment 2 (partial, corrupt)

    let (wal, info) = Wal::open(config).await?;

    // Segment 0 and 1 should be intact
    // Only segment 2 truncated
}
```

## Monitoring and Alerting

Users should monitor recovery events:

```rust
let (wal, info) = Wal::open(config).await?;

if info.corruption_detected {
    log::warn!(
        "WAL corruption detected: {} bytes truncated from segment",
        info.bytes_truncated
    );

    // Alert if significant data loss
    if info.bytes_truncated > 1024 * 1024 {  // > 1 MB
        alert_ops("Significant WAL data loss during recovery");
    }
}
```

## Conclusion

Prefix-valid recovery is the right choice for nori-wal because:

- **Simple and predictable** - Easy to understand and test
- **No false positives** - Never returns invalid data
- **Works with append-only** - Natural fit for append-only architecture
- **Industry standard** - Used by PostgreSQL, RocksDB, Kafka

The trade-off (losing data after corruption) is acceptable because:
- Corruption usually at tail (partial write)
- Segments bound the maximum loss
- Fsync policies control loss window
- Replication handles catastrophic failures (at higher levels)

This decision has proven stable and is unlikely to change.
