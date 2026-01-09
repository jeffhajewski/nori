# nori-swim

SWIM-like membership and failure detection protocol for cluster management.

## Features

- **Membership Tracking**: Track cluster members with state (Alive, Suspect, Failed, Left)
- **Failure Detection**: Periodic probing with direct and indirect pings
- **Suspicion Mechanism**: Configurable suspicion timeouts before declaring failures
- **Gossip Dissemination**: Efficient membership change propagation
- **Incarnation Numbers**: Conflict resolution for concurrent state changes
- **Chaos Testing**: Built-in chaos injection for testing resilience
- **Multiple Transports**: UDP, in-memory, and chaos-wrapped transports

## Quick Start

```rust
use nori_swim::{SwimNode, SwimConfig, SwimTransport, UdpTransport};
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = SwimConfig::default();
    let addr: SocketAddr = "127.0.0.1:8000".parse()?;

    // Create UDP transport
    let transport = UdpTransport::bind(addr).await?;

    // Create SWIM node
    let node = SwimNode::new("node1".to_string(), addr, config, transport);

    // Start the protocol (probing, gossip)
    node.start().await?;

    // Join cluster via seed node
    let seed: SocketAddr = "127.0.0.1:8001".parse()?;
    node.join(seed).await?;

    // Get current cluster view
    let view = node.view();
    println!("Cluster has {} members", view.members.len());

    // Subscribe to membership events
    let mut events = node.events();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            println!("Membership event: {:?}", event);
        }
    });

    Ok(())
}
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Membership Trait                                           │
│  - view() -> ClusterView                                    │
│  - events() -> Receiver<MembershipEvent>                    │
│  - join(seed) / leave()                                     │
└────────────────────────┬────────────────────────────────────┘
                         │
┌────────────────────────▼────────────────────────────────────┐
│  SwimNode                                                   │
│  ┌─────────────────┐  ┌─────────────────┐                  │
│  │  ProbeManager   │  │   MemberList    │                  │
│  │  - Direct ping  │  │  - State track  │                  │
│  │  - Indirect req │  │  - Incarnation  │                  │
│  └─────────────────┘  └─────────────────┘                  │
│  ┌─────────────────┐  ┌─────────────────┐                  │
│  │   Suspicion     │  │     Timer       │                  │
│  │  - Timeout mgmt │  │  - Periodic     │                  │
│  │  - Confirmation │  │  - Probe cycle  │                  │
│  └─────────────────┘  └─────────────────┘                  │
└────────────────────────┬────────────────────────────────────┘
                         │
┌────────────────────────▼────────────────────────────────────┐
│  Transport Layer                                            │
│  - UdpTransport (production)                                │
│  - InMemoryTransport (testing)                              │
│  - ChaosTransport (fault injection)                         │
└─────────────────────────────────────────────────────────────┘
```

## Modules

| Module | Description |
|--------|-------------|
| `config` | Protocol configuration (timeouts, fanout, intervals) |
| `node` | Main SwimNode implementation |
| `member_list` | Cluster member state tracking |
| `message` | Wire protocol messages (Ping, Ack, PingReq) |
| `probe` | Direct and indirect probe management |
| `suspicion` | Suspicion timeout and confirmation handling |
| `timer` | Periodic timers for probe cycles |
| `transport` | Transport abstraction (UDP, in-memory, chaos) |
| `chaos` | Chaos injection for testing (delays, drops, partitions) |

## Configuration

```rust
use nori_swim::SwimConfig;
use std::time::Duration;

let config = SwimConfig {
    // Probe settings
    probe_interval: Duration::from_millis(500),
    probe_timeout: Duration::from_millis(200),
    indirect_probes: 3,  // Number of indirect probe requests

    // Suspicion settings
    suspicion_mult: 4,   // Multiplier for suspicion timeout
    suspicion_max_timeout: Duration::from_secs(10),

    // Gossip settings
    gossip_fanout: 3,    // Number of nodes to gossip to
    gossip_interval: Duration::from_millis(200),

    // Retransmit settings
    retransmit_mult: 4,  // Retransmit multiplier for reliable delivery

    ..Default::default()
};
```

## Member States

```
    ┌─────────┐
    │  Alive  │◄───────────────────────────┐
    └────┬────┘                            │
         │ probe timeout                   │ refute (higher incarnation)
         ▼                                 │
    ┌─────────┐                            │
    │ Suspect │────────────────────────────┘
    └────┬────┘
         │ suspicion timeout
         ▼
    ┌─────────┐
    │ Failed  │
    └─────────┘

    ┌─────────┐
    │  Left   │  (graceful departure)
    └─────────┘
```

## Membership Events

```rust
use nori_swim::MembershipEvent;

// Subscribe to events
let mut rx = swim.events();
while let Ok(event) = rx.recv().await {
    match event {
        MembershipEvent::MemberJoined { id, addr } => {
            println!("Node {} joined at {}", id, addr);
        }
        MembershipEvent::MemberSuspect { id, incarnation } => {
            println!("Node {} suspected (incarnation {})", id, incarnation);
        }
        MembershipEvent::MemberFailed { id } => {
            println!("Node {} confirmed failed", id);
        }
        MembershipEvent::MemberLeft { id } => {
            println!("Node {} left gracefully", id);
        }
        MembershipEvent::MemberAlive { id, incarnation } => {
            println!("Node {} refuted suspicion (incarnation {})", id, incarnation);
        }
    }
}
```

## Chaos Testing

Built-in chaos injection for testing protocol resilience:

```rust
use nori_swim::{ChaosTransport, ChaosConfig, InMemoryTransport};

let base_transport = InMemoryTransport::new();
let chaos_config = ChaosConfig {
    drop_rate: 0.1,           // 10% message drop
    delay_ms: (10, 50),       // 10-50ms random delay
    partition_nodes: vec![],  // Network partition simulation
};

let transport = ChaosTransport::wrap(base_transport, chaos_config);
```

## Observability

SWIM emits `VizEvent::Swim` events for dashboard visualization:

- `Alive { node }` - Node confirmed alive
- `Suspect { node }` - Node suspected of failure
- `Confirm { node }` - Node confirmed failed
- `Leave { node }` - Node left gracefully

## Testing

```bash
# Run all tests
cargo test -p nori-swim

# Run with logging
RUST_LOG=nori_swim=debug cargo test -p nori-swim
```

## Design References

Based on the [SWIM paper](https://www.cs.cornell.edu/projects/Quicksilver/public_pdfs/SWIM.pdf):
"Scalable Weakly-consistent Infection-style Process Group Membership Protocol"
(Das, Gupta, Maniatis, 2002)

Extensions:
- Suspicion subprotocol for reduced false positives
- Incarnation numbers for conflict resolution
- Gossip piggybacking for efficient dissemination

See `context/32_membership.yaml` for complete specification.

## License

MIT
