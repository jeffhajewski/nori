//! Core traits for vector indices.
//!
//! The `VectorIndex` trait defines the common interface implemented by all
//! vector index types (BruteForce, HNSW, Vamana).

use crate::Result;

/// A match returned from vector search.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorMatch {
    /// The vector ID.
    pub id: String,
    /// Distance/score (lower = more similar).
    pub distance: f32,
    /// The vector data (optional, for returning with results).
    pub vector: Option<Vec<f32>>,
}

impl VectorMatch {
    /// Create a new vector match.
    pub fn new(id: impl Into<String>, distance: f32) -> Self {
        Self {
            id: id.into(),
            distance,
            vector: None,
        }
    }

    /// Create a new vector match with vector data.
    pub fn with_vector(id: impl Into<String>, distance: f32, vector: Vec<f32>) -> Self {
        Self {
            id: id.into(),
            distance,
            vector: Some(vector),
        }
    }
}

impl Eq for VectorMatch {}

impl PartialOrd for VectorMatch {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VectorMatch {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare by distance (lower is better)
        // Use total_cmp for proper NaN handling
        self.distance.total_cmp(&other.distance)
    }
}

/// Common interface for vector indices.
///
/// All vector index implementations (BruteForce, HNSW, Vamana) implement this
/// trait, allowing them to be used interchangeably.
///
/// # Thread Safety
///
/// Implementations must be `Send + Sync` to allow concurrent access from
/// multiple threads. Internal synchronization is the responsibility of each
/// implementation.
pub trait VectorIndex: Send + Sync {
    /// Insert a vector with the given ID.
    ///
    /// If a vector with the same ID already exists, it will be replaced.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The vector dimension doesn't match the index dimension
    /// - The vector contains invalid values (NaN, Inf)
    fn insert(&self, id: &str, vector: &[f32]) -> Result<()>;

    /// Delete a vector by ID.
    ///
    /// Returns `true` if the vector was found and deleted, `false` if not found.
    fn delete(&self, id: &str) -> Result<bool>;

    /// Search for the k nearest neighbors to the query vector.
    ///
    /// Returns up to `k` matches, sorted by distance (ascending).
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Maximum number of results to return
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The query dimension doesn't match the index dimension
    /// - The query contains invalid values (NaN, Inf)
    fn search(&self, query: &[f32], k: usize) -> Result<Vec<VectorMatch>>;

    /// Get a vector by ID.
    ///
    /// Returns `None` if not found.
    fn get(&self, id: &str) -> Result<Option<Vec<f32>>>;

    /// Check if a vector exists.
    fn contains(&self, id: &str) -> bool;

    /// Get the number of vectors in the index.
    fn len(&self) -> usize;

    /// Check if the index is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the dimension of vectors in this index.
    fn dimensions(&self) -> usize;

    /// Clear all vectors from the index.
    fn clear(&self);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_match_ordering() {
        let a = VectorMatch::new("a", 1.0);
        let b = VectorMatch::new("b", 2.0);
        let c = VectorMatch::new("c", 0.5);

        let mut matches = vec![a, b, c];
        matches.sort();

        assert_eq!(matches[0].id, "c");
        assert_eq!(matches[1].id, "a");
        assert_eq!(matches[2].id, "b");
    }
}
