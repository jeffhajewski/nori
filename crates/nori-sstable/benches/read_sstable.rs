use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nori_sstable::{Entry, SSTableBuilder, SSTableConfig, SSTableReader};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::runtime::Runtime;

// Helper to build an SSTable with specific number of entries
async fn build_sstable(dir: &TempDir, num_entries: usize) -> std::path::PathBuf {
    let path = dir.path().join("bench.sst");

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: num_entries,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..num_entries {
        let key = format!("key_{:08}", i);
        let value = "x".repeat(256);
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    builder.finish().await.unwrap();
    path
}

fn sequential_scan(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("sequential_scan");

    for size in [100, 1_000, 10_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let dir = TempDir::new().unwrap();
            let path = rt.block_on(build_sstable(&dir, size));
            let reader = rt.block_on(async { Arc::new(SSTableReader::open(path).await.unwrap()) });

            b.iter(|| {
                rt.block_on(async {
                    let mut iter = reader.clone().iter();
                    let mut count = 0;

                    while let Some(entry) = iter.try_next().await.unwrap() {
                        black_box(&entry);
                        count += 1;
                    }

                    assert_eq!(count, size);
                });
            });
        });
    }

    group.finish();
}

fn range_scan(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("range_scan");
    group.throughput(Throughput::Elements(10_000));

    // Build once, scan different ranges
    let dir = TempDir::new().unwrap();
    let path = rt.block_on(build_sstable(&dir, 10_000));
    let reader = rt.block_on(async { Arc::new(SSTableReader::open(path).await.unwrap()) });

    // 10% range
    group.bench_function("10pct_range", |b| {
        b.iter(|| {
            rt.block_on(async {
                let start = format!("key_{:08}", 4000);
                let end = format!("key_{:08}", 5000);
                let mut iter = reader.clone().iter_range(start.into(), end.into());
                let mut count = 0;

                while let Some(entry) = iter.try_next().await.unwrap() {
                    black_box(&entry);
                    count += 1;
                }

                assert_eq!(count, 1000);
            });
        });
    });

    // 50% range
    group.bench_function("50pct_range", |b| {
        b.iter(|| {
            rt.block_on(async {
                let start = format!("key_{:08}", 2500);
                let end = format!("key_{:08}", 7500);
                let mut iter = reader.clone().iter_range(start.into(), end.into());
                let mut count = 0;

                while let Some(entry) = iter.try_next().await.unwrap() {
                    black_box(&entry);
                    count += 1;
                }

                assert_eq!(count, 5000);
            });
        });
    });

    // 90% range
    group.bench_function("90pct_range", |b| {
        b.iter(|| {
            rt.block_on(async {
                let start = format!("key_{:08}", 500);
                let end = format!("key_{:08}", 9500);
                let mut iter = reader.clone().iter_range(start.into(), end.into());
                let mut count = 0;

                while let Some(entry) = iter.try_next().await.unwrap() {
                    black_box(&entry);
                    count += 1;
                }

                assert_eq!(count, 9000);
            });
        });
    });

    group.finish();
}

fn point_lookup_hit(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("point_lookup_hit");
    group.throughput(Throughput::Elements(1));

    for size in [100, 1_000, 10_000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let dir = TempDir::new().unwrap();
            let path = rt.block_on(build_sstable(&dir, size));
            let reader = rt.block_on(async { Arc::new(SSTableReader::open(path).await.unwrap()) });

            b.iter(|| {
                rt.block_on(async {
                    // Lookup middle key
                    let key = format!("key_{:08}", size / 2);
                    let entry = reader.get(black_box(key.as_bytes())).await.unwrap();
                    assert!(entry.is_some());
                    black_box(entry);
                });
            });
        });
    }

    group.finish();
}

fn point_lookup_miss(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("point_lookup_miss");
    group.throughput(Throughput::Elements(1));

    for size in [100, 1_000, 10_000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let dir = TempDir::new().unwrap();
            let path = rt.block_on(build_sstable(&dir, size));
            let reader = rt.block_on(async { Arc::new(SSTableReader::open(path).await.unwrap()) });

            b.iter(|| {
                rt.block_on(async {
                    // Lookup non-existent key (should be fast due to bloom filter)
                    let key = format!("missing_{:08}", size * 10);
                    let entry = reader.get(black_box(key.as_bytes())).await.unwrap();
                    assert!(entry.is_none());
                    black_box(entry);
                });
            });
        });
    }

    group.finish();
}

fn multi_block_lookups(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("multi_block_lookups");
    group.throughput(Throughput::Elements(100));

    let dir = TempDir::new().unwrap();
    let path = rt.block_on(build_sstable(&dir, 10_000));
    let reader = rt.block_on(async { Arc::new(SSTableReader::open(path).await.unwrap()) });

    group.bench_function("100_random_lookups", |b| {
        b.iter(|| {
            rt.block_on(async {
                // Lookup keys spread across different blocks
                for i in (0..10_000).step_by(100) {
                    let key = format!("key_{:08}", i);
                    let entry = reader.get(black_box(key.as_bytes())).await.unwrap();
                    assert!(entry.is_some());
                    black_box(entry);
                }
            });
        });
    });

    group.finish();
}

fn hot_key_pattern(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("hot_key_pattern");
    group.throughput(Throughput::Elements(1000));

    let dir = TempDir::new().unwrap();
    let path = rt.block_on(build_sstable(&dir, 10_000));
    let reader = rt.block_on(async { Arc::new(SSTableReader::open(path).await.unwrap()) });

    group.bench_function("80_20_pattern", |b| {
        b.iter(|| {
            rt.block_on(async {
                // 80% of lookups hit 20% of keys (first 2000 keys)
                for i in 0..1000 {
                    let key_idx = if i % 5 < 4 {
                        // 80% of time, hit first 2000 keys
                        (i / 5) % 2000
                    } else {
                        // 20% of time, hit remaining 8000 keys
                        2000 + ((i / 5) % 8000)
                    };

                    let key = format!("key_{:08}", key_idx);
                    let entry = reader.get(black_box(key.as_bytes())).await.unwrap();
                    assert!(entry.is_some());
                    black_box(entry);
                }
            });
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    sequential_scan,
    range_scan,
    point_lookup_hit,
    point_lookup_miss,
    multi_block_lookups,
    hot_key_pattern,
);
criterion_main!(benches);
