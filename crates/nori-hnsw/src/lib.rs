//! HNSW (Hierarchical Navigable Small World) index for NoriKV.
//!
//! Provides efficient approximate nearest neighbor (ANN) search using the HNSW
//! algorithm. HNSW builds a multi-layer graph where:
//!
//! - Higher layers have fewer nodes (exponential decay)
//! - Each layer is a navigable small-world graph
//! - Search starts at top layer and descends
//!
//! # Architecture
//!
//! HNSW is used for L1-L2 SSTables in NoriKV's tiered vector indexing:
//!
//! ```text
//! Memtable (L0):  nori-vector::BruteForceIndex (small, fast inserts)
//!      ↓ flush
//! L1-L2 SSTable:  nori-hnsw::HnswIndex (in-memory graph)  <-- This crate
//!      ↓ compact
//! L3+ SSTable:    nori-vamana (disk-resident graph with PQ)
//! ```
//!
//! # Parameters
//!
//! - `M`: Max connections per node per layer (default: 16)
//! - `ef_construction`: Beam width during index building (default: 200)
//! - `ef_search`: Beam width during search (default: 100)
//! - `max_layers`: Maximum number of layers (default: 16)
//!
//! # Example
//!
//! ```
//! use nori_hnsw::{HnswIndex, HnswConfig};
//! use nori_vector::{DistanceFunction, VectorIndex};
//!
//! // Create an HNSW index for 128-dimensional vectors
//! let config = HnswConfig::default();
//! let index = HnswIndex::new(128, DistanceFunction::Euclidean, config);
//!
//! // Insert vectors
//! index.insert("vec1", &[1.0; 128]).unwrap();
//! index.insert("vec2", &[2.0; 128]).unwrap();
//!
//! // Search for nearest neighbors
//! let results = index.search(&[1.5; 128], 10).unwrap();
//! ```

mod graph;
mod index;
mod layer;

pub use index::{HnswConfig, HnswIndex};

/// Error type for HNSW operations.
#[derive(Debug, thiserror::Error)]
pub enum HnswError {
    #[error("Vector error: {0}")]
    Vector(#[from] nori_vector::VectorError),

    #[error("Graph error: {0}")]
    Graph(String),

    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Result type for HNSW operations.
pub type Result<T> = std::result::Result<T, HnswError>;
