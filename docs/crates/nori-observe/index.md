# nori-observe

Vendor-neutral observability ABI with near-zero overhead for metrics and typed events.

---

## What is nori-observe?

**nori-observe** is a lightweight observability abstraction layer that provides:

- **Vendor-neutral metrics**: Counter, gauge, histogram primitives
- **Typed events**: Structured `VizEvent` enums for dashboard streaming
- **Zero-allocation hot paths**: < 80ns counters, < 200ns histograms
- **Pluggable backends**: Prometheus, OTLP, or custom exporters

### Design Philosophy

Core crates (nori-wal, nori-lsm, nori-raft) never import vendor-specific telemetry. Instead, they depend on the `Meter` trait from nori-observe, which is injected at construction time.

---

## Quick Example

```rust
use nori_observe::{Meter, VizEvent, CounterHandle, HistoHandle};

// Accept a Meter via dependency injection
struct MyComponent<M: Meter> {
    meter: M,
    ops_counter: CounterHandle,
    latency_histo: HistoHandle,
}

impl<M: Meter> MyComponent<M> {
    fn new(meter: M) -> Self {
        let ops_counter = meter.counter(
            "my_ops_total",
            &[("component", "my_component")],
        );
        let latency_histo = meter.histo(
            "my_latency_ms",
            &[1.0, 5.0, 10.0, 50.0, 100.0, 500.0],
            &[("component", "my_component")],
        );

        Self { meter, ops_counter, latency_histo }
    }

    fn do_work(&self) {
        let start = std::time::Instant::now();

        // ... perform work ...

        self.ops_counter.inc();
        self.latency_histo.record(start.elapsed().as_millis() as f64);

        // Emit a typed event for dashboard
        self.meter.emit(VizEvent::Custom("work_completed".into()));
    }
}
```

---

## Core Trait

The `Meter` trait defines the observability interface:

```rust
pub trait Meter: Send + Sync + 'static {
    /// Create or retrieve a counter
    fn counter(
        &self,
        name: &'static str,
        labels: &'static [(&'static str, &'static str)],
    ) -> CounterHandle;

    /// Create or retrieve a gauge
    fn gauge(
        &self,
        name: &'static str,
        labels: &'static [(&'static str, &'static str)],
    ) -> GaugeHandle;

    /// Create or retrieve a histogram
    fn histo(
        &self,
        name: &'static str,
        buckets: &'static [f64],
        labels: &'static [(&'static str, &'static str)],
    ) -> HistoHandle;

    /// Emit a typed visualization event
    fn emit(&self, evt: VizEvent);
}
```

---

## Metric Types

### Counter

Monotonically increasing value (e.g., requests, errors):

```rust
let counter = meter.counter("http_requests_total", &[("method", "GET")]);
counter.inc();      // +1
counter.add(5);     // +5
```

### Gauge

Point-in-time value (e.g., temperature, queue depth):

```rust
let gauge = meter.gauge("queue_depth", &[("queue", "jobs")]);
gauge.set(42.0);
gauge.inc();        // +1
gauge.dec();        // -1
```

### Histogram

Distribution of values (e.g., latencies):

```rust
let histo = meter.histo(
    "request_latency_ms",
    &[1.0, 5.0, 10.0, 50.0, 100.0, 500.0, 1000.0],
    &[("endpoint", "/api/v1")],
);
histo.record(23.5);
```

---

## VizEvent Schema

Typed events for dashboard visualization:

```rust
pub enum VizEvent {
    // WAL events
    Wal(WalEvent),

    // Compaction events
    Compaction(CompactionEvent),

    // Raft consensus events
    Raft(RaftEvent),

    // SWIM membership events
    Swim(SwimEvent),

    // Shard lifecycle events
    Shard(ShardEvent),

    // Cache statistics
    Cache(CacheEvent),

    // Custom events
    Custom(String),
}

pub enum WalEvent {
    SegmentRoll { bytes: u64 },
    Fsync { ms: u64 },
    CorruptionTruncated,
}

pub enum CompactionEvent {
    Scheduled,
    Start,
    Progress { pct: u8 },
    Finish { in_bytes: u64, out_bytes: u64 },
}

pub enum RaftEvent {
    VoteReq { from: NodeId },
    VoteGranted { from: NodeId },
    LeaderElected { node: NodeId },
    StepDown,
}

pub enum SwimEvent {
    Alive { addr: SocketAddr },
    Suspect { addr: SocketAddr },
    Confirm { addr: SocketAddr },
    Leave { addr: SocketAddr },
}

pub enum ShardEvent {
    Plan,
    SnapshotStart,
    SnapshotDone,
    Cutover,
}

pub enum CacheEvent {
    HitRatio { ratio: f64 },
}
```

---

## Label Cardinality Policy

To prevent metric explosion, nori-observe enforces label policies:

### Allowed Labels
- `node_id` - Node identifier
- `shard_id` - Shard identifier
- `role` - Node role (leader/follower)
- `level` - LSM level
- `outcome` - Operation result (success/error)
- `op` - Operation type (get/put/delete)

### Disallowed Labels
- `key` - Individual keys (unbounded cardinality)
- `client_id` - Individual clients
- `ip` - Client IP addresses

---

## Performance Budgets

| Operation | Budget | Allocation |
|-----------|--------|------------|
| Counter increment | ≤ 80ns | 0 |
| Histogram record | ≤ 200ns | 0 |

All hot-path operations are zero-allocation.

---

## Backend Adapters

### nori-observe-prom (Prometheus)

```rust
use nori_observe_prom::PrometheusMeter;

let meter = PrometheusMeter::new();
// Exposes /metrics endpoint in OpenMetrics format
```

### nori-observe-otlp (OpenTelemetry)

```rust
use nori_observe_otlp::OtlpMeter;

let meter = OtlpMeter::new("http://collector:4317")?;
// Exports via OTLP gRPC with optional trace exemplars
```

### norikv-vizd (Dashboard)

```rust
use norikv_vizd::VizMeter;

let meter = VizMeter::new();
// Streams VizEvents to WebSocket/gRPC-web for live dashboard
```

### NoopMeter (Testing)

```rust
use nori_observe::NoopMeter;

let meter = NoopMeter;
// Zero-cost no-op for testing
```

---

## Usage Pattern

Inject the meter at component construction:

```rust
// In library code (nori-lsm, nori-raft, etc.)
pub struct LsmEngine<M: Meter> {
    meter: M,
    // ...
}

impl<M: Meter> LsmEngine<M> {
    pub fn new(config: Config, meter: M) -> Self {
        // Use meter for observability
    }
}

// In application code
let meter = PrometheusMeter::new();
let engine = LsmEngine::new(config, meter);
```

---

## Installation

```toml
[dependencies]
nori-observe = "0.1"

# Optional backends
nori-observe-prom = "0.1"  # Prometheus
nori-observe-otlp = "0.1"  # OpenTelemetry
```

---

## See Also

- [Architecture: Observability](../../architecture/index.md) - Observability in NoriKV
- [Dashboard](../../architecture/index.md) - Live visualization
- [nori-raft](../nori-raft/index.md) - Consensus metrics
- [nori-swim](../nori-swim/index.md) - Membership metrics
