//! Benchmarks comparing v1 (Bloom filter) vs v2 (per-block Quotient Filter) formats.
//!
//! These benchmarks help quantify the tradeoffs between the two filter types:
//! - v1 (Bloom): Per-file filter, must rebuild during compaction
//! - v2 (QF): Per-block filter, can merge without re-hashing

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nori_sstable::{BloomFilter, QuotientFilter};

/// Compares build time: Bloom vs Quotient Filter
fn filter_build_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_build");

    for size in [100, 1_000, 10_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));

        // Bloom filter build
        group.bench_with_input(
            BenchmarkId::new("bloom", size),
            size,
            |b, &size| {
                b.iter(|| {
                    let mut bloom = BloomFilter::new(size, 10);
                    for i in 0..size {
                        let key = format!("key_{:08}", i);
                        bloom.add(black_box(key.as_bytes()));
                    }
                    black_box(bloom);
                });
            },
        );

        // Quotient filter build
        group.bench_with_input(
            BenchmarkId::new("quotient_filter", size),
            size,
            |b, &size| {
                b.iter(|| {
                    let mut qf = QuotientFilter::for_keys(size);
                    for i in 0..size {
                        let key = format!("key_{:08}", i);
                        qf.add(black_box(key.as_bytes()));
                    }
                    black_box(qf);
                });
            },
        );
    }

    group.finish();
}

/// Compares lookup time: Bloom vs Quotient Filter (key present)
fn filter_lookup_hit_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_lookup_hit");
    group.throughput(Throughput::Elements(1));

    for size in [100, 1_000, 10_000].iter() {
        // Setup filters
        let mut bloom = BloomFilter::new(*size, 10);
        let mut qf = QuotientFilter::for_keys(*size);

        for i in 0..*size {
            let key = format!("key_{:08}", i);
            bloom.add(key.as_bytes());
            qf.add(key.as_bytes());
        }

        let lookup_key = format!("key_{:08}", size / 2);

        // Bloom filter lookup
        group.bench_with_input(
            BenchmarkId::new("bloom", size),
            &lookup_key,
            |b, key| {
                b.iter(|| {
                    let result = bloom.contains(black_box(key.as_bytes()));
                    assert!(result);
                    black_box(result);
                });
            },
        );

        // Quotient filter lookup
        group.bench_with_input(
            BenchmarkId::new("quotient_filter", size),
            &lookup_key,
            |b, key| {
                b.iter(|| {
                    let result = qf.contains(black_box(key.as_bytes()));
                    assert!(result);
                    black_box(result);
                });
            },
        );
    }

    group.finish();
}

/// Compares lookup time: Bloom vs Quotient Filter (key absent)
fn filter_lookup_miss_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_lookup_miss");
    group.throughput(Throughput::Elements(1));

    for size in [100, 1_000, 10_000].iter() {
        // Setup filters
        let mut bloom = BloomFilter::new(*size, 10);
        let mut qf = QuotientFilter::for_keys(*size);

        for i in 0..*size {
            let key = format!("key_{:08}", i);
            bloom.add(key.as_bytes());
            qf.add(key.as_bytes());
        }

        let missing_key = format!("missing_{:08}", size * 10);

        // Bloom filter lookup
        group.bench_with_input(
            BenchmarkId::new("bloom", size),
            &missing_key,
            |b, key| {
                b.iter(|| {
                    let result = bloom.contains(black_box(key.as_bytes()));
                    black_box(result);
                });
            },
        );

        // Quotient filter lookup
        group.bench_with_input(
            BenchmarkId::new("quotient_filter", size),
            &missing_key,
            |b, key| {
                b.iter(|| {
                    let result = qf.contains(black_box(key.as_bytes()));
                    black_box(result);
                });
            },
        );
    }

    group.finish();
}

/// Compares simulated compaction: Bloom (rebuild) vs QF (merge)
///
/// This is the key advantage of QF - during compaction we can merge
/// fingerprints without re-hashing the original keys.
fn compaction_simulation_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("compaction_simulation");

    // Simulate compaction of N input SSTables
    for num_inputs in [2, 4, 8].iter() {
        let keys_per_input = 1000;
        let total_keys = num_inputs * keys_per_input;

        group.throughput(Throughput::Elements(total_keys as u64));

        // Create input filters (representing input SSTables)
        let input_blooms: Vec<BloomFilter> = (0..*num_inputs)
            .map(|input_idx| {
                let mut bloom = BloomFilter::new(keys_per_input, 10);
                for i in 0..keys_per_input {
                    let key = format!("input{}_{:08}", input_idx, i);
                    bloom.add(key.as_bytes());
                }
                bloom
            })
            .collect();

        let input_qfs: Vec<QuotientFilter> = (0..*num_inputs)
            .map(|input_idx| {
                let mut qf = QuotientFilter::for_keys(keys_per_input);
                for i in 0..keys_per_input {
                    let key = format!("input{}_{:08}", input_idx, i);
                    qf.add(key.as_bytes());
                }
                qf
            })
            .collect();

        // Bloom approach: must re-hash all keys to build output filter
        group.bench_with_input(
            BenchmarkId::new("bloom_rebuild", num_inputs),
            &input_blooms,
            |b, _| {
                b.iter(|| {
                    // In real compaction, we'd iterate through merged entries
                    // Here we simulate by re-hashing all keys
                    let mut output_bloom = BloomFilter::new(total_keys, 10);
                    for input_idx in 0..*num_inputs {
                        for i in 0..keys_per_input {
                            let key = format!("input{}_{:08}", input_idx, i);
                            output_bloom.add(black_box(key.as_bytes()));
                        }
                    }
                    black_box(output_bloom);
                });
            },
        );

        // QF approach: merge fingerprints without re-hashing
        group.bench_with_input(
            BenchmarkId::new("qf_merge", num_inputs),
            &input_qfs,
            |b, inputs: &Vec<QuotientFilter>| {
                b.iter(|| {
                    let refs: Vec<&QuotientFilter> = inputs.iter().collect();
                    let merged = QuotientFilter::merge_all(&refs).unwrap();
                    black_box(merged);
                });
            },
        );
    }

    group.finish();
}

/// Compares memory usage: Bloom vs Quotient Filter
fn filter_memory_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_memory");

    for size in [100, 1_000, 10_000].iter() {
        // Build and encode Bloom
        let mut bloom = BloomFilter::new(*size, 10);
        for i in 0..*size {
            bloom.add(format!("key_{:08}", i).as_bytes());
        }
        let bloom_encoded = bloom.encode();

        // Build and encode QF
        let mut qf = QuotientFilter::for_keys(*size);
        for i in 0..*size {
            qf.add(format!("key_{:08}", i).as_bytes());
        }
        let qf_encoded = qf.encode();
        let qf_compact = qf.encode_compact();

        group.bench_with_input(
            BenchmarkId::new("bloom_size", size),
            &bloom_encoded,
            |b, encoded: &Bytes| {
                b.iter(|| {
                    black_box(encoded.len());
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("qf_size", size),
            &qf_encoded,
            |b, encoded: &Bytes| {
                b.iter(|| {
                    black_box(encoded.len());
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("qf_compact_size", size),
            &qf_compact,
            |b, encoded: &Bytes| {
                b.iter(|| {
                    black_box(encoded.len());
                });
            },
        );
    }

    group.finish();
}

/// Compares encode/decode round-trip time
fn filter_codec_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_codec");

    for size in [100, 1_000, 10_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));

        // Bloom codec
        let mut bloom = BloomFilter::new(*size, 10);
        for i in 0..*size {
            bloom.add(format!("key_{:08}", i).as_bytes());
        }

        group.bench_with_input(
            BenchmarkId::new("bloom", size),
            &bloom,
            |b, bloom| {
                b.iter(|| {
                    let encoded = bloom.encode();
                    let decoded = BloomFilter::decode(&encoded).unwrap();
                    black_box(decoded);
                });
            },
        );

        // QF codec (standard)
        let mut qf = QuotientFilter::for_keys(*size);
        for i in 0..*size {
            qf.add(format!("key_{:08}", i).as_bytes());
        }

        group.bench_with_input(
            BenchmarkId::new("qf_standard", size),
            &qf,
            |b, qf: &QuotientFilter| {
                b.iter(|| {
                    let encoded = qf.encode();
                    let decoded = QuotientFilter::decode(&encoded).unwrap();
                    black_box(decoded);
                });
            },
        );

        // QF codec (compact)
        group.bench_with_input(
            BenchmarkId::new("qf_compact", size),
            &qf,
            |b, qf: &QuotientFilter| {
                b.iter(|| {
                    let encoded = qf.encode_compact();
                    let decoded = QuotientFilter::decode_compact(&encoded).unwrap();
                    black_box(decoded);
                });
            },
        );
    }

    group.finish();
}

/// Reports size comparison between filter types
fn filter_size_report(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_size_report");

    for size in [100, 1_000, 10_000].iter() {
        // Build filters
        let mut bloom = BloomFilter::new(*size, 10);
        let mut qf = QuotientFilter::for_keys(*size);

        for i in 0..*size {
            let key = format!("key_{:08}", i);
            bloom.add(key.as_bytes());
            qf.add(key.as_bytes());
        }

        let bloom_size = bloom.encode().len();
        let qf_size = qf.encode().len();
        let qf_compact_size = qf.encode_compact().len();

        // This benchmark just reports sizes (constant time)
        group.bench_with_input(
            BenchmarkId::new("report", size),
            size,
            |b, _| {
                b.iter(|| {
                    // Print once per measurement
                    black_box((bloom_size, qf_size, qf_compact_size));
                });
            },
        );

        println!(
            "\n=== {} keys ===\nBloom: {} bytes ({:.2} bytes/key)\nQF: {} bytes ({:.2} bytes/key)\nQF compact: {} bytes ({:.2} bytes/key)",
            size,
            bloom_size, bloom_size as f64 / *size as f64,
            qf_size, qf_size as f64 / *size as f64,
            qf_compact_size, qf_compact_size as f64 / *size as f64,
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    filter_build_comparison,
    filter_lookup_hit_comparison,
    filter_lookup_miss_comparison,
    compaction_simulation_comparison,
    filter_memory_comparison,
    filter_codec_comparison,
    filter_size_report,
);
criterion_main!(benches);
