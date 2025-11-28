# Internals

Deep dives into nori-wal implementation details.

This section documents the internal implementation of nori-wal for contributors, maintainers, and anyone curious about how it works under the hood.

## Target Audience

- **Contributors** - Want to add features or fix bugs
- **Maintainers** - Need to understand design for code review
- **Advanced Users** - Curious about implementation details
- **Library Authors** - Building similar systems

If you're just using nori-wal, start with [Getting Started](../getting-started/index.md) and [API Reference](../api-reference/index.md) instead.

## Module Structure

nori-wal is organized into focused modules:

```
crates/nori-wal/src/
├── lib.rs           # Public API exports
├── wal.rs           # High-level Wal API
├── segment.rs       # Segment management & fsync
├── record.rs        # Record encoding/decoding
├── recovery.rs      # Recovery logic
└── prealloc.rs      # Platform-specific file preallocation
```

### Module Responsibilities

| Module | Purpose | Key Types |
|--------|---------|-----------|
| [wal.rs](wal-module.md) | Public API, lifecycle | `Wal`, `WalConfig` |
| [segment.rs](segment-module.md) | File I/O, rotation | `SegmentManager`, `SegmentFile` |
| [record.rs](record-module.md) | Serialization format | `Record`, `Compression` |
| [recovery.rs](recovery-module.md) | Crash recovery | `RecoveryInfo` |
| [prealloc.rs](prealloc-module.md) | File preallocation | Platform-specific code |

## Key Internal Concepts

### [Segment Lifecycle](segment-lifecycle.md)
How segments are created, written to, and closed.

### [Locking Strategy](locking.md)
Where locks are used and how we avoid contention.

### [Buffer Management](buffers.md)
How we minimize allocations and copies.

### [Fsync Coordination](fsync-coordination.md)
How batched fsync works internally.

### [Error Handling](error-handling.md)
How errors propagate through the system.

## Code Walkthrough

### Append Path

The critical path for `wal.append()`:

```rust
// 1. User calls
wal.append(&record).await?;

// 2. Wal forwards to SegmentManager
self.manager.append(&record).await?;

// 3. SegmentManager acquires lock
let mut state = self.state.lock().await;

// 4. Check if rotation needed
if state.current_segment.size >= self.config.max_segment_size {
    self.rotate_segment(&mut state).await?;
}

// 5. Encode record
let encoded = record.encode();

// 6. Write to segment
state.current_segment.write(&encoded).await?;

// 7. Apply fsync policy
match self.config.fsync_policy {
    FsyncPolicy::Always => state.current_segment.sync().await?,
    FsyncPolicy::Batch(window) => self.maybe_sync_batch(window, &mut state).await?,
    FsyncPolicy::Os => { /* no fsync */ }
}

// 8. Return position
Ok(Position { segment_id, offset })
```

### Recovery Path

How recovery works on startup:

```rust
// 1. Wal::open() calls recovery::recover()
let recovery_info = recovery::recover(&config.dir).await?;

// 2. Find all segment files
let segments = find_segments(&dir)?;

// 3. For each segment
for segment_id in segments {
    // 4. Read entire segment into memory
    let data = read_segment(segment_id)?;

    // 5. Scan for valid records
    let mut offset = 0;
    while offset < data.len() {
        match Record::decode(&data[offset..]) {
            Ok((record, size)) => {
                valid_records += 1;
                offset += size;
            }
            Err(_) => {
                // 6. Found corruption, truncate here
                truncate_segment(segment_id, offset)?;
                break;
            }
        }
    }
}

// 7. Return recovery info
Ok(RecoveryInfo { valid_records, ... })
```

## Performance Considerations

### Hot Path Optimizations

**1. Lock-free reads**

Old segments are immutable:

```rust
// Readers don't need locks for closed segments
pub async fn read_from(&self, pos: Position) -> Result<SegmentReader> {
    if pos.segment_id < self.current_segment_id() {
        // Closed segment - no lock needed
        let file = File::open(segment_path(pos.segment_id)).await?;
        return Ok(SegmentReader::new(file, pos.offset));
    }

    // Active segment - need to coordinate
    let state = self.state.lock().await;
    // ...
}
```

**2. Minimal allocations**

```rust
// Reuse buffers
struct SegmentManager {
    write_buffer: Mutex<Vec<u8>>,  // Reused across writes
}

// Encode into pre-allocated buffer
pub fn encode_into(&self, buf: &mut Vec<u8>) {
    buf.clear();
    // Write directly to buf
}
```

**3. Batch fsync**

```rust
// Multiple appends between fsyncs
wal.append(&r1).await?;  // Buffered
wal.append(&r2).await?;  // Buffered
// 5ms passes...
// Automatic fsync for both records
```

### Cold Path Acceptable Cost

Recovery and rotation can be slower:

```rust
// Recovery: Read entire segments into memory
// - Happens once at startup
// - Acceptable to be slow (< 1 second even for GB of data)

// Rotation: Create and preallocate new segment
// - Happens every 30-60 seconds
// - Acceptable to take 10-50ms
```

## Testing Strategy

### Unit Tests

Each module has its own tests:

```rust
// record.rs
#[cfg(test)]
mod tests {
    #[test]
    fn test_encode_decode() { /* ... */ }
}

// segment.rs
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_rotation() { /* ... */ }
}
```

### Integration Tests

End-to-end scenarios:

```rust
// tests/integration_tests.rs
#[tokio::test]
async fn test_full_recovery_scenario() {
    // Write data
    // Crash (drop without sync)
    // Reopen
    // Verify recovery
}
```

### Property Tests

Use `proptest` for randomized testing:

```rust
proptest! {
    #[test]
    fn prop_record_roundtrip(
        key in vec(any::<u8>(), 0..1024),
        value in vec(any::<u8>(), 0..1024),
    ) {
        let record = Record::put(key, value);
        let encoded = record.encode();
        let (decoded, _) = Record::decode(&encoded)?;
        assert_eq!(record, decoded);
    }
}
```

## Contributing Guidelines

When modifying internals:

**1. Maintain invariants**

```rust
// INVARIANT: current_segment.size <= max_segment_size
// Check this invariant in every method that modifies size
```

**2. Document unsafe code**

```rust
// SAFETY: Buffer is guaranteed to be valid UTF-8 because...
let s = unsafe { std::str::from_utf8_unchecked(buf) };
```

**3. Add tests for edge cases**

```rust
#[test]
fn test_rotation_at_exact_boundary() {
    // What happens when size == max_segment_size?
}
```

**4. Benchmark performance changes**

```rust
// Run benchmarks before/after
cargo bench --bench wal_bench
```

## Further Reading

Each subsection goes deep on a specific internal component:

- **[Wal Module](wal-module.md)** - High-level API implementation
- **[Segment Module](segment-module.md)** - File management and I/O
- **[Record Module](record-module.md)** - Serialization format
- **[Recovery Module](recovery-module.md)** - Crash recovery logic
- **[Preallocation](prealloc-module.md)** - Platform-specific optimizations

These documents assume familiarity with Rust and systems programming.
