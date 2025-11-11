---
layout: default
title: REST API
parent: Operations
nav_order: 1
---

# REST API Reference
{: .no_toc }

HTTP endpoints for health checks, metrics, and cluster management.
{: .fs-6 .fw-300 }

---

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Overview

NoriKV exposes a REST API via HTTP for operational tasks:

- **Health checks** - Monitor server and shard health
- **Metrics** - Prometheus-format metrics
- **Admin operations** - Cluster management (future)

The HTTP server runs on a separate port from the gRPC API, allowing you to:
- Isolate operational traffic from client traffic
- Configure different firewall rules
- Use load balancer health checks without authentication

---

## Base Configuration

```bash
# Start server with HTTP API on port 8080
norikv-server \
  --http-addr 0.0.0.0:8080 \
  --rpc-addr 0.0.0.0:6000
```

**Default ports:**
- HTTP REST API: `8080`
- gRPC client API: `6000`

---

## Endpoints

### Health Checks

#### Quick Health Check

**Endpoint:** `GET /health/quick`

**Purpose:** Fast health check for load balancers and uptime monitors.

**Response:**
- **200 OK** - Server is healthy (shard 0 is responsive)
- **503 Service Unavailable** - Server is unhealthy

**Response Body:** Plain text `OK` or `UNAVAILABLE`

**Performance:**
- Latency: <1ms (checks only shard 0)
- No disk I/O
- Minimal CPU overhead

**Use Cases:**
- Kubernetes liveness/readiness probes
- Load balancer health checks
- Uptime monitoring services

**Example:**

```bash
# Check if server is healthy
curl -i http://localhost:8080/health/quick
# HTTP/1.1 200 OK
# OK

# Use in scripts
if curl -sf http://localhost:8080/health/quick > /dev/null; then
  echo "Server is healthy"
else
  echo "Server is down"
  exit 1
fi
```

**Kubernetes Example:**

```yaml
apiVersion: v1
kind: Pod
spec:
  containers:
  - name: norikv
    livenessProbe:
      httpGet:
        path: /health/quick
        port: 8080
      initialDelaySeconds: 5
      periodSeconds: 10
    readinessProbe:
      httpGet:
        path: /health/quick
        port: 8080
      initialDelaySeconds: 3
      periodSeconds: 5
```

---

#### Comprehensive Health Status

**Endpoint:** `GET /health`

**Purpose:** Detailed health status for all shards and components.

**Response:** JSON object with server and shard-level health

**Response Format:**

```json
{
  "status": "healthy",
  "node_id": "node0",
  "uptime_seconds": 3600,
  "total_shards": 1024,
  "active_shards": 42,
  "shards": [
    {
      "id": 0,
      "status": "healthy",
      "is_leader": true,
      "raft_term": 5,
      "commit_index": 12345,
      "last_applied": 12345
    },
    {
      "id": 1,
      "status": "healthy",
      "is_leader": false,
      "raft_term": 5,
      "commit_index": 8901,
      "last_applied": 8901
    }
  ]
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `status` | string | Overall status: `healthy`, `degraded`, `unhealthy` |
| `node_id` | string | Server node ID |
| `uptime_seconds` | number | Seconds since server start |
| `total_shards` | number | Total virtual shards (1024) |
| `active_shards` | number | Number of shards currently active on this node |
| `shards` | array | Health status for each active shard |

**Shard Health Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `id` | number | Shard ID (0-1023) |
| `status` | string | Shard status: `healthy`, `degraded`, `unavailable` |
| `is_leader` | boolean | True if this node is the Raft leader for this shard |
| `raft_term` | number | Current Raft term |
| `commit_index` | number | Last committed log index |
| `last_applied` | number | Last applied log index to LSM |

**Status Values:**

- **`healthy`** - All shards operational, leader elected
- **`degraded`** - Some shards not responding or no leader
- **`unhealthy`** - Critical failure, server not operational

**Performance:**
- Latency: ~5-10ms (queries all active shards)
- May involve I/O if checking LSM state
- Use sparingly in high-frequency monitoring

**Use Cases:**
- Debugging cluster issues
- Capacity planning (active shard count)
- Raft state inspection
- Detailed dashboards

**Example:**

```bash
# Get detailed health status
curl http://localhost:8080/health | jq

# Check if server is leader for shard 42
curl -s http://localhost:8080/health | \
  jq '.shards[] | select(.id == 42) | .is_leader'

# Count healthy shards
curl -s http://localhost:8080/health | \
  jq '[.shards[] | select(.status == "healthy")] | length'
```

**Monitoring Alert Example:**

```python
import requests

response = requests.get('http://localhost:8080/health')
health = response.json()

if health['status'] != 'healthy':
    alert(f"NoriKV unhealthy: {health['status']}")

unhealthy_shards = [
    s for s in health['shards']
    if s['status'] != 'healthy'
]
if unhealthy_shards:
    alert(f"Unhealthy shards: {[s['id'] for s in unhealthy_shards]}")
```

---

### Metrics

#### Prometheus Metrics Export

**Endpoint:** `GET /metrics`

**Purpose:** Export metrics in Prometheus text format for scraping.

**Response:** Plain text, Prometheus exposition format version 0.0.4

**Content-Type:** `text/plain; version=0.0.4`

**Example Response:**

```
# HELP kv_requests_total Total number of KV requests
# TYPE kv_requests_total counter
kv_requests_total{operation="put"} 12345
kv_requests_total{operation="get"} 67890
kv_requests_total{operation="delete"} 123

# HELP kv_request_duration_ms Request duration in milliseconds
# TYPE kv_request_duration_ms histogram
kv_request_duration_ms_bucket{operation="put",status="success",le="1.0"} 5000
kv_request_duration_ms_bucket{operation="put",status="success",le="2.0"} 8000
kv_request_duration_ms_bucket{operation="put",status="success",le="4.0"} 10000
kv_request_duration_ms_bucket{operation="put",status="success",le="+Inf"} 12345
kv_request_duration_ms_sum{operation="put",status="success"} 123450.5
kv_request_duration_ms_count{operation="put",status="success"} 12345
```

**Metrics Exposed:**

See **[Metrics Reference](metrics)** for complete list.

**Key Metrics:**
- `kv_requests_total` - Request counters by operation
- `kv_request_duration_ms` - Latency histograms
- `lsm_*` - LSM engine metrics (compaction, memtable)
- `raft_*` - Raft consensus metrics (elections, commits)
- `swim_*` - Membership metrics (cluster size, failures)

**Prometheus Scrape Configuration:**

```yaml
scrape_configs:
  - job_name: 'norikv'
    static_configs:
      - targets: ['localhost:8080']
    scrape_interval: 15s
    metrics_path: /metrics
```

**Kubernetes ServiceMonitor:**

```yaml
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: norikv
spec:
  selector:
    matchLabels:
      app: norikv
  endpoints:
  - port: http
    path: /metrics
    interval: 15s
```

**Use Cases:**
- Prometheus/Grafana dashboards
- Alerting on latency/error rates
- Capacity planning
- Performance debugging

**Example Queries:**

```promql
# Request rate (per second)
rate(kv_requests_total[5m])

# P95 latency
histogram_quantile(0.95,
  rate(kv_request_duration_ms_bucket[5m])
)

# Error rate
rate(kv_requests_total{status="error"}[5m])
  / rate(kv_requests_total[5m])
```

---

## HTTP Server Architecture

### Technology Stack

- **Framework:** [Axum](https://github.com/tokio-rs/axum) - Ergonomic async web framework
- **Runtime:** Tokio async runtime
- **Serialization:** Serde JSON for structured responses
- **Metrics:** prometheus-client for Prometheus export

### Code Organization

```rust
// apps/norikv-server/src/http.rs

pub struct HttpServer {
    addr: SocketAddr,
    state: HttpServerState,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    server_handle: Option<JoinHandle<Result<(), std::io::Error>>>,
}

#[derive(Clone)]
pub struct HttpServerState {
    health_checker: Arc<HealthChecker>,
    meter: Arc<PrometheusMeter>,
}
```

**Key Design Decisions:**

1. **Separate server from gRPC**
   - Allows different ports/firewall rules
   - No authentication required for ops endpoints
   - Can disable in production if not needed

2. **Axum router**
   - Type-safe route handlers
   - Extractors for state/headers
   - Graceful shutdown support

3. **Shared state via Arc**
   - Health checker shared with Node
   - Prometheus meter shared with gRPC server
   - No data duplication

4. **Async handlers**
   - Non-blocking health checks
   - Concurrent metric collection
   - Efficient resource usage

### Lifecycle

```
Node::start()
  ↓
Create HttpServer { state: { health_checker, meter } }
  ↓
Build Axum router with routes
  ↓
Bind TcpListener on http_addr
  ↓
Spawn server task with graceful shutdown
  ↓
Server running (handle requests)
  ↓
Node::shutdown()
  ↓
Send shutdown signal
  ↓
Await server task completion
  ↓
Server stopped
```

---

## Error Handling

### HTTP Status Codes

| Status | When | Example |
|--------|------|---------|
| 200 OK | Request succeeded | Health check passed, metrics returned |
| 500 Internal Server Error | Handler error | Health check failed to query shard |
| 503 Service Unavailable | Server unhealthy | Quick health check: server not ready |

### Error Response Format

```json
{
  "error": "Internal error: failed to query shard 0"
}
```

**Handler Error Conversion:**

```rust
struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!("Handler error: {:?}", self.0);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Internal error: {}", self.0),
        ).into_response()
    }
}
```

---

## Best Practices

### Load Balancer Integration

**Use `/health/quick` for load balancer health checks:**

```nginx
# Nginx upstream health check
upstream norikv {
    server 10.0.1.10:8080 max_fails=3 fail_timeout=30s;
    server 10.0.1.11:8080 max_fails=3 fail_timeout=30s;
    server 10.0.1.12:8080 max_fails=3 fail_timeout=30s;
}

server {
    location /health/quick {
        proxy_pass http://norikv;
        proxy_connect_timeout 1s;
        proxy_read_timeout 1s;
    }
}
```

**HAProxy:**

```
backend norikv
    balance roundrobin
    option httpchk GET /health/quick
    http-check expect status 200
    server node0 10.0.1.10:8080 check inter 5s
    server node1 10.0.1.11:8080 check inter 5s
    server node2 10.0.1.12:8080 check inter 5s
```

### Monitoring Setup

**Scrape metrics every 15 seconds:**

```yaml
# prometheus.yml
global:
  scrape_interval: 15s

scrape_configs:
  - job_name: 'norikv'
    static_configs:
      - targets:
        - '10.0.1.10:8080'
        - '10.0.1.11:8080'
        - '10.0.1.12:8080'
    scrape_interval: 15s
    scrape_timeout: 10s
```

**Alert on unhealthy status:**

```yaml
# alerts.yml
groups:
- name: norikv
  rules:
  - alert: NoriKVUnhealthy
    expr: up{job="norikv"} == 0
    for: 1m
    annotations:
      summary: "NoriKV instance {{ $labels.instance }} is down"
```

### Security Considerations

**Firewall rules:**

```bash
# Allow HTTP from Prometheus/monitoring only
iptables -A INPUT -p tcp --dport 8080 -s 10.0.2.0/24 -j ACCEPT
iptables -A INPUT -p tcp --dport 8080 -j DROP

# Allow gRPC from clients
iptables -A INPUT -p tcp --dport 6000 -j ACCEPT
```

**Kubernetes Network Policy:**

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: norikv-http
spec:
  podSelector:
    matchLabels:
      app: norikv
  ingress:
  - from:
    - namespaceSelector:
        matchLabels:
          name: monitoring
    ports:
    - protocol: TCP
      port: 8080
```

---

## Troubleshooting

### Server won't start

**Error:** `Failed to bind: Address already in use`

**Solution:** Check if another process is using port 8080:

```bash
lsof -i :8080
# Or change port
norikv-server --http-addr 0.0.0.0:9090
```

### Health check always returns 503

**Symptoms:** `/health/quick` returns `UNAVAILABLE`

**Debugging:**

```bash
# Check detailed health status
curl http://localhost:8080/health | jq

# Check if shard 0 is healthy
curl -s http://localhost:8080/health | \
  jq '.shards[] | select(.id == 0)'
```

**Common causes:**
- Shard 0 not yet created (server starting up)
- Raft leader not elected (multi-node: wait for quorum)
- LSM engine not initialized

### Metrics endpoint empty

**Symptoms:** `/metrics` returns no metrics

**Debugging:**

```bash
# Check if meter is wired to gRPC server
# Look for log line: "Enabling KV metrics collection"
journalctl -u norikv-server | grep metrics

# Send a request to generate metrics
grpcurl -plaintext localhost:6000 norikv.Kv/Get \
  -d '{"key":"dGVzdA=="}'

# Check metrics again
curl http://localhost:8080/metrics
```

**Solution:** Ensure meter is passed to both GrpcServer and HttpServer in Node::start()

---

## Next Steps

- **[Metrics Reference](metrics)** - All metrics explained
- **[Configuration](configuration)** - Server configuration options
- **[Deployment](deployment)** - Production deployment patterns
