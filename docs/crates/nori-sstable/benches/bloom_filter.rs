use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nori_sstable::BloomFilter;

fn build_bloom(c: &mut Criterion) {
    let mut group = c.benchmark_group("build_bloom");

    for size in [100, 10_000, 100_000, 1_000_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut bloom = BloomFilter::new(size, 10);

                for i in 0..size {
                    let key = format!("key_{:08}", i);
                    bloom.add(black_box(key.as_bytes()));
                }

                black_box(bloom);
            });
        });
    }

    group.finish();
}

fn bloom_contains_hit(c: &mut Criterion) {
    let mut group = c.benchmark_group("bloom_contains_hit");
    group.throughput(Throughput::Elements(1));

    for size in [100, 10_000, 100_000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut bloom = BloomFilter::new(size, 10);

            for i in 0..size {
                let key = format!("key_{:08}", i);
                bloom.add(key.as_bytes());
            }

            b.iter(|| {
                let key = format!("key_{:08}", size / 2);
                let result = bloom.contains(black_box(key.as_bytes()));
                assert!(result);
                black_box(result);
            });
        });
    }

    group.finish();
}

fn bloom_contains_miss(c: &mut Criterion) {
    let mut group = c.benchmark_group("bloom_contains_miss");
    group.throughput(Throughput::Elements(1));

    for size in [100, 10_000, 100_000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut bloom = BloomFilter::new(size, 10);

            for i in 0..size {
                let key = format!("key_{:08}", i);
                bloom.add(key.as_bytes());
            }

            b.iter(|| {
                let key = format!("missing_{:08}", size * 10);
                let result = bloom.contains(black_box(key.as_bytes()));
                // Most likely false, but bloom filters can have false positives
                black_box(result);
            });
        });
    }

    group.finish();
}

fn bloom_encode_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("bloom_encode_decode");

    for size in [100, 10_000, 100_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut bloom = BloomFilter::new(size, 10);

            for i in 0..size {
                let key = format!("key_{:08}", i);
                bloom.add(key.as_bytes());
            }

            b.iter(|| {
                let encoded = bloom.encode();
                let decoded = BloomFilter::decode(&encoded).unwrap();
                black_box(decoded);
            });
        });
    }

    group.finish();
}

fn bloom_optimal_bits_per_key(c: &mut Criterion) {
    let mut group = c.benchmark_group("bloom_bits_per_key");
    group.throughput(Throughput::Elements(10_000));

    for bits_per_key in [5, 10, 15, 20].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(bits_per_key),
            bits_per_key,
            |b, &bits_per_key| {
                b.iter(|| {
                    let mut bloom = BloomFilter::new(10_000, bits_per_key);

                    for i in 0..10_000 {
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

fn bloom_false_positive_rate(c: &mut Criterion) {
    let mut group = c.benchmark_group("bloom_false_positive_rate");
    group.throughput(Throughput::Elements(10_000));

    group.bench_function("measure_fp_rate", |b| {
        b.iter(|| {
            let mut bloom = BloomFilter::new(10_000, 10);

            // Add 10K keys
            for i in 0..10_000 {
                let key = format!("key_{:08}", i);
                bloom.add(key.as_bytes());
            }

            // Test 10K missing keys
            let mut false_positives = 0;
            for i in 10_000..20_000 {
                let key = format!("key_{:08}", i);
                if bloom.contains(black_box(key.as_bytes())) {
                    false_positives += 1;
                }
            }

            // FP rate should be around 0.9% (90 out of 10000)
            // Allow 0.5-1.5% range
            assert!(
                false_positives >= 50 && false_positives <= 150,
                "False positive rate out of range: {} / 10000",
                false_positives
            );

            black_box(false_positives);
        });
    });

    group.finish();
}

fn bloom_memory_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("bloom_memory_overhead");

    for size in [100, 10_000, 100_000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut bloom = BloomFilter::new(size, 10);

                for i in 0..size {
                    let key = format!("key_{:08}", i);
                    bloom.add(key.as_bytes());
                }

                let encoded = bloom.encode();
                let bytes_per_key = encoded.len() as f64 / size as f64;

                // With 10 bits/key, expect ~1.25 bytes per key (10/8)
                assert!(
                    bytes_per_key >= 1.0 && bytes_per_key <= 2.0,
                    "Unexpected bytes per key: {:.2}",
                    bytes_per_key
                );

                black_box((encoded, bytes_per_key));
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    build_bloom,
    bloom_contains_hit,
    bloom_contains_miss,
    bloom_encode_decode,
    bloom_optimal_bits_per_key,
    bloom_false_positive_rate,
    bloom_memory_overhead,
);
criterion_main!(benches);
