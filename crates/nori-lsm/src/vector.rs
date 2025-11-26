//! Vector storage for LSM engine.
//!
//! Manages vector indexes organized by namespace. Each namespace has its own
//! index configuration (dimensions, distance function, index type).
//!
//! This module integrates with Raft consensus through the Command enum,
//! allowing vector operations to be replicated across the cluster.

use crate::{Error, Result};
use nori_hnsw::{HnswConfig, HnswIndex};
use nori_vector::{BruteForceIndex, DistanceFunction, VectorIndex, VectorMatch};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Type of vector index to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VectorIndexType {
    /// Brute force linear scan (exact, for small datasets)
    BruteForce,
    /// HNSW graph index (approximate, for larger datasets)
    Hnsw,
}

impl Default for VectorIndexType {
    fn default() -> Self {
        Self::Hnsw
    }
}

/// Configuration for a vector index namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorNamespaceConfig {
    /// Vector dimensions
    pub dimensions: usize,
    /// Distance function
    pub distance: DistanceFunction,
    /// Index type
    pub index_type: VectorIndexType,
    /// HNSW-specific config (ignored for BruteForce)
    pub hnsw_config: Option<HnswConfig>,
}

impl VectorNamespaceConfig {
    /// Create a new namespace config with default HNSW index.
    pub fn new(dimensions: usize, distance: DistanceFunction) -> Self {
        Self {
            dimensions,
            distance,
            index_type: VectorIndexType::Hnsw,
            hnsw_config: None,
        }
    }

    /// Use brute force index instead of HNSW.
    pub fn with_brute_force(mut self) -> Self {
        self.index_type = VectorIndexType::BruteForce;
        self
    }

    /// Set custom HNSW config.
    pub fn with_hnsw_config(mut self, config: HnswConfig) -> Self {
        self.index_type = VectorIndexType::Hnsw;
        self.hnsw_config = Some(config);
        self
    }
}

/// A vector index that can be either BruteForce or HNSW.
enum IndexImpl {
    BruteForce(BruteForceIndex),
    Hnsw(HnswIndex),
}

impl IndexImpl {
    fn insert(&self, id: &str, vector: &[f32]) -> Result<()> {
        match self {
            IndexImpl::BruteForce(idx) => idx
                .insert(id, vector)
                .map_err(|e| Error::Internal(format!("Vector insert failed: {}", e))),
            IndexImpl::Hnsw(idx) => idx
                .insert(id, vector)
                .map_err(|e| Error::Internal(format!("Vector insert failed: {}", e))),
        }
    }

    fn delete(&self, id: &str) -> Result<bool> {
        match self {
            IndexImpl::BruteForce(idx) => idx
                .delete(id)
                .map_err(|e| Error::Internal(format!("Vector delete failed: {}", e))),
            IndexImpl::Hnsw(idx) => idx
                .delete(id)
                .map_err(|e| Error::Internal(format!("Vector delete failed: {}", e))),
        }
    }

    fn search(&self, query: &[f32], k: usize) -> Result<Vec<VectorMatch>> {
        match self {
            IndexImpl::BruteForce(idx) => idx
                .search(query, k)
                .map_err(|e| Error::Internal(format!("Vector search failed: {}", e))),
            IndexImpl::Hnsw(idx) => idx
                .search(query, k)
                .map_err(|e| Error::Internal(format!("Vector search failed: {}", e))),
        }
    }

    fn get(&self, id: &str) -> Result<Option<Vec<f32>>> {
        match self {
            IndexImpl::BruteForce(idx) => idx
                .get(id)
                .map_err(|e| Error::Internal(format!("Vector get failed: {}", e))),
            IndexImpl::Hnsw(idx) => idx
                .get(id)
                .map_err(|e| Error::Internal(format!("Vector get failed: {}", e))),
        }
    }

    fn len(&self) -> usize {
        match self {
            IndexImpl::BruteForce(idx) => idx.len(),
            IndexImpl::Hnsw(idx) => idx.len(),
        }
    }

    fn dimensions(&self) -> usize {
        match self {
            IndexImpl::BruteForce(idx) => idx.dimensions(),
            IndexImpl::Hnsw(idx) => idx.dimensions(),
        }
    }
}

/// A single vector namespace with its index and configuration.
struct VectorNamespace {
    config: VectorNamespaceConfig,
    index: IndexImpl,
}

impl VectorNamespace {
    fn new(config: VectorNamespaceConfig) -> Self {
        let index = match config.index_type {
            VectorIndexType::BruteForce => {
                IndexImpl::BruteForce(BruteForceIndex::new(config.dimensions, config.distance))
            }
            VectorIndexType::Hnsw => {
                let hnsw_config = config.hnsw_config.clone().unwrap_or_default();
                IndexImpl::Hnsw(HnswIndex::new(config.dimensions, config.distance, hnsw_config))
            }
        };

        Self { config, index }
    }
}

/// Vector storage manager.
///
/// Manages multiple vector namespaces, each with its own index configuration.
/// Thread-safe via internal locking.
pub struct VectorStorage {
    /// Namespaces by name
    namespaces: RwLock<HashMap<String, Arc<VectorNamespace>>>,
}

impl VectorStorage {
    /// Create a new vector storage manager.
    pub fn new() -> Self {
        Self {
            namespaces: RwLock::new(HashMap::new()),
        }
    }

    /// Create a new vector index namespace.
    ///
    /// Returns error if namespace already exists.
    pub fn create_namespace(&self, name: &str, config: VectorNamespaceConfig) -> Result<()> {
        let mut namespaces = self.namespaces.write();

        if namespaces.contains_key(name) {
            return Err(Error::Internal(format!(
                "Vector namespace '{}' already exists",
                name
            )));
        }

        let namespace = Arc::new(VectorNamespace::new(config));
        namespaces.insert(name.to_string(), namespace);

        Ok(())
    }

    /// Drop a vector index namespace.
    ///
    /// Returns true if namespace existed.
    pub fn drop_namespace(&self, name: &str) -> bool {
        self.namespaces.write().remove(name).is_some()
    }

    /// Check if a namespace exists.
    pub fn namespace_exists(&self, name: &str) -> bool {
        self.namespaces.read().contains_key(name)
    }

    /// Get namespace configuration.
    pub fn get_namespace_config(&self, name: &str) -> Option<VectorNamespaceConfig> {
        self.namespaces
            .read()
            .get(name)
            .map(|ns| ns.config.clone())
    }

    /// List all namespaces.
    pub fn list_namespaces(&self) -> Vec<String> {
        self.namespaces.read().keys().cloned().collect()
    }

    /// Insert a vector into a namespace.
    pub fn insert(&self, namespace: &str, id: &str, vector: &[f32]) -> Result<()> {
        let namespaces = self.namespaces.read();
        let ns = namespaces.get(namespace).ok_or_else(|| {
            Error::Internal(format!("Vector namespace '{}' not found", namespace))
        })?;

        if vector.len() != ns.config.dimensions {
            return Err(Error::Internal(format!(
                "Vector dimension mismatch: expected {}, got {}",
                ns.config.dimensions,
                vector.len()
            )));
        }

        ns.index.insert(id, vector)
    }

    /// Delete a vector from a namespace.
    pub fn delete(&self, namespace: &str, id: &str) -> Result<bool> {
        let namespaces = self.namespaces.read();
        let ns = namespaces.get(namespace).ok_or_else(|| {
            Error::Internal(format!("Vector namespace '{}' not found", namespace))
        })?;

        ns.index.delete(id)
    }

    /// Search for nearest neighbors in a namespace.
    pub fn search(&self, namespace: &str, query: &[f32], k: usize) -> Result<Vec<VectorMatch>> {
        let namespaces = self.namespaces.read();
        let ns = namespaces.get(namespace).ok_or_else(|| {
            Error::Internal(format!("Vector namespace '{}' not found", namespace))
        })?;

        if query.len() != ns.config.dimensions {
            return Err(Error::Internal(format!(
                "Query dimension mismatch: expected {}, got {}",
                ns.config.dimensions,
                query.len()
            )));
        }

        ns.index.search(query, k)
    }

    /// Get a vector by ID from a namespace.
    pub fn get(&self, namespace: &str, id: &str) -> Result<Option<Vec<f32>>> {
        let namespaces = self.namespaces.read();
        let ns = namespaces.get(namespace).ok_or_else(|| {
            Error::Internal(format!("Vector namespace '{}' not found", namespace))
        })?;

        ns.index.get(id)
    }

    /// Get the number of vectors in a namespace.
    pub fn len(&self, namespace: &str) -> Result<usize> {
        let namespaces = self.namespaces.read();
        let ns = namespaces.get(namespace).ok_or_else(|| {
            Error::Internal(format!("Vector namespace '{}' not found", namespace))
        })?;

        Ok(ns.index.len())
    }

    /// Get dimensions for a namespace.
    pub fn dimensions(&self, namespace: &str) -> Result<usize> {
        let namespaces = self.namespaces.read();
        let ns = namespaces.get(namespace).ok_or_else(|| {
            Error::Internal(format!("Vector namespace '{}' not found", namespace))
        })?;

        Ok(ns.index.dimensions())
    }

    /// Get total vector count across all namespaces.
    pub fn total_vectors(&self) -> usize {
        self.namespaces
            .read()
            .values()
            .map(|ns| ns.index.len())
            .sum()
    }

    /// Get number of namespaces.
    pub fn namespace_count(&self) -> usize {
        self.namespaces.read().len()
    }
}

impl Default for VectorStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_namespace() {
        let storage = VectorStorage::new();

        let config = VectorNamespaceConfig::new(128, DistanceFunction::Euclidean);
        storage.create_namespace("test", config).unwrap();

        assert!(storage.namespace_exists("test"));
        assert!(!storage.namespace_exists("nonexistent"));
    }

    #[test]
    fn test_create_duplicate_namespace() {
        let storage = VectorStorage::new();

        let config = VectorNamespaceConfig::new(128, DistanceFunction::Euclidean);
        storage.create_namespace("test", config.clone()).unwrap();

        let result = storage.create_namespace("test", config);
        assert!(result.is_err());
    }

    #[test]
    fn test_drop_namespace() {
        let storage = VectorStorage::new();

        let config = VectorNamespaceConfig::new(128, DistanceFunction::Euclidean);
        storage.create_namespace("test", config).unwrap();

        assert!(storage.drop_namespace("test"));
        assert!(!storage.namespace_exists("test"));
        assert!(!storage.drop_namespace("test")); // Already dropped
    }

    #[test]
    fn test_insert_and_search() {
        let storage = VectorStorage::new();

        let config = VectorNamespaceConfig::new(4, DistanceFunction::Euclidean);
        storage.create_namespace("test", config).unwrap();

        // Insert vectors
        storage
            .insert("test", "vec1", &[1.0, 0.0, 0.0, 0.0])
            .unwrap();
        storage
            .insert("test", "vec2", &[0.0, 1.0, 0.0, 0.0])
            .unwrap();
        storage
            .insert("test", "vec3", &[0.0, 0.0, 1.0, 0.0])
            .unwrap();

        assert_eq!(storage.len("test").unwrap(), 3);

        // Search
        let results = storage.search("test", &[1.0, 0.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "vec1");
        assert!(results[0].distance < 0.01); // Should be very close
    }

    #[test]
    fn test_insert_wrong_dimensions() {
        let storage = VectorStorage::new();

        let config = VectorNamespaceConfig::new(4, DistanceFunction::Euclidean);
        storage.create_namespace("test", config).unwrap();

        let result = storage.insert("test", "vec1", &[1.0, 0.0]); // Wrong dims
        assert!(result.is_err());
    }

    #[test]
    fn test_brute_force_index() {
        let storage = VectorStorage::new();

        let config =
            VectorNamespaceConfig::new(4, DistanceFunction::Euclidean).with_brute_force();
        storage.create_namespace("test", config).unwrap();

        storage
            .insert("test", "vec1", &[1.0, 0.0, 0.0, 0.0])
            .unwrap();
        storage
            .insert("test", "vec2", &[0.0, 1.0, 0.0, 0.0])
            .unwrap();

        let results = storage.search("test", &[1.0, 0.0, 0.0, 0.0], 1).unwrap();
        assert_eq!(results[0].id, "vec1");
    }

    #[test]
    fn test_get_vector() {
        let storage = VectorStorage::new();

        let config = VectorNamespaceConfig::new(4, DistanceFunction::Euclidean);
        storage.create_namespace("test", config).unwrap();

        storage
            .insert("test", "vec1", &[1.0, 2.0, 3.0, 4.0])
            .unwrap();

        let vec = storage.get("test", "vec1").unwrap().unwrap();
        assert_eq!(vec, vec![1.0, 2.0, 3.0, 4.0]);

        assert!(storage.get("test", "nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_delete_vector() {
        let storage = VectorStorage::new();

        let config = VectorNamespaceConfig::new(4, DistanceFunction::Euclidean);
        storage.create_namespace("test", config).unwrap();

        storage
            .insert("test", "vec1", &[1.0, 0.0, 0.0, 0.0])
            .unwrap();
        assert_eq!(storage.len("test").unwrap(), 1);

        assert!(storage.delete("test", "vec1").unwrap());
        assert_eq!(storage.len("test").unwrap(), 0);

        assert!(!storage.delete("test", "vec1").unwrap()); // Already deleted
    }

    #[test]
    fn test_list_namespaces() {
        let storage = VectorStorage::new();

        let config = VectorNamespaceConfig::new(4, DistanceFunction::Euclidean);
        storage.create_namespace("ns1", config.clone()).unwrap();
        storage.create_namespace("ns2", config).unwrap();

        let mut namespaces = storage.list_namespaces();
        namespaces.sort();
        assert_eq!(namespaces, vec!["ns1", "ns2"]);
    }
}
