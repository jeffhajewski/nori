//! DiskANN/Vamana index for NoriKV.
//!
//! Provides disk-resident approximate nearest neighbor (ANN) search using the
//! Vamana algorithm from Microsoft's DiskANN paper. Designed for L3+ SSTables
//! where memory efficiency is critical.
//!
//! # Architecture
//!
//! Vamana is used for L3+ SSTables in NoriKV's tiered vector indexing:
//!
//! ```text
//! Memtable (L0):  nori-vector::BruteForceIndex (small, fast inserts)
//!      ↓ flush
//! L1-L2 SSTable:  nori-hnsw (in-memory graph)
//!      ↓ compact
//! L3+ SSTable:    nori-vamana::VamanaIndex (disk-resident)  <-- This crate
//! ```
//!
//! # Key Features
//!
//! - **Disk-resident graph**: Graph stored on disk, accessed via mmap
//! - **Product Quantization**: 32x+ memory compression for distance estimation
//! - **Two-phase search**: Fast PQ search + exact re-ranking
//! - **High degree graph**: R=64-128 for fewer disk seeks
//!
//! # Components
//!
//! - [`ProductQuantizer`]: Compresses vectors for memory-efficient search
//! - [`VamanaIndex`]: Disk-resident vector index
//! - [`VamanaBuilder`]: Builds Vamana index from vectors
//!
//! # Example
//!
//! ```ignore
//! use nori_vamana::{VamanaBuilder, VamanaConfig, ProductQuantizer};
//! use nori_vector::DistanceFunction;
//!
//! // Build index from vectors
//! let config = VamanaConfig::default();
//! let builder = VamanaBuilder::new(128, DistanceFunction::Euclidean, config);
//!
//! for (id, vector) in vectors {
//!     builder.add(&id, &vector)?;
//! }
//!
//! let index = builder.build("/path/to/index")?;
//!
//! // Search
//! let results = index.search(&query, 10)?;
//! ```

mod pq;
mod graph;
mod builder;
mod index;

pub use pq::{ProductQuantizer, PQConfig};
pub use builder::{VamanaBuilder, VamanaConfig};
pub use index::VamanaIndex;

/// Error type for Vamana operations.
#[derive(Debug, thiserror::Error)]
pub enum VamanaError {
    #[error("Vector error: {0}")]
    Vector(#[from] nori_vector::VectorError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("PQ error: {0}")]
    ProductQuantization(String),

    #[error("Graph error: {0}")]
    Graph(String),

    #[error("Build error: {0}")]
    Build(String),
}

/// Result type for Vamana operations.
pub type Result<T> = std::result::Result<T, VamanaError>;
