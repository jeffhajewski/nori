# Operations

Production deployment, monitoring, and maintenance of NoriKV clusters.

---

---

## Overview

This section covers operational aspects of running NoriKV in production:

- **[REST API](rest-api.md)** - HTTP endpoints for health checks and metrics
- **[Metrics](metrics.md)** - Prometheus metrics reference and monitoring
- **[Configuration](configuration.md)** - Server configuration options
- **[Deployment](deployment.md)** - Deployment topologies and best practices
- **[Troubleshooting](troubleshooting.md)** - Common issues and solutions

---

## Quick Start

### Running a Single Node

```bash
# Start server with default configuration
norikv-server --node-id node0 \
  --rpc-addr 127.0.0.1:6000 \
  --http-addr 127.0.0.1:8080 \
  --data-dir /var/lib/norikv
```

### Health Check

```bash
# Quick health check (for load balancers)
curl http://localhost:8080/health/quick

# Comprehensive health status
curl http://localhost:8080/health
```

### Metrics Collection

```bash
# Prometheus metrics endpoint
curl http://localhost:8080/metrics
```

---

## Production Checklist

### Before Deployment

- [ ] Configure data directory with sufficient disk space
- [ ] Set up Prometheus/Grafana for metrics collection
- [ ] Configure health check endpoints in load balancer
- [ ] Review and tune LSM compaction settings
- [ ] Configure Raft timeouts for your network latency
- [ ] Set up log aggregation (structured JSON logs)

### Monitoring

- [ ] Track `kv_requests_total` - Request volume per operation
- [ ] Monitor `kv_request_duration_ms` - Latency percentiles
- [ ] Watch `lsm_compaction_duration_ms` - Compaction performance
- [ ] Alert on `raft_leader_changes_total` - Cluster stability
- [ ] Check disk space and WAL/SSTable growth

### High Availability

- [ ] Deploy at least 3 nodes for fault tolerance
- [ ] Configure replication factor ≥3
- [ ] Distribute nodes across availability zones
- [ ] Set up automated backups (snapshots)
- [ ] Test failure scenarios (node crash, network partition)

---

## Architecture Integration

### Server Components

```
┌─────────────────────────────────────────┐
│  HTTP Server (Axum)                     │
│  - GET /health/quick                    │
│  - GET /health                          │
│  - GET /metrics (Prometheus)            │
└──────────────────┬──────────────────────┘
                   │
┌──────────────────▼──────────────────────┐
│  gRPC Server (Tonic)                    │
│  - KV Service (Put/Get/Delete)          │
│  - Meta Service (WatchCluster)          │
│  - Admin Service (SnapshotShard)        │
│  - Raft Service (peer-to-peer RPC)      │
└──────────────────┬──────────────────────┘
                   │
┌──────────────────▼──────────────────────┐
│  Multi-Shard Backend                    │
│  - Routes requests to correct shard     │
│  - 1024 virtual shards                  │
└──────────────────┬──────────────────────┘
                   │
        ┌──────────┴──────────┐
        ▼                     ▼
┌───────────────┐    ┌───────────────┐
│  Shard 0      │    │  Shard N      │
│  - Raft       │    │  - Raft       │
│  - LSM        │    │  - LSM        │
└───────────────┘    └───────────────┘
```

### Key Design Decisions

1. **Separate HTTP and gRPC servers**
   - HTTP for ops/monitoring (health, metrics)
   - gRPC for client API (high performance, streaming)

2. **Lazy shard initialization**
   - Shards created on first access
   - Reduces startup time and memory usage

3. **Prometheus pull model**
   - Standard /metrics endpoint
   - No agent required
   - Works with Kubernetes service discovery

4. **SWIM-based topology tracking**
   - Gossip protocol for scalability
   - Automatic failure detection
   - Integrates with ClusterView for client routing

---

## Next Steps

- **[REST API Reference](rest-api.md)** - HTTP endpoints and usage
- **[Metrics Guide](metrics.md)** - All metrics explained
- **[Configuration](configuration.md)** - Tuning parameters
- **[Deployment](deployment.md)** - Production deployment patterns
