# Segment Lifecycle

How segments are created, written to, and closed.

## Table of contents

---

## Overview

A segment goes through several states during its lifecycle:

```
[Creating] → [Active] → [Closing] → [Closed] → [Deleted]
```

Each state has specific guarantees and allowed operations.

## Segment States

### Creating

**When:** New segment is being initialized.

**Operations:**
- Create file on disk
- Pre-allocate space (if enabled)
- Initialize metadata

**Code:**

```rust
async fn create_segment(id: u64, config: &SegmentConfig) -> Result<SegmentFile> {
    let path = segment_path(&config.dir, id);

    // Create file
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .read(true)
        .open(&path)
        .await?;

    // Pre-allocate if configured
    if config.preallocate {
        preallocate(&file, config.max_segment_size).await?;
        file.sync_all().await?;  // Ensure allocation is durable
    }

    Ok(SegmentFile {
        id,
        file,
        size: 0,  // No data written yet
        path,
    })
}
```

**Guarantees:**
- File exists on disk after creation
- If pre-allocation enabled, space is reserved
- File handle is open for writing

**Failure Modes:**
- Disk full (caught during pre-allocation)
- Permission denied
- Too many open files

### Active

**When:** Segment is currently being written to.

**Operations:**
- Append records
- Fsync (based on policy)
- Query current size

**Code:**

```rust
impl SegmentFile {
    async fn write(&mut self, data: &[u8]) -> Result<()> {
        self.file.write_all(data).await?;
        self.size += data.len() as u64;
        Ok(())
    }

    async fn sync(&mut self) -> Result<()> {
        self.file.sync_all().await
    }

    fn current_size(&self) -> u64 {
        self.size
    }
}
```

**Invariants:**
- `size <= max_segment_size` (always checked before write)
- File handle is open
- Data may be buffered (not yet on disk until fsync)

**Thread Safety:**
- Only one active segment at a time
- Protected by `Mutex<State>` in `SegmentManager`
- Appends are serialized

### Closing

**When:** Segment has reached max size or is being closed gracefully.

**Operations:**
- Finalize writes
- Fsync remaining data
- Truncate to actual size (if pre-allocated)
- Update metadata

**Code:**

```rust
async fn close_segment(segment: &mut SegmentFile, preallocated: bool) -> Result<()> {
    // Ensure all data is written
    segment.sync().await?;

    // If we pre-allocated, truncate to actual size
    if preallocated {
        segment.file.set_len(segment.size).await?;
        segment.file.sync_all().await?;
    }

    // Emit event
    emit_event(WalEvt::SegmentRoll {
        bytes: segment.size,
    });

    Ok(())
}
```

**Why truncate?**

Pre-allocation reserves space:

```
Created:  [reserved 128 MB]
Written:  [data: 75 MB][unused: 53 MB]
Truncate: [data: 75 MB]  ← Reclaim 53 MB
```

**Guarantees:**
- All buffered data is on disk
- File size matches actual data
- Segment is immutable after closing

### Closed

**When:** Segment is complete and no longer being written to.

**Operations:**
- Read from anywhere in segment
- No writes allowed
- Can be safely copied/backed up

**Code:**

```rust
// Reading from closed segment (no lock needed)
async fn read_closed_segment(id: u64) -> Result<SegmentReader> {
    let path = segment_path(id);

    let file = File::open(path).await?;

    Ok(SegmentReader {
        file,
        buffer: vec![0u8; 64 * 1024],  // 64KB read buffer
    })
}
```

**Guarantees:**
- Data is immutable
- Multiple readers can access simultaneously
- File may be deleted by garbage collection

### Deleted

**When:** Segment is no longer needed (garbage collected).

**Operations:**
- Remove file from disk
- Free resources

**Code:**

```rust
async fn delete_segment(id: u64, dir: &Path) -> Result<()> {
    let path = segment_path(dir, id);

    // Remove file
    tokio::fs::remove_file(&path).await?;

    // Emit event
    emit_event(WalEvt::SegmentDeleted { segment_id: id });

    Ok(())
}
```

**Safety:**
- Only delete segments before checkpoint
- Ensure no readers are accessing segment
- Handle "file not found" (idempotent delete)

## Rotation Process

Rotation happens when active segment reaches max size:

```rust
async fn check_and_rotate(&mut self) -> Result<()> {
    let mut state = self.state.lock().await;

    if state.current_segment.size >= self.config.max_segment_size {
        self.rotate_segment_locked(&mut state).await?;
    }

    Ok(())
}

async fn rotate_segment_locked(&mut self, state: &mut State) -> Result<()> {
    let old_segment = &mut state.current_segment;

    // 1. Close current segment
    old_segment.sync().await?;

    if self.config.preallocate {
        old_segment.file.set_len(old_segment.size).await?;
    }

    // 2. Create new segment
    let new_id = old_segment.id + 1;
    let new_segment = create_segment(new_id, &self.config).await?;

    // 3. Swap segments
    state.current_segment = new_segment;

    // 4. Emit event
    self.meter.emit(VizEvent::Wal(WalEvt::SegmentRoll {
        bytes: old_segment.size,
    }));

    Ok(())
}
```

**Timing:**

```
Before rotation:
  Segment 5: [====================] 128 MB (full)

During rotation (10-50ms):
  Segment 5: Closing...
  Segment 6: Creating...

After rotation:
  Segment 5: [====================] 128 MB (closed)
  Segment 6: [                    ] 0 MB (active)
```

**What happens to in-flight writes during rotation?**

They wait for the lock:

```rust
// Writer 1: Triggers rotation
wal.append(&record1).await?;  // Acquires lock, sees size > max, rotates

// Writer 2: Waits during rotation
wal.append(&record2).await?;  // Waits for lock, then writes to new segment
```

## File Naming Convention

Segments are named with zero-padded IDs:

```rust
fn segment_path(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{:06}.wal", id))
}
```

Examples:

```
000000.wal  ← First segment
000001.wal
000002.wal
...
000999.wal
001000.wal
```

**Why 6 digits?**

- Supports up to 1 million segments
- Lexicographic ordering matches numeric ordering
- Easy to glob: `*.wal`

**Why .wal extension?**

- Distinguishes from other files in directory
- Standard for write-ahead logs
- Helps monitoring tools identify log files

## Recovery Impact

Segments make recovery efficient:

```rust
async fn recover(dir: &Path) -> Result<RecoveryInfo> {
    let segments = find_all_segments(dir).await?;

    let mut info = RecoveryInfo::default();

    for segment_id in segments {
        // Only validate last segment for corruption
        if segment_id == segments.last() {
            info += validate_segment(segment_id).await?;
        } else {
            // Earlier segments were validated when closed
            info += count_records(segment_id).await?;
        }
    }

    Ok(info)
}
```

**Optimization:**

Closed segments were already validated, so only scan for record count (fast). Only the last (active) segment needs full CRC validation.

## State Machine Diagram

```
┌──────────┐
│ Creating │
└────┬─────┘
     │ create_segment()
     ↓
┌──────────┐
│  Active  │────────────────┐
└────┬─────┘                │
     │                      │ size >= max
     │ append()             │
     │ sync()               │
     ↓                      ↓
┌──────────┐          ┌──────────┐
│  Active  │────────→ │ Closing  │
└──────────┘  rotate  └────┬─────┘
                           │ close_segment()
                           ↓
                      ┌──────────┐
                      │  Closed  │
                      └────┬─────┘
                           │ delete_segments_before()
                           ↓
                      ┌──────────┐
                      │ Deleted  │
                      └──────────┘
```

## Observability

Segments emit events at lifecycle transitions:

```rust
// Rotation
emit(WalEvt::SegmentRoll {
    segment_id: new_id,
    bytes: old_size,
});

// Deletion
emit(WalEvt::SegmentDeleted {
    segment_id: id,
});
```

**Monitoring:**

```rust
// Track active segment
metrics.gauge("wal.active_segment_id", current_id);
metrics.gauge("wal.active_segment_bytes", current_size);

// Track rotations
metrics.counter("wal.segments_rotated", 1);

// Track deletions
metrics.counter("wal.segments_deleted", count);
```

**Alerting:**

- Too many segments → Need garbage collection
- No rotations for long time → Workload might be too light
- Frequent rotations → Might need larger segments

## Edge Cases

### Rotation During Recovery

If recovery finds active segment is full:

```rust
if last_segment.size >= config.max_segment_size {
    // Start new segment immediately
    rotate_before_any_writes();
}
```

### Empty Segments

Possible if rotation happens with no writes:

```rust
Segment 5: [data: 128 MB]
Rotate → Segment 6: [data: 0 MB]
No writes...
Shutdown

// On recovery:
// Segment 6 exists but is empty (size = 0)
// This is valid - just delete it
```

### Pre-allocation Failure

If pre-allocation fails (disk full):

```rust
match create_segment(id, config).await {
    Err(SegmentError::Io(e)) if e.kind() == ErrorKind::NoSpaceLeft => {
        // Can't create new segment!
        // Options:
        // 1. Trigger emergency GC
        // 2. Fail writes until space available
        // 3. Alert operators
        return Err(SegmentError::Io(e));
    }
    other => other,
}
```

### Concurrent Readers During Rotation

Readers of old segments are unaffected:

```rust
// Reader is reading segment 3
let mut reader = wal.read_from(Position { segment_id: 3, offset: 0 });

// Writer rotates from segment 5 to 6
wal.append(&record);  // Triggers rotation

// Reader continues unaffected
while let Some(record) = reader.next().await? {
    // Still reading from segment 3
}
```

Only readers of the active segment need to coordinate with writers.

## Testing

**Lifecycle tests:**

```rust
#[tokio::test]
async fn test_segment_rotation() {
    let config = WalConfig {
        max_segment_size: 1024,  // Small for testing
        ..Default::default()
    };

    let (wal, _) = Wal::open(config).await?;

    // Write enough to trigger rotation
    for _ in 0..100 {
        wal.append(&Record::put(b"k", &[0u8; 100])).await?;
    }

    // Check that segments were created
    let segments = list_segments(&wal.config().dir)?;
    assert!(segments.len() > 1);
}
```

**Edge case tests:**

```rust
#[tokio::test]
async fn test_rotation_at_exact_boundary() {
    // Write exactly max_segment_size bytes
    // Verify rotation happens correctly
}

#[tokio::test]
async fn test_empty_segment_on_shutdown() {
    // Rotate
    // Shutdown immediately (no writes to new segment)
    // Verify recovery handles empty segment
}
```

## Conclusion

Segment lifecycle management is critical to nori-wal's design. Key points:

- **States:** Creating → Active → Closing → Closed → Deleted
- **Rotation:** Happens at size threshold
- **Immutability:** Closed segments never change
- **Recovery:** Only validate last segment
- **Observability:** Events at each transition

Understanding this lifecycle is essential for contributing to segment-related code.
