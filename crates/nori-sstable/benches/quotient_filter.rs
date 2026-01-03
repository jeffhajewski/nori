use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nori_sstable::{QuotientFilter, QuotientFilterConfig};

fn build_qf(c: &mut Criterion) {
    let mut group = c.benchmark_group("build_qf");

    for size in [100, 10_000, 100_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut qf = QuotientFilter::for_keys(size);

                for i in 0..size {
                    let key = format!("key_{:08}", i);
                    qf.add(black_box(key.as_bytes()));
                }

                black_box(qf);
            });
        });
    }

    group.finish();
}

fn qf_contains_hit(c: &mut Criterion) {
    let mut group = c.benchmark_group("qf_contains_hit");
    group.throughput(Throughput::Elements(1));

    for size in [100, 10_000, 100_000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut qf = QuotientFilter::for_keys(size);

            for i in 0..size {
                let key = format!("key_{:08}", i);
                qf.add(key.as_bytes());
            }

            b.iter(|| {
                let key = format!("key_{:08}", size / 2);
                let result = qf.contains(black_box(key.as_bytes()));
                assert!(result);
                black_box(result);
            });
        });
    }

    group.finish();
}

fn qf_contains_miss(c: &mut Criterion) {
    let mut group = c.benchmark_group("qf_contains_miss");
    group.throughput(Throughput::Elements(1));

    for size in [100, 10_000, 100_000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut qf = QuotientFilter::for_keys(size);

            for i in 0..size {
                let key = format!("key_{:08}", i);
                qf.add(key.as_bytes());
            }

            b.iter(|| {
                let key = format!("missing_{:08}", size * 10);
                let result = qf.contains(black_box(key.as_bytes()));
                // Most likely false, but QF can have false positives
                black_box(result);
            });
        });
    }

    group.finish();
}

fn qf_encode_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("qf_encode_decode");

    for size in [100, 10_000, 100_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut qf = QuotientFilter::for_keys(size);

            for i in 0..size {
                let key = format!("key_{:08}", i);
                qf.add(key.as_bytes());
            }

            b.iter(|| {
                let encoded = qf.encode();
                let decoded = QuotientFilter::decode(&encoded).unwrap();
                black_box(decoded);
            });
        });
    }

    group.finish();
}

fn qf_encode_decode_compact(c: &mut Criterion) {
    let mut group = c.benchmark_group("qf_encode_decode_compact");

    for size in [100, 10_000, 100_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut qf = QuotientFilter::for_keys(size);

            for i in 0..size {
                let key = format!("key_{:08}", i);
                qf.add(key.as_bytes());
            }

            b.iter(|| {
                let encoded = qf.encode_compact();
                let decoded = QuotientFilter::decode_compact(&encoded).unwrap();
                black_box(decoded);
            });
        });
    }

    group.finish();
}

fn qf_merge(c: &mut Criterion) {
    let mut group = c.benchmark_group("qf_merge");

    for size in [100, 1_000, 10_000].iter() {
        group.throughput(Throughput::Elements(*size as u64 * 2));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let config = QuotientFilterConfig::for_keys(size);

            let mut qf1 = QuotientFilter::new(config);
            let mut qf2 = QuotientFilter::new(config);

            for i in 0..size {
                qf1.add(format!("a{:08}", i).as_bytes());
                qf2.add(format!("b{:08}", i).as_bytes());
            }

            b.iter(|| {
                let merged = QuotientFilter::merge_all(&[&qf1, &qf2]).unwrap();
                black_box(merged);
            });
        });
    }

    group.finish();
}

fn qf_extract_fingerprints(c: &mut Criterion) {
    let mut group = c.benchmark_group("qf_extract_fingerprints");

    for size in [100, 1_000, 10_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut qf = QuotientFilter::for_keys(size);

            for i in 0..size {
                qf.add(format!("key_{:08}", i).as_bytes());
            }

            b.iter(|| {
                let fps = qf.extract_fingerprints();
                black_box(fps);
            });
        });
    }

    group.finish();
}

fn qf_false_positive_rate(c: &mut Criterion) {
    let mut group = c.benchmark_group("qf_false_positive_rate");
    group.throughput(Throughput::Elements(10_000));

    group.bench_function("measure_fp_rate", |b| {
        b.iter(|| {
            let mut qf = QuotientFilter::for_keys(10_000);

            // Add 10K keys
            for i in 0..10_000 {
                let key = format!("key_{:08}", i);
                qf.add(key.as_bytes());
            }

            // Test 10K missing keys
            let mut false_positives = 0;
            for i in 10_000..20_000 {
                let key = format!("key_{:08}", i);
                if qf.contains(black_box(key.as_bytes())) {
                    false_positives += 1;
                }
            }

            // FPR should be less than 3% (target is ~0.78% for r=7)
            assert!(
                false_positives <= 300,
                "False positive rate out of range: {} / 10000",
                false_positives
            );

            black_box(false_positives);
        });
    });

    group.finish();
}

fn qf_memory_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("qf_memory_overhead");

    for size in [100, 10_000, 100_000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut qf = QuotientFilter::for_keys(size);

                for i in 0..size {
                    let key = format!("key_{:08}", i);
                    qf.add(key.as_bytes());
                }

                let encoded = qf.encode();
                let bytes_per_key = encoded.len() as f64 / size as f64;

                // QF standard encoding: ~7 bytes per occupied slot + overhead
                // At 75% load factor, slots/keys = 1.33, so ~9 bytes/key is typical
                assert!(
                    bytes_per_key >= 1.0 && bytes_per_key <= 15.0,
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
    build_qf,
    qf_contains_hit,
    qf_contains_miss,
    qf_encode_decode,
    qf_encode_decode_compact,
    qf_merge,
    qf_extract_fingerprints,
    qf_false_positive_rate,
    qf_memory_overhead,
);
criterion_main!(benches);
