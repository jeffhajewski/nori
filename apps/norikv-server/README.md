# norikv-server

Main server binary for NoriKV - a sharded, Raft-replicated, log-structured key-value store.

## Features

- **Multi-Shard Management**: Routes operations to correct shards via Jump Consistent Hash
- **Raft Replication**: Each shard uses Raft consensus for strong consistency
- **SWIM Membership**: Cluster health monitoring and failure detection
- **gRPC API**: High-performance client communication (port 7447)
- **HTTP API**: REST endpoints for admin, health, and metrics (port 8080)
- **Prometheus Metrics**: Built-in metrics endpoint (port 9090)
- **OTLP Tracing**: Optional distributed tracing support

## Quick Start

```bash
# Build the server
cargo build --release -p norikv-server

# Run with configuration file
./target/release/norikv-server --config config.yaml

# Run with environment variables
NORIKV_NODE_ID=node1 \
NORIKV_DATA_DIR=/var/lib/norikv \
NORIKV_SEED_NODES=localhost:7448,localhost:7449 \
./target/release/norikv-server
```

## Configuration

### YAML Configuration

```yaml
node_id: "node1"
rpc_addr: "0.0.0.0:7447"
http_addr: "0.0.0.0:8080"
data_dir: "/var/lib/norikv"

cluster:
  seed_nodes:
    - "10.0.1.10:7447"
    - "10.0.1.11:7447"
  total_shards: 1024
  replication_factor: 3

telemetry:
  prometheus:
    enabled: true
    port: 9090
  otlp:
    enabled: false
    endpoint: "http://localhost:4317"
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `NORIKV_NODE_ID` | Unique node identifier | (required) |
| `NORIKV_RPC_ADDR` | gRPC listen address | `0.0.0.0:7447` |
| `NORIKV_HTTP_ADDR` | HTTP listen address | `0.0.0.0:8080` |
| `NORIKV_DATA_DIR` | Data directory | (required) |
| `NORIKV_SEED_NODES` | Comma-separated seed nodes | (empty) |
| `RUST_LOG` | Log level | `info` |

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  norikv-server                                              │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ gRPC API (:7447)     │ HTTP API (:8080)                 ││
│  │ - Put/Get/Delete     │ - /health, /ready                ││
│  │ - Watch              │ - /api/cluster                   ││
│  │ - Admin ops          │ - /metrics                       ││
│  └──────────────────────┴──────────────────────────────────┘│
│                              │                               │
│  ┌───────────────────────────▼──────────────────────────────┐│
│  │ Router                                                   ││
│  │ - Shard assignment (Jump Consistent Hash)               ││
│  │ - Leader routing                                         ││
│  └──────────────────────────────────────────────────────────┘│
│                              │                               │
│  ┌───────────────────────────▼──────────────────────────────┐│
│  │ ShardManager                                             ││
│  │ ┌───────────┐ ┌───────────┐ ┌───────────┐              ││
│  │ │ Shard 0   │ │ Shard 1   │ │ Shard N   │ ...          ││
│  │ │ (Raft)    │ │ (Raft)    │ │ (Raft)    │              ││
│  │ │ (LSM)     │ │ (LSM)     │ │ (LSM)     │              ││
│  │ └───────────┘ └───────────┘ └───────────┘              ││
│  └──────────────────────────────────────────────────────────┘│
│                              │                               │
│  ┌───────────────────────────▼──────────────────────────────┐│
│  │ SWIM Membership                                          ││
│  │ - Cluster view tracking                                  ││
│  │ - Failure detection                                      ││
│  └──────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────┘
```

## Modules

| Module | Description |
|--------|-------------|
| `config` | Configuration loading (YAML, env vars) |
| `node` | Node lifecycle management |
| `shard_manager` | Multi-shard coordination |
| `multi_shard_backend` | Backend for multi-shard operations |
| `cluster_view` | Cluster topology tracking |
| `health` | Health check endpoints |
| `http` | HTTP REST API handlers |
| `metrics` | Prometheus metrics integration |

## HTTP Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Liveness check (always 200 if running) |
| `/ready` | GET | Readiness check (200 when ready to serve) |
| `/metrics` | GET | Prometheus metrics |
| `/api/cluster/view` | GET | Current cluster topology |
| `/api/cluster/shards` | GET | Shard leadership information |

## Data Directory Structure

```
/var/lib/norikv/
├── raft/           # Raft log and snapshots
│   ├── shard-0/
│   ├── shard-1/
│   └── ...
└── lsm/            # LSM storage (WAL, SSTables)
    ├── shard-0/
    ├── shard-1/
    └── ...
```

## Cluster Setup

### 3-Node Cluster Example

**Node 1** (`node1.yaml`):
```yaml
node_id: "node1"
rpc_addr: "10.0.1.10:7447"
data_dir: "/var/lib/norikv"
cluster:
  seed_nodes: ["10.0.1.11:7447", "10.0.1.12:7447"]
```

**Node 2** (`node2.yaml`):
```yaml
node_id: "node2"
rpc_addr: "10.0.1.11:7447"
data_dir: "/var/lib/norikv"
cluster:
  seed_nodes: ["10.0.1.10:7447", "10.0.1.12:7447"]
```

**Node 3** (`node3.yaml`):
```yaml
node_id: "node3"
rpc_addr: "10.0.1.12:7447"
data_dir: "/var/lib/norikv"
cluster:
  seed_nodes: ["10.0.1.10:7447", "10.0.1.11:7447"]
```

## Docker

For local development with Docker, see the [`docker/`](../../docker/) directory:

```bash
docker compose up -d  # Start 3-node cluster
```

## Observability

### Metrics

Key Prometheus metrics:
- `norikv_operations_total{op="get|put|delete"}` - Operation counts
- `norikv_operation_latency_ms{op="..."}` - Operation latencies
- `norikv_raft_term` - Current Raft term per shard
- `norikv_raft_leader` - Leadership status per shard
- `norikv_lsm_level_bytes{level="0-6"}` - LSM level sizes
- `norikv_swim_members` - Cluster member count

### Logging

```bash
# Info level (default)
RUST_LOG=info ./norikv-server

# Debug level for specific modules
RUST_LOG=norikv_server=debug,nori_raft=info ./norikv-server

# Trace level for troubleshooting
RUST_LOG=trace ./norikv-server
```

## Development

```bash
# Build
cargo build -p norikv-server

# Run tests
cargo test -p norikv-server

# Run with debug logging
RUST_LOG=debug cargo run -p norikv-server -- --config config.yaml
```

## See Also

- [Docker Setup](../../docker/) - Local cluster with Docker Compose
- [Client SDKs](../../sdks/) - TypeScript, Python, Go, Java clients
- [Protocol Definitions](../../proto/) - gRPC service definitions

## License

MIT
