//! Benchmark for WAL segment garbage collection performance.
//!
//! Tests:
//! - GC latency with varying numbers of segments
//! - Throughput of segment deletion
//! - Impact of segment size on GC performance

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nori_wal::{FsyncPolicy, Record, Wal, WalConfig};
use tempfile::TempDir;

/// Benchmark GC performance with different numbers of segments
fn gc_latency_by_segment_count(c: &mut Criterion) {
    let mut group = c.benchmark_group("gc_latency");
    group.sample_size(20); // Fewer samples since GC involves file I/O

    // Test with different numbers of segments to GC
    let segment_counts = vec![5, 10, 20, 50];

    for segment_count in segment_counts {
        group.bench_function(BenchmarkId::new("delete_segments", segment_count), |b| {
            b.to_async(tokio::runtime::Runtime::new().unwrap())
                .iter_custom(|iters| async move {
                    let mut total_elapsed = std::time::Duration::ZERO;

                    for _ in 0..iters {
                        // Setup: create WAL with multiple segments
                        let temp_dir = TempDir::new().unwrap();
                        let config = WalConfig {
                            dir: temp_dir.path().to_path_buf(),
                            fsync_policy: FsyncPolicy::Os, // Fast for setup
                            max_segment_size: 1024 * 1024, // 1MB segments for quick rotation
                            ..Default::default()
                        };
                        let (wal, _) = Wal::open(config).await.unwrap();

                        // Write enough data to create the desired number of segments
                        let record_size = 100 * 1024; // 100KB records
                        let records_per_segment = 10;

                        for _ in 0..(segment_count * records_per_segment) {
                            let record = Record::put(
                                b"gc_benchmark_key".as_slice(),
                                vec![0u8; record_size],
                            );
                            wal.append(&record).await.unwrap();
                        }

                        // Get the current position (marks segments before this as eligible for GC)
                        let current_pos = wal.current_position().await;

                        // Benchmark: Delete old segments
                        let start = std::time::Instant::now();
                        black_box(wal.delete_segments_before(current_pos).await.unwrap());
                        total_elapsed += start.elapsed();
                    }

                    total_elapsed
                });
        });
    }

    group.finish();
}

/// Benchmark GC throughput (segments deleted per second)
fn gc_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("gc_throughput");
    group.sample_size(15);

    // Test with different segment sizes
    let segment_sizes = vec![
        ("1MB", 1024 * 1024),
        ("8MB", 8 * 1024 * 1024),
        ("32MB", 32 * 1024 * 1024),
    ];

    for (size_name, segment_size) in segment_sizes {
        group.throughput(Throughput::Bytes(segment_size as u64 * 10)); // 10 segments
        group.bench_function(BenchmarkId::new("delete_10_segments", size_name), |b| {
            b.to_async(tokio::runtime::Runtime::new().unwrap())
                .iter_custom(|iters| async move {
                    let mut total_elapsed = std::time::Duration::ZERO;

                    for _ in 0..iters {
                        let temp_dir = TempDir::new().unwrap();
                        let config = WalConfig {
                            dir: temp_dir.path().to_path_buf(),
                            fsync_policy: FsyncPolicy::Os,
                            max_segment_size: segment_size,
                            ..Default::default()
                        };
                        let (wal, _) = Wal::open(config).await.unwrap();

                        // Fill 10 segments
                        let record_size = segment_size / 20; // ~20 records per segment
                        for _ in 0..(10 * 20) {
                            let record = Record::put(
                                b"throughput_key".as_slice(),
                                vec![0u8; record_size as usize],
                            );
                            wal.append(&record).await.unwrap();
                        }

                        let current_pos = wal.current_position().await;

                        // Measure GC throughput
                        let start = std::time::Instant::now();
                        black_box(wal.delete_segments_before(current_pos).await.unwrap());
                        total_elapsed += start.elapsed();
                    }

                    total_elapsed
                });
        });
    }

    group.finish();
}

/// Benchmark GC with immediately following writes
fn gc_followed_by_writes(c: &mut Criterion) {
    let mut group = c.benchmark_group("gc_sequential");
    group.sample_size(15);

    group.bench_function("gc_then_writes", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter_custom(|iters| async move {
                let mut total_elapsed = std::time::Duration::ZERO;

                for _ in 0..iters {
                    let temp_dir = TempDir::new().unwrap();
                    let config = WalConfig {
                        dir: temp_dir.path().to_path_buf(),
                        fsync_policy: FsyncPolicy::Os,
                        max_segment_size: 2 * 1024 * 1024, // 2MB segments
                        ..Default::default()
                    };
                    let (wal, _) = Wal::open(config).await.unwrap();

                    // Pre-fill some segments for GC
                    for _ in 0..40 {
                        let record = Record::put(b"prefill_key".as_slice(), vec![0u8; 50 * 1024]);
                        wal.append(&record).await.unwrap();
                    }

                    let gc_position = wal.current_position().await;

                    // Benchmark: GC followed by writes (tests recovery of write performance)
                    let start = std::time::Instant::now();

                    // Perform GC
                    wal.delete_segments_before(gc_position).await.unwrap();

                    // Perform writes immediately after GC
                    for _ in 0..20 {
                        let record = Record::put(b"after_gc_key".as_slice(), vec![0u8; 10 * 1024]);
                        black_box(wal.append(&record).await.unwrap());
                    }

                    total_elapsed += start.elapsed();
                }

                total_elapsed
            });
    });

    group.finish();
}

/// Benchmark GC with different file system states
fn gc_with_fragmentation(c: &mut Criterion) {
    let mut group = c.benchmark_group("gc_fragmentation");
    group.sample_size(10);

    // Test GC performance with many small vs few large segments
    let scenarios = vec![
        ("many_small", 50, 512 * 1024),    // 50 x 512KB segments
        ("few_large", 5, 5 * 1024 * 1024), // 5 x 5MB segments
    ];

    for (scenario_name, segment_count, segment_size) in scenarios {
        group.bench_function(BenchmarkId::new("scenario", scenario_name), |b| {
            b.to_async(tokio::runtime::Runtime::new().unwrap())
                .iter_custom(|iters| async move {
                    let mut total_elapsed = std::time::Duration::ZERO;

                    for _ in 0..iters {
                        let temp_dir = TempDir::new().unwrap();
                        let config = WalConfig {
                            dir: temp_dir.path().to_path_buf(),
                            fsync_policy: FsyncPolicy::Os,
                            max_segment_size: segment_size,
                            ..Default::default()
                        };
                        let (wal, _) = Wal::open(config).await.unwrap();

                        // Fill segments
                        let record_size = segment_size / 10;
                        for _ in 0..(segment_count * 10) {
                            let record = Record::put(
                                b"frag_key".as_slice(),
                                vec![0u8; record_size as usize],
                            );
                            wal.append(&record).await.unwrap();
                        }

                        let current_pos = wal.current_position().await;

                        // Measure GC
                        let start = std::time::Instant::now();
                        black_box(wal.delete_segments_before(current_pos).await.unwrap());
                        total_elapsed += start.elapsed();
                    }

                    total_elapsed
                });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    gc_latency_by_segment_count,
    gc_throughput,
    gc_followed_by_writes,
    gc_with_fragmentation
);
criterion_main!(benches);
