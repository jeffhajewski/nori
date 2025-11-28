# Troubleshooting Guide

Common issues and their solutions for NoriKV operations.

---

---

## Overview

This guide covers common problems encountered when operating NoriKV and provides step-by-step solutions.

**Quick diagnosis:**

```bash
# Check if server is running
curl -f http://localhost:8080/health/quick || echo "Server down"

# Check cluster status
curl -s http://localhost:8080/health | jq '.status'

# Check recent errors
sudo journalctl -u norikv --since "5 minutes ago" | grep ERROR

# Check metrics for issues
curl -s http://localhost:8080/metrics | grep -E "(error|failed)"
```

---

## Server Won't Start

### Symptoms
- `systemctl status norikv` shows "failed"
- No process listening on port 7447
- Logs show startup errors

---

### Port Already in Use

**Error:**
```
Error: Failed to bind: Address already in use (os error 98)
```

**Diagnosis:**

```bash
# Check what's using the port
sudo lsof -i :7447
sudo lsof -i :8080
```

**Solution 1: Kill conflicting process**

```bash
# Find and kill process
sudo kill $(sudo lsof -t -i:7447)
```

**Solution 2: Change port**

```yaml
# config.yaml
rpc_addr: "0.0.0.0:7448"  # Use different port
http_addr: "0.0.0.0:8081"
```

---

### Permission Denied (Data Directory)

**Error:**
```
Error: Cannot create data_dir: Permission denied
```

**Diagnosis:**

```bash
# Check permissions
ls -ld /var/lib/norikv
ls -l /var/lib/norikv
```

**Solution:**

```bash
# Fix ownership
sudo chown -R norikv:norikv /var/lib/norikv

# Fix permissions
sudo chmod 750 /var/lib/norikv
```

---

### Invalid Configuration

**Error:**
```
Error: Invalid field: node_id cannot be empty
Error: Invalid rpc_addr: invalid socket address syntax
```

**Diagnosis:**

```bash
# Validate config manually
cat /etc/norikv/config.yaml

# Check for YAML syntax errors
yamllint /etc/norikv/config.yaml
```

**Common mistakes:**

```yaml
# WRONG: Empty node_id
node_id: ""

# RIGHT:
node_id: "node0"

# WRONG: Missing port
rpc_addr: "10.0.1.10"

# RIGHT:
rpc_addr: "10.0.1.10:7447"

# WRONG: Invalid total_shards
cluster:
  total_shards: 0

# RIGHT:
cluster:
  total_shards: 1024
```

---

### Out of Memory

**Error:**
```
Error: Cannot allocate memory
FATAL: Out of memory
```

**Diagnosis:**

```bash
# Check available memory
free -h

# Check server memory usage
ps aux | grep norikv-server
```

**Solution 1: Increase system memory**

```bash
# Add swap (temporary)
sudo fallocate -l 8G /swapfile
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile
```

**Solution 2: Reduce memtable size (requires code change)**

Currently memtable size is hardcoded at 64 MiB. Future: expose in config.

**Solution 3: Reduce active shards**

```yaml
# Reduce total shards for low-memory systems
cluster:
  total_shards: 128  # Instead of 1024
```

---

## Cluster Formation Issues

### Nodes Can't Join Cluster

**Symptoms:**
- Node starts but doesn't join cluster
- `swim_cluster_size` metric shows 1 instead of 3
- Logs show "Failed to join cluster" errors

---

#### Network Connectivity

**Error:**
```
Failed to join cluster: Connection refused (os error 111)
```

**Diagnosis:**

```bash
# Test connectivity from node1 to node0
telnet 10.0.1.10 7447

# Check firewall
sudo iptables -L -n | grep 7447

# Check if seed node is up
curl http://10.0.1.10:8080/health/quick
```

**Solution:**

```bash
# Open firewall (Ubuntu/Debian)
sudo ufw allow 7447/tcp
sudo ufw allow 8080/tcp

# Open firewall (RHEL/CentOS)
sudo firewall-cmd --permanent --add-port=7447/tcp
sudo firewall-cmd --permanent --add-port=8080/tcp
sudo firewall-cmd --reload
```

---

#### Wrong Seed Nodes

**Error:**
```
No seed nodes reachable
```

**Diagnosis:**

```bash
# Check seed nodes in config
grep -A5 "seed_nodes:" /etc/norikv/config.yaml

# Test each seed
for seed in 10.0.1.10:7447 10.0.1.11:7447 10.0.1.12:7447; do
  echo "Testing $seed..."
  nc -zv ${seed%:*} ${seed#*:}
done
```

**Solution:**

Fix seed_nodes addresses:

```yaml
# WRONG: Using hostnames that don't resolve
cluster:
  seed_nodes:
    - "node0:7447"  # DNS lookup fails

# RIGHT: Use IP addresses or resolvable hostnames
cluster:
  seed_nodes:
    - "10.0.1.10:7447"
    - "10.0.1.11:7447"
    - "10.0.1.12:7447"
```

---

#### Node ID Conflicts

**Error:**
```
Node with ID 'node0' already exists in cluster
```

**Diagnosis:**

```bash
# Check if multiple nodes have same node_id
for node in node0 node1 node2; do
  ssh $node "grep node_id /etc/norikv/config.yaml"
done
```

**Solution:**

Ensure each node has unique node_id:

```yaml
# Node 0:
node_id: "node0"

# Node 1:
node_id: "node1"  # NOT "node0"

# Node 2:
node_id: "node2"  # NOT "node0"
```

---

## Write Failures

### "Not Leader" Errors

**Error:**
```
ERROR: not_leader: This node is not the Raft leader for shard 42
```

**Diagnosis:**

```bash
# Check shard leadership
curl -s http://localhost:8080/health | jq '.shards[] | select(.id == 42)'

# Check Raft metrics
curl -s http://localhost:8080/metrics | grep 'raft_leader{shard_id="42"}'
```

**Root causes:**

1. **Election in progress** (wait 1-2 seconds)
2. **Cluster split** (network partition)
3. **Majority offline** (can't elect leader)

**Solution 1: Retry (client-side)**

SDK automatically retries on "not_leader" errors.

**Solution 2: Check cluster health**

```bash
# Ensure majority (≥2 of 3) nodes are alive
curl http://10.0.1.10:8080/health/quick
curl http://10.0.1.11:8080/health/quick
curl http://10.0.1.12:8080/health/quick
```

**Solution 3: Wait for election**

```bash
# Watch leader election
watch 'curl -s http://localhost:8080/metrics | grep raft_leader | grep shard_id=\"42\"'
```

---

### L0 Write Stalls

**Error:**
```
ERROR: L0 stall: Too many L0 files (12), writes blocked
```

**Diagnosis:**

```bash
# Check L0 file count
curl -s http://localhost:8080/metrics | grep 'lsm_sstable_count{level="L0"}'

# Check compaction rate
curl -s http://localhost:8080/metrics | grep 'lsm_compaction_duration_ms_count'
```

**Root causes:**

1. **Write amplification** (high write rate, slow compaction)
2. **Slow disk** (HDD instead of SSD)
3. **CPU bottleneck** (compaction can't keep up)

**Solution 1: Wait for compaction**

L0 stall is self-resolving (compaction will reduce L0 count).

**Solution 2: Increase compaction concurrency** (requires code change)

Currently hardcoded at 4 concurrent compactions. Future: expose in config.

**Solution 3: Reduce write rate**

```bash
# Throttle writes from client
# (Implement rate limiting in your application)
```

**Prevention:**

- Use SSD/NVMe (not HDD)
- Monitor `lsm_sstable_count{level="L0"}` and alert at >8

---

### Disk Full

**Error:**
```
ERROR: No space left on device (os error 28)
```

**Diagnosis:**

```bash
# Check disk space
df -h /var/lib/norikv

# Check largest directories
du -sh /var/lib/norikv/* | sort -rh | head -10
```

**Solution 1: Free up space**

```bash
# Delete old snapshots (if any)
find /var/lib/norikv/raft/*/snapshot -name "snapshot-*.dat" -mtime +7 -delete

# Truncate large log files
sudo journalctl --vacuum-time=7d
```

**Solution 2: Add more storage**

```bash
# Mount larger disk
sudo mkdir /mnt/norikv-data
sudo mount /dev/sdb1 /mnt/norikv-data

# Update config
# data_dir: "/mnt/norikv-data"

# Migrate data
sudo rsync -av /var/lib/norikv/ /mnt/norikv-data/
```

**Prevention:**

- Monitor disk usage: `disk_free_bytes < 10GB` → alert
- Plan for 3× dataset size (replication + compaction)

---

## Read Failures

### Slow Reads

**Symptoms:**
- p95 GET latency > 50ms
- Timeouts on read requests
- Metrics show `kv_request_duration_ms` high

**Diagnosis:**

```bash
# Check latency percentiles
curl -s http://localhost:8080/metrics | grep kv_request_duration_ms

# Check if reads hitting L0 (cache misses)
curl -s http://localhost:8080/metrics | grep 'lsm_sstable_count{level="L0"}'

# Check compaction backlog
curl -s http://localhost:8080/metrics | grep lsm_compaction_duration_ms_count
```

**Root causes:**

1. **L0 storm** (many L0 files, poor read amplification)
2. **No bloom filters** (checking all SSTables)
3. **Hot key** (single key getting all traffic)
4. **Disk I/O bottleneck**

**Solution 1: Wait for compaction**

L0 files will be compacted to lower levels automatically.

**Solution 2: Increase filter budget** (requires code change)

Currently filter budget is 256 MiB. Increasing improves read performance.

**Solution 3: Add read caching**

Use a caching layer (Redis, Memcached) in front of NoriKV for hot keys.

**Solution 4: Scale reads**

- Use `ConsistencyLevel::Stale` for non-critical reads (reads from followers)
- Add more replicas to distribute read load

---

### Key Not Found (Expected)

**Error:**
```
GET("nonexistent_key") → None
```

**Diagnosis:**

```bash
# Verify key actually exists
grpcurl -plaintext localhost:7447 norikv.Kv/Get \
  -d '{"key":"base64_encoded_key"}'

# Check if key was deleted
# (Look for tombstone in logs)
sudo journalctl -u norikv | grep "delete.*nonexistent_key"
```

**Root causes:**

1. **Key never written** (typo in key name)
2. **Key deleted** (tombstone exists)
3. **Wrong shard** (client routing to wrong shard - SDK bug)

**Solution:**

Verify key spelling and ensure PUT succeeded before GET.

---

## Performance Degradation

### High CPU Usage

**Symptoms:**
- Server using 100% CPU
- Requests timing out
- Metrics show `cpu_usage > 90%`

**Diagnosis:**

```bash
# Check CPU usage
top -p $(pgrep norikv-server)

# Profile CPU usage
sudo perf record -p $(pgrep norikv-server) -g -- sleep 10
sudo perf report
```

**Root causes:**

1. **Compaction storm** (too many concurrent compactions)
2. **Hot shard** (one shard getting all traffic)
3. **Inefficient queries** (large scans)

**Solution 1: Limit compaction concurrency**

Currently hardcoded at 4. Future: expose `io.max_background_compactions` in config.

**Solution 2: Distribute load**

- Use better key distribution (avoid hot keys)
- Add more nodes to cluster

**Solution 3: Vertical scaling**

- Increase CPU cores (scale up instance)

---

### High Memory Usage

**Symptoms:**
- Memory usage growing unbounded
- OOM kills
- Metrics show `memory_usage > 80%`

**Diagnosis:**

```bash
# Check memory usage
free -h
ps aux | grep norikv-server

# Check active shards
curl -s http://localhost:8080/health | jq '.active_shards'

# Check memtable sizes
curl -s http://localhost:8080/metrics | grep lsm_memtable_size_bytes
```

**Root causes:**

1. **Too many active shards** (1024 shards × 64 MiB memtable = 64 GB)
2. **Memtable not flushing** (slow disk writes)
3. **Memory leak** (bug - report it!)

**Solution 1: Reduce active shards**

Shards are created lazily. Workload determines active count.

**Solution 2: Force memtable flush** (manual intervention)

Currently no admin API. Future: `Admin.FlushShard(shard_id)`

**Solution 3: Add more memory**

- Vertical scaling (increase RAM)

---

### Frequent Leader Elections

**Symptoms:**
- Writes timing out sporadically
- Metrics show `raft_leader_changes_total` increasing rapidly
- Logs show "Starting election for term X"

**Diagnosis:**

```bash
# Check election rate
curl -s http://localhost:8080/metrics | grep raft_leader_changes_total

# Check heartbeat failures
sudo journalctl -u norikv | grep "Heartbeat timeout"

# Check network latency
ping -c 10 10.0.1.11
```

**Root causes:**

1. **Network instability** (packet loss, high latency)
2. **Node overloaded** (can't send heartbeats in time)
3. **Split brain** (network partition)

**Solution 1: Tune Raft timeouts for network**

Currently timeouts are hardcoded. Future: expose in config.

**For high-latency networks:**
- heartbeat_interval: 500ms (default: 150ms)
- election_timeout_min: 1000ms (default: 300ms)
- election_timeout_max: 2000ms (default: 600ms)

**Solution 2: Fix network issues**

```bash
# Check for packet loss
ping -c 100 10.0.1.11 | grep loss

# Check MTU mismatches
tracepath 10.0.1.11
```

**Solution 3: Scale up nodes**

Ensure nodes have sufficient CPU to handle Raft heartbeats.

---

## Health Check Failures

### `/health/quick` Returns 503

**Diagnosis:**

```bash
# Check shard 0 specifically
curl -s http://localhost:8080/health | jq '.shards[] | select(.id == 0)'

# Check if shard 0 has leader
curl -s http://localhost:8080/metrics | grep 'raft_leader{shard_id="0"}'
```

**Root causes:**

1. **Shard 0 not created** (server still starting up)
2. **No Raft leader elected** (cluster forming)
3. **Shard 0 unresponsive** (disk I/O blocked)

**Solution:**

Wait 30-60 seconds for server initialization. If persists, check logs:

```bash
sudo journalctl -u norikv -f | grep "shard-0"
```

---

### `/health` Shows `degraded` Status

**Diagnosis:**

```bash
# Check which shards are unhealthy
curl -s http://localhost:8080/health | \
  jq '.shards[] | select(.status != "healthy")'

# Check leadership
curl -s http://localhost:8080/health | \
  jq '.shards[] | select(.is_leader == false)'
```

**Root causes:**

1. **Some shards have no leader** (elections in progress)
2. **Raft replication lag** (follower behind leader)

**Solution:**

If transient (during elections), wait. If persistent, investigate specific shards.

---

## Monitoring & Alerting Issues

### Prometheus Not Scraping

**Symptoms:**
- Grafana dashboards empty
- Prometheus targets showing "DOWN"

**Diagnosis:**

```bash
# Check if /metrics endpoint works
curl http://localhost:8080/metrics

# Check Prometheus targets
curl http://prometheus:9090/api/v1/targets | jq
```

**Root causes:**

1. **Firewall blocking port 8080**
2. **Wrong Prometheus scrape config**
3. **Metrics disabled in config**

**Solution 1: Fix firewall**

```bash
# Allow Prometheus server to access port 8080
sudo iptables -A INPUT -p tcp --dport 8080 -s <prometheus-ip> -j ACCEPT
```

**Solution 2: Fix Prometheus config**

```yaml
# prometheus.yml
scrape_configs:
  - job_name: 'norikv'
    static_configs:
      - targets:
        - '10.0.1.10:8080'  # Correct IP and port
        - '10.0.1.11:8080'
        - '10.0.1.12:8080'
    scrape_interval: 15s
```

**Solution 3: Enable metrics**

```yaml
# config.yaml
telemetry:
  prometheus:
    enabled: true  # Must be true
```

---

### Metrics Missing

**Symptoms:**
- Some metrics not showing up in Prometheus
- `kv_requests_total` is 0 despite traffic

**Diagnosis:**

```bash
# Check all metrics
curl -s http://localhost:8080/metrics | sort

# Check if meter is wired
sudo journalctl -u norikv | grep "Enabling KV metrics"
```

**Root causes:**

1. **No traffic yet** (no requests = no metrics)
2. **Meter not wired** (code bug - should see log "Enabling KV metrics")

**Solution:**

Send test traffic to generate metrics:

```bash
grpcurl -plaintext localhost:7447 norikv.Kv/Put \
  -d '{"key":"dGVzdA==","value":"dmFsdWU="}'

# Check metrics again
curl -s http://localhost:8080/metrics | grep kv_requests_total
```

---

## Data Loss / Corruption

### Missing Data After Restart

**Symptoms:**
- Keys that existed before restart are now gone
- `data_dir` is empty after restart

**Diagnosis:**

```bash
# Check if data dir exists
ls -la /var/lib/norikv/

# Check for WAL/SST files
find /var/lib/norikv -name "*.wal" -o -name "*.sst"
```

**Root causes:**

1. **Wrong data_dir** (config changed)
2. **Data deleted** (manual deletion or script)
3. **Ephemeral storage** (Docker volume not mounted)

**Solution:**

**If data exists on disk:**

```bash
# Fix config to point to correct data_dir
data_dir: "/var/lib/norikv"  # Not /tmp/norikv
```

**If data truly lost:**

Restore from backup (if available). If no backup, data is unrecoverable.

**Prevention:**

- Use persistent storage (not tmpfs, not ephemeral Docker volumes)
- Set up daily backups (snapshot Raft + LSM dirs)
- Monitor disk usage and alerts

---

### Corrupted SSTable

**Error:**
```
ERROR: SSTable checksum mismatch: file corrupted
```

**Diagnosis:**

```bash
# Find corrupted file
sudo journalctl -u norikv | grep "checksum mismatch"

# Check file
ls -lh /var/lib/norikv/lsm/shard-0/sst/L1-000042.sst
```

**Root causes:**

1. **Disk corruption** (bad sectors)
2. **Incomplete write** (power loss during compaction)
3. **Bit rot** (silent data corruption over time)

**Solution:**

Currently no recovery mechanism. Future: Use Raft snapshots to rebuild lost data.

**Prevention:**

- Use ECC RAM
- Use enterprise SSDs with power-loss protection
- Monitor disk SMART metrics (`smartctl -a /dev/sda`)

---

## Diagnostic Commands

### Quick Health Check

```bash
#!/bin/bash
# healthcheck.sh

echo "=== NoriKV Health Check ==="

# 1. Check if server is running
if ! pgrep norikv-server > /dev/null; then
  echo " Server not running"
  exit 1
fi
echo " Server running (PID: $(pgrep norikv-server))"

# 2. Check HTTP endpoint
if ! curl -sf http://localhost:8080/health/quick > /dev/null; then
  echo " Health check failed"
  exit 1
fi
echo " Health check OK"

# 3. Check cluster size
cluster_size=$(curl -s http://localhost:8080/metrics | grep '^swim_cluster_size' | awk '{print $2}')
if [ "$cluster_size" -lt 3 ]; then
  echo "  Warning: Cluster size is $cluster_size (expected 3)"
else
  echo " Cluster size: $cluster_size"
fi

# 4. Check for errors in last 5 minutes
error_count=$(sudo journalctl -u norikv --since "5 minutes ago" | grep -c ERROR || echo 0)
if [ "$error_count" -gt 0 ]; then
  echo "  Warning: $error_count errors in last 5 minutes"
else
  echo " No recent errors"
fi

echo " All checks passed"
```

---

### Performance Profiling

```bash
# CPU profiling (requires perf)
sudo perf record -F 99 -p $(pgrep norikv-server) -g -- sleep 30
sudo perf report

# Memory profiling (requires valgrind)
valgrind --tool=massif --massif-out-file=massif.out norikv-server --config config.yaml

# Trace syscalls
sudo strace -p $(pgrep norikv-server) -c -f

# Network profiling
sudo tcpdump -i any port 7447 -w norikv.pcap
```

---

## Getting Help

### Information to Include in Bug Reports

When reporting issues, please include:

1. **Version:** `norikv-server --version`
2. **OS:** `uname -a`, `cat /etc/os-release`
3. **Config:** `cat /etc/norikv/config.yaml` (redact secrets)
4. **Logs:** Last 1000 lines of logs (`sudo journalctl -u norikv -n 1000`)
5. **Metrics:** `curl http://localhost:8080/metrics`
6. **Health:** `curl http://localhost:8080/health | jq`
7. **Steps to reproduce:** Exact commands that trigger the issue

### Where to Ask

- **GitHub Issues:** https://github.com/norikv/norikv/issues
- **Discord:** https://discord.gg/norikv (future)
- **Email:** support@norikv.io (enterprise support)

---

## Next Steps

- **[Configuration Reference](configuration.md)** - Fine-tune settings
- **[Metrics Reference](metrics.md)** - Understand metrics for debugging
- **[Deployment Guide](deployment.md)** - Proper deployment prevents issues
