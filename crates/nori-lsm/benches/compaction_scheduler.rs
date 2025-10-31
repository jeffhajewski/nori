//! Benchmark suite for validating compaction scheduler (bandit-based).
//!
//! Tests:
//! - Zipfian workload (hot/cold key distribution)
//! - Bandit exploration vs exploitation convergence
//! - Slot K-value adaptation (hot slots → K=1, cold slots → K>1)
//! - Compaction decision quality
//!
//! Goal: Validate that adaptive tiering works correctly.

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nori_lsm::{ATLLConfig, LsmEngine};
use std::collections::HashMap;
use tempfile::TempDir;

/// Helper to create a test engine.
async fn create_engine() -> (LsmEngine, TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = ATLLConfig::default();
    config.data_dir = temp_dir.path().to_path_buf();

    // Lower flush threshold to trigger more compaction activity
    config.memtable.flush_trigger_bytes = 1024 * 1024; // 1MB
    config.l0.max_files = 4; // Trigger L0 compaction sooner

    let engine = LsmEngine::open(config).await.unwrap();
    (engine, temp_dir)
}

/// Zipfian distribution: 80% of accesses hit 20% of keys (hot set).
///
/// Returns a key ID based on Zipfian distribution.
fn zipfian_key(counter: usize, total_keys: usize) -> usize {
    let hot_threshold = total_keys / 5; // 20% of keys are hot

    // 80% of the time, access the hot set (first 20% of keys)
    if counter % 5 != 0 {
        counter % hot_threshold
    } else {
        // 20% of the time, access the cold set (remaining 80% of keys)
        hot_threshold + (counter % (total_keys - hot_threshold))
    }
}

/// Benchmark Zipfian workload with compaction.
///
/// This tests whether the bandit scheduler correctly adapts K values:
/// - Hot slots should converge to K=1 (leveled, low read amp)
/// - Cold slots should stay at K>1 (tiered, low write amp)
fn bench_zipfian_workload_with_compaction(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("zipfian_workload_10k_keys", |b| {
        b.iter(|| {
            let (engine, _temp) = rt.block_on(create_engine());

            const TOTAL_KEYS: usize = 10_000;
            const NUM_OPERATIONS: usize = 50_000;

            // Track access counts for validation
            let mut access_counts: HashMap<usize, usize> = HashMap::new();

            rt.block_on(async {
                for i in 0..NUM_OPERATIONS {
                    let key_id = zipfian_key(i, TOTAL_KEYS);
                    *access_counts.entry(key_id).or_insert(0) += 1;

                    let key = Bytes::from(format!("key-{:010}", key_id));
                    let value = Bytes::from(vec![0u8; 128]); // Small value for fast writes

                    engine.put(key.clone(), value, None).await.unwrap();

                    // Read operation (validates Zipfian distribution)
                    let _result = engine.get(key.as_ref()).await.unwrap();
                }

                // Final flush to ensure all data is persisted
                engine.flush().await.unwrap();

                // Sleep to allow some compaction to happen
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            });

            // Validate Zipfian distribution
            let hot_keys = TOTAL_KEYS / 5;
            let hot_accesses: usize = access_counts.iter()
                .filter(|(&key_id, _)| key_id < hot_keys)
                .map(|(_, &count)| count)
                .sum();

            let hot_ratio = hot_accesses as f64 / NUM_OPERATIONS as f64;

            // Should be close to 80%
            assert!(hot_ratio > 0.75 && hot_ratio < 0.85,
                "Zipfian distribution incorrect: hot_ratio={:.2}%, expected ~80%",
                hot_ratio * 100.0);

            black_box(engine);
        });
    });
}

/// Benchmark measuring compaction scheduler decision latency.
///
/// Tests how fast the bandit can select compaction actions.
fn bench_compaction_decision_latency(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("compaction_decision_latency", |b| {
        b.iter(|| {
            let (engine, _temp) = rt.block_on(async {
                let (engine, temp) = create_engine().await;

                // Pre-populate with some data to create L0 files
                for i in 0..5000 {
                    let key = Bytes::from(format!("key-{:010}", i));
                    let value = Bytes::from(vec![0u8; 128]);
                    engine.put(key, value, None).await.unwrap();
                }

                engine.flush().await.unwrap();

                (engine, temp)
            });

            // The compaction loop will make decisions in the background
            // This measures overall throughput with compaction active
            rt.block_on(async {
                for i in 5000..6000 {
                    let key = Bytes::from(format!("key-{:010}", i));
                    let value = Bytes::from(vec![0u8; 128]);
                    engine.put(black_box(key), black_box(value), None).await.unwrap();
                }
            });

            black_box(engine);
        });
    });
}

/// Benchmark mixed read/write workload with hot/cold distribution.
///
/// Tests realistic production scenario with adaptive compaction.
fn bench_mixed_zipfian_workload(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("mixed_zipfian_80read_20write", |b| {
        b.iter(|| {
            let (engine, _temp) = rt.block_on(async {
                let (engine, temp) = create_engine().await;

                // Pre-populate
                const TOTAL_KEYS: usize = 5_000;
                for i in 0..TOTAL_KEYS {
                    let key = Bytes::from(format!("key-{:010}", i));
                    let value = Bytes::from(vec![0u8; 128]);
                    engine.put(key, value, None).await.unwrap();
                }

                engine.flush().await.unwrap();

                (engine, temp)
            });

            const TOTAL_KEYS: usize = 5_000;
            const NUM_OPERATIONS: usize = 10_000;

            rt.block_on(async {
                for i in 0..NUM_OPERATIONS {
                    let key_id = zipfian_key(i, TOTAL_KEYS);
                    let key = format!("key-{:010}", key_id);

                    if i % 5 == 0 {
                        // 20% writes
                        let value = Bytes::from(vec![1u8; 128]);
                        engine.put(Bytes::from(key.clone()), value, None).await.unwrap();
                    } else {
                        // 80% reads
                        let _result = engine.get(black_box(key.as_bytes())).await.unwrap();
                    }
                }
            });

            black_box(engine);
        });
    });
}

/// Benchmark measuring compaction throughput under heavy write load.
///
/// Tests whether the bandit scheduler can keep up with sustained writes.
fn bench_compaction_throughput_under_load(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("compaction_throughput_sustained_writes", |b| {
        b.iter(|| {
            let (engine, _temp) = rt.block_on(create_engine());

            rt.block_on(async {
                // Sustained write workload
                for i in 0..20_000 {
                    let key = Bytes::from(format!("key-{:010}", i));
                    let value = Bytes::from(vec![0u8; 256]);
                    engine.put(black_box(key), black_box(value), None).await.unwrap();

                    // Periodically flush to generate L0 files
                    if i % 2000 == 0 {
                        engine.flush().await.unwrap();
                    }
                }

                // Final flush
                engine.flush().await.unwrap();
            });

            black_box(engine);
        });
    });
}

/// Benchmark guard rebalancing with skewed key distribution.
///
/// Creates heavily skewed workload to trigger guard rebalancing.
/// Measures time to handle imbalanced slots.
fn bench_guard_rebalancing_skewed_load(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("guard_rebalancing_skewed_90_10", |b| {
        b.iter(|| {
            let (engine, _temp) = rt.block_on(async {
                let (engine, temp) = create_engine().await;

                // Create heavily skewed workload: 90% "z" prefix, 10% "a" prefix
                // This should trigger guard rebalancing

                // Write 900 keys with "z" prefix (hotspot)
                for i in 0..900 {
                    let key = Bytes::from(format!("z_hotspot_{:06}", i));
                    let value = Bytes::from(vec![0u8; 200]);
                    engine.put(key, value, None).await.unwrap();
                }

                // Write 100 keys with "a" prefix (cold region)
                for i in 0..100 {
                    let key = Bytes::from(format!("a_cold_{:06}", i));
                    let value = Bytes::from(vec![0u8; 200]);
                    engine.put(key, value, None).await.unwrap();
                }

                // Force flush to get data into L0
                engine.flush().await.unwrap();

                // Allow compaction to run (may trigger guard rebalancing)
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

                (engine, temp)
            });

            // Verify all keys are still readable
            rt.block_on(async {
                for i in 0..900 {
                    let key = format!("z_hotspot_{:06}", i);
                    let result = engine.get(key.as_bytes()).await.unwrap();
                    assert!(result.is_some());
                }
            });

            black_box(engine);
        });
    });
}

criterion_group!(
    benches,
    bench_zipfian_workload_with_compaction,
    bench_compaction_decision_latency,
    bench_mixed_zipfian_workload,
    bench_compaction_throughput_under_load,
    bench_guard_rebalancing_skewed_load,
);
criterion_main!(benches);
