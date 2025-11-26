//! Brute force vector index.
//!
//! Linear scan search - O(n) but simple and fast for small datasets.
//! Used for memtable-level vectors where the dataset is small and
//! fast insertion is important.

use crate::distance::DistanceFunction;
use crate::traits::{VectorIndex, VectorMatch};
use crate::{Result, VectorError};
use parking_lot::RwLock;
use std::collections::HashMap;

/// Brute force vector index.
///
/// Stores vectors in a HashMap and performs linear scan for search.
/// Thread-safe via RwLock.
///
/// # Use Cases
///
/// - Memtable-level search (small dataset, fast inserts)
/// - Baseline for benchmarking other indices
/// - Testing and verification
///
/// # Performance
///
/// - Insert: O(1)
/// - Delete: O(1)
/// - Search: O(n * d) where n = vectors, d = dimensions
///
/// For datasets larger than ~10K vectors, consider HNSW or Vamana.
pub struct BruteForceIndex {
    /// Vector storage: id -> vector
    vectors: RwLock<HashMap<String, Vec<f32>>>,
    /// Vector dimensions (all vectors must have this dimension)
    dimensions: usize,
    /// Distance function to use
    distance: DistanceFunction,
}

impl BruteForceIndex {
    /// Create a new brute force index.
    ///
    /// # Arguments
    ///
    /// * `dimensions` - The dimension of vectors to be stored
    /// * `distance` - Distance function to use for similarity
    ///
    /// # Example
    ///
    /// ```
    /// use nori_vector::{BruteForceIndex, DistanceFunction};
    ///
    /// let index = BruteForceIndex::new(128, DistanceFunction::Euclidean);
    /// ```
    pub fn new(dimensions: usize, distance: DistanceFunction) -> Self {
        Self {
            vectors: RwLock::new(HashMap::new()),
            dimensions,
            distance,
        }
    }

    /// Get the distance function used by this index.
    pub fn distance_function(&self) -> DistanceFunction {
        self.distance
    }

    /// Validate a vector's dimensions and values.
    fn validate_vector(&self, vector: &[f32]) -> Result<()> {
        if vector.len() != self.dimensions {
            return Err(VectorError::DimensionMismatch {
                expected: self.dimensions,
                actual: vector.len(),
            });
        }

        // Check for invalid values
        for (i, &v) in vector.iter().enumerate() {
            if v.is_nan() {
                return Err(VectorError::InvalidVector(format!(
                    "NaN value at index {}",
                    i
                )));
            }
            if v.is_infinite() {
                return Err(VectorError::InvalidVector(format!(
                    "Infinite value at index {}",
                    i
                )));
            }
        }

        Ok(())
    }
}

impl VectorIndex for BruteForceIndex {
    fn insert(&self, id: &str, vector: &[f32]) -> Result<()> {
        self.validate_vector(vector)?;

        let mut vectors = self.vectors.write();
        vectors.insert(id.to_string(), vector.to_vec());
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<bool> {
        let mut vectors = self.vectors.write();
        Ok(vectors.remove(id).is_some())
    }

    fn search(&self, query: &[f32], k: usize) -> Result<Vec<VectorMatch>> {
        self.validate_vector(query)?;

        if k == 0 {
            return Ok(vec![]);
        }

        let vectors = self.vectors.read();

        // Compute distances for all vectors
        let mut results: Vec<VectorMatch> = vectors
            .iter()
            .map(|(id, vec)| {
                let dist = self.distance.distance(query, vec);
                VectorMatch::new(id.clone(), dist)
            })
            .collect();

        // Sort by distance (ascending)
        results.sort();

        // Take top-k
        results.truncate(k);

        Ok(results)
    }

    fn get(&self, id: &str) -> Result<Option<Vec<f32>>> {
        let vectors = self.vectors.read();
        Ok(vectors.get(id).cloned())
    }

    fn contains(&self, id: &str) -> bool {
        let vectors = self.vectors.read();
        vectors.contains_key(id)
    }

    fn len(&self) -> usize {
        let vectors = self.vectors.read();
        vectors.len()
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn clear(&self) {
        let mut vectors = self.vectors.write();
        vectors.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_index() -> BruteForceIndex {
        BruteForceIndex::new(3, DistanceFunction::Euclidean)
    }

    #[test]
    fn test_insert_and_get() {
        let index = create_test_index();

        index.insert("vec1", &[1.0, 2.0, 3.0]).unwrap();
        index.insert("vec2", &[4.0, 5.0, 6.0]).unwrap();

        assert_eq!(index.len(), 2);
        assert!(index.contains("vec1"));
        assert!(index.contains("vec2"));
        assert!(!index.contains("vec3"));

        let vec1 = index.get("vec1").unwrap().unwrap();
        assert_eq!(vec1, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_insert_replaces_existing() {
        let index = create_test_index();

        index.insert("vec1", &[1.0, 2.0, 3.0]).unwrap();
        index.insert("vec1", &[7.0, 8.0, 9.0]).unwrap();

        assert_eq!(index.len(), 1);
        let vec1 = index.get("vec1").unwrap().unwrap();
        assert_eq!(vec1, vec![7.0, 8.0, 9.0]);
    }

    #[test]
    fn test_delete() {
        let index = create_test_index();

        index.insert("vec1", &[1.0, 2.0, 3.0]).unwrap();
        assert_eq!(index.len(), 1);

        let deleted = index.delete("vec1").unwrap();
        assert!(deleted);
        assert_eq!(index.len(), 0);

        let deleted_again = index.delete("vec1").unwrap();
        assert!(!deleted_again);
    }

    #[test]
    fn test_search_euclidean() {
        let index = create_test_index();

        // Insert some vectors
        index.insert("origin", &[0.0, 0.0, 0.0]).unwrap();
        index.insert("near", &[1.0, 1.0, 1.0]).unwrap();
        index.insert("far", &[10.0, 10.0, 10.0]).unwrap();

        // Search from origin
        let results = index.search(&[0.0, 0.0, 0.0], 3).unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].id, "origin");
        assert!(results[0].distance < 0.001);
        assert_eq!(results[1].id, "near");
        assert_eq!(results[2].id, "far");
    }

    #[test]
    fn test_search_top_k() {
        let index = create_test_index();

        for i in 0..10 {
            index
                .insert(&format!("vec{}", i), &[i as f32, 0.0, 0.0])
                .unwrap();
        }

        // Search for top 3
        let results = index.search(&[0.0, 0.0, 0.0], 3).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].id, "vec0");
        assert_eq!(results[1].id, "vec1");
        assert_eq!(results[2].id, "vec2");
    }

    #[test]
    fn test_search_empty_index() {
        let index = create_test_index();
        let results = index.search(&[1.0, 2.0, 3.0], 5).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_k_zero() {
        let index = create_test_index();
        index.insert("vec1", &[1.0, 2.0, 3.0]).unwrap();

        let results = index.search(&[1.0, 2.0, 3.0], 0).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_dimension_mismatch() {
        let index = create_test_index();

        let result = index.insert("vec1", &[1.0, 2.0]); // Wrong dimension
        assert!(matches!(result, Err(VectorError::DimensionMismatch { .. })));

        index.insert("vec1", &[1.0, 2.0, 3.0]).unwrap();
        let result = index.search(&[1.0, 2.0], 1); // Wrong dimension
        assert!(matches!(result, Err(VectorError::DimensionMismatch { .. })));
    }

    #[test]
    fn test_invalid_values() {
        let index = create_test_index();

        let result = index.insert("vec1", &[1.0, f32::NAN, 3.0]);
        assert!(matches!(result, Err(VectorError::InvalidVector(_))));

        let result = index.insert("vec1", &[1.0, f32::INFINITY, 3.0]);
        assert!(matches!(result, Err(VectorError::InvalidVector(_))));
    }

    #[test]
    fn test_clear() {
        let index = create_test_index();

        index.insert("vec1", &[1.0, 2.0, 3.0]).unwrap();
        index.insert("vec2", &[4.0, 5.0, 6.0]).unwrap();
        assert_eq!(index.len(), 2);

        index.clear();
        assert_eq!(index.len(), 0);
        assert!(index.is_empty());
    }

    #[test]
    fn test_cosine_distance() {
        let index = BruteForceIndex::new(2, DistanceFunction::Cosine);

        // Same direction
        index.insert("same", &[1.0, 0.0]).unwrap();
        // Orthogonal
        index.insert("ortho", &[0.0, 1.0]).unwrap();
        // Opposite
        index.insert("opposite", &[-1.0, 0.0]).unwrap();

        let results = index.search(&[1.0, 0.0], 3).unwrap();

        assert_eq!(results[0].id, "same");
        assert!(results[0].distance < 0.001); // cos dist = 0

        assert_eq!(results[1].id, "ortho");
        assert!((results[1].distance - 1.0).abs() < 0.001); // cos dist = 1

        assert_eq!(results[2].id, "opposite");
        assert!((results[2].distance - 2.0).abs() < 0.001); // cos dist = 2
    }

    #[test]
    fn test_inner_product() {
        let index = BruteForceIndex::new(3, DistanceFunction::InnerProduct);

        // High inner product (negative distance = better)
        index.insert("high", &[1.0, 1.0, 1.0]).unwrap();
        // Low inner product
        index.insert("low", &[0.1, 0.1, 0.1]).unwrap();
        // Negative inner product
        index.insert("neg", &[-1.0, -1.0, -1.0]).unwrap();

        let results = index.search(&[1.0, 1.0, 1.0], 3).unwrap();

        // Inner product uses negative for distance, so highest IP = lowest distance
        assert_eq!(results[0].id, "high"); // IP = 3, dist = -3
        assert_eq!(results[1].id, "low"); // IP = 0.3, dist = -0.3
        assert_eq!(results[2].id, "neg"); // IP = -3, dist = 3
    }

    #[test]
    fn test_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let index = Arc::new(BruteForceIndex::new(3, DistanceFunction::Euclidean));

        let mut handles = vec![];

        // Spawn writers
        for i in 0..10 {
            let index = Arc::clone(&index);
            handles.push(thread::spawn(move || {
                index
                    .insert(&format!("vec{}", i), &[i as f32, 0.0, 0.0])
                    .unwrap();
            }));
        }

        // Spawn readers
        for _ in 0..10 {
            let index = Arc::clone(&index);
            handles.push(thread::spawn(move || {
                let _ = index.search(&[0.0, 0.0, 0.0], 5);
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(index.len(), 10);
    }
}
