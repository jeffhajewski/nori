//! Benchmark suite for measuring bloom filter impact on SSTable read latency.
//!
//! Tests:
//! - SSTable read latency with different bits_per_key values
//! - Impact on point lookups (bloom filter effectiveness)
//! - Trade-offs between FP rate and memory overhead
//!
//! Goal: Determine optimal bloom filter configuration for production workloads.

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use nori_sstable::{Compression, Entry, SSTableBuilder, SSTableConfig, SSTableReader};
use tempfile::TempDir;

/// Helper to create an SSTable with specific bloom filter configuration.
async fn create_sstable_with_bloom_config(
    bits_per_key: usize,
    num_keys: usize,
) -> (SSTableReader, TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let sst_path = temp_dir.path().join("test.sst");

    let config = SSTableConfig {
        path: sst_path.clone(),
        block_size: 4096,
        restart_interval: 16,
        compression: Compression::None,
        bloom_bits_per_key: bits_per_key,
        block_cache_mb: 64,
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    // Add keys
    for i in 0..num_keys {
        let key = Bytes::from(format!("key-{:010}", i));
        let value = Bytes::from(vec![0u8; 1024]); // 1KB value
        builder.add(Entry { key, value }).await.unwrap();
    }

    let (_path, _meta) = builder.finish().await.unwrap();

    // Open reader
    let reader = SSTableReader::open(sst_path).await.unwrap();

    (reader, temp_dir)
}

/// Benchmark point lookup latency with different bloom filter configurations.
///
/// This tests the effectiveness of bloom filters in reducing disk I/O for
/// non-existent keys.
fn bench_point_lookup_with_bloom_config(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("point_lookup_bloom_config");

    const NUM_KEYS: usize = 10_000;

    for bits_per_key in [6, 8, 10, 12, 14].iter() {
        // Setup: Create SSTable with specific bloom config
        let (reader, _temp) = rt.block_on(
            create_sstable_with_bloom_config(*bits_per_key, NUM_KEYS)
        );

        // Benchmark: GET for keys that exist (bloom filter HIT, should read SSTable)
        group.bench_with_input(
            BenchmarkId::new("hit", bits_per_key),
            bits_per_key,
            |b, _| {
                let mut counter = 0usize;
                b.iter(|| {
                    let key = format!("key-{:010}", counter % NUM_KEYS);
                    counter += 1;
                    let result = rt.block_on(reader.get(black_box(key.as_bytes()))).unwrap();
                    assert!(result.is_some());
                    black_box(result);
                });
            },
        );

        // Benchmark: GET for keys that DON'T exist (bloom filter MISS, should skip SSTable)
        group.bench_with_input(
            BenchmarkId::new("miss", bits_per_key),
            bits_per_key,
            |b, _| {
                let mut counter = 0usize;
                b.iter(|| {
                    let key = format!("missing-{:010}", counter);
                    counter += 1;
                    let result = rt.block_on(reader.get(black_box(key.as_bytes()))).unwrap();
                    assert!(result.is_none());
                    black_box(result);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark mixed workload with different bloom filter configurations.
///
/// Tests realistic production scenario: 80% hits, 20% misses.
fn bench_mixed_workload_with_bloom_config(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("mixed_workload_bloom_config");

    const NUM_KEYS: usize = 10_000;

    for bits_per_key in [6, 10, 14].iter() {
        let (reader, _temp) = rt.block_on(
            create_sstable_with_bloom_config(*bits_per_key, NUM_KEYS)
        );

        group.bench_with_input(
            BenchmarkId::from_parameter(bits_per_key),
            bits_per_key,
            |b, _| {
                let mut counter = 0usize;
                b.iter(|| {
                    counter += 1;

                    if counter % 5 == 0 {
                        // 20% misses
                        let key = format!("missing-{:010}", counter);
                        let result = rt.block_on(reader.get(black_box(key.as_bytes()))).unwrap();
                        assert!(result.is_none());
                        black_box(result);
                    } else {
                        // 80% hits
                        let key = format!("key-{:010}", counter % NUM_KEYS);
                        let result = rt.block_on(reader.get(black_box(key.as_bytes()))).unwrap();
                        assert!(result.is_some());
                        black_box(result);
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark false positive impact on point lookup latency.
///
/// Measures the cost of false positives (unnecessary SSTable reads).
fn bench_false_positive_cost(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("false_positive_cost");

    const NUM_KEYS: usize = 10_000;
    const TEST_KEYS: usize = 1_000;

    // Compare low vs high bits_per_key (high FP rate vs low FP rate)
    for bits_per_key in [6, 14].iter() {
        let (reader, _temp) = rt.block_on(
            create_sstable_with_bloom_config(*bits_per_key, NUM_KEYS)
        );

        group.bench_with_input(
            BenchmarkId::from_parameter(bits_per_key),
            bits_per_key,
            |b, _| {
                b.iter(|| {
                    // Query many non-existent keys
                    for i in NUM_KEYS..(NUM_KEYS + TEST_KEYS) {
                        let key = format!("key-{:010}", i);
                        let result = rt.block_on(reader.get(black_box(key.as_bytes()))).unwrap();
                        assert!(result.is_none());
                        black_box(result);
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_point_lookup_with_bloom_config,
    bench_mixed_workload_with_bloom_config,
    bench_false_positive_cost,
);
criterion_main!(benches);
