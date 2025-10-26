---
layout: default
title: Cold Storage
parent: Recipes
grand_parent: nori-sstable
nav_order: 3
---

# Cold Storage
{: .no_toc }

Optimizing SSTables for archival and infrequent access.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Scenario

**Characteristics:**
- Infrequent reads (< 1 QPS)
- Large dataset (100GB+)
- Storage cost is primary concern
- Latency requirements relaxed (p95 < 100ms acceptable)

**Examples:**
- Historical logs (>90 days old)
- Backup snapshots
- Regulatory compliance data
- Archive tiers in tiered storage

---

## Optimal Configuration

```rust
use nori_sstable::{SSTableConfig, Compression};

let config = SSTableConfig {
    path: PathBuf::from("archive.sst"),
    estimated_entries: 10_000_000,

    // Maximum compression
    compression: Compression::Zstd,

    // No cache (save RAM)
    block_cache_mb: 0,

    // Larger blocks for better compression
    block_size: 8192,

    // Standard bloom filter (8 bits saves RAM)
    bloom_bits_per_key: 8,

    ..Default::default()
};
```

**Rationale:**
- Zstd: 3.5-5x compression (vs 2.5x LZ4)
- No cache: Save 100MB+ RAM per SSTable
- 8KB blocks: 10-15% better compression ratio
- 8 bits/key bloom: 100K keys = 100KB (vs 125KB)

---

## Compression Savings

### Example Dataset

```
1TB uncompressed data:
  10 billion entries
  100 bytes avg per entry

With LZ4 (2.5x):
  Compressed: 400 GB
  Savings: 600 GB (60%)

With Zstd (3.5x):
  Compressed: 286 GB
  Savings: 714 GB (71%)

Extra savings: 114 GB (11% more than LZ4)
```

**Cost analysis (AWS S3):**
```
Standard tier: $0.023/GB/month

LZ4 (400GB):  $9.20/month
Zstd (286GB): $6.58/month

Annual savings: $31.44/TB
```

---

## Build Time Trade-off

### Compression Speed

```
100GB dataset build:

No compression:
  Write time: 142s (704 MB/s)

LZ4:
  Write time: 178s (+25%)
  Throughput: 562 MB/s

Zstd (level 3):
  Write time: 312s (+120%)
  Throughput: 320 MB/s
```

**Is 2x build time acceptable?**
- Yes! Archive build is one-time cost
- 170s extra = 2.8 minutes for 114GB savings
- Amortized over years of storage

---

## Read Performance

### Cold Read Path

```
Single key lookup (cache disabled):

1. Check bloom filter
   Time: 67ns

2. Binary search index
   Time: 150ns (larger SSTable = more blocks)

3. Read compressed block from disk
   Time: 28µs (1.17KB Zstd vs 4KB uncompressed)

4. Decompress Zstd
   Time: 3.4µs

5. Parse block
   Time: 0.8µs

Total: 32.4µs
```

**Compare to LZ4 (no cache):**
```
LZ4 cold read: 39.8µs
Zstd cold read: 32.4µs

Zstd is 18% FASTER (smaller disk reads!)
```

---

## Range Scan Performance

### Sequential Reads

```
Scan 1 million consecutive entries:

No compression:
  Blocks to read: 20,000 × 4KB = 80MB
  Disk time: 1,905ms (42 MB/s SSD)
  Parse time: 16ms
  Total: 1,921ms

LZ4 (2.5x):
  Blocks to read: 20,000 × 1.6KB = 32MB
  Disk time: 762ms
  Decompress: 21ms (20,000 × 1.05µs)
  Parse time: 16ms
  Total: 799ms (2.4x faster)

Zstd (3.5x):
  Blocks to read: 20,000 × 1.14KB = 22.8MB
  Disk time: 543ms
  Decompress: 68ms (20,000 × 3.4µs)
  Parse time: 16ms
  Total: 627ms (3.1x faster)
```

**Key insight:** Even with slower decompression, Zstd wins on scans!

---

## Larger Block Size

### 8KB Blocks

```rust
let config = SSTableConfig {
    block_size: 8192,  // Double default
    compression: Compression::Zstd,
    ..Default::default()
};
```

**Benefits:**
```
Time-series data (100GB):

4KB blocks:
  Compressed (Zstd): 286 GB (3.5x ratio)

8KB blocks:
  Compressed (Zstd): 256 GB (3.9x ratio)

Extra savings: 30 GB (11% better compression)
```

**Trade-offs:**
```
Point lookup:
  4KB: Read 1.14KB, decompress 3.4µs
  8KB: Read 2.05KB, decompress 6.8µs

  Impact: +3.4µs latency (still < 100ms SLO!)

Cache miss amplification:
  4KB: Miss wastes 4KB RAM if cached
  8KB: Miss wastes 8KB RAM (but cache disabled anyway!)
```

---

## Integration with LSM

### Tiered Storage

```rust
// L0/L1: Hot data (frequent access)
let hot_config = SSTableConfig {
    compression: Compression::Lz4,
    block_cache_mb: 256,
    block_size: 4096,
    ..Default::default()
};

// L2+: Warm data (occasional access)
let warm_config = SSTableConfig {
    compression: Compression::Lz4,
    block_cache_mb: 64,
    block_size: 4096,
    ..Default::default()
};

// Archive tier: Cold data (rare access)
let cold_config = SSTableConfig {
    compression: Compression::Zstd,
    block_cache_mb: 0,
    block_size: 8192,
    bloom_bits_per_key: 8,
    ..Default::default()
};
```

---

### Age-Based Compaction

```rust
pub async fn compact_to_archive(
    old_sstables: Vec<PathBuf>,
    archive_path: PathBuf,
) -> Result<()> {
    let config = SSTableConfig {
        path: archive_path,
        compression: Compression::Zstd,
        block_size: 8192,
        block_cache_mb: 0,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await?;

    // Merge old SSTables
    for sstable in old_sstables {
        let reader = SSTableReader::open(sstable).await?;
        let mut iter = reader.iter_all();

        while let Some(entry) = iter.try_next().await? {
            builder.add(&entry).await?;
        }
    }

    let metadata = builder.finish().await?;
    println!("Archived to {} ({} MB)",
        archive_path.display(),
        metadata.file_size / 1_000_000
    );

    Ok(())
}
```

---

## Network Transfer

### Replication Optimization

```
Replicate 100GB SSTable across 3 datacenters:

Uncompressed (100GB):
  Transfer time: 100GB × 3 = 300GB
  @ 100 Mbps: 6.67 hours
  Cost (inter-region): $3 ($0.01/GB)

LZ4 (40GB):
  Transfer: 40GB × 3 = 120GB
  @ 100 Mbps: 2.67 hours
  Cost: $1.20

Zstd (28.6GB):
  Transfer: 28.6GB × 3 = 85.8GB
  @ 100 Mbps: 1.9 hours
  Cost: $0.86

Savings: $2.14 per SSTable replication
```

---

## S3 Glacier Integration

### Deep Archive

```rust
use aws_sdk_s3::{Client, types::StorageClass};

pub async fn archive_to_glacier(
    local_path: PathBuf,
    bucket: &str,
    key: &str,
) -> Result<()> {
    let client = Client::new(&aws_config::load_from_env().await);

    // Upload SSTable to S3 Glacier Deep Archive
    client.put_object()
        .bucket(bucket)
        .key(key)
        .body(ByteStream::from_path(local_path).await?)
        .storage_class(StorageClass::DeepArchive)
        .send()
        .await?;

    println!("Archived to s3://{}/{}", bucket, key);
    Ok(())
}
```

**Cost comparison (1TB for 1 year):**
```
S3 Standard: $23/month = $276/year
S3 Glacier:  $4/month = $48/year
S3 Deep Archive: $1/month = $12/year

With Zstd (286GB instead of 1TB):
  S3 Standard: $6.58/month = $79/year
  S3 Glacier: $1.14/month = $13.68/year
  S3 Deep Archive: $0.29/month = $3.44/year ✅

Annual savings: $272.56 (vs S3 Standard uncompressed)
```

---

## Monitoring Cold Storage

### Key Metrics

```rust
pub struct ArchiveMetrics {
    pub total_size_bytes: u64,
    pub compression_ratio: f64,
    pub read_count: AtomicU64,
    pub avg_read_latency_us: AtomicU64,
}

impl ArchiveMetrics {
    pub fn report(&self) {
        println!("Archive SSTable Stats:");
        println!("  Size: {} GB", self.total_size_bytes / 1_000_000_000);
        println!("  Compression: {:.1}x", self.compression_ratio);
        println!("  Reads/day: {}", self.read_count.load(Ordering::Relaxed));
        println!("  Avg latency: {}µs", self.avg_read_latency_us.load(Ordering::Relaxed));
    }

    pub fn should_remain_archived(&self) -> bool {
        // If reads/day < 100, keep in archive tier
        self.read_count.load(Ordering::Relaxed) < 100
    }
}
```

---

## Migration Checklist

### Moving to Cold Storage

**Before archiving:**
- [ ] Verify data age (> 90 days old)
- [ ] Confirm read rate (< 1 QPS sustained)
- [ ] Check compliance retention period
- [ ] Estimate compressed size (`file_size / 3.5`)

**During archiving:**
```rust
// 1. Build archive SSTable
let archive_config = SSTableConfig {
    compression: Compression::Zstd,
    block_size: 8192,
    block_cache_mb: 0,
    ..Default::default()
};
let metadata = build_archive(source, archive_config).await?;

// 2. Verify integrity
verify_sstable(&metadata.path).await?;

// 3. Upload to cold storage (S3 Glacier)
upload_to_glacier(&metadata.path).await?;

// 4. Delete local copy (after verification)
std::fs::remove_file(&source)?;
```

**After archiving:**
- [ ] Monitor retrieval latency SLO
- [ ] Track cost savings
- [ ] Set up alerts for unexpected read spikes

---

## Retrieval Strategy

### On-Demand Restore

```rust
pub async fn restore_from_archive(
    s3_key: &str,
    restore_days: u32,
) -> Result<()> {
    let client = Client::new(&aws_config::load_from_env().await);

    // Initiate Glacier restore (takes 12-48 hours)
    client.restore_object()
        .bucket("archive-bucket")
        .key(s3_key)
        .restore_request(
            RestoreRequest::builder()
                .days(restore_days)
                .tier(Tier::Bulk)  // Cheapest (12-48h)
                .build()
        )
        .send()
        .await?;

    println!("Restore initiated. Available in 12-48 hours.");
    Ok(())
}
```

---

## Summary

**Cold storage configuration:**
- Zstd compression (3.5-5x ratio)
- 8KB blocks (11% better compression)
- No cache (save RAM)
- 8 bits/key bloom (save RAM)

**Benefits:**
- 71% space savings (vs uncompressed)
- 18% faster reads (vs LZ4, due to smaller disk I/O)
- $31/TB/year savings on cloud storage
- 3.1x faster range scans

**Use when:**
- Read rate < 1 QPS
- Data age > 90 days
- Storage cost matters more than latency
- p95 latency SLO > 50ms
