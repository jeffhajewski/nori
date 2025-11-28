# Getting Started with nori-wal

Everything you need to start using nori-wal in your project.

---

## Quick Navigation

<div class="code-example" markdown="1">

**Brand new to WALs?**
→ Start with [What is a Write-Ahead Log?](../core-concepts/what-is-wal.md)

**Ready to code?**
→ Follow the [5-Minute Quickstart](quickstart.md)

**Need to configure?**
→ Check the [Configuration Guide](configuration.md)

**Want deeper understanding?**
→ Explore [How It Works](../how-it-works/index.md)

</div>

---

## Learning Path

We recommend this order for learning nori-wal:

### 1. [Installation](installation.md)
Add nori-wal to your project and verify it works.

### 2. [5-Minute Quickstart](quickstart.md)
Write your first WAL program and see recovery in action.

### 3. [Configuration Guide](configuration.md)
Understand all configuration options and pick the right settings.

### 4. [Core Concepts](../core-concepts/index.md)
Learn the fundamentals: what WALs are, how they work, when to use them.

### 5. [How It Works](../how-it-works/index.md)
Deep dive into internals: record format, recovery, concurrency, etc.

### 6. [Recipes](../recipes/index.md)
Build real applications with nori-wal.

---

## Common Tasks

**Writing records:**
```rust
let record = Record::put(b"key", b"value");
let position = wal.append(&record).await?;
wal.sync().await?;  // Ensure durability
```

**Reading records:**
```rust
let mut reader = wal.read_from(Position { segment_id: 0, offset: 0 }).await?;
while let Some((record, pos)) = reader.next_record().await? {
    // Process record
}
```

**Batch writes:**
```rust
let records = vec![
    Record::put(b"key1", b"value1"),
    Record::put(b"key2", b"value2"),
];
let positions = wal.append_batch(&records).await?;
```

**Recovery:**
```rust
let (wal, recovery_info) = Wal::open(config).await?;
println!("Recovered {} records", recovery_info.valid_records);
```

---

## Common Questions

**Q: Do I need to call `sync()` after every write?**

A: It depends on your `FsyncPolicy`:
- `Always`: No, sync happens automatically
- `Batch`: No, syncs happen within the time window
- `Os`: Only if you need durability guarantees

**Q: How do I delete old data?**

A: Use `delete_segments_before()` after you've compacted/replicated the data:
```rust
let cutoff = Position { segment_id: 5, offset: 0 };
let deleted = wal.delete_segments_before(cutoff).await?;
```

**Q: Can I use nori-wal in multi-threaded code?**

A: Yes! `Wal` is `Send + Sync` and can be shared across threads:
```rust
let wal = Arc::new(wal);
// Share wal across threads
```

**Q: What happens if I crash mid-write?**

A: The WAL recovery process scans all segments, validates each record with CRC32C, and truncates any partial/corrupt data at the tail. All valid records are preserved.

---

## Next Steps

Choose your path:

- **[Start coding](quickstart.md)** - Get hands-on immediately
- **[Learn concepts](../core-concepts/index.md)** - Understand WALs deeply
- **[Configure](configuration.md)** - Tune for your workload
- **[Build something](../recipes/index.md)** - See real-world examples
