# SWIM Topology Tracking

How NoriKV uses SWIM for failure detection and maintains a consistent cluster topology view.

---

---

## Overview

NoriKV uses **SWIM (Scalable Weakly-consistent Infection-style Process Group Membership Protocol)** for distributed failure detection and cluster membership management. The topology watcher integrates SWIM events with the ClusterView system to provide clients with up-to-date routing information.

**Key Features:**
- **Gossip-based failure detection** - O(log N) message complexity
- **Scalable** - Works efficiently in clusters of 100+ nodes
- **Eventually consistent** - Membership view converges across cluster
- **Integration with Raft** - Automatic reconfiguration on membership changes
- **Client routing** - Live topology updates via Meta.WatchCluster

---

## Architecture

### Component Interaction

```
┌────────────────────────────────────────────┐
│  SWIM Membership                           │
│  - Gossip protocol                         │
│  - Failure detection                       │
│  - Incarnation numbers                     │
└────────────┬───────────────────────────────┘
             │ MembershipEvent stream
             ▼
┌────────────────────────────────────────────┐
│  Topology Watcher (background task)        │
│  - Subscribes to SWIM events               │
│  - Updates ClusterView                     │
│  - Triggers refresh on changes             │
└────────────┬───────────────────────────────┘
             │
             ▼
┌────────────────────────────────────────────┐
│  ClusterViewManager                        │
│  - Epoch-versioned cluster state           │
│  - Node list with roles                    │
│  - Shard assignments                       │
└────────────┬───────────────────────────────┘
             │ Broadcast channel
             ▼
┌────────────────────────────────────────────┐
│  Meta.WatchCluster (gRPC streaming)        │
│  - Streams updates to clients              │
│  - Clients update routing tables           │
└────────────────────────────────────────────┘
```

### Data Flow

```
Node join:
  SWIM detects new member
    ↓ MembershipEvent::MemberJoined
  Topology Watcher receives event
    ↓
  ClusterView.add_node(id, addr)
    ↓ epoch++
  Notify subscribers
    ↓
  Clients receive update
    ↓
  Routing table updated

Node failure:
  SWIM suspects member (missed pings)
    ↓ MembershipEvent::MemberSuspect
  Topology Watcher receives event
    ↓
  ClusterView.update_node_role(id, "suspect")
    ↓ epoch++
  Clients route away from suspect node
    ↓ MembershipEvent::MemberFailed (timeout)
  ClusterView.update_node_role(id, "failed")
    ↓
  Raft reconfiguration triggered
    ↓
  New leaders elected for affected shards
```

---

## SWIM Protocol

### How SWIM Works

SWIM uses a **gossip-based** approach for scalable failure detection:

**1. Ping Protocol**

Each node periodically (every 1 second):
1. Selects a random member to ping
2. Sends PING message
3. Waits for ACK (timeout: 500ms)

**2. Indirect Ping (Suspicion)**

If no ACK received:
1. Mark member as "suspect"
2. Select k random members (k=3)
3. Ask them to ping the suspect
4. If any ACK received → member alive
5. If no ACK from anyone → member failed

**3. Incarnation Numbers**

Each node has an **incarnation number** that increases on:
- Refuting a suspicion (node is alive)
- Recovering from a crash

Higher incarnation number wins in conflict resolution.

**4. Gossip Piggyback**

Membership updates piggyback on PING/ACK messages:
- MemberJoined events
- MemberLeft events
- MemberFailed events
- MemberAlive refutations

### Why SWIM?

**vs. Heartbeat-based (all-to-all pings):**
- SWIM: O(log N) messages per period
- Heartbeat: O(N²) messages per period
- SWIM scales to 100+ nodes efficiently

**vs. Consul (uses SWIM):**
- NoriKV uses nori-swim, a minimal SWIM implementation
- No external dependencies
- Integrated directly with Raft

**vs. Raft membership:**
- Raft: Strong consistency, requires quorum
- SWIM: Eventually consistent, faster detection
- Used together: SWIM detects, Raft reconfigures

---

## Topology Watcher Implementation

### Code Structure

**Location:** `apps/norikv-server/src/cluster_view.rs`

```rust
impl ClusterViewManager {
    /// Start topology watcher task.
    pub fn start_topology_watcher(
        self: Arc<Self>,
        swim: Arc<nori_swim::SwimMembership>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut events = swim.events();
            tracing::info!("Topology watcher started, listening for SWIM events");

            loop {
                match events.recv().await {
                    Ok(event) => {
                        if let Err(e) = self.handle_membership_event(event).await {
                            tracing::error!("Failed to handle membership event: {:?}", e);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!("Topology watcher lagged, skipped {} events", skipped);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::info!("SWIM event channel closed, topology watcher exiting");
                        break;
                    }
                }
            }
        })
    }
}
```

### Event Handling

**Five Event Types:**

```rust
pub enum MembershipEvent {
    MemberJoined { id: String, addr: SocketAddr },
    MemberSuspect { id: String, incarnation: u64 },
    MemberFailed { id: String },
    MemberLeft { id: String },
    MemberAlive { id: String, incarnation: u64 },
}
```

**Handler Logic:**

```rust
async fn handle_membership_event(
    &self,
    event: nori_swim::MembershipEvent,
) -> Result<(), ClusterViewError> {
    use nori_swim::MembershipEvent;

    match event {
        MembershipEvent::MemberJoined { id, addr } => {
            tracing::info!("Member joined: {} at {}", id, addr);
            self.add_node(id, addr.to_string()).await?;
        }
        MembershipEvent::MemberSuspect { id, incarnation } => {
            tracing::warn!("Member suspected: {} (incarnation {})", id, incarnation);
            self.update_node_role(&id, "suspect").await?;
        }
        MembershipEvent::MemberFailed { id } => {
            tracing::warn!("Member failed: {}", id);
            self.update_node_role(&id, "failed").await?;
        }
        MembershipEvent::MemberLeft { id } => {
            tracing::info!("Member left: {}", id);
            self.remove_node(&id).await?;
        }
        MembershipEvent::MemberAlive { id, incarnation } => {
            tracing::info!("Member alive: {} (incarnation {})", id, incarnation);
            self.update_node_role(&id, "follower").await?;
        }
    }

    // Trigger a refresh to update shard assignments
    self.refresh().await?;

    Ok(())
}
```

### Epoch Versioning

**ClusterView uses monotonic epoch numbers** for conflict-free updates:

```rust
pub struct ClusterView {
    /// Monotonically increasing version number
    pub epoch: u64,

    /// All nodes in the cluster
    pub nodes: Vec<ClusterNode>,

    /// Shard assignments and leadership
    pub shards: Vec<ShardInfo>,
}
```

**Update Pattern:**

```rust
async fn update_node_role(&self, node_id: &str, role: &str) -> Result<(), ClusterViewError> {
    let mut view = self.view.write();

    // Check if update is needed
    let needs_update = view.nodes.iter()
        .find(|n| n.id == node_id)
        .map(|n| n.role != role)
        .unwrap_or(false);

    if needs_update {
        // Increment epoch first
        view.epoch += 1;
        let new_epoch = view.epoch;

        // Then update the role
        if let Some(node) = view.nodes.iter_mut().find(|n| n.id == node_id) {
            node.role = role.to_string();
            tracing::info!("Updated node {} role to {} (epoch {})", node_id, role, new_epoch);
        }
    }

    Ok(())
}
```

**Why epochs?**
- Clients can detect stale views (older epoch)
- No need for vector clocks or version vectors
- Simple monotonic counter
- Works across network partitions (highest epoch wins)

---

## Design Decisions

### 1. Separate Watcher Task

**Decision:** Run topology watcher as a separate tokio task.

**Rationale:**
- **Non-blocking:** SWIM events don't block main server
- **Isolation:** Failures in watcher don't crash server
- **Clean shutdown:** Can abort task independently
- **Testable:** Can mock SWIM events

**Alternative:** Handle events in Node::start() loop
-  Blocks server startup
-  Harder to test
-  Mixed concerns

---

### 2. Broadcast Channel for Updates

**Decision:** Use `tokio::sync::broadcast` for ClusterView updates.

**Rationale:**
- **Multiple subscribers:** Meta.WatchCluster, internal monitoring
- **No missed updates:** New subscribers get current view
- **Backpressure:** Lagging subscribers skip old events
- **Efficient:** Lock-free, clone-on-read

**Code:**

```rust
pub struct ClusterViewManager {
    /// Current cluster view
    view: Arc<RwLock<ClusterView>>,

    /// Broadcast channel for cluster view updates
    update_tx: broadcast::Sender<ClusterView>,
}

impl ClusterViewManager {
    pub fn subscribe(&self) -> broadcast::Receiver<ClusterView> {
        self.update_tx.subscribe()
    }
}
```

**Alternative:** Polling current()
-  High CPU usage
-  Delayed updates
-  No event-driven routing

---

### 3. Refresh After Every Event

**Decision:** Call `refresh()` after handling each SWIM event.

**Rationale:**
- **Shard assignments:** Update which shards are on which nodes
- **Leader info:** Query Raft for current leaders
- **Consistency:** Single atomic view update

**Code:**

```rust
match event {
    MembershipEvent::MemberJoined { id, addr } => {
        self.add_node(id, addr.to_string()).await?;
    }
    // ... other events
}

// Trigger a refresh to update shard assignments
self.refresh().await?;
```

**Alternative:** Periodic refresh only
-  Delayed routing updates
-  Clients may route to dead nodes
-  Longer recovery time

---

### 4. Role-Based Status

**Decision:** Track node roles: `follower`, `suspect`, `failed`, `leader`.

**Rationale:**
- **Client routing:** Avoid suspected nodes
- **Debugging:** Understand cluster state
- **Gradual degradation:** Suspect → Failed transition

**Status Transitions:**

```
                    ┌─────────────┐
                    │   Unknown   │
                    └──────┬──────┘
                           │ MemberJoined
                           ▼
                    ┌─────────────┐
            ┌──────▶│  Follower   │◀──────┐
            │       └──────┬──────┘       │ MemberAlive
            │              │               │
            │              │ MemberSuspect │
            │              ▼               │
            │       ┌─────────────┐        │
            │       │   Suspect   │────────┘
            │       └──────┬──────┘
            │              │ timeout
            │              ▼
            │       ┌─────────────┐
            │       │   Failed    │
            │       └──────┬──────┘
            │              │ MemberLeft
            │              ▼
            │       ┌─────────────┐
            └───────│   Removed   │
                    └─────────────┘
```

---

### 5. Borrow Checker Workaround

**Problem:** Can't mutate `view.epoch` while holding mutable iterator to `view.nodes`.

**Code (broken):**

```rust
// This doesn't compile!
if let Some(node) = view.nodes.iter_mut().find(|n| n.id == node_id) {
    if node.role != role {
        view.epoch += 1;  //  Error: can't borrow view again
        node.role = role.to_string();
    }
}
```

**Solution:** Check if update is needed first, then increment epoch before calling iter_mut():

```rust
// Check if update is needed
let needs_update = view.nodes.iter()
    .find(|n| n.id == node_id)
    .map(|n| n.role != role)
    .unwrap_or(false);

if needs_update {
    // Increment epoch first
    view.epoch += 1;
    let new_epoch = view.epoch;

    // Then update the role (no conflict)
    if let Some(node) = view.nodes.iter_mut().find(|n| n.id == node_id) {
        node.role = role.to_string();
        tracing::info!("Updated node {} role to {} (epoch {})", node_id, role, new_epoch);
    }
}
```

**Lesson:** Separate read and write phases when working with complex data structures.

---

## Performance Characteristics

### SWIM Overhead

**Message complexity:**
- **Per-node:** O(log N) messages per gossip interval
- **Cluster-wide:** O(N log N) total messages per interval
- **Bandwidth:** ~1KB per message × 10 messages/sec = 10KB/sec per node

**Detection time:**
- **Suspect:** 500ms (ping timeout)
- **Failed:** 5 seconds (indirect ping + suspicion timeout)
- **Propagation:** O(log N) gossip rounds = ~5 seconds for 100 nodes

### Topology Watcher Overhead

**CPU:**
- Event handling: <100µs per event
- ClusterView update: <500µs (RwLock + epoch increment)
- Refresh: ~5-10ms (queries all active shards)

**Memory:**
- ClusterView: ~1KB per node (100 nodes = 100KB)
- Broadcast channel: 16-slot ring buffer (~16KB)

**Event rate:**
- Steady state: 0-1 events/sec (gossip updates)
- Failure scenario: 5-10 events/sec (suspect → failed)
- Cluster expansion: 100 events (all nodes join)

---

## Failure Scenarios

### Node Crash

**Timeline:**

```
t=0:     Node B crashes
t=500ms: Node A pings B, no ACK → suspects B
t=1s:    Node A gossips "B suspected"
t=2s:    Cluster converges on "B suspected"
t=5s:    Suspicion timeout → B marked failed
t=5.5s:  ClusterView updated, epoch++
t=6s:    Clients receive update, reroute
t=7s:    Raft reconfiguration for B's shards
```

**Recovery:**

Node B restarts → increments incarnation → gossips "B alive" → role updated to "follower"

---

### Network Partition

**Scenario:** Cluster splits into [A, B] and [C, D, E]

**Minority partition [A, B]:**
- A and B suspect C, D, E
- Update ClusterView (epoch++)
- **Cannot commit writes** (no Raft quorum)
- Clients see "not_leader" errors

**Majority partition [C, D, E]:**
- C, D, E suspect A, B
- Update ClusterView (epoch++)
- **Continue operating** (Raft quorum)
- New leaders elected for A/B's shards

**Partition heals:**
- A and B receive higher epoch view from C/D/E
- A and B adopt majority view
- A and B catch up via Raft log replication

**Winner:** Highest epoch view wins (majority partition)

---

### Split-Brain Prevention

**SWIM alone doesn't prevent split-brain** - both partitions can accept writes.

**NoriKV uses Raft for consistency:**
- Writes require Raft quorum (majority)
- Minority partition **cannot commit writes**
- Clients automatically retry on majority partition

**Combined approach:**
- SWIM: Fast failure detection (5 seconds)
- Raft: Strong consistency (quorum required)
- ClusterView: Client routing (avoid failed nodes)

---

## Client Integration

### Meta.WatchCluster Service

**gRPC streaming API:**

```protobuf
service Meta {
  rpc WatchCluster(WatchClusterRequest) returns (stream ClusterView);
}

message ClusterView {
  uint64 epoch = 1;
  repeated ClusterNode nodes = 2;
  repeated ShardInfo shards = 3;
}

message ClusterNode {
  string id = 1;
  string addr = 2;
  string role = 3;  // "leader", "follower", "suspect", "failed"
}
```

**Client usage:**

```rust
let mut stream = client.watch_cluster().await?;

while let Some(view) = stream.next().await {
    let view = view?;

    // Update local routing table
    routing_table.update(view.epoch, view.nodes, view.shards);

    tracing::info!("Cluster view updated to epoch {}", view.epoch);
}
```

### Smart Routing

**Client routing logic:**

```rust
fn route_key(&self, key: &[u8]) -> Result<String, Error> {
    // Hash key to shard
    let shard_id = self.router.shard_for_key(key);

    // Find leader for shard
    let shard = self.routing_table.get_shard(shard_id)?;
    let leader = shard.replicas.iter()
        .find(|r| r.is_leader && r.role != "suspect" && r.role != "failed")
        .ok_or(Error::NoLeader)?;

    Ok(leader.addr.clone())
}
```

**Benefits:**
- Direct routing to leader (no redirects)
- Avoid failed/suspected nodes
- Automatic failover (new view pushed)

---

## Monitoring

### Key Metrics

```promql
# Cluster size
swim_cluster_size

# Failure rate
rate(swim_failed_members_total[5m])

# Topology updates per minute
rate(cluster_view_epoch[1m]) * 60

# Nodes in suspect state
count(cluster_nodes{role="suspect"})
```

### Logging

**Structured logs with context:**

```rust
tracing::info!(
    node_id = %id,
    addr = %addr,
    incarnation = incarnation,
    "Member joined cluster"
);

tracing::warn!(
    node_id = %id,
    incarnation = incarnation,
    "Member suspected, starting indirect ping"
);
```

**Log aggregation query (JSON logs):**

```bash
# Count suspect events per hour
jq 'select(.fields.event == "MemberSuspect") | .fields.node_id' \
  | sort | uniq -c

# Detect flapping nodes (frequent suspect/alive cycles)
jq 'select(.fields.event == "MemberAlive" or .fields.event == "MemberSuspect") | .fields.node_id' \
  | uniq -c | sort -rn
```

---

## Testing

### Unit Tests

**Test event handling:**

```rust
#[tokio::test]
async fn test_member_joined() {
    let cluster_view = ClusterViewManager::new(...);

    cluster_view.handle_membership_event(
        MembershipEvent::MemberJoined {
            id: "node1".to_string(),
            addr: "10.0.1.1:6000".parse().unwrap(),
        }
    ).await.unwrap();

    let view = cluster_view.current();
    assert_eq!(view.nodes.len(), 2); // includes self
    assert_eq!(view.epoch, 1);
}
```

### Integration Tests

**Test failure detection:**

```rust
#[tokio::test]
async fn test_node_failure_detection() {
    // Start 3-node cluster
    let nodes = start_cluster(3).await;

    // Kill node 2
    nodes[2].shutdown().await;

    // Wait for SWIM to detect failure
    tokio::time::sleep(Duration::from_secs(10)).await;

    // Check cluster view
    let view = nodes[0].cluster_view().current();
    assert!(view.nodes.iter().any(|n| n.id == "node2" && n.role == "failed"));
}
```

---

## Next Steps

- **[Multi-Shard Architecture](multi-shard-server.md)** - How shards are managed
- **[SWIM Protocol (nori-swim)](../crates/nori-swim/index.md)** - Deep dive into SWIM implementation
- **[Client Routing](../sdks/index.md)**  - SDK integration with ClusterView
