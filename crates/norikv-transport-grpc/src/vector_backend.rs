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

/// Adapter to make ReplicatedLSM implement VectorBackend.
pub struct SingleShardVectorBackend {
    lsm: std::sync::Arc<nori_raft::ReplicatedLSM>,
}

impl SingleShardVectorBackend {
    /// Create a new single-shard vector backend.
    pub fn new(lsm: std::sync::Arc<nori_raft::ReplicatedLSM>) -> Self {
        Self { lsm }
    }

    /// Convert backend distance function to nori_lsm type.
    fn to_lsm_distance(dist: DistanceFunction) -> nori_lsm::DistanceFunction {
        match dist {
            DistanceFunction::Euclidean => nori_lsm::DistanceFunction::Euclidean,
            DistanceFunction::Cosine => nori_lsm::DistanceFunction::Cosine,
            DistanceFunction::InnerProduct => nori_lsm::DistanceFunction::InnerProduct,
        }
    }

    /// Convert backend index type to nori_lsm type.
    fn to_lsm_index_type(idx_type: VectorIndexType) -> nori_lsm::VectorIndexType {
        match idx_type {
            VectorIndexType::BruteForce => nori_lsm::VectorIndexType::BruteForce,
            VectorIndexType::Hnsw => nori_lsm::VectorIndexType::Hnsw,
        }
    }
}

#[async_trait::async_trait]
impl VectorBackend for SingleShardVectorBackend {
    async fn create_index(
        &self,
        namespace: String,
        dimensions: usize,
        distance: DistanceFunction,
        index_type: VectorIndexType,
    ) -> Result<bool, RaftError> {
        self.lsm
            .replicated_vector_create_index(
                namespace,
                dimensions,
                Self::to_lsm_distance(distance),
                Self::to_lsm_index_type(index_type),
            )
            .await?;
        Ok(true)
    }

    async fn drop_index(&self, namespace: String) -> Result<bool, RaftError> {
        self.lsm.replicated_vector_drop_index(namespace).await?;
        Ok(true)
    }

    async fn insert(
        &self,
        namespace: String,
        id: String,
        vector: Vec<f32>,
    ) -> Result<LogIndex, RaftError> {
        self.lsm
            .replicated_vector_insert(namespace, id, vector)
            .await
    }

    async fn delete(&self, namespace: String, id: String) -> Result<bool, RaftError> {
        self.lsm.replicated_vector_delete(namespace, id).await?;
        Ok(true)
    }

    async fn search(
        &self,
        namespace: &str,
        query: &[f32],
        k: usize,
        include_vectors: bool,
    ) -> Result<Vec<VectorMatch>, RaftError> {
        let matches = self
            .lsm
            .replicated_vector_search(namespace, query, k)
            .await?;

        Ok(matches
            .into_iter()
            .map(|m| VectorMatch {
                id: m.id,
                distance: m.distance,
                vector: if include_vectors { m.vector } else { None },
            })
            .collect())
    }

    async fn get(&self, namespace: &str, id: &str) -> Result<Option<Vec<f32>>, RaftError> {
        self.lsm.replicated_vector_get(namespace, id).await
    }

    fn is_leader(&self) -> bool {
        self.lsm.is_leader()
    }

    fn leader(&self) -> Option<NodeId> {
        self.lsm.leader()
    }

    fn current_term(&self) -> Term {
        self.lsm.raft().current_term()
    }

    fn commit_index(&self) -> LogIndex {
        self.lsm.raft().commit_index()
    }
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
