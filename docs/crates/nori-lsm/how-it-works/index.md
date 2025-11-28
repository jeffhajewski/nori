# How It Works

Deep dive into nori-lsm's internals: memtable management, flush process, slot routing, compaction lifecycle, and snapshots.

---

## What You'll Learn

This section explains the technical implementation details of nori-lsm's ATLL architecture. If you want to understand:

- How writes flow from memtable → L0 → slots
- How keys are routed to slots using guard-based partitioning
- How compaction works with the bandit scheduler
- How memtable rotation and flushing work
- How consistent snapshots are created for backups
- How the entire system coordinates write path, read path, and background jobs

...then you're in the right place.

---

## Navigation

### [Memtable Management](memtable-management.md)
The in-memory skiplist: writes, rotation triggers, concurrent access, and memory accounting.

### [Flush Process](flush-process.md)
How memtables are flushed to L0 SSTables. Sorting, serialization, bloom filter generation, and atomic swaps.

### [Slot Routing](slot-routing.md)
How keys are mapped to slots using binary search on guard keys. Range queries and slot overlap detection.

### [Compaction Lifecycle](compaction-lifecycle.md)
The full compaction process: slot selection via bandit scheduler, K-way merge, heat updates, and metrics.

### [Snapshot Process](snapshot-process.md)
Creating consistent point-in-time snapshots for backups, replication, and disaster recovery.

---

## Who Should Read This

**You should read this section if**:
- You're implementing your own LSM-tree
- You're debugging nori-lsm behavior
- You're contributing to nori-lsm
- You want deep technical understanding of ATLL
- You're evaluating LSM implementations

**You can skip this section if**:
- You just want to use nori-lsm (see Getting Started - coming soon)
- You want high-level concepts (see [Core Concepts](../core-concepts/index.md))
- You want design rationale (see [Design Decisions](../design-decisions/index.md))

---

## Prerequisites

Before diving in, make sure you understand:

- **[What is LSM](../core-concepts/what-is-lsm.md)** - The fundamental concept
- **[ATLL Architecture](../core-concepts/atll-architecture.md)** - Guard keys, K-way fanout, heat tracking
- **[Write Path](../core-concepts/write-path.md)** - High-level flow: WAL → memtable → L0 → compaction
- **[Read Path](../core-concepts/read-path.md)** - High-level flow: memtable → L0 → slot

If you haven't read those yet, start there first!

---

## Architecture Overview

Here's how nori-lsm components fit together:

```
┌──────────────────────────────────────────────────────────────┐
│                      Application Layer                        │
│                  put(key, value) / get(key)                   │
└──────────────────────┬───────────────────────┬────────────────┘
                       │                       │
             ┌─────────▼──────────┐   ┌────────▼────────┐
             │   Write Path       │   │   Read Path     │
             └─────────┬──────────┘   └────────┬────────┘
                       │                       │
        ┌──────────────▼──────────────┐       │
        │  1. WAL (durability)        │       │
        │  2. Memtable (in-memory)    │       │
        └──────────────┬──────────────┘       │
                       │                       │
                   Flush (async)               │
                       │                       │
        ┌──────────────▼──────────────────────▼────────────┐
        │              L0 (unsorted)                        │
        │  [SST-001] [SST-002] [SST-003] [SST-004] ...     │
        └──────────────┬───────────────────────────────────┘
                       │
                 Slot Routing
                 (guard keys)
                       │
        ┌──────────────▼──────────────────────────────┐
        │         Slot 0 [0x00, 0x40)                 │
        │   [Run 0] K-max=1 (leveled, hot)            │
        ├──────────────────────────────────────────────┤
        │         Slot 1 [0x40, 0x80)                 │
        │   [Run 0] [Run 1] K-max=2 (hybrid, warm)    │
        ├──────────────────────────────────────────────┤
        │         Slot 2 [0x80, 0xC0)                 │
        │   [Run 0] [Run 1] [Run 2] K-max=3 (cold)    │
        ├──────────────────────────────────────────────┤
        │         Slot 3 [0xC0, 0xFF]                 │
        │   [Run 0] [Run 1] [Run 2] [Run 3] K-max=4   │
        └──────────────┬───────────────────────────────┘
                       │
                 Compaction
              (bandit scheduler)
                       │
        ┌──────────────▼──────────────────────────────┐
        │     Merged SSTables (sorted, non-overlapping │
        │              per slot)                       │
        └──────────────────────────────────────────────┘
```

**Key components**:
- **Memtable**: In-memory skiplist, bounded by size (default 64 MB)
- **L0**: Unsorted SSTables from memtable flushes
- **Slots**: Range-partitioned levels with guard keys
- **Compaction Scheduler**: Bandit-based slot selection
- **Snapshot Manager**: Consistent backups via reference counting

---

## Code Organization

nori-lsm is organized into modules:

```
nori-lsm/
  src/
    lib.rs              - Public API (LSM, Config, Storage trait)
    memtable.rs         - In-memory skiplist with rotation
    flush.rs            - Memtable → L0 SSTable conversion
    slot.rs             - Slot management (guard keys, runs, heat)
    compaction.rs       - K-way merge algorithm
    scheduler.rs        - Bandit-based compaction scheduling
    snapshot.rs         - Snapshot creation and management
    manifest.rs         - Metadata persistence (slots, runs, versions)
    pressure.rs         - 4-zone backpressure system
    observe.rs          - Observability (metrics, VizEvent)
```

Each "How It Works" page corresponds to one or more of these modules.

---

## Reading Order

We recommend reading in this order:

1. **[Memtable Management](memtable-management.md)** - Start here to understand the write buffer
2. **[Flush Process](flush-process.md)** - How memtables become SSTables
3. **[Slot Routing](slot-routing.md)** - How guard-based partitioning works
4. **[Compaction Lifecycle](compaction-lifecycle.md)** - The most complex subsystem
5. **[Snapshot Process](snapshot-process.md)** - Backups and consistency

You can read them independently, but they build on each other.

---

## Operational Flow

Here's a typical sequence of operations:

### Write Flow
```
1. Client: put("user:12345", "data")
2. WAL: Append to 000042.wal, fsync
3. Memtable: Insert into skiplist (50ns)
4. Check pressure: Green zone, no delay
5. Return Ok(())

[Background thread, triggered when memtable > 64 MB]
6. Rotate memtable: Active → Immutable
7. Create new active memtable
8. Flush immutable memtable:
   - Sort 1M entries
   - Build SSTable with bloom filter
   - Write 000015.sst to L0
9. Update manifest: L0 += 000015.sst
10. Delete WAL segment (no longer needed)
```

### Compaction Flow
```
1. Scheduler: Select slot via epsilon-greedy + UCB
2. Slot 1 selected (heat=0.8, k_max=1, 3 L0 files overlap)
3. Compaction:
   - Open 3 L0 SSTables + 1 existing run
   - 4-way merge (K=4 → K=1 leveled)
   - Write merged SSTable: 000016.sst
   - Build bloom filter (10,000 keys, 0.01 FPR)
4. Update slot:
   - Remove old run
   - Add new run: 000016.sst
   - Update heat score (EWMA decay)
5. Update manifest atomically
6. Delete old SSTables (reference counting)
7. Emit VizEvent::Compaction with metrics
```

### Read Flow
```
1. Client: get("user:12345")
2. Memtable: Check active, miss
3. Memtable: Check immutable (if exists), miss
4. L0: Check bloom filters (6 files)
   - 5 negative (bloom says "no")
   - 1 maybe (bloom says "maybe")
   - Read SSTable 000015.sst, found!
5. Return value (skip slot traversal)

[Alternative: key not in L0]
6. Slot routing: Binary search guard keys → Slot 1
7. Slot 1: Check bloom filter → maybe
8. Slot 1: Binary search index, read block
9. Return value or NotFound
```

---

## Visual Learning

Each page includes:

- **Mermaid diagrams** - Flowcharts and sequence diagrams
- **ASCII diagrams** - Memory layouts, tree structures
- **Code snippets** - Actual implementation from nori-lsm
- **Examples** - Concrete scenarios and edge cases
- **Metrics** - Performance characteristics and benchmarks

If you're a visual learner, you'll love this section!

---

## Concurrency Model

nori-lsm uses a hybrid concurrency approach:

**Lock-Free Reads**:
- Memtable: Lock-free skiplist (crossbeam-skiplist)
- SSTables: Immutable files, no locks needed
- Manifest: RwLock (many readers, single writer)

**Synchronized Writes**:
- WAL append: Mutex (sequential writes for durability)
- Memtable rotation: Mutex (rare, <1/min)
- Compaction: Per-slot locks (parallel compaction across slots)

**Example concurrency**:
```
Thread 1: put("a", "1")  → WAL mutex → memtable lock-free
Thread 2: put("b", "2")  → WAL mutex → memtable lock-free
Thread 3: get("c")       → memtable lock-free → L0 lock-free
Thread 4: compaction     → Slot 0 mutex (doesn't block Slot 1-15)
```

---

## Performance Characteristics

From benchmarks (see [Performance](../performance/index.md) for details):

**Write latency**:
- Memtable insert: 50ns (in-memory skiplist)
- WAL append: 1-2ms (fsync, dominant cost)
- p95: 2.1ms (Green zone), 20-40ms (Yellow zone)

**Flush latency**:
- 64 MB memtable → SSTable: ~200ms
- Bloom filter generation: ~50ms (100K keys)
- Total: ~250ms per flush

**Compaction latency**:
- 10 MB 4-way merge: ~100ms
- 100 MB 4-way merge: ~800ms
- 1 GB 4-way merge: ~8 seconds

**Throughput**:
- Sustained writes: 20K ops/sec (single memtable)
- Peak writes: 50K ops/sec (before backpressure)
- Compaction throughput: 100-150 MB/sec

---

## Next Steps

Ready to dive in? Start with:

- **[Memtable Management](memtable-management.md)** - Learn how the write buffer works
- Or jump to a specific topic that interests you

If you're looking for something else:
- **[Core Concepts](../core-concepts/index.md)** - High-level architecture and theory
- **[Design Decisions](../design-decisions/index.md)** - Why ATLL is built this way
- **[Performance](../performance/index.md)** - Benchmarks and tuning guides (coming soon)
