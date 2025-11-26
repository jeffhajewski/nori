//! Benchmarks for distance functions.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use nori_vector::{cosine_distance, euclidean_distance, inner_product};

fn generate_vectors(dims: usize) -> (Vec<f32>, Vec<f32>) {
    let a: Vec<f32> = (0..dims).map(|i| (i as f32) * 0.1).collect();
    let b: Vec<f32> = (0..dims).map(|i| (i as f32) * 0.2 + 0.5).collect();
    (a, b)
}

fn bench_euclidean(c: &mut Criterion) {
    let mut group = c.benchmark_group("euclidean_distance");

    for dims in [128, 256, 512, 768, 1024, 1536].iter() {
        let (a, b) = generate_vectors(*dims);
        group.bench_with_input(BenchmarkId::from_parameter(dims), dims, |bencher, _| {
            bencher.iter(|| euclidean_distance(black_box(&a), black_box(&b)))
        });
    }

    group.finish();
}

fn bench_cosine(c: &mut Criterion) {
    let mut group = c.benchmark_group("cosine_distance");

    for dims in [128, 256, 512, 768, 1024, 1536].iter() {
        let (a, b) = generate_vectors(*dims);
        group.bench_with_input(BenchmarkId::from_parameter(dims), dims, |bencher, _| {
            bencher.iter(|| cosine_distance(black_box(&a), black_box(&b)))
        });
    }

    group.finish();
}

fn bench_inner_product(c: &mut Criterion) {
    let mut group = c.benchmark_group("inner_product");

    for dims in [128, 256, 512, 768, 1024, 1536].iter() {
        let (a, b) = generate_vectors(*dims);
        group.bench_with_input(BenchmarkId::from_parameter(dims), dims, |bencher, _| {
            bencher.iter(|| inner_product(black_box(&a), black_box(&b)))
        });
    }

    group.finish();
}

criterion_group!(benches, bench_euclidean, bench_cosine, bench_inner_product);
criterion_main!(benches);
