//! Vector backend trait for abstracting over vector storage.

use nori_raft::{LogIndex, NodeId, RaftError, Term};

/// Distance function for vector similarity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistanceFunction {
    Euclidean,
    Cosine,
    InnerProduct,
}

/// Vector index type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorIndexType {
    BruteForce,
    Hnsw,
}

/// A vector search match result.
#[derive(Debug, Clone)]
pub struct VectorMatch {
    pub id: String,
    pub distance: f32,
    pub vector: Option<Vec<f32>>,
}

/// Backend interface for vector operations.
///
/// This trait abstracts over:
/// - Single-shard: Direct VectorStorage/VectorEngine
/// - Multi-shard: Router that distributes vectors across shards by namespace
#[async_trait::async_trait]
pub trait VectorBackend: Send + Sync {
    /// Create a vector index for a namespace.
    async fn create_index(
        &self,
        namespace: String,
        dimensions: usize,
        distance: DistanceFunction,
        index_type: VectorIndexType,
    ) -> Result<bool, RaftError>;

    /// Drop a vector index.
    async fn drop_index(&self, namespace: String) -> Result<bool, RaftError>;

    /// Insert a vector.
    async fn insert(
        &self,
        namespace: String,
        id: String,
        vector: Vec<f32>,
    ) -> Result<LogIndex, RaftError>;

    /// Delete a vector.
    async fn delete(&self, namespace: String, id: String) -> Result<bool, RaftError>;

    /// Search for similar vectors.
    async fn search(
        &self,
        namespace: &str,
        query: &[f32],
        k: usize,
        include_vectors: bool,
    ) -> Result<Vec<VectorMatch>, RaftError>;

    /// Get a specific vector by ID.
    async fn get(&self, namespace: &str, id: &str) -> Result<Option<Vec<f32>>, RaftError>;

    /// Check if this backend is currently leader (for the relevant shard).
    fn is_leader(&self) -> bool;

    /// Get the leader node ID (if known).
    fn leader(&self) -> Option<NodeId>;

    /// Get the current Raft term.
    fn current_term(&self) -> Term;

    /// Get the commit index.
    fn commit_index(&self) -> LogIndex;
}

#[cfg(test)]
pub struct MockVectorBackend;

#[cfg(test)]
impl MockVectorBackend {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl VectorBackend for MockVectorBackend {
    async fn create_index(
        &self,
        _namespace: String,
        _dimensions: usize,
        _distance: DistanceFunction,
        _index_type: VectorIndexType,
    ) -> Result<bool, RaftError> {
        Ok(true)
    }

    async fn drop_index(&self, _namespace: String) -> Result<bool, RaftError> {
        Ok(true)
    }

    async fn insert(
        &self,
        _namespace: String,
        _id: String,
        _vector: Vec<f32>,
    ) -> Result<LogIndex, RaftError> {
        Ok(LogIndex(1))
    }

    async fn delete(&self, _namespace: String, _id: String) -> Result<bool, RaftError> {
        Ok(true)
    }

    async fn search(
        &self,
        _namespace: &str,
        _query: &[f32],
        k: usize,
        include_vectors: bool,
    ) -> Result<Vec<VectorMatch>, RaftError> {
        let matches: Vec<VectorMatch> = (0..k.min(3))
            .map(|i| VectorMatch {
                id: format!("mock-{}", i),
                distance: i as f32 * 0.1,
                vector: if include_vectors {
                    Some(vec![0.0; 128])
                } else {
                    None
                },
            })
            .collect();
        Ok(matches)
    }

    async fn get(&self, _namespace: &str, _id: &str) -> Result<Option<Vec<f32>>, RaftError> {
        Ok(Some(vec![0.0; 128]))
    }

    fn is_leader(&self) -> bool {
        true
    }

    fn leader(&self) -> Option<NodeId> {
        Some(NodeId::new("mock-leader"))
    }

    fn current_term(&self) -> Term {
        Term(1)
    }

    fn commit_index(&self) -> LogIndex {
        LogIndex(1)
    }
}
