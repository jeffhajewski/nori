---
layout: default
title: Hardware Recommendations
parent: Performance
nav_order: 3
---

# Hardware Recommendations
{: .no_toc }

What hardware to use for different performance targets.
{: .fs-6 .fw-300 }

## Table of contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Quick Reference

| Target Workload | CPU | RAM | Disk | Cost |
|----------------|-----|-----|------|------|
| Development/Testing | 2 cores | 4 GB | Any SSD | $ |
| Light Production | 4 cores | 8 GB | SATA SSD | $$ |
| Standard Production | 8 cores | 16 GB | NVMe SSD | $$$ |
| High Performance | 16+ cores | 32 GB | Multiple NVMe | $$$$ |

## Disk (Most Important)

WAL performance is heavily disk-bound. Disk choice has the biggest impact.

### HDD (Not Recommended)

**Performance:**
- Random IOPS: 100-200
- Sequential throughput: 100-200 MB/s
- Fsync latency: 5-10ms

**WAL Performance:**
- Max writes/sec: 100-200 (with `FsyncPolicy::Always`)
- With batching: 10-20K writes/sec
- Recovery: 100 MB/s

**Use case:** Archival/backup only. Not suitable for production WAL.

**Cost:** $20-30 per TB

### SATA SSD

**Performance:**
- Random IOPS: 10K-50K
- Sequential throughput: 300-500 MB/s
- Fsync latency: 0.5-2ms

**WAL Performance:**
- Max writes/sec: 500-1000 (with `FsyncPolicy::Always`)
- With batching: 40-60K writes/sec
- Recovery: 300-500 MB/s

**Use case:** Budget-conscious deployments, light workloads.

**Cost:** $100-150 per TB

**Recommendation:** Samsung 870 EVO, Crucial MX500

### NVMe SSD (Recommended)

**Performance:**
- Random IOPS: 100K-500K
- Sequential throughput: 2-7 GB/s
- Fsync latency: 0.1-0.5ms

**WAL Performance:**
- Max writes/sec: 2000-5000 (with `FsyncPolicy::Always`)
- With batching: 80-120K writes/sec
- Recovery: 2-3 GB/s

**Use case:** Production deployments, standard recommendation.

**Cost:** $150-300 per TB

**Recommendation:**
- Consumer: Samsung 980 Pro, WD Black SN850
- Enterprise: Intel P5520, Samsung PM9A3

### Optane SSD (Premium)

**Performance:**
- Random IOPS: 500K+
- Sequential throughput: 2-3 GB/s
- Fsync latency: <0.1ms (extremely low)

**WAL Performance:**
- Max writes/sec: 10,000+ (with `FsyncPolicy::Always`)
- With batching: 150K+ writes/sec
- Recovery: 3+ GB/s

**Use case:** Latency-critical, high-durability workloads.

**Cost:** $1000+ per TB

**Recommendation:** Intel Optane P5800X

### Disk Comparison

| Metric | HDD | SATA SSD | NVMe SSD | Optane |
|--------|-----|----------|----------|--------|
| Fsync latency | 5-10ms | 0.5-2ms | 0.1-0.5ms | <0.1ms |
| Max writes/sec (Always) | 200 | 1000 | 5000 | 10000 |
| Max writes/sec (Batch) | 20K | 60K | 120K | 150K |
| Recovery (100 MB) | 1s | 300ms | 50ms | 30ms |
| Cost per TB | $ | $$ | $$$ | $$$$ |

**Bottom line:** Use NVMe SSD minimum. Optane if budget allows.

## CPU

WAL is not CPU-intensive, but parallelism helps for recovery and compression.

### 2 Cores (Development)

**Suitable for:**
- Local development
- Testing
- Low-rate workloads (<1K writes/sec)

**Not suitable for:**
- Production
- High concurrency
- Parallel recovery

### 4-8 Cores (Production)

**Suitable for:**
- Standard production workloads
- Up to 50K writes/sec
- Single-node deployments

**Performance:**
- Compression: 4 cores can compress 100K records/sec with LZ4
- Recovery: 4 cores can replay 1M records/sec

**Recommendation:** Intel Xeon Silver, AMD EPYC 7002 series, or equivalent

### 16+ Cores (High Performance)

**Suitable for:**
- >100K writes/sec
- Large-scale distributed systems
- Parallel compaction/recovery

**Performance:**
- Compression: 16 cores can compress 400K records/sec with LZ4
- Recovery: 16 cores can replay 4M records/sec

**Recommendation:** Intel Xeon Gold, AMD EPYC 7003 series

### CPU Comparison

| Cores | Compression (LZ4) | Recovery | Use Case |
|-------|------------------|----------|----------|
| 2 | 50K records/sec | 500K records/sec | Dev/test |
| 4 | 100K records/sec | 1M records/sec | Light prod |
| 8 | 200K records/sec | 2M records/sec | Standard prod |
| 16 | 400K records/sec | 4M records/sec | High perf |

**Bottom line:** 4-8 cores is sufficient for most workloads.

## RAM

WAL uses minimal memory. RAM is mainly for OS page cache and in-memory indexes.

### 4 GB (Minimum)

**Suitable for:**
- Development
- Very small datasets (<1 GB WAL)

**Limitations:**
- Limited OS page cache
- Frequent disk I/O
- Slow recovery

### 8-16 GB (Recommended)

**Suitable for:**
- Production workloads
- WAL size: 10-50 GB
- In-memory indexes for KV stores

**Benefits:**
- OS can cache active segments in RAM
- Fast read access to recent data
- Good recovery performance

### 32+ GB (High Performance)

**Suitable for:**
- Large datasets (>100 GB WAL)
- Multiple concurrent readers
- Complex in-memory state

**Benefits:**
- Entire active segment set cached
- No disk reads for recent data
- Very fast recovery

### RAM Sizing Guide

**Rule of thumb:** RAM ≥ 2x active segment size

```
Active segment size: 128 MB
Min RAM for caching: 256 MB
Recommended: 2-4 GB (for OS + other processes)
```

**For in-memory indexes (KV store):**

```
Keys: 1M
Avg key size: 32 bytes
Avg value size: 512 bytes

Memory for HashMap:
  Keys: 1M × 32 bytes = 32 MB
  Values: 1M × 512 bytes = 512 MB
  Overhead: ~50% = 272 MB
  Total: ~816 MB

Recommended RAM: 2 GB (for index) + 4 GB (for OS/cache) = 6 GB
```

**Bottom line:** 8-16 GB is sufficient for most deployments.

## Network (Distributed Systems)

For replicated WAL (multi-node setups).

### 1 Gbps (Basic)

**Throughput:** 125 MB/s

**Suitable for:**
- Replication lag <1 second
- Write rate <50 MB/s
- 2-3 replicas

**Bottleneck:** Network becomes bottleneck at >50 MB/s sustained write rate.

### 10 Gbps (Recommended)

**Throughput:** 1.25 GB/s

**Suitable for:**
- Standard production
- Write rate <500 MB/s
- 3-5 replicas
- Fast catchup after failures

**Performance:**
- Can replicate 100 MB/s writes to 5 replicas with <100ms lag
- Catchup: 1 GB/s (8 seconds for 8 GB lag)

### 25+ Gbps (High Performance)

**Throughput:** 3+ GB/s

**Suitable for:**
- High-throughput replication
- Many replicas (>5)
- Large bursts

**Performance:**
- Can handle 500+ MB/s sustained writes
- Catchup: 3 GB/s (1 second for 3 GB lag)

**Bottom line:** 10 Gbps is sufficient unless replicating >500 MB/s.

## Storage Configurations

### Single Disk (Development)

```
/data/wal/
  ├── 000000.wal
  ├── 000001.wal
  └── ...
```

**Pros:**
- Simple
- Low cost

**Cons:**
- No redundancy
- Disk failure = data loss

**Use case:** Development, non-critical data

### RAID 1 (Mirroring)

```
/dev/md0 (mirror of /dev/sda1 + /dev/sdb1)
  └── /data/wal/
```

**Pros:**
- Disk redundancy
- Read performance 2x (reads from either disk)

**Cons:**
- Write performance same as single disk
- 50% capacity (2x cost)

**Use case:** Production with durability requirements

### RAID 10 (Stripe + Mirror)

```
/dev/md0 (stripe of two mirrors)
  ├── Mirror 1: /dev/sda1 + /dev/sdb1
  └── Mirror 2: /dev/sdc1 + /dev/sdd1
```

**Pros:**
- Redundancy
- 2x write performance (parallel writes to mirrors)
- 2x read performance

**Cons:**
- 50% capacity (4 disks → 2 disks capacity)
- Higher cost

**Use case:** High-performance production

### Separate WAL and Data Disks

```
/data/wal/     ← Fast NVMe SSD (for writes)
/data/archive/ ← Slower SATA SSD (for old segments)
```

**Strategy:**
1. Write to fast NVMe
2. Move old segments to slower disk for archival

**Pros:**
- Optimize cost (fast disk only for active data)
- Good write performance

**Cons:**
- More complex setup
- Need automated archival

## Cloud Configurations

### AWS

**Recommended instance types:**

| Workload | Instance | Disk | Cost/month |
|----------|----------|------|------------|
| Light | t3.medium | 100 GB gp3 | $50 |
| Standard | m6i.xlarge | 500 GB gp3 | $300 |
| High perf | m6i.2xlarge | 1 TB io2 | $800 |

**Storage options:**
- **gp3:** 3000 IOPS baseline, 125 MB/s. Good for most workloads.
- **io2:** Provisioned IOPS (up to 64K). For high-performance needs.
- **Local NVMe:** Best performance, but ephemeral (data lost on stop).

**Network:** Use Enhanced Networking (10-100 Gbps) for replication.

### GCP

**Recommended instance types:**

| Workload | Instance | Disk | Cost/month |
|----------|----------|------|------------|
| Light | e2-standard-2 | 100 GB pd-ssd | $80 |
| Standard | n2-standard-4 | 500 GB pd-ssd | $350 |
| High perf | n2-standard-8 | 1 TB pd-extreme | $900 |

**Storage options:**
- **pd-ssd:** 30 IOPS/GB. Good balance.
- **pd-extreme:** Provisioned IOPS (up to 100K). For high-performance.
- **Local SSD:** 375 GB, very fast, but ephemeral.

**Network:** 10-32 Gbps depending on instance size.

### Azure

**Recommended instance types:**

| Workload | Instance | Disk | Cost/month |
|----------|----------|------|------------|
| Light | Standard_D2s_v3 | 100 GB Premium SSD | $100 |
| Standard | Standard_D4s_v3 | 500 GB Premium SSD | $400 |
| High perf | Standard_D8s_v3 | 1 TB Ultra Disk | $1000 |

**Storage options:**
- **Premium SSD:** Up to 20K IOPS. Good for most workloads.
- **Ultra Disk:** Up to 160K IOPS. For high-performance.

**Network:** 10-40 Gbps depending on instance size.

## Sizing Examples

### Small Deployment (10K writes/sec)

**Hardware:**
- CPU: 4 cores
- RAM: 8 GB
- Disk: 256 GB NVMe SSD

**Configuration:**

```rust
let config = WalConfig {
    max_segment_size: 128 * 1024 * 1024,
    fsync_policy: FsyncPolicy::Batch(Duration::from_millis(5)),
    preallocate: true,
    node_id: 0,
};
```

**Expected Performance:**
- 10K writes/sec sustained
- 1 GB/day WAL growth
- Recovery time: <1 second for full disk

**Cost:** $200-300/month (cloud)

### Medium Deployment (50K writes/sec)

**Hardware:**
- CPU: 8 cores
- RAM: 16 GB
- Disk: 1 TB NVMe SSD

**Configuration:**

```rust
let config = WalConfig {
    max_segment_size: 256 * 1024 * 1024,
    fsync_policy: FsyncPolicy::Batch(Duration::from_millis(5)),
    preallocate: true,
    node_id: 0,
};
```

**Expected Performance:**
- 50K writes/sec sustained
- 5 GB/day WAL growth
- Recovery time: <5 seconds for full disk

**Cost:** $500-700/month (cloud)

### Large Deployment (200K writes/sec)

**Hardware:**
- CPU: 16 cores
- RAM: 32 GB
- Disk: 2x 2 TB NVMe SSD (RAID 1)

**Configuration:**

```rust
let config = WalConfig {
    max_segment_size: 512 * 1024 * 1024,
    fsync_policy: FsyncPolicy::Batch(Duration::from_millis(10)),
    preallocate: true,
    node_id: 0,
};
```

**Expected Performance:**
- 200K writes/sec sustained
- 20 GB/day WAL growth
- Recovery time: <30 seconds for full disk

**Cost:** $1500-2000/month (cloud)

## Performance Testing

Before deploying, benchmark on your hardware:

### Disk Throughput

```bash
# Sequential write
dd if=/dev/zero of=/path/to/wal/testfile bs=1M count=1000 oflag=direct

# Result: X MB/s
```

### Disk Latency

```bash
# Single fsync
time dd if=/dev/zero of=/path/to/wal/testfile bs=4k count=1 conv=fsync

# Result: X ms
```

### WAL Benchmark

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let config = WalConfig {
        dir: PathBuf::from("/path/to/wal"),
        ..Default::default()
    };

    let (wal, _) = Wal::open(config).await?;

    // Measure write throughput
    let start = Instant::now();
    for i in 0..100_000 {
        wal.append(&Record::put(&i.to_le_bytes(), b"value")).await?;
    }
    let elapsed = start.elapsed();

    println!("Throughput: {} writes/sec", 100_000.0 / elapsed.as_secs_f64());

    Ok(())
}
```

## Conclusion

**For most deployments:**
- **Disk:** NVMe SSD (Samsung 980 Pro or equivalent)
- **CPU:** 4-8 cores
- **RAM:** 8-16 GB
- **Network:** 10 Gbps (if replicated)

**Total cost:** $300-700/month (cloud) or $2000-5000 (on-prem hardware)

This gives 50-100K writes/sec with good durability and recovery characteristics.
