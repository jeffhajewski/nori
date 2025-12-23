# Architecture

Understanding how NoriKV components fit together to build a distributed key-value store.

---

## System Overview

NoriKV is a **sharded, Raft-replicated, log-structured key-value store** built from composable components. Each component solves a specific problem and can be used independently or as part of the complete system.

```
┌─────────────────────────────────────────────────┐
│  Client SDKs (TypeScript, Python, Go, Java)    │
└──────────────────┬──────────────────────────────┘
                   │ gRPC
┌──────────────────▼──────────────────────────────┐
│  NoriKV Server Node (DI composition)            │
│                                                  │
│  ┌─────────────────────────────────────────┐   │
│  │  Adapters Layer                         │   │
│  │  - LSM Storage Adapter                  │   │
│  │  - Raft Consensus Adapter               │   │
│  │  - SWIM Membership Adapter              │   │
│  │  - gRPC/HTTP Transport Adapter          │   │
│  └─────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────┐   │
│  │  Ports (Traits)                         │   │
│  │  - Storage                              │   │
│  │  - ReplicatedLog                        │   │
│  │  - Membership                           │   │
│  │  - Transport, Router                    │   │
│  └─────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────┐   │
│  │  Domain Layer                           │   │
│  │  - Types, IDs, Versions                 │   │
│  │  - Sharding (Jump Consistent Hash)      │   │
│  │  - Error handling                       │   │
│  └─────────────────────────────────────────┘   │
└─────────────────────────────────────────────────┘
```

---

## Layering Model

NoriKV uses **Hexagonal Architecture** (Ports & Adapters) for clean separation of concerns:

### Domain Layer
Core business logic, types, and rules. No dependencies on infrastructure.

- Key/value types
- Shard IDs, replica placement
- Versioning and conflict resolution
- Error codes

### Ports (Traits)
Abstract interfaces defining what the system needs.

```rust
pub trait Storage {
    async fn get(&self, key: &[u8]) -> Result<Option<Bytes>>;
    async fn put(&self, key: &[u8], value: &[u8]) -> Result<()>;
    async fn delete(&self, key: &[u8]) -> Result<()>;
    async fn scan(&self, start: &[u8], end: &[u8]) -> Result<Vec<(Bytes, Bytes)>>;
}

pub trait ReplicatedLog {
    async fn append(&self, entry: &[u8]) -> Result<LogIndex>;
    async fn read(&self, index: LogIndex) -> Result<Option<Bytes>>;
    async fn wait_committed(&self, index: LogIndex) -> Result<()>;
}

pub trait Membership {
    fn alive_members(&self) -> Vec<NodeId>;
    fn is_alive(&self, node: NodeId) -> bool;
    async fn join(&self, seeds: Vec<SocketAddr>) -> Result<()>;
}
```

### Adapters
Concrete implementations of ports using specific technologies.

- **LSM Adapter** - Implements `Storage` using nori-lsm (WAL + SSTables)
- **Raft Adapter** - Implements `ReplicatedLog` using nori-raft
- **SWIM Adapter** - Implements `Membership` using nori-swim
- **gRPC Adapter** - Implements `Transport` using Tonic

---

## Data Flow

### Write Path

```
Client PUT("user:42", "alice")
  ↓ gRPC
Router (hash key → shard 15)
  ↓
Raft Leader (shard 15 replica 0)
  ↓ replicate to followers
[Follower 1, Follower 2] ack
  ↓ commit to LSM
WAL append (fsync)
  ↓
Memtable (in-memory)
  ↓ (later, async compaction)
SSTable flush to disk
  ↓
Client receives success
```

**Latency breakdown:**
- gRPC: ~1ms
- Raft replication: ~5ms (2 RTTs + fsync)
- WAL append: ~100µs (batched fsync)
- Total p95: ~20ms

### Read Path (Linearizable)

```
Client GET("user:42")
  ↓ gRPC
Router (hash key → shard 15)
  ↓
Raft Leader (read-index or lease)
  ↓ check committed index
LSM read (memtable → L0 → L1...)
  ↓
Return value to client
```

**Latency breakdown:**
- gRPC: ~1ms
- Raft read-index: ~1ms (with leases: ~0µs)
- LSM read (cache hit): ~5µs
- Total p95: ~10ms

---

## Component Details

### Storage: nori-lsm

The LSM engine combines:
- **nori-wal** - Append-only write-ahead log
- **nori-sstable** - Immutable sorted tables
- Memtable (in-memory skip list)
- Background compaction (leveled strategy)

[Learn more about LSM →](../crates/nori-lsm/index.md)

---

### Consensus: nori-raft

Raft provides:
- Leader election
- Log replication with majority quorum
- Read-index optimization for consistent reads
- Lease-based reads (linearizable without log appends)
- Joint consensus for membership changes
- Snapshot support for log compaction

[Learn more about Raft →](../crates/nori-raft/index.md)

---

### Membership: nori-swim

SWIM provides:
- Gossip-based failure detection
- Scalable health checks (O(log N) message overhead)
- Eventual consistency for cluster state
- Integration with Raft for automatic reconfiguration

[Learn more about SWIM →](../crates/nori-swim/index.md)

---

### Sharding & Placement

NoriKV uses **Jump Consistent Hash** for deterministic shard assignment:

```rust
fn key_to_shard(key: &[u8], num_shards: u32) -> u32 {
    let hash = xxhash64(key, seed: 0);
    jump_consistent_hash(hash, num_shards)
}
```

**Why Jump Consistent Hash?**
- Deterministic (no routing table)
- Minimal movement on resize (only K/N keys move)
- Lock-free lookups
- Simple implementation (~10 lines of code)

**Default configuration:**
- 1024 virtual shards
- Replication factor: 3
- Replica placement: Hash mod ring + offset

---

### Observability: nori-observe

Every component emits typed events via the `Meter` trait:

```rust
pub trait Meter: Send + Sync {
    fn emit(&self, event: VizEvent);
}

pub enum VizEvent {
    Wal(WalEvt),      // WAL operations
    Lsm(LsmEvt),      // LSM compactions
    Raft(RaftEvt),    // Raft elections, commits
    Swim(SwimEvt),    // Membership changes
}
```

**Exporters:**
- Prometheus/OpenMetrics
- OTLP (with trace exemplars)
- Live dashboard (WebSocket stream)

[Learn more about observability →](../crates/nori-observe/index.md)

---

## Deployment Topologies

### Single Node (Development)

```
┌─────────────────┐
│  NoriKV Node    │
│  - All shards   │
│  - RF=1         │
└─────────────────┘
```

**Use case:** Local development, testing, single-machine workloads.

---

### 3-Node Cluster (Production)

```
┌─────────┐   ┌─────────┐   ┌─────────┐
│ Node 0  │   │ Node 1  │   │ Node 2  │
│ Shards: │   │ Shards: │   │ Shards: │
│ 0,1,2   │   │ 0,1,2   │   │ 0,1,2   │
│ (leader)│   │(follower│   │(follower│
└─────────┘   └─────────┘   └─────────┘
```

**Configuration:**
- 3 nodes, 3 shards (1024 virtual)
- RF=3 (full replication)
- Each shard has 1 leader, 2 followers

**Use case:** Small production deployments.

---

### 9-Node Cluster (Large Scale)

```
┌─────────┐   ┌─────────┐       ┌─────────┐
│ Node 0  │   │ Node 1  │  ...  │ Node 8  │
│ Shards: │   │ Shards: │       │ Shards: │
│ 0-340   │   │ 341-681 │       │ 682-1023│
└─────────┘   └─────────┘       └─────────┘
```

**Configuration:**
- 9 nodes, 1024 virtual shards
- RF=3 (each shard on 3 nodes)
- Leader election per shard

**Use case:** High-throughput production workloads.

---

## Consistency Guarantees

### Strong Consistency (Default)

- **Linearizable reads** via Raft read-index or leases
- **Serializable writes** via Raft log replication
- **Exactly-once semantics** for committed writes

### Tunable Consistency

```rust
// Linearizable read (default)
client.get("key").await?;

// Stale read (follower, faster)
client.get_with_consistency("key", Consistency::Stale).await?;
```

---

## Failure Modes

### Node Failure

- SWIM detects failure within ~5 seconds
- Raft elects new leader for affected shards
- Clients retry with exponential backoff
- No data loss (majority quorum)

### Network Partition

- Minority partition: Can't commit writes (no quorum)
- Majority partition: Continues operating
- Partition heals: Minority catches up via log replication

### Disk Corruption

- CRC32C checksums detect corruption
- WAL recovery: Prefix-valid truncation
- SSTable: Block-level checksums

---

## Performance Characteristics

| Operation | Latency (p95) | Throughput |
|-----------|---------------|------------|
| PUT | 20ms | 50K/sec/node |
| GET (linearizable) | 10ms | 100K/sec/node |
| GET (stale) | 5ms | 200K/sec/node |
| SCAN (1KB range) | 15ms | - |

**Assumptions:** 3-node cluster, SSD, 1Gbps network, 1KB values

---

## Next Steps

### Server Architecture Deep-Dives

- **[Multi-Shard Server Architecture](multi-shard-server.md)** - How 1024 virtual shards are managed
- **[SWIM Topology Tracking](swim-topology.md)** - Failure detection and cluster membership

### Learn About Specific Components

- [Storage Layer (LSM)](../crates/nori-lsm/index.md)
- [Consensus (Raft)](../crates/nori-raft/index.md)
- [Membership (SWIM)](../crates/nori-swim/index.md)
- [Observability](../crates/nori-observe/index.md)

### Operations & Monitoring

- **[REST API Reference](../operations/rest-api.md)** - HTTP endpoints for health and metrics
- **[Metrics Reference](../operations/metrics.md)** - Complete Prometheus metrics guide
- [Operations Guide](../operations/index.md)

### Build with NoriKV

- [Client SDKs](../sdks/index.md)
