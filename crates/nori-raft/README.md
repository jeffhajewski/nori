# nori-raft

Raft consensus implementation with read-index, leader leases, and LSM integration.

## Features

- **Leader Election**: Randomized timeouts with efficient vote handling
- **Log Replication**: AppendEntries with efficient backtracking for divergent logs
- **Leader Leases**: Fast linearizable reads when lease is valid
- **Read-Index**: Quorum-based linearizable reads as lease fallback
- **Snapshots**: InstallSnapshot RPC for log compaction and follower catch-up
- **Joint Consensus**: Safe membership changes via two-phase protocol
- **LSM Integration**: `ReplicatedLSM` adapter for NoriKV storage
- **Observability**: VizEvent emission for dashboard visualization

## Quick Start

```rust
use nori_raft::{Raft, RaftConfig, ReplicatedLog};
use bytes::Bytes;

#[tokio::main]
async fn main() -> nori_raft::Result<()> {
    let config = RaftConfig::default();
    let raft = Raft::new(config).await?;

    // Propose a command (only succeeds on leader)
    let index = raft.propose(Bytes::from("set key=value")).await?;
    println!("Command replicated at index {}", index);

    // Perform a linearizable read
    raft.read_index().await?;
    // Now safe to read from state machine

    Ok(())
}
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  ReplicatedLog Trait                                        │
│  - propose(cmd) -> LogIndex                                 │
│  - read_index() -> ()                                       │
│  - is_leader() / leader()                                   │
│  - install_snapshot()                                       │
│  - subscribe_applied()                                      │
└────────────────────────┬────────────────────────────────────┘
                         │
┌────────────────────────▼────────────────────────────────────┐
│  Raft Core                                                  │
│  ┌─────────────────┐  ┌─────────────────┐                  │
│  │    Election     │  │   Replication   │                  │
│  │  - RequestVote  │  │  - AppendEntries│                  │
│  │  - Timeouts     │  │  - Commit index │                  │
│  └─────────────────┘  └─────────────────┘                  │
│  ┌─────────────────┐  ┌─────────────────┐                  │
│  │     Lease       │  │   Read-Index    │                  │
│  │  - Fast reads   │  │  - Quorum check │                  │
│  └─────────────────┘  └─────────────────┘                  │
│  ┌─────────────────┐  ┌─────────────────┐                  │
│  │    Snapshot     │  │   State Machine │                  │
│  │  - Compaction   │  │  - Apply channel│                  │
│  └─────────────────┘  └─────────────────┘                  │
└─────────────────────────────────────────────────────────────┘
```

## Modules

| Module | Description |
|--------|-------------|
| `config` | Raft configuration (timeouts, cluster size) |
| `raft` | Main Raft struct implementing ReplicatedLog |
| `state` | Node state (Follower, Candidate, Leader) |
| `log` | Replicated log with persistence |
| `election` | Leader election with randomized timeouts |
| `replication` | AppendEntries and log synchronization |
| `lease` | Leader lease management |
| `read_index` | Quorum-based read consistency |
| `snapshot` | Log compaction and snapshot transfer |
| `transport` | RPC abstraction for network communication |
| `lsm_adapter` | Adapter for nori-lsm storage backend |
| `replicated_lsm` | Full ReplicatedLSM integration |
| `rpc_handler` | Incoming RPC request handling |

## Configuration

```rust
use nori_raft::RaftConfig;
use std::time::Duration;

let config = RaftConfig {
    node_id: 1,
    cluster_nodes: vec![1, 2, 3],

    // Election timeouts (randomized within range)
    election_timeout_min: Duration::from_millis(150),
    election_timeout_max: Duration::from_millis(300),

    // Heartbeat interval (leader sends AppendEntries)
    heartbeat_interval: Duration::from_millis(50),

    // Leader lease duration (must be < election_timeout_min)
    lease_duration: Duration::from_millis(100),

    // Log compaction threshold
    snapshot_threshold: 10_000,

    ..Default::default()
};
```

## ReplicatedLog Trait

The `ReplicatedLog` trait provides the high-level interface:

```rust
#[async_trait]
pub trait ReplicatedLog: Send + Sync {
    /// Propose a command to be replicated (leader only)
    async fn propose(&self, cmd: Bytes) -> Result<LogIndex>;

    /// Ensure linearizable read (waits for quorum if lease expired)
    async fn read_index(&self) -> Result<()>;

    /// Check if this node is the leader
    fn is_leader(&self) -> bool;

    /// Get current leader (if known)
    fn leader(&self) -> Option<NodeId>;

    /// Install snapshot from leader (for lagging followers)
    async fn install_snapshot(&self, snap: Box<dyn Read + Send>) -> Result<()>;

    /// Subscribe to applied log entries
    fn subscribe_applied(&self) -> Receiver<(LogIndex, Bytes)>;
}
```

## Read Consistency

Two mechanisms for linearizable reads:

### Leader Lease (Fast Path)
```rust
// If lease is valid, read directly from leader's state machine
if raft.is_leader() && lease.is_valid() {
    return state_machine.read(key);
}
```

### Read-Index (Fallback)
```rust
// Quorum-based: ensures leader is still authoritative
raft.read_index().await?;
// Now safe to read - all prior commits are visible
state_machine.read(key)
```

## Observability

Raft emits `VizEvent::Raft` events for dashboard visualization:

- `LeaderElected { node, term }` - New leader elected
- `VoteReq { from, term }` - Vote request sent
- `VoteGranted { from, term }` - Vote granted
- `StepDown { term }` - Leader stepped down
- `LogReplicated { index, term }` - Entry replicated

## Testing

```bash
# Run all tests
cargo test -p nori-raft

# Run specific test
cargo test -p nori-raft test_leader_election

# Run integration tests
cargo test -p nori-raft --test integration_tests
```

## Design References

Based on the [Raft paper](https://raft.github.io/raft.pdf) by Ongaro & Ousterhout (2014) with extensions for:
- Leader leases (Spanner-style fast reads)
- Read-index protocol (etcd-style)
- Joint consensus for membership changes

See `context/31_consensus.yaml` for complete specification.

## License

MIT
