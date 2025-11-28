# Core Concepts

Fundamental concepts you need to understand to use nori-wal effectively.

---

## What You'll Learn

This section covers the essential concepts behind write-ahead logs:

### [What is a Write-Ahead Log?](what-is-wal.md)
The fundamental concept of WALs, why they exist, and how they're used in modern systems.

### [Append-Only Architecture](append-only.md)
Why WALs are append-only and what that means for your application.

### [Durability & Fsync Policies](fsync-policies.md)
How to balance durability and performance with different fsync strategies.

### [Recovery Guarantees](recovery-guarantees.md)
What happens after a crash and what guarantees you can rely on.

### [When to Use a WAL](when-to-use.md)
Scenarios where WALs shine and where they don't.

---

## Prerequisites

Before diving into these concepts, you should:

- Know basic Rust (async/await, Result types)
- Understand what "durability" means in databases
- Have completed the [Quickstart](../getting-started/quickstart.md)

---

## Quick Concept Check

Test your understanding with these questions:

**Q: What does "write-ahead" mean?**
<details markdown="1">
<summary>Click to reveal answer</summary>

It means you write to the log **before** updating your main data structures. The log is the source of truth - if you crash before applying changes, you can replay the log to recover.

</details>

**Q: Why use append-only storage?**
<details markdown="1">
<summary>Click to reveal answer</summary>

Append-only is simple and fast: no complex in-place updates, no corruption from partial writes, easy to reason about. Trade-off: you need compaction/garbage collection eventually.

</details>

**Q: What's the difference between `fsync()` and `flush()`?**
<details markdown="1">
<summary>Click to reveal answer</summary>

- `flush()`: Writes data from application buffer to OS buffer (not durable!)
- `fsync()`: Forces OS to write buffers to physical disk (durable!)

Only `fsync()` guarantees durability.

</details>

---

## Learning Path

Recommended order:

1. **Start here**: [What is a WAL?](what-is-wal.md)
2. Understand [Append-Only Architecture](append-only.md)
3. Learn about [Durability & Fsync](fsync-policies.md)
4. Grasp [Recovery Guarantees](recovery-guarantees.md)
5. Apply knowledge: [When to Use a WAL](when-to-use.md)

Then move on to [How It Works](../how-it-works/index.md) for deeper technical details.
