# Performance

Benchmarks, optimization guides, and performance tuning for nori-wal.

This section helps you understand nori-wal's performance characteristics and optimize for your workload.

## Quick Reference

**Typical Performance (Apple M2 Pro, NVMe SSD):**

| Operation | Throughput | Latency p99 |
|-----------|-----------|-------------|
| Sequential writes (OS fsync) | 110K writes/sec | 15µs |
| Sequential writes (Batch 5ms) | 86K writes/sec | 5.5ms |
| Batch append (1000 records) | 102 MiB/s | 9.5ms |
| Sequential reads | 52 MiB/s | N/A |
| Recovery (10 MB) | 3.3 GiB/s | 3ms |

## Sections

### [Benchmarks](benchmarks.md)
Complete benchmark results with methodology and hardware details.

### [Tuning Guide](tuning.md)
How to optimize nori-wal for your specific workload.

### [Hardware Recommendations](hardware.md)
What hardware to use for different performance targets.

### [Profiling](profiling.md)
How to measure and analyze performance in your application.

## Performance Philosophy

nori-wal is designed with these priorities:

**1. Correctness First**
- Never sacrifice durability for speed
- Performance within correctness constraints

**2. Optimize Hot Paths**
- Append and sync are heavily optimized
- Recovery and startup can be slower

**3. Predictable Performance**
- No hidden allocation spikes
- Configurable trade-offs
- No "magic" heuristics

**4. Real-World Workloads**
- Optimized for typical database/queue patterns
- Not micro-benchmark optimized

## When to Care About Performance

**You should optimize if:**
- Writing >10K records/sec
- Have strict latency requirements (<10ms p99)
- Limited disk I/O budget
- Need maximum throughput

**You probably don't need to optimize if:**
- Writing <1K records/sec
- Latency requirements >100ms
- Have spare I/O capacity
- Durability is more important than speed

## Common Performance Questions

### "Why is my throughput low?"

Check these first:

1. **Fsync policy** - `FsyncPolicy::Always` is slow by design
2. **Batch size** - Are you batching writes?
3. **Segment size** - Too small segments cause frequent rotation
4. **Disk** - Is your disk actually fast? (check `iostat`)

See the [Tuning Guide](tuning.md) for details.

### "Why does latency spike?"

Common causes:

1. **Fsync batch window** - Batch(5ms) means p99 ≈ 5ms
2. **Segment rotation** - 10-50ms every 30-60 seconds
3. **OS page cache flush** - Background disk sync
4. **CPU steal** - Virtualized environments

See [Profiling](profiling.md) for how to diagnose.

### "Can I make it faster?"

Options:

1. **Use batching** - Amortize fsync cost across multiple writes
2. **Increase segment size** - Reduce rotation overhead
3. **Use compression** - Reduce I/O volume
4. **Use faster storage** - NVMe >> SATA >> HDD
5. **Tune fsync policy** - Trade durability for speed

See [Tuning Guide](tuning.md) for strategies.

## Benchmark Methodology

All benchmarks follow these principles:

**1. Realistic Workloads**
- Mix of record sizes
- Typical access patterns
- Include cold starts

**2. Reproducible**
- Fixed hardware configuration
- Documented OS settings
- Repeatable steps

**3. Transparent**
- Show source code
- Explain methodology
- Include variance

**4. Fair Comparisons**
- Apples-to-apples configurations
- Same durability guarantees
- Realistic scenarios

## Hardware Used

Unless otherwise noted, benchmarks use:

```
CPU: Apple M2 Pro (10 cores, 3.5 GHz)
RAM: 16 GB
Disk: 1 TB NVMe SSD
OS: macOS 14.0
Filesystem: APFS
```

Your results will vary based on hardware.

## Further Reading

- **[Benchmarks](benchmarks.md)** - Detailed performance measurements
- **[Tuning Guide](tuning.md)** - Optimization strategies
- **[Hardware](hardware.md)** - Hardware selection guide
- **[Profiling](profiling.md)** - Measurement and analysis
