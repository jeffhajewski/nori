//! Benchmark suite for range scan operations.
//!
//! Tests:
//! - Range scan throughput
//! - Varying range sizes
//! - Scans across different levels (memtable, L0, L1+)

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use nori_lsm::{ATLLConfig, LsmEngine};
use tempfile::TempDir;

/// Helper to create a test engine with temporary directory.
async fn create_engine() -> (LsmEngine, TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = ATLLConfig::default();
    config.data_dir = temp_dir.path().to_path_buf();
    config.memtable.flush_trigger_bytes = 64 * 1024 * 1024; // 64MB

    let engine = LsmEngine::open(config).await.unwrap();
    (engine, temp_dir)
}

/// Benchmark range scans from memtable.
fn bench_range_scan_memtable(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("range_scan_memtable");

    for num_keys in [10, 100, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            num_keys,
            |b, &num_keys| {
                let (engine, _temp) = rt.block_on(async {
                    let (engine, temp) = create_engine().await;

                    // Pre-populate with sequential keys
                    for i in 0..10000 {
                        let key = Bytes::from(format!("key-{:010}", i));
                        let value = Bytes::from(vec![0u8; 1024]);
                        engine.put(key, value).await.unwrap();
                    }

                    (engine, temp)
                });

                b.iter(|| {
                    rt.block_on(async {
                        let start_key = format!("key-{:010}", 5000);
                        let end_key = format!("key-{:010}", 5000 + num_keys);

                        let mut iter = engine
                            .iter_range(
                                black_box(start_key.as_bytes()),
                                black_box(end_key.as_bytes()),
                            )
                            .await
                            .unwrap();

                        let mut count = 0;
                        while let Some(_) = iter.next().await.unwrap() {
                            count += 1;
                        }

                        black_box(count);
                    });
                });
            },
        );
    }

    group.finish();
}

/// Benchmark range scans from L0.
fn bench_range_scan_l0(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("range_scan_l0");

    for num_keys in [10, 100, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            num_keys,
            |b, &num_keys| {
                let (engine, _temp) = rt.block_on(async {
                    let (engine, temp) = create_engine().await;

                    // Pre-populate with sequential keys
                    for i in 0..10000 {
                        let key = Bytes::from(format!("key-{:010}", i));
                        let value = Bytes::from(vec![0u8; 1024]);
                        engine.put(key, value).await.unwrap();
                    }

                    // Flush to L0
                    engine.flush().await.unwrap();

                    (engine, temp)
                });

                b.iter(|| {
                    rt.block_on(async {
                        let start_key = format!("key-{:010}", 5000);
                        let end_key = format!("key-{:010}", 5000 + num_keys);

                        let mut iter = engine
                            .iter_range(
                                black_box(start_key.as_bytes()),
                                black_box(end_key.as_bytes()),
                            )
                            .await
                            .unwrap();

                        let mut count = 0;
                        while let Some(_) = iter.next().await.unwrap() {
                            count += 1;
                        }

                        black_box(count);
                    });
                });
            },
        );
    }

    group.finish();
}

/// Benchmark full table scan.
fn bench_full_scan(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("full_scan_10k_keys", |b| {
        let (engine, _temp) = rt.block_on(async {
            let (engine, temp) = create_engine().await;

            // Pre-populate with 10k keys
            for i in 0..10000 {
                let key = Bytes::from(format!("key-{:010}", i));
                let value = Bytes::from(vec![0u8; 1024]);
                engine.put(key, value).await.unwrap();
            }

            (engine, temp)
        });

        b.iter(|| {
            rt.block_on(async {
                let mut iter = engine
                    .iter_range(black_box(b""), black_box(b"~"))
                    .await
                    .unwrap();

                let mut count = 0;
                while let Some(_) = iter.next().await.unwrap() {
                    count += 1;
                }

                black_box(count);
            });
        });
    });
}

/// Benchmark range scan with sparse data.
fn bench_sparse_range_scan(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("sparse_scan_1000_gaps", |b| {
        let (engine, _temp) = rt.block_on(async {
            let (engine, temp) = create_engine().await;

            // Pre-populate with keys that have gaps
            for i in (0..10000).step_by(10) {
                let key = Bytes::from(format!("key-{:010}", i));
                let value = Bytes::from(vec![0u8; 1024]);
                engine.put(key, value).await.unwrap();
            }

            (engine, temp)
        });

        b.iter(|| {
            rt.block_on(async {
                let mut iter = engine
                    .iter_range(black_box(b"key-0000005000"), black_box(b"key-0000006000"))
                    .await
                    .unwrap();

                let mut count = 0;
                while let Some(_) = iter.next().await.unwrap() {
                    count += 1;
                }

                black_box(count);
            });
        });
    });
}

criterion_group!(
    benches,
    bench_range_scan_memtable,
    bench_range_scan_l0,
    bench_full_scan,
    bench_sparse_range_scan
);
criterion_main!(benches);
