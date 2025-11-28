# Metrics Reference

Complete reference for all Prometheus metrics exposed by NoriKV.

---

---

## Overview

NoriKV exposes metrics in **Prometheus text format** via the `/metrics` HTTP endpoint. Metrics are collected using the **prometheus-client** crate and implement the **nori-observe::Meter** trait for vendor-neutral instrumentation.

**Metrics Categories:**
- **KV Metrics** - Client request counters and latencies
- **LSM Metrics** - Storage engine performance
- **Raft Metrics** - Consensus and replication
- **SWIM Metrics** - Membership and failure detection
- **System Metrics** - Resource usage and health

---

## Metrics Architecture

### Design Decisions

**1. Vendor-Neutral Instrumentation**

All components emit metrics via the `nori_observe::Meter` trait:

```rust
pub trait Meter: Send + Sync {
    fn counter(&self, name: &'static str, labels: &'static [(&'static str, &'static str)])
        -> Box<dyn Counter>;
    fn gauge(&self, name: &'static str, labels: &'static [(&'static str, &'static str)])
        -> Box<dyn Gauge>;
    fn histo(&self, name: &'static str, buckets: &'static [f64], labels: &'static [(&'static str, &'static str)])
        -> Box<dyn Histogram>;
}
```

**Benefits:**
- Core crates (nori-lsm, nori-raft, nori-swim) have **zero dependencies** on Prometheus
- Can swap backends (OTLP, StatsD, custom) without changing core code
- Enables testing with mock meters

**2. Prometheus Implementation**

The server implements `Meter` using **prometheus-client**:

```rust
// apps/norikv-server/src/metrics.rs
pub struct PrometheusMeter {
    registry: Arc<Mutex<Registry>>,
    counters: Arc<Mutex<HashMap<String, Family<Vec<(String, String)>, PromCounter>>>>,
    gauges: Arc<Mutex<HashMap<String, Family<Vec<(String, String)>, PromGauge>>>>,
    histograms: Arc<Mutex<HashMap<String, Family<Vec<(String, String)>, PromHistogram>>>>,
}
```

**Key Features:**
- **Thread-safe** with `Arc<Mutex<...>>` for concurrent updates
- **Metric families** for automatic label handling
- **Exponential histogram buckets** (1.0, 2.0, 4.0, ..., 512.0) for latency
- **Registry** for text format export

**3. Wiring**

Metrics are wired at server startup:

```rust
// Create meter
let meter = Arc::new(PrometheusMeter::new());

// Wire to gRPC server
let grpc_server = GrpcServer::with_backend(addr, backend)
    .with_meter(meter.clone() as Arc<dyn Meter>);

// Wire to HTTP server
let http_server = HttpServer::new(addr, health_checker, meter.clone());
```

**4. Histogram Buckets**

Exponential buckets optimized for latency distribution:

```rust
exponential_buckets(1.0, 2.0, 10)
// → [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0, 512.0]
```

**Rationale:**
- Covers range from 1ms (fast cache hit) to 512ms (slow compaction/network)
- Good precision for typical latencies (1-20ms)
- Standard Prometheus practice for latency metrics

---

## KV Metrics

### kv_requests_total

**Type:** Counter

**Description:** Total number of KV requests by operation type.

**Labels:**
- `operation` - Request type: `put`, `get`, `delete`, `scan`

**Example:**

```
kv_requests_total{operation="put"} 12345
kv_requests_total{operation="get"} 67890
kv_requests_total{operation="delete"} 123
kv_requests_total{operation="scan"} 456
```

**Use Cases:**
- Calculate request rate: `rate(kv_requests_total[5m])`
- Track operation mix: `sum by (operation) (rate(kv_requests_total[1m]))`
- Alert on zero traffic: `rate(kv_requests_total[5m]) == 0`

**PromQL Examples:**

```promql
# Total requests per second (all operations)
sum(rate(kv_requests_total[5m]))

# Requests per second by operation
rate(kv_requests_total{operation="put"}[5m])

# Operation distribution (%)
100 * rate(kv_requests_total[5m])
  / ignoring(operation) group_left
  sum(rate(kv_requests_total[5m]))
```

---

### kv_request_duration_ms

**Type:** Histogram

**Description:** Request latency in milliseconds, bucketed by operation, status, and result.

**Labels:**
- `operation` - Request type: `put`, `get`, `delete`, `scan`
- `status` - Request outcome: `success`, `not_leader`, `error`
- `result` - For GET: `found`, `not_found` (only for successful GETs)

**Buckets:** `[1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0, 512.0]` (ms)

**Example:**

```
kv_request_duration_ms_bucket{operation="put",status="success",le="1.0"} 5000
kv_request_duration_ms_bucket{operation="put",status="success",le="2.0"} 8000
kv_request_duration_ms_bucket{operation="put",status="success",le="4.0"} 10000
kv_request_duration_ms_bucket{operation="put",status="success",le="8.0"} 11500
kv_request_duration_ms_bucket{operation="put",status="success",le="16.0"} 12000
kv_request_duration_ms_bucket{operation="put",status="success",le="+Inf"} 12345
kv_request_duration_ms_sum{operation="put",status="success"} 123450.5
kv_request_duration_ms_count{operation="put",status="success"} 12345
```

**Use Cases:**
- Calculate p95 latency: `histogram_quantile(0.95, rate(kv_request_duration_ms_bucket[5m]))`
- Alert on high latency: `histogram_quantile(0.95, ...) > 50`
- Track cache hit latency: filter by `result="found"`

**PromQL Examples:**

```promql
# P95 PUT latency
histogram_quantile(0.95,
  rate(kv_request_duration_ms_bucket{operation="put"}[5m])
)

# P99 GET latency (found vs not found)
histogram_quantile(0.99,
  rate(kv_request_duration_ms_bucket{operation="get",result="found"}[5m])
)

# Average latency across all operations
rate(kv_request_duration_ms_sum[5m])
  / rate(kv_request_duration_ms_count[5m])

# Slow requests (>100ms)
sum(rate(kv_request_duration_ms_bucket{le="128.0"}[5m]))
  - sum(rate(kv_request_duration_ms_bucket{le="64.0"}[5m]))
```

**Grafana Dashboard:**

```json
{
  "targets": [
    {
      "expr": "histogram_quantile(0.50, rate(kv_request_duration_ms_bucket{operation=\"put\"}[5m]))",
      "legendFormat": "p50"
    },
    {
      "expr": "histogram_quantile(0.95, rate(kv_request_duration_ms_bucket{operation=\"put\"}[5m]))",
      "legendFormat": "p95"
    },
    {
      "expr": "histogram_quantile(0.99, rate(kv_request_duration_ms_bucket{operation=\"put\"}[5m]))",
      "legendFormat": "p99"
    }
  ]
}
```

---

## LSM Metrics

### lsm_compaction_duration_ms

**Type:** Histogram

**Description:** Time spent in compaction, in milliseconds.

**Labels:**
- `level` - Source level: `L0`, `L1`, `L2`, etc.
- `result` - Compaction outcome: `success`, `error`

**Buckets:** Exponential from 1ms to 10 seconds

**Use Cases:**
- Detect compaction storms: `rate(lsm_compaction_duration_ms_count[1m]) > 10`
- Alert on slow compaction: `histogram_quantile(0.95, ...) > 5000`
- Track compaction efficiency by level

**PromQL Examples:**

```promql
# Compactions per minute
rate(lsm_compaction_duration_ms_count[1m]) * 60

# Average compaction time
rate(lsm_compaction_duration_ms_sum[5m])
  / rate(lsm_compaction_duration_ms_count[5m])

# P99 L0 compaction time
histogram_quantile(0.99,
  rate(lsm_compaction_duration_ms_bucket{level="L0"}[5m])
)
```

---

### lsm_memtable_size_bytes

**Type:** Gauge

**Description:** Current memtable size in bytes.

**Labels:**
- `shard_id` - Shard ID

**Use Cases:**
- Monitor memory usage
- Detect memtable bloat
- Tune flush threshold

**PromQL Examples:**

```promql
# Total memtable memory across all shards
sum(lsm_memtable_size_bytes)

# Largest memtable
max(lsm_memtable_size_bytes)

# Alert on memtable >64MB (should flush at 4MB)
lsm_memtable_size_bytes > 67108864
```

---

### lsm_sstable_count

**Type:** Gauge

**Description:** Number of SSTables per level.

**Labels:**
- `level` - Level: `L0`, `L1`, `L2`, etc.
- `shard_id` - Shard ID

**Use Cases:**
- Monitor compaction health
- Detect L0 storms (too many L0 SSTables)
- Capacity planning

**PromQL Examples:**

```promql
# L0 SSTable count
sum(lsm_sstable_count{level="L0"})

# Alert on L0 storm (>10 files triggers compaction throttling)
lsm_sstable_count{level="L0"} > 10

# Total SSTables across all levels
sum(lsm_sstable_count)
```

---

## Raft Metrics

### raft_leader_changes_total

**Type:** Counter

**Description:** Total number of leader changes (elections).

**Labels:**
- `shard_id` - Shard ID

**Use Cases:**
- Detect cluster instability
- Alert on frequent elections
- Track split-brain scenarios

**PromQL Examples:**

```promql
# Leader changes per hour
rate(raft_leader_changes_total[1h]) * 3600

# Alert on frequent elections (>2 per hour)
rate(raft_leader_changes_total[1h]) * 3600 > 2
```

---

### raft_commit_index

**Type:** Gauge

**Description:** Current committed log index.

**Labels:**
- `shard_id` - Shard ID
- `node_id` - Node ID

**Use Cases:**
- Monitor replication lag
- Detect stalled followers
- Track write throughput

**PromQL Examples:**

```promql
# Replication lag (leader vs follower)
max(raft_commit_index{shard_id="0"})
  - raft_commit_index{shard_id="0"}

# Commit rate (writes per second)
rate(raft_commit_index[1m])
```

---

### raft_term

**Type:** Gauge

**Description:** Current Raft term.

**Labels:**
- `shard_id` - Shard ID

**Use Cases:**
- Track election history
- Detect term drift

**PromQL Examples:**

```promql
# Current term
max(raft_term)

# Term changes (indicates elections)
delta(raft_term[1h])
```

---

## SWIM Metrics

### swim_cluster_size

**Type:** Gauge

**Description:** Number of alive members in the cluster.

**Labels:** None

**Use Cases:**
- Monitor cluster membership
- Alert on node failures
- Capacity tracking

**PromQL Examples:**

```promql
# Current cluster size
swim_cluster_size

# Alert on cluster size drop
swim_cluster_size < 3
```

---

### swim_failed_members_total

**Type:** Counter

**Description:** Total number of members marked as failed.

**Labels:** None

**Use Cases:**
- Track failure rate
- Alert on frequent failures
- SLA reporting

**PromQL Examples:**

```promql
# Failures per hour
rate(swim_failed_members_total[1h]) * 3600

# Alert on >1 failure per hour
rate(swim_failed_members_total[1h]) * 3600 > 1
```

---

### swim_gossip_latency_ms

**Type:** Histogram

**Description:** Gossip message round-trip time in milliseconds.

**Labels:** None

**Buckets:** Exponential from 1ms to 1s

**Use Cases:**
- Monitor network health
- Detect network degradation
- Tune gossip interval

**PromQL Examples:**

```promql
# P95 gossip latency
histogram_quantile(0.95,
  rate(swim_gossip_latency_ms_bucket[5m])
)

# Alert on high gossip latency (>100ms indicates network issues)
histogram_quantile(0.95, rate(swim_gossip_latency_ms_bucket[5m])) > 100
```

---

## System Metrics

### process_cpu_seconds_total

**Type:** Counter

**Description:** Total CPU time consumed by the process (from Prometheus client library).

**Labels:** None

**PromQL Examples:**

```promql
# CPU usage percentage
rate(process_cpu_seconds_total[1m]) * 100
```

---

### process_resident_memory_bytes

**Type:** Gauge

**Description:** Resident memory size in bytes (from Prometheus client library).

**Labels:** None

**PromQL Examples:**

```promql
# Memory usage in GB
process_resident_memory_bytes / 1024^3
```

---

## Alerting Rules

### Critical Alerts

```yaml
groups:
- name: norikv_critical
  rules:
  # Server down
  - alert: NoriKVDown
    expr: up{job="norikv"} == 0
    for: 1m
    labels:
      severity: critical
    annotations:
      summary: "NoriKV instance {{ $labels.instance }} is down"

  # High error rate
  - alert: NoriKVHighErrorRate
    expr: |
      rate(kv_requests_total{status="error"}[5m])
      / rate(kv_requests_total[5m]) > 0.05
    for: 2m
    labels:
      severity: critical
    annotations:
      summary: "NoriKV error rate >5% on {{ $labels.instance }}"

  # Cluster size drop
  - alert: NoriKVClusterShrunk
    expr: swim_cluster_size < 3
    for: 5m
    labels:
      severity: critical
    annotations:
      summary: "NoriKV cluster size dropped to {{ $value }}"
```

---

### Warning Alerts

```yaml
groups:
- name: norikv_warning
  rules:
  # High latency
  - alert: NoriKVHighLatency
    expr: |
      histogram_quantile(0.95,
        rate(kv_request_duration_ms_bucket{operation="put"}[5m])
      ) > 50
    for: 5m
    labels:
      severity: warning
    annotations:
      summary: "NoriKV p95 PUT latency >50ms on {{ $labels.instance }}"

  # Frequent leader elections
  - alert: NoriKVFrequentElections
    expr: rate(raft_leader_changes_total[1h]) * 3600 > 2
    for: 10m
    labels:
      severity: warning
    annotations:
      summary: "NoriKV shard {{ $labels.shard_id }} having frequent elections"

  # L0 storm
  - alert: NoriKVL0Storm
    expr: lsm_sstable_count{level="L0"} > 10
    for: 5m
    labels:
      severity: warning
    annotations:
      summary: "NoriKV shard {{ $labels.shard_id }} has {{ $value }} L0 SSTables"
```

---

## Grafana Dashboards

### KV Request Dashboard

```json
{
  "dashboard": {
    "title": "NoriKV - KV Requests",
    "rows": [
      {
        "title": "Request Rate",
        "panels": [
          {
            "type": "graph",
            "targets": [
              {
                "expr": "sum(rate(kv_requests_total[5m])) by (operation)",
                "legendFormat": "{{ operation }}"
              }
            ]
          }
        ]
      },
      {
        "title": "Latency Percentiles",
        "panels": [
          {
            "type": "graph",
            "targets": [
              {
                "expr": "histogram_quantile(0.50, rate(kv_request_duration_ms_bucket[5m]))",
                "legendFormat": "p50"
              },
              {
                "expr": "histogram_quantile(0.95, rate(kv_request_duration_ms_bucket[5m]))",
                "legendFormat": "p95"
              },
              {
                "expr": "histogram_quantile(0.99, rate(kv_request_duration_ms_bucket[5m]))",
                "legendFormat": "p99"
              }
            ]
          }
        ]
      },
      {
        "title": "Error Rate",
        "panels": [
          {
            "type": "graph",
            "targets": [
              {
                "expr": "rate(kv_requests_total{status=\"error\"}[5m]) / rate(kv_requests_total[5m])",
                "legendFormat": "error_rate"
              }
            ]
          }
        ]
      }
    ]
  }
}
```

---

## Performance Considerations

### Metric Collection Overhead

**Counter increment:** ~80ns
- Lock-free atomic operations
- No allocations
- Negligible impact on hot path

**Histogram observation:** ~200ns
- Binary search for bucket
- Atomic increment
- Acceptable for request handlers

**Registry export (GET /metrics):** ~1-5ms
- Iterates all metrics
- Formats as text
- Called infrequently (15s scrape interval)

### Best Practices

1. **Use histograms for latencies** - Not averages (lose distribution)
2. **Scrape every 15s** - Good balance of resolution and overhead
3. **Limit cardinality** - Avoid high-cardinality labels (user IDs, keys)
4. **Pre-allocate labels** - Use `&'static [(&'static str, &'static str)]`
5. **Export at /metrics** - Standard Prometheus convention

---

## Troubleshooting

### Missing Metrics

**Problem:** Expected metrics not appearing

**Debug:**

```bash
# Check if server started with meter
journalctl -u norikv-server | grep "Enabling KV metrics"

# Verify meter is wired
curl http://localhost:8080/metrics | grep kv_requests_total
```

**Solution:** Ensure meter is passed to both GrpcServer and HttpServer

---

### Stale Metrics

**Problem:** Metrics not updating

**Debug:**

```bash
# Send a request
grpcurl -plaintext localhost:6000 norikv.Kv/Put \
  -d '{"key":"dGVzdA==","value":"dmFsdWU="}'

# Check metric immediately
curl -s http://localhost:8080/metrics | grep kv_requests_total
```

**Solution:** Check if meter is being used in KvService handlers

---

## Next Steps

- **[REST API](rest-api.md)** - HTTP endpoints for metrics export
- **[Configuration](configuration.md)** - Tune metric collection
- **[Deployment](deployment.md)** - Prometheus/Grafana setup
