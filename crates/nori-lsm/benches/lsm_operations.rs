//! Benchmark suite for basic LSM operations.
//!
//! Tests:
//! - PUT latency (memtable writes)
//! - GET latency from different levels (memtable, L0, L1+)
//! - DELETE latency
//! - Mixed workloads
//!
//! SLO Targets:
//! - p95 GET < 10ms
//! - p95 PUT < 20ms

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use nori_lsm::{ATLLConfig, LsmEngine};
use tempfile::TempDir;

/// Helper to create a test engine with temporary directory.
async fn create_engine() -> (LsmEngine, TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = ATLLConfig::default();
    config.data_dir = temp_dir.path().to_path_buf();
    config.memtable.flush_trigger_bytes = 64 * 1024 * 1024; // 64MB - prevent flushes during bench

    let engine = LsmEngine::open(config).await.unwrap();
    (engine, temp_dir)
}

/// Benchmark PUT operations (memtable writes).
fn bench_put(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("put_1kb", |b| {
        let (engine, _temp) = rt.block_on(create_engine());
        let value = Bytes::from(vec![0u8; 1024]); // 1KB value
        let mut counter = 0u64;

        b.iter(|| {
            counter += 1;
            let key = Bytes::from(format!("key-{:010}", counter));
            rt.block_on(engine.put(black_box(key), black_box(value.clone()))).unwrap();
        });
    });

    c.bench_function("put_4kb", |b| {
        let (engine, _temp) = rt.block_on(create_engine());
        let value = Bytes::from(vec![0u8; 4096]); // 4KB value
        let mut counter = 0u64;

        b.iter(|| {
            counter += 1;
            let key = Bytes::from(format!("key-{:010}", counter));
            rt.block_on(engine.put(black_box(key), black_box(value.clone()))).unwrap();
        });
    });
}

/// Benchmark GET operations from memtable (hot reads).
fn bench_get_memtable(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("get_memtable_1kb", |b| {
        let (engine, _temp) = rt.block_on(async {
            let (engine, temp) = create_engine().await;

            // Pre-populate with 1000 keys
            for i in 0..1000 {
                let key = Bytes::from(format!("key-{:010}", i));
                let value = Bytes::from(vec![0u8; 1024]);
                engine.put(key, value).await.unwrap();
            }

            (engine, temp)
        });

        let mut counter = 0usize;

        b.iter(|| {
            let key = format!("key-{:010}", counter % 1000);
            counter += 1;
            rt.block_on(engine.get(black_box(key.as_bytes()))).unwrap();
        });
    });
}

/// Benchmark GET operations from L0 (cold reads after flush).
fn bench_get_l0(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("get_l0_1kb", |b| {
        let (engine, _temp) = rt.block_on(async {
            let (engine, temp) = create_engine().await;

            // Pre-populate with 1000 keys
            for i in 0..1000 {
                let key = Bytes::from(format!("key-{:010}", i));
                let value = Bytes::from(vec![0u8; 1024]);
                engine.put(key, value).await.unwrap();
            }

            // Trigger flush to move data to L0
            engine.flush().await.unwrap();

            (engine, temp)
        });

        let mut counter = 0usize;

        b.iter(|| {
            let key = format!("key-{:010}", counter % 1000);
            counter += 1;
            rt.block_on(engine.get(black_box(key.as_bytes()))).unwrap();
        });
    });
}

/// Benchmark DELETE operations.
fn bench_delete(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("delete", |b| {
        let (engine, _temp) = rt.block_on(async {
            let (engine, temp) = create_engine().await;

            // Pre-populate with keys to delete
            for i in 0..10000 {
                let key = Bytes::from(format!("key-{:010}", i));
                let value = Bytes::from(vec![0u8; 1024]);
                engine.put(key, value).await.unwrap();
            }

            (engine, temp)
        });

        let mut counter = 0usize;

        b.iter(|| {
            let key = format!("key-{:010}", counter % 10000);
            counter += 1;
            rt.block_on(engine.delete(black_box(key.as_bytes()))).unwrap();
        });
    });
}

/// Benchmark mixed workload (80% reads, 20% writes).
fn bench_mixed_workload(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("mixed_80read_20write", |b| {
        let (engine, _temp) = rt.block_on(async {
            let (engine, temp) = create_engine().await;

            // Pre-populate with 1000 keys
            for i in 0..1000 {
                let key = Bytes::from(format!("key-{:010}", i));
                let value = Bytes::from(vec![0u8; 1024]);
                engine.put(key, value).await.unwrap();
            }

            (engine, temp)
        });

        let mut counter = 0usize;

        b.iter(|| {
            counter += 1;

            if counter % 5 == 0 {
                // 20% writes
                let key = Bytes::from(format!("key-{:010}", counter % 1000));
                let value = Bytes::from(vec![0u8; 1024]);
                rt.block_on(engine.put(black_box(key), black_box(value))).unwrap();
            } else {
                // 80% reads
                let key = format!("key-{:010}", counter % 1000);
                rt.block_on(engine.get(black_box(key.as_bytes()))).unwrap();
            }
        });
    });
}

/// Benchmark varying value sizes.
fn bench_value_sizes(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("put_value_sizes");

    for size in [128, 512, 1024, 4096, 16384].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let (engine, _temp) = rt.block_on(create_engine());
            let value = Bytes::from(vec![0u8; size]);
            let mut counter = 0u64;

            b.iter(|| {
                counter += 1;
                let key = Bytes::from(format!("key-{:010}", counter));
                rt.block_on(engine.put(black_box(key), black_box(value.clone()))).unwrap();
            });
        });
    }

    group.finish();
}

/// Benchmark sequential batch operations.
fn bench_concurrent(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("sequential_100_puts", |b| {
        let (engine, _temp) = rt.block_on(create_engine());

        b.iter(|| {
            rt.block_on(async {
                for i in 0..100 {
                    let key = Bytes::from(format!("key-{:010}", i));
                    let value = Bytes::from(vec![0u8; 1024]);
                    engine.put(key, value).await.unwrap();
                }
            });
        });
    });
}

criterion_group!(
    benches,
    bench_put,
    bench_get_memtable,
    bench_get_l0,
    bench_delete,
    bench_mixed_workload,
    bench_value_sizes,
    bench_concurrent
);
criterion_main!(benches);
