//! Vector similarity search for NoriKV.
//!
//! This crate provides the core abstractions and implementations for vector
//! similarity search:
//!
//! - **Distance functions**: Euclidean (L2), Cosine, Inner Product
//! - **VectorIndex trait**: Common interface for vector indices
//! - **BruteForceIndex**: Linear scan search (baseline, used for memtable)
//!
//! # Architecture
//!
//! This crate is part of NoriKV's tiered vector indexing strategy:
//!
//! ```text
//! Memtable (L0):  nori-vector::BruteForceIndex (small, fast inserts)
//!      ↓ flush
//! L1-L2 SSTable:  nori-hnsw (in-memory graph)
//!      ↓ compact
//! L3+ SSTable:    nori-vamana (disk-resident graph with PQ)
//! ```
//!
//! # Example
//!
//! ```
//! use nori_vector::{BruteForceIndex, DistanceFunction, VectorIndex};
//!
//! // Create an index for 128-dimensional vectors
//! let mut index = BruteForceIndex::new(128, DistanceFunction::Euclidean);
//!
//! // Insert vectors
//! index.insert("vec1", &[1.0; 128]).unwrap();
//! index.insert("vec2", &[2.0; 128]).unwrap();
//!
//! // Search for nearest neighbors
//! let results = index.search(&[1.5; 128], 2).unwrap();
//! assert_eq!(results.len(), 2);
//! ```

mod brute;
mod distance;
mod traits;

pub use brute::BruteForceIndex;
pub use distance::{cosine_distance, euclidean_distance, inner_product, DistanceFunction};
pub use traits::{VectorIndex, VectorMatch};

/// Error type for vector operations.
#[derive(Debug, thiserror::Error)]
pub enum VectorError {
    #[error("Dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    #[error("Vector not found: {0}")]
    NotFound(String),

    #[error("Invalid vector: {0}")]
    InvalidVector(String),

    #[error("Index error: {0}")]
    IndexError(String),
}

/// Result type for vector operations.
pub type Result<T> = std::result::Result<T, VectorError>;
