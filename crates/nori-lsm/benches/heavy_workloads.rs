//! Benchmark suite for heavy workload scenarios.
//!
//! Tests realistic production scenarios:
//! - Sustained write throughput
//! - Cache hit/miss scenarios
//! - Compaction impact
//! - Concurrent readers and writers

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use nori_lsm::{ATLLConfig, LsmEngine};
use std::sync::Arc;
use tempfile::TempDir;

/// Helper to create a test engine with temporary directory.
async fn create_engine() -> (LsmEngine, TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = ATLLConfig::default();
    config.data_dir = temp_dir.path().to_path_buf();

    let engine = LsmEngine::open(config).await.unwrap();
    (engine, temp_dir)
}

/// Benchmark sustained write throughput.
fn bench_sustained_writes(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("sustained_writes");
    group.throughput(Throughput::Elements(10000));

    group.bench_function("write_10k_1kb", |b| {
        b.iter_batched(
            || rt.block_on(create_engine()),
            |(engine, _temp)| {
                rt.block_on(async move {
                    for i in 0..10000 {
                        let key = Bytes::from(format!("key-{:010}", i));
                        let value = Bytes::from(vec![0u8; 1024]);
                        engine.put(key, value).await.unwrap();
                    }
                    black_box(engine);
                });
            },
            criterion::BatchSize::LargeInput,
        );
    });

    group.finish();
}

/// Benchmark cache hit vs. miss scenarios.
fn bench_cache_scenarios(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Cache hits: repeated access to same keys
    c.bench_function("cache_hits_100_keys", |b| {
        let (engine, _temp) = rt.block_on(async {
            let (engine, temp) = create_engine().await;

            // Pre-populate with 100 keys and flush to L0
            for i in 0..100 {
                let key = Bytes::from(format!("key-{:010}", i));
                let value = Bytes::from(vec![0u8; 4096]);
                engine.put(key, value).await.unwrap();
            }
            engine.flush().await.unwrap();

            (engine, temp)
        });

        let mut counter = 0usize;

        b.iter(|| {
            // Repeatedly access the same 10 keys (should hit cache)
            let key = format!("key-{:010}", counter % 10);
            counter += 1;
            rt.block_on(engine.get(black_box(key.as_bytes()))).unwrap();
        });
    });

    // Cache misses: access to different keys each time
    c.bench_function("cache_misses_10k_keys", |b| {
        let (engine, _temp) = rt.block_on(async {
            let (engine, temp) = create_engine().await;

            // Pre-populate with 10k keys and flush to L0
            for i in 0..10000 {
                let key = Bytes::from(format!("key-{:010}", i));
                let value = Bytes::from(vec![0u8; 4096]);
                engine.put(key, value).await.unwrap();
            }
            engine.flush().await.unwrap();

            (engine, temp)
        });

        let mut counter = 0usize;

        b.iter(|| {
            // Access different keys (should miss cache)
            let key = format!("key-{:010}", counter % 10000);
            counter += 1;
            rt.block_on(engine.get(black_box(key.as_bytes()))).unwrap();
        });
    });
}

/// Benchmark concurrent readers.
fn bench_concurrent_readers(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("concurrent_100_readers", |b| {
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

        let engine = Arc::new(engine);

        b.iter(|| {
            rt.block_on(async {
                let mut handles = Vec::new();

                // Spawn 100 concurrent readers
                for _ in 0..100 {
                    let engine = engine.clone();
                    let handle = tokio::spawn(async move {
                        for i in 0..10 {
                            let key = format!("key-{:010}", i);
                            engine.get(key.as_bytes()).await.unwrap();
                        }
                    });
                    handles.push(handle);
                }

                for handle in handles {
                    handle.await.unwrap();
                }
            });
        });
    });
}

/// Benchmark sequential mixed read/write workload.
fn bench_concurrent_mixed(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("sequential_500_mixed_ops", |b| {
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
            rt.block_on(async {
                // 500 mixed operations (50% read, 50% write)
                for i in 0..500 {
                    if i % 2 == 0 {
                        // Read
                        let key = format!("key-{:010}", counter % 1000);
                        engine.get(key.as_bytes()).await.unwrap();
                    } else {
                        // Write
                        let key = Bytes::from(format!("key-{:010}", counter % 1000));
                        let value = Bytes::from(vec![0u8; 1024]);
                        engine.put(key, value).await.unwrap();
                    }
                    counter += 1;
                }
            });
        });
    });
}

/// Benchmark write amplification scenario.
fn bench_write_amplification(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("write_amp_sequential_overwrites", |b| {
        let (engine, _temp) = rt.block_on(async {
            let (engine, temp) = create_engine().await;

            // Initial write
            for i in 0..1000 {
                let key = Bytes::from(format!("key-{:010}", i));
                let value = Bytes::from(vec![0u8; 1024]);
                engine.put(key, value).await.unwrap();
            }
            engine.flush().await.unwrap();

            (engine, temp)
        });

        b.iter(|| {
            rt.block_on(async {
                // Overwrite all keys (tests write amplification)
                for i in 0..1000 {
                    let key = Bytes::from(format!("key-{:010}", i));
                    let value = Bytes::from(vec![1u8; 1024]);
                    engine.put(black_box(key), black_box(value)).await.unwrap();
                }
            });
        });
    });
}

/// Benchmark point reads with varying hit rates.
fn bench_zipfian_reads(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("zipfian_reads_80_20", |b| {
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

        let mut counter = 0usize;

        b.iter(|| {
            counter += 1;

            // Simulate Zipfian: 80% of reads hit 20% of keys
            let key_id = if counter % 5 != 0 {
                counter % 2000 // Hot set: first 2000 keys
            } else {
                2000 + (counter % 8000) // Cold set: remaining keys
            };

            let key = format!("key-{:010}", key_id);
            rt.block_on(engine.get(black_box(key.as_bytes()))).unwrap();
        });
    });
}

criterion_group!(
    benches,
    bench_sustained_writes,
    bench_cache_scenarios,
    bench_concurrent_readers,
    bench_concurrent_mixed,
    bench_write_amplification,
    bench_zipfian_reads
);
criterion_main!(benches);
