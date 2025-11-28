# Configuration Reference

Complete reference for all NoriKV server configuration options.

---

---

## Overview

NoriKV configuration can be loaded from:
1. **YAML file** - Recommended for production
2. **Environment variables** - Useful for containers/Docker
3. **Command-line arguments** - Override specific settings

**Configuration hierarchy:**
```
Defaults → YAML file → Environment variables → CLI args
                       (lowest priority → highest priority)
```

---

## Quick Start

### Minimal Configuration

**`config.yaml`:**

```yaml
node_id: "node0"
data_dir: "/var/lib/norikv"
```

**Start server:**

```bash
norikv-server --config config.yaml
```

### Production Configuration

```yaml
# Node identity
node_id: "node0"
rpc_addr: "0.0.0.0:7447"
http_addr: "0.0.0.0:8080"
data_dir: "/var/lib/norikv"

# Cluster settings
cluster:
  seed_nodes:
    - "10.0.1.10:7447"
    - "10.0.1.11:7447"
    - "10.0.1.12:7447"
  total_shards: 1024
  replication_factor: 3

# Telemetry
telemetry:
  prometheus:
    enabled: true
    port: 9090
  otlp:
    enabled: false
    endpoint: "http://otlp-collector:4317"
```

---

## Server Configuration

### node_id

**Type:** `string` (required)

**Description:** Unique identifier for this server node.

**Rules:**
- Must be unique across the cluster
- Non-empty string
- Used as Raft node ID and SWIM member ID

**Examples:**

```yaml
# Simple ID
node_id: "node0"

# Hostname-based
node_id: "norikv-prod-us-east-1a-0"

# UUID-based (for dynamic clusters)
node_id: "550e8400-e29b-41d4-a716-446655440000"
```

**Environment variable:** `NORIKV_NODE_ID`

```bash
export NORIKV_NODE_ID="node0"
```

---

### rpc_addr

**Type:** `string` (default: `0.0.0.0:7447`)

**Description:** gRPC server listen address for client requests and Raft peer-to-peer RPC.

**Format:** `host:port` (must be valid SocketAddr)

**Examples:**

```yaml
# Listen on all interfaces (production)
rpc_addr: "0.0.0.0:7447"

# Listen on specific interface
rpc_addr: "10.0.1.10:7447"

# Development (localhost only)
rpc_addr: "127.0.0.1:6000"
```

**Firewall rules:**

```bash
# Allow gRPC from clients and peers
iptables -A INPUT -p tcp --dport 7447 -j ACCEPT
```

**Environment variable:** `NORIKV_RPC_ADDR`

**Default port:** `7447` (NoriKV default)

---

### http_addr

**Type:** `string` (default: `0.0.0.0:8080`)

**Description:** HTTP REST API listen address for health checks and metrics.

**Format:** `host:port` (must be valid SocketAddr)

**Examples:**

```yaml
# Standard HTTP port
http_addr: "0.0.0.0:8080"

# Separate monitoring network
http_addr: "192.168.1.10:8080"

# Development
http_addr: "127.0.0.1:3000"
```

**Firewall rules:**

```bash
# Allow HTTP from monitoring tools only
iptables -A INPUT -p tcp --dport 8080 -s 10.0.2.0/24 -j ACCEPT
iptables -A INPUT -p tcp --dport 8080 -j DROP
```

**Environment variable:** `NORIKV_HTTP_ADDR`

---

### data_dir

**Type:** `PathBuf` (required)

**Description:** Base directory for all persistent data (Raft WAL, LSM SSTables, manifests).

**Directory structure:**

```
/var/lib/norikv/
├── raft/
│   ├── shard-0/
│   │   ├── wal/
│   │   │   ├── 000001.log
│   │   │   └── 000002.log
│   │   └── snapshot/
│   │       └── snapshot-00012345.dat
│   ├── shard-1/
│   └── ...
└── lsm/
    ├── shard-0/
    │   ├── sst/
    │   │   ├── L0-000001.sst
    │   │   └── L1-000002.sst
    │   ├── manifest/
    │   │   └── MANIFEST-000001
    │   └── wal/
    │       └── 000001.wal
    ├── shard-1/
    └── ...
```

**Disk requirements:**
- **Type:** SSD strongly recommended (NVMe preferred)
- **Size:** Plan for 3x your dataset size (replication + compaction)
- **IOPs:** 10,000+ recommended for production

**Examples:**

```yaml
# Production
data_dir: "/var/lib/norikv"

# Development
data_dir: "./data"

# Mounted volume
data_dir: "/mnt/nvme0n1/norikv"
```

**Permissions:**

```bash
# Create directory with correct permissions
sudo mkdir -p /var/lib/norikv
sudo chown norikv:norikv /var/lib/norikv
sudo chmod 750 /var/lib/norikv
```

**Environment variable:** `NORIKV_DATA_DIR`

---

## Cluster Configuration

### cluster.seed_nodes

**Type:** `Vec<String>` (default: `[]` for single-node)

**Description:** Initial seed nodes for SWIM membership bootstrap.

**Format:** List of `host:port` addresses

**Rules:**
- Empty list = single-node mode (no SWIM)
- Non-empty = multi-node cluster mode
- Addresses must be reachable from this node
- At least one seed must be alive to join

**Examples:**

**Single-node (development):**

```yaml
cluster:
  seed_nodes: []  # Empty = single-node mode
```

**3-node cluster:**

```yaml
cluster:
  seed_nodes:
    - "10.0.1.10:7447"
    - "10.0.1.11:7447"
    - "10.0.1.12:7447"
```

**Kubernetes (headless service):**

```yaml
cluster:
  seed_nodes:
    - "norikv-0.norikv.default.svc.cluster.local:7447"
    - "norikv-1.norikv.default.svc.cluster.local:7447"
    - "norikv-2.norikv.default.svc.cluster.local:7447"
```

**Environment variable:** `NORIKV_SEED_NODES` (comma-separated)

```bash
export NORIKV_SEED_NODES="10.0.1.10:7447,10.0.1.11:7447,10.0.1.12:7447"
```

**Bootstrapping a new cluster:**

1. Start node0 with empty seed_nodes (single-node mode)
2. Start node1 with `seed_nodes: ["node0:7447"]`
3. Start node2 with `seed_nodes: ["node0:7447", "node1:7447"]`
4. All nodes converge on same membership view

---

### cluster.total_shards

**Type:** `u32` (default: `1024`)

**Description:** Total number of virtual shards in the cluster.

**Rules:**
- Must be > 0
- Fixed at cluster creation (cannot change later)
- Power of 2 recommended (128, 256, 512, 1024, 2048)

**Tuning guidance:**

| Cluster Size | Dataset Size | Recommended Shards | Shards/Node |
|--------------|--------------|-------------------|-------------|
| 1 node | <100GB | 128-256 | 128-256 |
| 3 nodes | 100-500GB | 512-1024 | 170-340 |
| 9 nodes | 500GB-5TB | 1024-2048 | 113-227 |
| 27 nodes | 5TB+ | 2048-4096 | 75-151 |

**Trade-offs:**

**More shards:**
-  Finer-grained load balancing
-  Easier rebalancing (smaller units)
-  More Raft overhead (more leaders, more heartbeats)
-  Higher memory usage

**Fewer shards:**
-  Lower overhead
-  Less memory usage
-  Coarse-grained balancing
-  Harder to rebalance

**Example:**

```yaml
cluster:
  total_shards: 1024  # Default, works for most cases
```

---

### cluster.replication_factor

**Type:** `usize` (default: `3`)

**Description:** Number of replicas per shard for fault tolerance.

**Rules:**
- Must be > 0
- Should be ≤ cluster size
- Odd numbers preferred (3, 5, 7) for quorum

**Fault tolerance:**

| RF | Tolerated Failures | Quorum Size | Recommended For |
|----|-------------------|-------------|-----------------|
| 1 | 0 | 1 | Development only |
| 3 | 1 | 2 | Standard production |
| 5 | 2 | 3 | Mission-critical |
| 7 | 3 | 4 | Maximum durability |

**Storage overhead:**

```
Total storage = dataset_size × replication_factor

Example: 100GB dataset, RF=3 → 300GB total
```

**Examples:**

```yaml
# Development (no replication)
cluster:
  replication_factor: 1

# Production (standard)
cluster:
  replication_factor: 3

# Mission-critical (high availability)
cluster:
  replication_factor: 5
```

---

## Telemetry Configuration

### telemetry.prometheus.enabled

**Type:** `bool` (default: `true`)

**Description:** Enable Prometheus metrics export via `/metrics` endpoint.

**Examples:**

```yaml
telemetry:
  prometheus:
    enabled: true  # Export metrics on http_addr/metrics
```

**Disable metrics:**

```yaml
telemetry:
  prometheus:
    enabled: false  # No /metrics endpoint
```

---

### telemetry.prometheus.port

**Type:** `u16` (default: `9090`)

**Description:** Prometheus metrics port (currently unused, metrics served on `http_addr`).

---

### telemetry.otlp.enabled

**Type:** `bool` (default: `false`)

**Description:** Enable OTLP (OpenTelemetry Protocol) exporter for traces and metrics.

**Examples:**

```yaml
telemetry:
  otlp:
    enabled: true
    endpoint: "http://otlp-collector:4317"
```

---

### telemetry.otlp.endpoint

**Type:** `Option<String>` (default: `None`)

**Description:** OTLP collector endpoint (gRPC).

**Format:** `http://host:port` or `https://host:port`

**Examples:**

```yaml
# Local collector
telemetry:
  otlp:
    enabled: true
    endpoint: "http://localhost:4317"

# Jaeger
telemetry:
  otlp:
    enabled: true
    endpoint: "http://jaeger:4317"

# Honeycomb
telemetry:
  otlp:
    enabled: true
    endpoint: "https://api.honeycomb.io:443"
```

---

## Raft Configuration (Advanced)

Raft tuning parameters are currently set in code (not exposed via YAML). These are the defaults and their tuning guidance.

### Timeouts

**heartbeat_interval**
- **Default:** `150ms`
- **Description:** Leader sends AppendEntries (heartbeat) at this interval
- **Tuning:** Must be < `election_timeout_min`
  - Increase for high-latency networks (WAN: 500ms)
  - Decrease for low-latency (same datacenter: 50ms)

**election_timeout_min**
- **Default:** `300ms`
- **Description:** Minimum election timeout (randomized between min and max)
- **Tuning:**
  - High-latency networks: `1000-2000ms`
  - Low-latency: `150-300ms`

**election_timeout_max**
- **Default:** `600ms`
- **Description:** Maximum election timeout
- **Tuning:** Should be 2x `election_timeout_min`

**lease_duration**
- **Default:** `2000ms`
- **Description:** Leader lease duration for fast reads
- **Tuning:** Should be ≥ 2× `heartbeat_interval`

**Recommended network tuning:**

| Network Type | heartbeat_interval | election_timeout_min | election_timeout_max | lease_duration |
|--------------|-------------------|---------------------|---------------------|---------------|
| **Same datacenter (LAN)** | 50ms | 150ms | 300ms | 500ms |
| **Multi-AZ (low WAN)** | 150ms | 300ms | 600ms | 2000ms |
| **Multi-region (high WAN)** | 500ms | 1000ms | 2000ms | 5000ms |

---

### Replication Settings

**max_entries_per_append**
- **Default:** `1000 entries`
- **Description:** Maximum entries per AppendEntries RPC
- **Tuning:**
  - Larger = fewer RPCs, more memory per message
  - Smaller = more RPCs, less memory
  - Increase for high-throughput: `5000-10000`

---

### Snapshot Settings

**snapshot_log_size_bytes**
- **Default:** `256 MiB`
- **Description:** Trigger snapshot when log exceeds this size
- **Tuning:**
  - Smaller = more frequent snapshots, less recovery time
  - Larger = fewer snapshots, less I/O overhead
  - Recommendation: `128 MiB` for fast recovery, `512 MiB` for lower overhead

**snapshot_entry_count**
- **Default:** `1,000,000 entries`
- **Description:** Trigger snapshot when entry count exceeds this
- **Tuning:** Depends on entry size
  - Small entries (100B): `10,000,000`
  - Large entries (10KB): `100,000`

**snapshot_chunk_size**
- **Default:** `1 MiB`
- **Description:** InstallSnapshot chunk size for streaming
- **Tuning:**
  - High-bandwidth networks: `4-8 MiB`
  - Low-bandwidth: `256-512 KiB`

---

### Read-Index Settings

**max_pending_read_index**
- **Default:** `1000`
- **Description:** Maximum concurrent read-index requests
- **Tuning:** Increase for high read throughput (10,000+)

---

### Apply Settings

**apply_batch_size**
- **Default:** `100 commands`
- **Description:** Apply commands to LSM in batches
- **Tuning:**
  - Larger = higher throughput, more latency variance
  - Smaller = lower latency, less throughput
  - High throughput: `500-1000`
  - Low latency: `50-100`

---

## LSM Configuration (Advanced)

LSM tuning parameters are currently set in code. These are the defaults and tuning guidance.

### Compaction Settings

**fanout** (Tiering ratio)
- **Default:** `10`
- **Description:** Level size ratio (L1 = 10× L0, L2 = 10× L1, etc.)
- **Tuning:**
  - Smaller (5): More compaction, better read performance
  - Larger (20): Less compaction, better write performance

**max_levels**
- **Default:** `7`
- **Description:** Maximum LSM levels (L0-L6)
- **Tuning:** Usually don't change (7 is sufficient for PB-scale)

**l1_slot_count**
- **Default:** `32`
- **Description:** Number of guard-based partitions in L1
- **Tuning:** Increase for skewed workloads (64, 128)

---

### L0 Settings

**l0.max_files**
- **Default:** `12`
- **Description:** Maximum L0 files before hard write stall
- **Tuning:**
  - Increase for write-heavy: `20-30`
  - Decrease for stable latency: `8-10`

**l0.soft_throttle_threshold**
- **Default:** `6`
- **Description:** L0 count triggering write throttling
- **Tuning:** Usually 50% of `max_files`

**l0.soft_throttle_base_delay_ms**
- **Default:** `1ms`
- **Description:** Base delay for throttling (linear backoff)
- **Tuning:** Adjust based on workload
  - Aggressive throttling: `5-10ms`
  - Gentle throttling: `0.5-1ms`

---

### Memtable Settings

**memtable.flush_trigger_bytes**
- **Default:** `64 MiB`
- **Description:** Trigger memtable flush at this size
- **Tuning:**
  - Larger = fewer flushes, more memory
  - Smaller = more flushes, less memory
  - High-memory systems: `128-256 MiB`
  - Low-memory systems: `16-32 MiB`

---

### Filter Settings

**filters.total_budget_mib**
- **Default:** `256 MiB`
- **Description:** Total bloom filter memory budget
- **Tuning:** Increase for better read performance (512 MiB, 1 GiB)

**filters.target_fp_point_lookups**
- **Default:** `0.001` (0.1% false positive rate)
- **Tuning:**
  - Lower = more memory, fewer false positives
  - Higher = less memory, more false positives

---

### I/O Settings

**io.max_background_compactions**
- **Default:** `4`
- **Description:** Maximum concurrent background compactions
- **Tuning:** Set to number of CPU cores - 2 (leave for requests)

**io.compaction_interval_sec**
- **Default:** `10 seconds`
- **Description:** Compaction loop check interval
- **Tuning:** Decrease for faster compaction (5s), increase for lower overhead (30s)

**io.rate_limit_mb_s**
- **Default:** `None` (no limit)
- **Description:** Compaction I/O rate limit
- **Tuning:** Set to prevent compaction from saturating disk
  - SSD: `500-1000 MB/s`
  - HDD: `50-100 MB/s`

---

## Environment Variables

All configuration can be overridden via environment variables:

| Variable | Config Field | Example |
|----------|-------------|---------|
| `NORIKV_NODE_ID` | `node_id` | `node0` |
| `NORIKV_RPC_ADDR` | `rpc_addr` | `0.0.0.0:7447` |
| `NORIKV_HTTP_ADDR` | `http_addr` | `0.0.0.0:8080` |
| `NORIKV_DATA_DIR` | `data_dir` | `/var/lib/norikv` |
| `NORIKV_SEED_NODES` | `cluster.seed_nodes` | `10.0.1.10:7447,10.0.1.11:7447` |

**Docker example:**

```bash
docker run -d \
  -e NORIKV_NODE_ID="node0" \
  -e NORIKV_RPC_ADDR="0.0.0.0:7447" \
  -e NORIKV_HTTP_ADDR="0.0.0.0:8080" \
  -e NORIKV_DATA_DIR="/data" \
  -e NORIKV_SEED_NODES="node1:7447,node2:7447" \
  -v /var/lib/norikv:/data \
  norikv/norikv-server:latest
```

---

## Configuration Examples

### Single-Node Development

**`dev-config.yaml`:**

```yaml
node_id: "dev-node"
rpc_addr: "127.0.0.1:6000"
http_addr: "127.0.0.1:8080"
data_dir: "./data"

cluster:
  seed_nodes: []  # Single-node mode
  total_shards: 128  # Lower for dev
  replication_factor: 1  # No replication

telemetry:
  prometheus:
    enabled: true
```

---

### 3-Node Production Cluster

**`prod-node0.yaml`:**

```yaml
node_id: "norikv-prod-node0"
rpc_addr: "10.0.1.10:7447"
http_addr: "10.0.1.10:8080"
data_dir: "/var/lib/norikv"

cluster:
  seed_nodes:
    - "10.0.1.10:7447"
    - "10.0.1.11:7447"
    - "10.0.1.12:7447"
  total_shards: 1024
  replication_factor: 3

telemetry:
  prometheus:
    enabled: true
  otlp:
    enabled: true
    endpoint: "http://otlp-collector:4317"
```

**`prod-node1.yaml`:** (similar, change node_id and rpc_addr)

**`prod-node2.yaml`:** (similar, change node_id and rpc_addr)

---

### Kubernetes StatefulSet

**`k8s-config.yaml` (ConfigMap):**

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: norikv-config
data:
  config.yaml: |
    node_id: "$(POD_NAME)"  # Injected by StatefulSet
    rpc_addr: "0.0.0.0:7447"
    http_addr: "0.0.0.0:8080"
    data_dir: "/data"
    cluster:
      seed_nodes:
        - "norikv-0.norikv:7447"
        - "norikv-1.norikv:7447"
        - "norikv-2.norikv:7447"
      total_shards: 1024
      replication_factor: 3
    telemetry:
      prometheus:
        enabled: true
```

---

## Validation

### Configuration Validation

Config is validated on startup:

```rust
ServerConfig::load_from_file("config.yaml")?.validate()?;
```

**Validation checks:**
- `node_id` is non-empty
- `rpc_addr` is valid SocketAddr
- `http_addr` is valid SocketAddr
- `data_dir` exists and is writable
- `total_shards` > 0
- `replication_factor` > 0
- All `seed_nodes` are valid SocketAddr

**Validation errors:**

```
Error: Invalid field: total_shards must be > 0
Error: Invalid rpc_addr: invalid socket address syntax
Error: Cannot create data_dir: permission denied
```

---

## Best Practices

### Production Checklist

- [ ] Set unique `node_id` for each node
- [ ] Use persistent storage for `data_dir` (not ephemeral)
- [ ] Configure `seed_nodes` for multi-node clusters
- [ ] Set `replication_factor` ≥ 3 for fault tolerance
- [ ] Enable Prometheus metrics for monitoring
- [ ] Use SSD/NVMe for `data_dir`
- [ ] Allocate 3× dataset size for storage (replication + compaction)
- [ ] Set firewall rules for `rpc_addr` and `http_addr`

### Security Checklist

- [ ] Restrict `http_addr` to monitoring network only
- [ ] Use TLS for `rpc_addr` (future: mTLS support)
- [ ] Set file permissions on `data_dir` (750 or stricter)
- [ ] Run server as non-root user
- [ ] Enable audit logging (future feature)

### Performance Tuning Checklist

- [ ] Tune Raft timeouts for network latency
- [ ] Adjust memtable size for memory availability
- [ ] Set L0 thresholds for latency/throughput trade-off
- [ ] Configure compaction concurrency for CPU cores
- [ ] Enable rate limiting to prevent compaction storms

---

## Next Steps

- **[Deployment Guide](deployment.md)** - Production deployment patterns
- **[Metrics Reference](metrics.md)** - Monitor your configuration
- **[Troubleshooting](troubleshooting.md)** - Common configuration issues
