# nori-raft

Per-shard Raft consensus with leader leases and read-index optimization.

---

## What is nori-raft?

**nori-raft** is a production-ready implementation of the Raft consensus algorithm designed for per-shard replication in distributed key-value stores. It provides strong consistency guarantees with optimized read paths using leader leases.

### Key Features

- **Per-Shard Consensus**: Each shard runs an independent Raft group
- **Leader Leases**: Fast reads without round-trips (2000ms lease duration)
- **Read-Index Fallback**: Linearizable reads when lease expires
- **Joint Consensus**: Safe membership reconfiguration
- **Snapshot Support**: Efficient state transfer for new/recovering nodes
- **Split-Brain Protection**: Term monotonicity + majority quorum guards

---

## Quick Example

```rust
use nori_raft::{RaftNode, RaftConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = RaftConfig {
        heartbeat_ms: 150,
        election_timeout_ms: 300..600,
        lease_ms: 2000,
        ..Default::default()
    };

    let node = RaftNode::new(config).await?;

    // Propose a command (write)
    if node.is_leader() {
        let index = node.propose(b"SET key value").await?;
        println!("Command committed at index: {}", index);
    }

    // Read with lease (fast path)
    node.read_index().await?;
    // Now safe to read from local state machine

    Ok(())
}
```

---

## Core Trait

The `ReplicatedLog` trait defines the interface for consensus:

```rust
#[async_trait::async_trait]
pub trait ReplicatedLog: Send + Sync {
    /// Propose a command to the replicated log
    async fn propose(&self, cmd: Bytes) -> Result<LogIndex, Error>;

    /// Wait for read-index (linearizable read)
    async fn read_index(&self) -> Result<(), Error>;

    /// Check if this node is the leader
    fn is_leader(&self) -> bool;

    /// Get the current leader (if known)
    fn leader(&self) -> Option<NodeId>;

    /// Install a snapshot from another node
    async fn install_snapshot(&self, snap: Box<dyn Read + Send>) -> Result<(), Error>;

    /// Subscribe to applied commands
    fn subscribe_applied(&self) -> tokio::sync::mpsc::Receiver<(LogIndex, Bytes)>;
}
```

---

## Read Strategies

| Strategy | Latency | Consistency | Use Case |
|----------|---------|-------------|----------|
| **Leader Lease** (default) | <1ms | Linearizable* | Most reads |
| **Read-Index** | ~RTT | Linearizable | When lease expires |
| **Stale Read** | <1ms | Eventual | Analytics, caching |

*Linearizable within lease validity window

### Leader Lease Flow

```
1. Leader acquires lease (2000ms)
2. Client reads → leader serves from local state
3. Lease expires → fallback to read-index
4. Read-index confirms commit → serve read
```

---

## Timing Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `heartbeat_ms` | 150ms | Leader heartbeat interval |
| `election_timeout_ms` | 300-600ms | Randomized election timeout |
| `lease_ms` | 2000ms | Leader lease duration |

**Election timeout must be >> heartbeat interval** to prevent unnecessary elections.

---

## Reconfiguration

nori-raft uses **joint consensus** for safe membership changes:

```rust
// Add a new node
node.add_member(new_node_id, new_node_addr).await?;

// Remove a node
node.remove_member(old_node_id).await?;
```

The cluster transitions through joint configuration automatically:
1. `C_old` → `C_old,new` (joint)
2. `C_old,new` → `C_new`

---

## Snapshots

Snapshots are triggered when:
- Log exceeds 256 MiB
- Applied index exceeds last snapshot by 1,000,000 entries

```rust
// Manual snapshot
node.trigger_snapshot().await?;

// Snapshot is automatically transferred to lagging followers
```

---

## Failure Behavior

| Scenario | Behavior |
|----------|----------|
| **Quorum Loss** | Reject writes; serve stale reads if requested |
| **Leader Failure** | New election within `election_timeout_ms` |
| **Network Partition** | Minority side steps down; majority continues |
| **Split Brain** | Prevented by term monotonicity + majority quorum |

---

## Observability

### Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `raft_commit_ms` | Histogram | Time to commit entries |
| `raft_apply_ms` | Histogram | Time to apply entries |
| `raft_elections_total` | Counter | Total elections triggered |
| `raft_role` | Gauge | Current role (0=follower, 1=candidate, 2=leader) |
| `read_index_ms` | Histogram | Read-index latency |

### Events

```rust
VizEvent::Raft(RaftEvent::VoteReq { from })
VizEvent::Raft(RaftEvent::VoteGranted { from })
VizEvent::Raft(RaftEvent::LeaderElected { node })
VizEvent::Raft(RaftEvent::StepDown)
VizEvent::Shard(ShardEvent::SnapshotStart)
VizEvent::Shard(ShardEvent::SnapshotDone)
```

---

## Testing

nori-raft includes comprehensive testing:

- **Linearizability**: Verified under partitions and restarts
- **Election Storms**: Single leader per term with ±200ms clock skew
- **Deterministic Simulation**: Reproducible failure scenarios via nori-testkit

---

## Installation

```toml
[dependencies]
nori-raft = "0.1"
```

---

## See Also

- [Architecture: Consensus](../../architecture/index.md) - How Raft fits in NoriKV
- [nori-swim](../nori-swim/index.md) - Membership and failure detection
- [nori-observe](../nori-observe/index.md) - Observability integration
