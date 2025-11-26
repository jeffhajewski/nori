//! HNSW benchmarks.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use nori_hnsw::{HnswConfig, HnswIndex};
use nori_vector::{DistanceFunction, VectorIndex};

fn generate_vectors(n: usize, dims: usize) -> Vec<Vec<f32>> {
    (0..n)
        .map(|i| (0..dims).map(|j| ((i * j) % 100) as f32 / 100.0).collect())
        .collect()
}

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_insert");

    for n in [100, 1000].iter() {
        let vectors = generate_vectors(*n, 128);
        let config = HnswConfig::default();

        group.bench_with_input(BenchmarkId::from_parameter(n), n, |bencher, _| {
            bencher.iter(|| {
                let index = HnswIndex::new(128, DistanceFunction::Euclidean, config.clone());
                for (i, vec) in vectors.iter().enumerate() {
                    index.insert(&format!("vec{}", i), black_box(vec)).unwrap();
                }
            })
        });
    }

    group.finish();
}

fn bench_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_search");

    for n in [1000, 10000].iter() {
        let vectors = generate_vectors(*n, 128);
        let config = HnswConfig::default();
        let index = HnswIndex::new(128, DistanceFunction::Euclidean, config);

        for (i, vec) in vectors.iter().enumerate() {
            index.insert(&format!("vec{}", i), vec).unwrap();
        }

        let query: Vec<f32> = (0..128).map(|i| i as f32 / 128.0).collect();

        group.bench_with_input(BenchmarkId::from_parameter(n), n, |bencher, _| {
            bencher.iter(|| index.search(black_box(&query), 10).unwrap())
        });
    }

    group.finish();
}

criterion_group!(benches, bench_insert, bench_search);
criterion_main!(benches);
