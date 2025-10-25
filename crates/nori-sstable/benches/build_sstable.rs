use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nori_sstable::{Entry, SSTableBuilder, SSTableConfig};
use tempfile::TempDir;
use tokio::runtime::Runtime;

fn build_small_sstable(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("build_small");
    group.throughput(Throughput::Elements(100));

    group.bench_function("100_entries_100b", |b| {
        b.iter(|| {
            rt.block_on(async {
                let dir = TempDir::new().unwrap();
                let path = dir.path().join("bench.sst");

                let config = SSTableConfig {
                    path,
                    estimated_entries: 100,
                    ..Default::default()
                };

                let mut builder = SSTableBuilder::new(config).await.unwrap();

                for i in 0..100 {
                    let key = format!("key_{:08}", i);
                    let value = "x".repeat(100);
                    builder
                        .add(&Entry::put(black_box(key), black_box(value)))
                        .await
                        .unwrap();
                }

                builder.finish().await.unwrap();
            });
        });
    });

    group.finish();
}

fn build_medium_sstable(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("build_medium");
    group.throughput(Throughput::Elements(10_000));
    group.sample_size(10); // Fewer samples for longer benchmark

    group.bench_function("10k_entries_1kb", |b| {
        b.iter(|| {
            rt.block_on(async {
                let dir = TempDir::new().unwrap();
                let path = dir.path().join("bench.sst");

                let config = SSTableConfig {
                    path,
                    estimated_entries: 10_000,
                    ..Default::default()
                };

                let mut builder = SSTableBuilder::new(config).await.unwrap();

                for i in 0..10_000 {
                    let key = format!("key_{:08}", i);
                    let value = "x".repeat(1024);
                    builder
                        .add(&Entry::put(black_box(key), black_box(value)))
                        .await
                        .unwrap();
                }

                builder.finish().await.unwrap();
            });
        });
    });

    group.finish();
}

fn build_large_sstable(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("build_large");
    group.throughput(Throughput::Elements(100_000));
    group.sample_size(10); // Fewer samples for long benchmark

    group.bench_function("100k_entries_4kb", |b| {
        b.iter(|| {
            rt.block_on(async {
                let dir = TempDir::new().unwrap();
                let path = dir.path().join("bench.sst");

                let config = SSTableConfig {
                    path,
                    estimated_entries: 100_000,
                    ..Default::default()
                };

                let mut builder = SSTableBuilder::new(config).await.unwrap();

                for i in 0..100_000 {
                    let key = format!("key_{:08}", i);
                    let value = "x".repeat(4096);
                    builder
                        .add(&Entry::put(black_box(key), black_box(value)))
                        .await
                        .unwrap();
                }

                builder.finish().await.unwrap();
            });
        });
    });

    group.finish();
}

fn build_many_small_blocks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("build_many_blocks");
    group.throughput(Throughput::Elements(1_000));

    group.bench_function("1k_entries_small_blocks", |b| {
        b.iter(|| {
            rt.block_on(async {
                let dir = TempDir::new().unwrap();
                let path = dir.path().join("bench.sst");

                let config = SSTableConfig {
                    path,
                    estimated_entries: 1_000,
                    block_size: 512, // Small blocks to force many flushes
                    ..Default::default()
                };

                let mut builder = SSTableBuilder::new(config).await.unwrap();

                for i in 0..1_000 {
                    let key = format!("key_{:08}", i);
                    let value = "x".repeat(200);
                    builder
                        .add(&Entry::put(black_box(key), black_box(value)))
                        .await
                        .unwrap();
                }

                builder.finish().await.unwrap();
            });
        });
    });

    group.finish();
}

fn build_with_tombstones(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("build_tombstones");
    group.throughput(Throughput::Elements(1_000));

    group.bench_function("1k_entries_10pct_tombstones", |b| {
        b.iter(|| {
            rt.block_on(async {
                let dir = TempDir::new().unwrap();
                let path = dir.path().join("bench.sst");

                let config = SSTableConfig {
                    path,
                    estimated_entries: 1_000,
                    ..Default::default()
                };

                let mut builder = SSTableBuilder::new(config).await.unwrap();

                for i in 0..1_000 {
                    let key = format!("key_{:08}", i);

                    if i % 10 == 0 {
                        // 10% tombstones
                        builder.add(&Entry::delete(black_box(key))).await.unwrap();
                    } else {
                        let value = "x".repeat(100);
                        builder
                            .add(&Entry::put(black_box(key), black_box(value)))
                            .await
                            .unwrap();
                    }
                }

                builder.finish().await.unwrap();
            });
        });
    });

    group.finish();
}

fn build_varying_sizes(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("build_varying_sizes");

    for size in [100, 1_000, 10_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                rt.block_on(async {
                    let dir = TempDir::new().unwrap();
                    let path = dir.path().join("bench.sst");

                    let config = SSTableConfig {
                        path,
                        estimated_entries: size,
                        ..Default::default()
                    };

                    let mut builder = SSTableBuilder::new(config).await.unwrap();

                    for i in 0..size {
                        let key = format!("key_{:08}", i);
                        let value = "x".repeat(256);
                        builder
                            .add(&Entry::put(black_box(key), black_box(value)))
                            .await
                            .unwrap();
                    }

                    builder.finish().await.unwrap();
                });
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    build_small_sstable,
    build_medium_sstable,
    build_large_sstable,
    build_many_small_blocks,
    build_with_tombstones,
    build_varying_sizes,
);
criterion_main!(benches);
