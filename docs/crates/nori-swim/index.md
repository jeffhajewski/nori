# nori-swim

SWIM-based membership protocol with failure detection and cluster view management.

---

## What is nori-swim?

**nori-swim** is a production-ready implementation of the SWIM (Scalable Weakly-consistent Infection-style Process Group Membership) protocol. It provides efficient failure detection and membership dissemination for distributed systems.

### Key Features

- **Efficient Failure Detection**: O(1) probing with indirect probes
- **Epidemic Dissemination**: Piggybacked membership updates
- **Configurable Suspicion**: Tunable false-positive rates
- **Transport Agnostic**: Works with UDP, TCP, QUIC, or in-memory
- **Event Streaming**: Subscribe to membership changes
- **Simulation Support**: Memory transport for deterministic testing

---

## Quick Example

```rust
use nori_swim::{SwimNode, SwimConfig};
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = SwimConfig {
        probe_interval_ms: 1000,
        suspicion_timeout_ms: 5000,
        dissemination_fanout: 3,
        ..Default::default()
    };

    let node = SwimNode::new(config).await?;

    // Join an existing cluster
    let seed: SocketAddr = "192.168.1.10:7946".parse()?;
    node.join(seed).await?;

    // Get current cluster view
    let view = node.view();
    for member in view.members() {
        println!("Member: {} (state: {:?})", member.addr, member.state);
    }

    // Subscribe to membership events
    let mut events = node.events();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            match event {
                MembershipEvent::Alive(addr) => println!("Node joined: {}", addr),
                MembershipEvent::Suspect(addr) => println!("Node suspect: {}", addr),
                MembershipEvent::Confirm(addr) => println!("Node failed: {}", addr),
                MembershipEvent::Leave(addr) => println!("Node left: {}", addr),
            }
        }
    });

    // Graceful leave
    node.leave().await?;

    Ok(())
}
```

---

## Core Trait

The `Membership` trait defines the interface for cluster membership:

```rust
#[async_trait::async_trait]
pub trait Membership: Send + Sync {
    /// Get the current cluster view
    fn view(&self) -> ClusterView;

    /// Subscribe to membership events
    fn events(&self) -> broadcast::Receiver<MembershipEvent>;

    /// Join an existing cluster via seed node
    async fn join(&self, seed: SocketAddr) -> Result<(), Error>;

    /// Gracefully leave the cluster
    async fn leave(&self) -> Result<(), Error>;
}
```

---

## SWIM Protocol Overview

### Failure Detection

```
1. Every `probe_interval_ms`, probe a random member (PING)
2. If no ACK within timeout, send indirect probes via K random members
3. If still no ACK, mark as SUSPECT
4. After `suspicion_timeout_ms`, mark as CONFIRMED (failed)
```

### Dissemination

Membership updates piggyback on protocol messages:
- PING/ACK carry membership deltas
- Updates spread epidemically in O(log N) rounds
- Lamport timestamps prevent stale updates

---

## Configuration

| Parameter | Default | Description |
|-----------|---------|-------------|
| `probe_interval_ms` | 1000ms | Time between probe rounds |
| `suspicion_timeout_ms` | 5000ms | Time before suspect → confirmed |
| `dissemination_fanout` | 3 | Number of indirect probe targets |

### Tuning for Your Environment

**High-reliability (fewer false positives):**
```rust
SwimConfig {
    probe_interval_ms: 500,
    suspicion_timeout_ms: 10000,  // Longer suspicion window
    dissemination_fanout: 5,
    ..Default::default()
}
```

**Fast detection (more aggressive):**
```rust
SwimConfig {
    probe_interval_ms: 200,
    suspicion_timeout_ms: 2000,
    dissemination_fanout: 3,
    ..Default::default()
}
```

---

## Member States

| State | Description | Transition |
|-------|-------------|------------|
| **Alive** | Member is responsive | Initial state, or recovered from suspect |
| **Suspect** | No response to probes | After probe timeout |
| **Confirmed** | Member has failed | After suspicion timeout |
| **Left** | Graceful departure | Explicit leave message |

```
        probe_timeout           suspicion_timeout
ALIVE ───────────────► SUSPECT ────────────────► CONFIRMED
  ▲                       │
  └───────────────────────┘
        successful probe
```

---

## Transport Adapters

nori-swim is transport-agnostic:

```rust
// UDP (default, best for LAN)
let transport = UdpTransport::new(bind_addr).await?;

// TCP (reliable, for WAN)
let transport = TcpTransport::new(bind_addr).await?;

// Memory (for testing)
let transport = MemTransport::new();

let node = SwimNode::with_transport(config, transport).await?;
```

---

## Observability

### Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `swim_probes_total` | Counter | Total probes sent |
| `swim_rtt_ms` | Histogram | Probe round-trip time |
| `membership_flaps_total` | Counter | Member state changes |

### Events

```rust
VizEvent::Swim(SwimEvent::Alive { addr })
VizEvent::Swim(SwimEvent::Suspect { addr })
VizEvent::Swim(SwimEvent::Confirm { addr })
VizEvent::Swim(SwimEvent::Leave { addr })
```

---

## Testing

nori-swim includes comprehensive testing:

- **Eventual Convergence**: Verified with loss, duplication, and reordering
- **False-Positive Budget**: Configurable and tested
- **Deterministic Simulation**: Reproducible scenarios via nori-testkit

---

## Installation

```toml
[dependencies]
nori-swim = "0.1"
```

---

## See Also

- [Architecture: Membership](../../architecture/swim-topology.md) - How SWIM fits in NoriKV
- [nori-raft](../nori-raft/index.md) - Consensus for replicated state
- [nori-observe](../nori-observe/index.md) - Observability integration
