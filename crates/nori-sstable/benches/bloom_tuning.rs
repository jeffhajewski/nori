//! Benchmark suite for bloom filter tuning.
//!
//! This benchmark measures the impact of different bits_per_key values on:
//! - False positive rate
//! - Memory overhead
//! - Lookup performance
//!
//! Goal: Find optimal bits_per_key for production workloads.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use nori_sstable::BloomFilter;

/// Measures false positive rate for different bits_per_key values.
fn bench_fp_rate_by_bits_per_key(c: &mut Criterion) {
    let mut group = c.benchmark_group("fp_rate_by_bits_per_key");

    const NUM_KEYS: usize = 10_000;
    const TEST_KEYS: usize = 100_000;

    // Test bits_per_key values: 6, 8, 10, 12, 14
    for bits_per_key in [6, 8, 10, 12, 14].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(bits_per_key),
            bits_per_key,
            |b, &bits_per_key| {
                b.iter(|| {
                    let mut bloom = BloomFilter::new(NUM_KEYS, bits_per_key);

                    // Add NUM_KEYS keys
                    for i in 0..NUM_KEYS {
                        let key = format!("key_{:08}", i);
                        bloom.add(key.as_bytes());
                    }

                    // Test with keys NOT in the set
                    let mut false_positives = 0;
                    for i in NUM_KEYS..(NUM_KEYS + TEST_KEYS) {
                        let key = format!("key_{:08}", i);
                        if bloom.contains(black_box(key.as_bytes())) {
                            false_positives += 1;
                        }
                    }

                    let fp_rate = false_positives as f64 / TEST_KEYS as f64;

                    black_box((bloom, fp_rate, false_positives));
                });
            },
        );
    }

    group.finish();
}

/// Measures memory overhead for different bits_per_key values.
fn bench_memory_overhead_by_bits_per_key(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_overhead_by_bits_per_key");

    const NUM_KEYS: usize = 10_000;

    for bits_per_key in [6, 8, 10, 12, 14].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(bits_per_key),
            bits_per_key,
            |b, &bits_per_key| {
                b.iter(|| {
                    let mut bloom = BloomFilter::new(NUM_KEYS, bits_per_key);

                    for i in 0..NUM_KEYS {
                        let key = format!("key_{:08}", i);
                        bloom.add(key.as_bytes());
                    }

                    let encoded = bloom.encode();
                    let bytes_per_key = encoded.len() as f64 / NUM_KEYS as f64;
                    let num_hash_fns = bloom.num_hash_functions();

                    black_box((bloom, bytes_per_key, num_hash_fns));
                });
            },
        );
    }

    group.finish();
}

/// Measures lookup latency for different bits_per_key values.
fn bench_lookup_latency_by_bits_per_key(c: &mut Criterion) {
    let mut group = c.benchmark_group("lookup_latency_by_bits_per_key");

    const NUM_KEYS: usize = 10_000;

    for bits_per_key in [6, 8, 10, 12, 14].iter() {
        // Setup: Create bloom filter
        let mut bloom = BloomFilter::new(NUM_KEYS, *bits_per_key);
        for i in 0..NUM_KEYS {
            let key = format!("key_{:08}", i);
            bloom.add(key.as_bytes());
        }

        group.bench_with_input(
            BenchmarkId::new("hit", bits_per_key),
            bits_per_key,
            |b, _| {
                let mut counter = 0;
                b.iter(|| {
                    let key = format!("key_{:08}", counter % NUM_KEYS);
                    counter += 1;
                    let result = bloom.contains(black_box(key.as_bytes()));
                    assert!(result); // Should always be true (hit)
                    black_box(result);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("miss", bits_per_key),
            bits_per_key,
            |b, _| {
                let mut counter = 0;
                b.iter(|| {
                    let key = format!("missing_{:08}", counter);
                    counter += 1;
                    let result = bloom.contains(black_box(key.as_bytes()));
                    black_box(result);
                });
            },
        );
    }

    group.finish();
}

/// Detailed false positive rate analysis with statistics.
fn bench_detailed_fp_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("detailed_fp_analysis");
    group.sample_size(20); // Fewer samples for detailed analysis

    const NUM_KEYS: usize = 10_000;
    const TEST_KEYS: usize = 100_000;

    for bits_per_key in [6, 8, 10, 12, 14].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(bits_per_key),
            bits_per_key,
            |b, &bits_per_key| {
                // Expected FP rate from theory: (1 - e^(-kn/m))^k
                // where k = num_hash_functions, n = num_keys, m = num_bits
                let k = ((bits_per_key as f64 * 0.693).ceil() as u32).max(1);
                let expected_fp_rate = (1.0 - (-1.0 * k as f64 / bits_per_key as f64).exp()).powi(k as i32);

                b.iter(|| {
                    let mut bloom = BloomFilter::new(NUM_KEYS, bits_per_key);

                    // Add keys
                    for i in 0..NUM_KEYS {
                        let key = format!("key_{:08}", i);
                        bloom.add(key.as_bytes());
                    }

                    // Measure false positives
                    let mut false_positives = 0;
                    for i in NUM_KEYS..(NUM_KEYS + TEST_KEYS) {
                        let key = format!("key_{:08}", i);
                        if bloom.contains(black_box(key.as_bytes())) {
                            false_positives += 1;
                        }
                    }

                    let actual_fp_rate = false_positives as f64 / TEST_KEYS as f64;
                    let memory_bytes = bloom.encode().len();
                    let bytes_per_key = memory_bytes as f64 / NUM_KEYS as f64;

                    // Print summary
                    println!(
                        "\nbits_per_key={}: k={}, FP={:.3}% (expected {:.3}%), mem={:.2} bytes/key",
                        bits_per_key,
                        k,
                        actual_fp_rate * 100.0,
                        expected_fp_rate * 100.0,
                        bytes_per_key
                    );

                    black_box((
                        bloom,
                        actual_fp_rate,
                        expected_fp_rate,
                        false_positives,
                        memory_bytes,
                    ));
                });
            },
        );
    }

    group.finish();
}

/// Test build time for different bits_per_key values.
fn bench_build_time_by_bits_per_key(c: &mut Criterion) {
    let mut group = c.benchmark_group("build_time_by_bits_per_key");

    const NUM_KEYS: usize = 10_000;

    for bits_per_key in [6, 8, 10, 12, 14].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(bits_per_key),
            bits_per_key,
            |b, &bits_per_key| {
                b.iter(|| {
                    let mut bloom = BloomFilter::new(NUM_KEYS, bits_per_key);

                    for i in 0..NUM_KEYS {
                        let key = format!("key_{:08}", i);
                        bloom.add(black_box(key.as_bytes()));
                    }

                    black_box(bloom);
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_fp_rate_by_bits_per_key,
    bench_memory_overhead_by_bits_per_key,
    bench_lookup_latency_by_bits_per_key,
    bench_detailed_fp_analysis,
    bench_build_time_by_bits_per_key,
);
criterion_main!(benches);
