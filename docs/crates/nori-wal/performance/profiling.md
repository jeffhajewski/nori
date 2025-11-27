# Performance Profiling

How to measure and analyze nori-wal performance in your application.

## Table of contents

---

## Quick Start

Add basic timing to your code:

```rust
use std::time::Instant;

// Measure write latency
let start = Instant::now();
wal.append(&record).await?;
let latency = start.elapsed();
println!("Write latency: {:?}", latency);

// Measure throughput
let start = Instant::now();
for i in 0..10_000 {
    wal.append(&Record::put(&i.to_le_bytes(), b"value")).await?;
}
let elapsed = start.elapsed();
println!("Throughput: {} writes/sec", 10_000.0 / elapsed.as_secs_f64());
```

## Metrics to Track

### Write Performance

**Append latency (p50, p95, p99):**

```rust
use hdrhistogram::Histogram;

let mut histogram = Histogram::<u64>::new(3)?;

for _ in 0..100_000 {
    let start = Instant::now();
    wal.append(&record).await?;
    histogram.record(start.elapsed().as_micros() as u64)?;
}

println!("p50: {}µs", histogram.value_at_quantile(0.50));
println!("p95: {}µs", histogram.value_at_quantile(0.95));
println!("p99: {}µs", histogram.value_at_quantile(0.99));
```

**Typical values:**
- p50: 8-15µs (in-memory write)
- p95: 20-50µs
- p99: 100µs - 5ms (depends on fsync policy)

**Fsync latency:**

```rust
let start = Instant::now();
wal.sync().await?;
let fsync_latency = start.elapsed();
println!("Fsync latency: {:?}", fsync_latency);
```

**Typical values:**
- Clean file: 10-50µs
- 1 MB dirty: 1-2ms
- 10 MB dirty: 5-10ms

**Throughput (writes/sec):**

```rust
let start = Instant::now();
let count = 10_000;

for i in 0..count {
    wal.append(&Record::put(&i.to_le_bytes(), b"value")).await?;
}

let throughput = count as f64 / start.elapsed().as_secs_f64();
println!("Throughput: {:.0} writes/sec", throughput);
```

**Typical values:**
- With `FsyncPolicy::Always`: 400-500 writes/sec
- With `FsyncPolicy::Batch(5ms)`: 80-100K writes/sec
- With `FsyncPolicy::Os`: 100-120K writes/sec

### Read Performance

**Sequential scan throughput:**

```rust
let start = Instant::now();
let mut count = 0u64;
let mut bytes = 0u64;

let mut reader = wal.read_from(Position { segment_id: 0, offset: 0 }).await?;
while let Some((record, _)) = reader.next_record().await? {
    count += 1;
    bytes += record.key.len() as u64 + record.value.len() as u64;
}

let elapsed = start.elapsed();
let throughput_mbs = (bytes as f64 / 1_000_000.0) / elapsed.as_secs_f64();

println!("Read {} records in {:?}", count, elapsed);
println!("Throughput: {:.1} MB/s", throughput_mbs);
```

**Typical values:**
- Sequential read: 50-100 MB/s (decode + CRC validation bottleneck)
- Raw disk read: 2-7 GB/s (NVMe)

### Recovery Performance

**Recovery time:**

```rust
let start = Instant::now();
let (wal, recovery_info) = Wal::open(config).await?;
let elapsed = start.elapsed();

println!("Recovery complete:");
println!("  Time: {:?}", elapsed);
println!("  Records: {}", recovery_info.valid_records);
println!("  Segments: {}", recovery_info.segments_scanned);
println!("  Throughput: {:.1} GiB/s",
    (recovery_info.bytes_scanned as f64 / 1_073_741_824.0) / elapsed.as_secs_f64()
);
```

**Typical values:**
- 10 MB: 2-5ms (3.3 GiB/s)
- 100 MB: 30ms
- 1 GB: 300ms

### Segment Operations

**Rotation overhead:**

```rust
// Track when rotation happens
let old_segment_id = wal.current_segment_id();

let start = Instant::now();
wal.append(&large_record).await?;  // Triggers rotation
let elapsed = start.elapsed();

if wal.current_segment_id() != old_segment_id {
    println!("Segment rotation took: {:?}", elapsed);
}
```

**Typical values:**
- With pre-allocation: 10-15ms
- Without pre-allocation: 2-3ms

**Segment deletion:**

```rust
let start = Instant::now();
wal.delete_segments_before(segment_id).await?;
let elapsed = start.elapsed();
println!("Deleted segments in {:?}", elapsed);
```

## Profiling Tools

### CPU Profiling

Use `cargo flamegraph` to identify hot paths:

```bash
cargo install flamegraph

# Profile your application
cargo flamegraph --bin your_app

# Open flamegraph.svg in browser
```

**What to look for:**
- High time in `write_all()` → Disk bottleneck
- High time in `crc32()` → Consider larger records (amortize CRC cost)
- High time in `compress()` → Consider disabling compression

### Memory Profiling

Use `heaptrack` or `valgrind` to track allocations:

```bash
# Install heaptrack
sudo apt install heaptrack

# Profile
heaptrack ./target/release/your_app

# Analyze
heaptrack_gui heaptrack.your_app.*.gz
```

**What to look for:**
- Large allocations in hot path → Use object pools
- Memory growth over time → Memory leak

### Disk I/O Profiling

#### Linux (iostat)

```bash
iostat -x 1

# Look at:
# - %util: Disk utilization (>80% = bottleneck)
# - await: Average I/O latency (>10ms = slow)
# - w/s: Writes per second
# - wMB/s: Write throughput
```

#### macOS (fs_usage)

```bash
sudo fs_usage -f filesys -w

# Filter for your process:
sudo fs_usage -f filesys -w | grep your_app
```

#### Windows (Performance Monitor)

```powershell
perfmon /res

# Add counters:
# - PhysicalDisk: Disk Writes/sec
# - PhysicalDisk: Avg. Disk Write Queue Length
# - PhysicalDisk: % Disk Time
```

### Network I/O (Replication)

#### Linux (iftop)

```bash
sudo iftop -i eth0

# Shows:
# - Bandwidth usage per connection
# - Peak/avg throughput
```

#### Application-level

```rust
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

let bytes_sent = Arc::new(AtomicU64::new(0));

// In replication code:
bytes_sent.fetch_add(record.len() as u64, Ordering::Relaxed);

// Periodically report:
let sent = bytes_sent.swap(0, Ordering::Relaxed);
println!("Replication throughput: {} MB/s", sent / 1_000_000);
```

## Benchmarking Framework

Create a reusable benchmarking harness:

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use nori_wal::*;

fn bench_append(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("append");

    for size in [100, 1024, 10_240] {
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &size,
            |b, &size| {
                b.to_async(&rt).iter(|| async {
                    let config = WalConfig {
                        dir: PathBuf::from("/tmp/bench_wal"),
                        fsync_policy: FsyncPolicy::Os,
                        ..Default::default()
                    };

                    let (wal, _) = Wal::open(config).await.unwrap();
                    let record = Record::put(b"key", &vec![0u8; size]);

                    black_box(wal.append(&record).await.unwrap());
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_append);
criterion_main!(benches);
```

Run benchmarks:

```bash
cargo bench

# Results:
# append/100    9.8 µs
# append/1024   11.2 µs
# append/10240  45.3 µs
```

## Production Monitoring

### Prometheus Metrics

Export metrics for monitoring:

```rust
use prometheus::{Counter, Histogram, Gauge, register_counter, register_histogram, register_gauge};

lazy_static! {
    static ref APPEND_DURATION: Histogram = register_histogram!(
        "wal_append_duration_seconds",
        "Time to append a record"
    ).unwrap();

    static ref APPEND_TOTAL: Counter = register_counter!(
        "wal_append_total",
        "Total number of appends"
    ).unwrap();

    static ref FSYNC_DURATION: Histogram = register_histogram!(
        "wal_fsync_duration_seconds",
        "Time to fsync"
    ).unwrap();

    static ref ACTIVE_SEGMENT_SIZE: Gauge = register_gauge!(
        "wal_active_segment_bytes",
        "Size of active segment in bytes"
    ).unwrap();

    static ref SEGMENT_ROTATIONS: Counter = register_counter!(
        "wal_segment_rotations_total",
        "Total number of segment rotations"
    ).unwrap();
}

// In append:
let timer = APPEND_DURATION.start_timer();
wal.append(&record).await?;
timer.observe_duration();
APPEND_TOTAL.inc();

// In sync:
let timer = FSYNC_DURATION.start_timer();
wal.sync().await?;
timer.observe_duration();

// Periodically update:
ACTIVE_SEGMENT_SIZE.set(wal.current_segment_size() as f64);
```

### OpenTelemetry Tracing

Add distributed tracing:

```rust
use tracing::{info_span, instrument};

#[instrument]
pub async fn append(&mut self, record: &Record) -> Result<()> {
    let span = info_span!("wal.append", record_size = record.len());
    let _enter = span.enter();

    // ... append logic ...

    Ok(())
}
```

View traces in Jaeger/Zipkin:

```
Request                             [==================] 45ms
  ├─ wal.append                    [====              ]  8ms
  │   ├─ serialize                 [==                ]  2ms
  │   ├─ write                     [==                ]  3ms
  │   └─ crc                       [=                 ]  1ms
  └─ wal.sync                      [              ====] 35ms
      └─ fsync                     [              ====] 35ms
```

### Custom Dashboards

Track key metrics:

**Grafana Dashboard:**

```json
{
  "panels": [
    {
      "title": "Write Throughput",
      "targets": [
        {
          "expr": "rate(wal_append_total[1m])"
        }
      ]
    },
    {
      "title": "Append Latency (p99)",
      "targets": [
        {
          "expr": "histogram_quantile(0.99, wal_append_duration_seconds)"
        }
      ]
    },
    {
      "title": "Fsync Latency",
      "targets": [
        {
          "expr": "rate(wal_fsync_duration_seconds_sum[1m]) / rate(wal_fsync_duration_seconds_count[1m])"
        }
      ]
    },
    {
      "title": "Segment Size",
      "targets": [
        {
          "expr": "wal_active_segment_bytes"
        }
      ]
    }
  ]
}
```

## Troubleshooting Performance Issues

### Symptom: Low Throughput

**Check:**

1. Fsync policy:
```rust
println!("Fsync policy: {:?}", wal.config().fsync_policy);
```

2. Batching:
```rust
// Are you syncing after every write?
for record in records {
    wal.append(&record).await?;
    wal.sync().await?;  // ← This kills throughput
}
```

3. Disk speed:
```bash
iostat -x 1
# Look at %util and wMB/s
```

**Fix:**
- Use `FsyncPolicy::Batch(5ms)`
- Batch multiple appends before sync
- Upgrade to faster disk

### Symptom: High p99 Latency

**Check:**

1. Segment rotation:
```rust
// Add logging
println!("Segment rotation: {:?}", elapsed);
```

2. Fsync spikes:
```bash
# Linux
sudo iotop -ao
```

3. CPU throttling:
```bash
# Linux
cat /proc/cpuinfo | grep MHz
```

**Fix:**
- Increase segment size
- Monitor for GC pauses (if using JVM languages)
- Check for thermal throttling

### Symptom: Slow Recovery

**Check:**

1. Segment count:
```bash
ls -l /path/to/wal/ | wc -l
```

2. CRC validation time:
```rust
let start = Instant::now();
let (wal, info) = Wal::open(config).await?;
println!("Validated {} records in {:?}", info.valid_records, start.elapsed());
```

**Fix:**
- Reduce number of segments (increase segment size)
- Run compaction to merge segments
- Use parallel recovery (if available)

## Performance Testing Checklist

Before deploying:

- [ ] Benchmarked on target hardware
- [ ] Measured p99 latency under load
- [ ] Tested recovery time with realistic data size
- [ ] Profiled CPU usage (no hot spots >20%)
- [ ] Profiled memory usage (no leaks)
- [ ] Monitored disk I/O (utilization <80%)
- [ ] Tested segment rotation overhead (<10ms)
- [ ] Set up production monitoring (Prometheus/Grafana)
- [ ] Created alerts for slow operations
- [ ] Load tested for 24+ hours

## Conclusion

Key metrics to monitor:

1. **Append latency (p99):** Should be <10ms
2. **Throughput:** Should match workload requirements
3. **Fsync latency:** Should be <5ms
4. **Recovery time:** Should be <1 second per GB

Use profiling tools to identify bottlenecks:
- **CPU:** flamegraph
- **Memory:** heaptrack
- **Disk:** iostat
- **Application:** Prometheus + Grafana

For more details, see:
- [Benchmarks](benchmarks) - Performance measurements
- [Tuning Guide](tuning) - Optimization strategies
- [Hardware](hardware) - Hardware selection
