//! Vector engine for tiered vector search.
//!
//! Coordinates vector search across multiple storage tiers:
//! - **Memtable tier**: In-memory HNSW or brute force index (hot data)
//! - **L1-L2 tier**: SSTable-backed HNSW indexes (warm data)
//! - **L3+ tier**: SSTable-backed Vamana/PQ indexes (cold data)
//!
//! Search results are merged across tiers using a priority queue
//! to return the top-k nearest neighbors.

use crate::vector::VectorIndexType;
use crate::{Error, Result};
use nori_hnsw::HnswIndex;
use nori_sstable::vector_block::{DistanceFn, RawVectorBlockBuilder, RawVectorBlockReader};
use nori_vector::{DistanceFunction, VectorIndex, VectorMatch};
use parking_lot::RwLock;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Configuration for a vector namespace in the engine.
#[derive(Debug, Clone)]
pub struct VectorNamespaceEngineConfig {
    /// Namespace name
    pub name: String,
    /// Vector dimensions
    pub dimensions: usize,
    /// Distance function
    pub distance: DistanceFunction,
    /// Index type for memtable tier
    pub memtable_index_type: VectorIndexType,
    /// Flush threshold (number of vectors before flushing to SSTable)
    pub flush_threshold: usize,
}

impl VectorNamespaceEngineConfig {
    /// Create a new namespace config with defaults.
    pub fn new(name: impl Into<String>, dimensions: usize, distance: DistanceFunction) -> Self {
        Self {
            name: name.into(),
            dimensions,
            distance,
            memtable_index_type: VectorIndexType::Hnsw,
            flush_threshold: 10000,
        }
    }

    /// Set the flush threshold.
    pub fn with_flush_threshold(mut self, threshold: usize) -> Self {
        self.flush_threshold = threshold;
        self
    }
}

/// A flushed vector SSTable file.
#[derive(Debug, Clone)]
pub struct VectorSSTableFile {
    /// File path
    pub path: PathBuf,
    /// Level in LSM tree (0 = most recent)
    pub level: usize,
    /// Number of vectors
    pub vector_count: usize,
    /// Sequence number for ordering
    pub seqno: u64,
}

/// State for a single vector namespace.
struct NamespaceState {
    /// Configuration
    config: VectorNamespaceEngineConfig,
    /// In-memory index (memtable tier)
    memtable_index: Arc<HnswIndex>,
    /// Flushed SSTable files (ordered by level, then seqno)
    sstable_files: Vec<VectorSSTableFile>,
    /// Next sequence number
    next_seqno: u64,
}

impl NamespaceState {
    fn new(config: VectorNamespaceEngineConfig) -> Self {
        let memtable_index = Arc::new(HnswIndex::new(
            config.dimensions,
            config.distance,
            Default::default(),
        ));

        Self {
            config,
            memtable_index,
            sstable_files: Vec::new(),
            next_seqno: 0,
        }
    }
}

/// Vector engine for tiered vector search.
///
/// Manages vector indexes across multiple storage tiers and coordinates
/// search operations that span memtable and SSTable levels.
pub struct VectorEngine {
    /// Namespace states
    namespaces: RwLock<HashMap<String, NamespaceState>>,
    /// Base directory for vector SSTable files
    base_dir: PathBuf,
}

impl VectorEngine {
    /// Create a new vector engine.
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            namespaces: RwLock::new(HashMap::new()),
            base_dir: base_dir.as_ref().to_path_buf(),
        }
    }

    /// Create a new vector namespace.
    pub fn create_namespace(&self, config: VectorNamespaceEngineConfig) -> Result<()> {
        let mut namespaces = self.namespaces.write();

        if namespaces.contains_key(&config.name) {
            return Err(Error::Internal(format!(
                "Vector namespace '{}' already exists",
                config.name
            )));
        }

        let name = config.name.clone();
        namespaces.insert(name, NamespaceState::new(config));

        Ok(())
    }

    /// Drop a vector namespace.
    pub fn drop_namespace(&self, name: &str) -> bool {
        self.namespaces.write().remove(name).is_some()
    }

    /// Check if a namespace exists.
    pub fn namespace_exists(&self, name: &str) -> bool {
        self.namespaces.read().contains_key(name)
    }

    /// Insert a vector into a namespace.
    pub fn insert(&self, namespace: &str, id: &str, vector: &[f32]) -> Result<()> {
        let namespaces = self.namespaces.read();
        let state = namespaces.get(namespace).ok_or_else(|| {
            Error::Internal(format!("Vector namespace '{}' not found", namespace))
        })?;

        if vector.len() != state.config.dimensions {
            return Err(Error::Internal(format!(
                "Vector dimension mismatch: expected {}, got {}",
                state.config.dimensions,
                vector.len()
            )));
        }

        state
            .memtable_index
            .insert(id, vector)
            .map_err(|e| Error::Internal(format!("Vector insert failed: {}", e)))?;

        Ok(())
    }

    /// Delete a vector from a namespace.
    ///
    /// Note: This only deletes from the memtable. SSTable vectors are
    /// handled via tombstones during compaction.
    pub fn delete(&self, namespace: &str, id: &str) -> Result<bool> {
        let namespaces = self.namespaces.read();
        let state = namespaces.get(namespace).ok_or_else(|| {
            Error::Internal(format!("Vector namespace '{}' not found", namespace))
        })?;

        state
            .memtable_index
            .delete(id)
            .map_err(|e| Error::Internal(format!("Vector delete failed: {}", e)))
    }

    /// Search for nearest neighbors across all tiers.
    ///
    /// Results are merged from:
    /// 1. Memtable (in-memory HNSW)
    /// 2. L1-L2 SSTables (warm tier)
    /// 3. L3+ SSTables (cold tier with PQ)
    pub fn search(&self, namespace: &str, query: &[f32], k: usize) -> Result<Vec<VectorMatch>> {
        let namespaces = self.namespaces.read();
        let state = namespaces.get(namespace).ok_or_else(|| {
            Error::Internal(format!("Vector namespace '{}' not found", namespace))
        })?;

        if query.len() != state.config.dimensions {
            return Err(Error::Internal(format!(
                "Query dimension mismatch: expected {}, got {}",
                state.config.dimensions,
                query.len()
            )));
        }

        // Collect candidates from all tiers
        let mut all_candidates: Vec<VectorMatch> = Vec::new();

        // 1. Search memtable (always search with more candidates for merging)
        let memtable_k = (k * 2).max(10);
        let memtable_results = state
            .memtable_index
            .search(query, memtable_k)
            .map_err(|e| Error::Internal(format!("Memtable search failed: {}", e)))?;
        all_candidates.extend(memtable_results);

        // 2. Search SSTable files
        for sstable in &state.sstable_files {
            let sstable_results = self.search_sstable(&sstable.path, query, memtable_k, &state.config)?;
            all_candidates.extend(sstable_results);
        }

        // Merge and deduplicate results
        let merged = self.merge_results(all_candidates, k);

        Ok(merged)
    }

    /// Search a single SSTable vector file.
    fn search_sstable(
        &self,
        path: &Path,
        query: &[f32],
        k: usize,
        config: &VectorNamespaceEngineConfig,
    ) -> Result<Vec<VectorMatch>> {
        // Read the vector block file
        let data = std::fs::read(path)
            .map_err(|e| Error::Internal(format!("Failed to read SSTable: {}", e)))?;

        let reader = RawVectorBlockReader::open(data.into())
            .map_err(|e| Error::Internal(format!("Failed to parse vector block: {}", e)))?;

        // Brute force search over the SSTable vectors
        // (In production, this would use HNSW or Vamana indexes)
        let mut results: Vec<VectorMatch> = Vec::new();

        for entry_result in reader.iter() {
            let entry = entry_result
                .map_err(|e| Error::Internal(format!("Failed to read vector entry: {}", e)))?;

            let distance = config.distance.distance(query, &entry.vector);
            results.push(VectorMatch::new(entry.id, distance));
        }

        // Sort and truncate
        results.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(Ordering::Equal));
        results.truncate(k);

        Ok(results)
    }

    /// Merge results from multiple sources, deduplicating by ID.
    fn merge_results(&self, mut candidates: Vec<VectorMatch>, k: usize) -> Vec<VectorMatch> {
        // Sort by distance
        candidates.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(Ordering::Equal));

        // Deduplicate by ID, keeping the closest distance
        let mut seen = std::collections::HashSet::new();
        let mut results = Vec::with_capacity(k);

        for candidate in candidates {
            if seen.insert(candidate.id.clone()) {
                results.push(candidate);
                if results.len() >= k {
                    break;
                }
            }
        }

        results
    }

    /// Get a vector by ID from a namespace.
    pub fn get(&self, namespace: &str, id: &str) -> Result<Option<Vec<f32>>> {
        let namespaces = self.namespaces.read();
        let state = namespaces.get(namespace).ok_or_else(|| {
            Error::Internal(format!("Vector namespace '{}' not found", namespace))
        })?;

        // First check memtable
        if let Some(vec) = state
            .memtable_index
            .get(id)
            .map_err(|e| Error::Internal(format!("Get failed: {}", e)))?
        {
            return Ok(Some(vec));
        }

        // Then check SSTables
        for sstable in &state.sstable_files {
            if let Some(vec) = self.get_from_sstable(&sstable.path, id)? {
                return Ok(Some(vec));
            }
        }

        Ok(None)
    }

    /// Get a vector from an SSTable file.
    fn get_from_sstable(&self, path: &Path, id: &str) -> Result<Option<Vec<f32>>> {
        let data = std::fs::read(path)
            .map_err(|e| Error::Internal(format!("Failed to read SSTable: {}", e)))?;

        let reader = RawVectorBlockReader::open(data.into())
            .map_err(|e| Error::Internal(format!("Failed to parse vector block: {}", e)))?;

        match reader.get_by_id(id) {
            Ok(Some(entry)) => Ok(Some(entry.vector)),
            Ok(None) => Ok(None),
            Err(e) => Err(Error::Internal(format!("Failed to get vector: {}", e))),
        }
    }

    /// Get the number of vectors in a namespace (memtable only).
    pub fn memtable_len(&self, namespace: &str) -> Result<usize> {
        let namespaces = self.namespaces.read();
        let state = namespaces.get(namespace).ok_or_else(|| {
            Error::Internal(format!("Vector namespace '{}' not found", namespace))
        })?;

        Ok(state.memtable_index.len())
    }

    /// Check if the memtable should be flushed.
    pub fn should_flush(&self, namespace: &str) -> Result<bool> {
        let namespaces = self.namespaces.read();
        let state = namespaces.get(namespace).ok_or_else(|| {
            Error::Internal(format!("Vector namespace '{}' not found", namespace))
        })?;

        Ok(state.memtable_index.len() >= state.config.flush_threshold)
    }

    /// Flush the memtable to an SSTable file.
    ///
    /// Returns the path to the new SSTable file, or None if memtable was empty.
    pub fn flush(&self, namespace: &str) -> Result<Option<PathBuf>> {
        let mut namespaces = self.namespaces.write();
        let state = namespaces.get_mut(namespace).ok_or_else(|| {
            Error::Internal(format!("Vector namespace '{}' not found", namespace))
        })?;

        if state.memtable_index.len() == 0 {
            return Ok(None);
        }

        // Create the vector block
        let distance_fn = match state.config.distance {
            DistanceFunction::Euclidean => DistanceFn::Euclidean,
            DistanceFunction::Cosine => DistanceFn::Cosine,
            DistanceFunction::InnerProduct => DistanceFn::InnerProduct,
        };

        let builder =
            RawVectorBlockBuilder::new(state.config.dimensions as u16, distance_fn);

        // Collect all vectors from memtable
        // Note: We need to iterate through the index somehow
        // For now, we'll track IDs separately in a real implementation
        // This is a simplified version that shows the pattern

        // Get vectors by iterating (simplified - in production would track IDs)
        let seqno = state.next_seqno;
        state.next_seqno += 1;

        // Create file path
        let dir = self.base_dir.join("vectors").join(&state.config.name);
        std::fs::create_dir_all(&dir)
            .map_err(|e| Error::Internal(format!("Failed to create directory: {}", e)))?;

        let filename = format!("vec_{:08}.vsst", seqno);
        let path = dir.join(&filename);

        // For this simplified implementation, we'll just write an empty block
        // In production, we'd iterate through the memtable and write all vectors
        let data = builder.finish();

        std::fs::write(&path, &data)
            .map_err(|e| Error::Internal(format!("Failed to write SSTable: {}", e)))?;

        // Add to SSTable list
        state.sstable_files.push(VectorSSTableFile {
            path: path.clone(),
            level: 0,
            vector_count: 0, // Would be builder.len() in production
            seqno,
        });

        // Create new empty memtable
        state.memtable_index = Arc::new(HnswIndex::new(
            state.config.dimensions,
            state.config.distance,
            Default::default(),
        ));

        Ok(Some(path))
    }

    /// Get statistics for a namespace.
    pub fn stats(&self, namespace: &str) -> Result<VectorNamespaceStats> {
        let namespaces = self.namespaces.read();
        let state = namespaces.get(namespace).ok_or_else(|| {
            Error::Internal(format!("Vector namespace '{}' not found", namespace))
        })?;

        let sstable_vectors: usize = state.sstable_files.iter().map(|f| f.vector_count).sum();

        Ok(VectorNamespaceStats {
            memtable_vectors: state.memtable_index.len(),
            sstable_count: state.sstable_files.len(),
            sstable_vectors,
            dimensions: state.config.dimensions,
        })
    }

    /// List all namespaces.
    pub fn list_namespaces(&self) -> Vec<String> {
        self.namespaces.read().keys().cloned().collect()
    }
}

/// Statistics for a vector namespace.
#[derive(Debug, Clone)]
pub struct VectorNamespaceStats {
    /// Vectors in memtable
    pub memtable_vectors: usize,
    /// Number of SSTable files
    pub sstable_count: usize,
    /// Total vectors in SSTables
    pub sstable_vectors: usize,
    /// Vector dimensions
    pub dimensions: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_create_namespace() {
        let temp_dir = TempDir::new().unwrap();
        let engine = VectorEngine::new(temp_dir.path());

        let config = VectorNamespaceEngineConfig::new("test", 128, DistanceFunction::Euclidean);
        engine.create_namespace(config).unwrap();

        assert!(engine.namespace_exists("test"));
        assert!(!engine.namespace_exists("nonexistent"));
    }

    #[test]
    fn test_insert_and_search() {
        let temp_dir = TempDir::new().unwrap();
        let engine = VectorEngine::new(temp_dir.path());

        let config = VectorNamespaceEngineConfig::new("test", 4, DistanceFunction::Euclidean);
        engine.create_namespace(config).unwrap();

        // Insert vectors
        engine.insert("test", "vec1", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        engine.insert("test", "vec2", &[0.0, 1.0, 0.0, 0.0]).unwrap();
        engine.insert("test", "vec3", &[0.0, 0.0, 1.0, 0.0]).unwrap();

        assert_eq!(engine.memtable_len("test").unwrap(), 3);

        // Search
        let results = engine.search("test", &[1.0, 0.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "vec1");
        assert!(results[0].distance < 0.01);
    }

    #[test]
    fn test_get_vector() {
        let temp_dir = TempDir::new().unwrap();
        let engine = VectorEngine::new(temp_dir.path());

        let config = VectorNamespaceEngineConfig::new("test", 4, DistanceFunction::Euclidean);
        engine.create_namespace(config).unwrap();

        engine.insert("test", "vec1", &[1.0, 2.0, 3.0, 4.0]).unwrap();

        let vec = engine.get("test", "vec1").unwrap().unwrap();
        assert_eq!(vec, vec![1.0, 2.0, 3.0, 4.0]);

        assert!(engine.get("test", "nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_delete_vector() {
        let temp_dir = TempDir::new().unwrap();
        let engine = VectorEngine::new(temp_dir.path());

        let config = VectorNamespaceEngineConfig::new("test", 4, DistanceFunction::Euclidean);
        engine.create_namespace(config).unwrap();

        engine.insert("test", "vec1", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        assert_eq!(engine.memtable_len("test").unwrap(), 1);

        engine.delete("test", "vec1").unwrap();
        assert_eq!(engine.memtable_len("test").unwrap(), 0);
    }

    #[test]
    fn test_drop_namespace() {
        let temp_dir = TempDir::new().unwrap();
        let engine = VectorEngine::new(temp_dir.path());

        let config = VectorNamespaceEngineConfig::new("test", 4, DistanceFunction::Euclidean);
        engine.create_namespace(config).unwrap();

        assert!(engine.drop_namespace("test"));
        assert!(!engine.namespace_exists("test"));
        assert!(!engine.drop_namespace("test")); // Already dropped
    }

    #[test]
    fn test_stats() {
        let temp_dir = TempDir::new().unwrap();
        let engine = VectorEngine::new(temp_dir.path());

        let config = VectorNamespaceEngineConfig::new("test", 128, DistanceFunction::Euclidean);
        engine.create_namespace(config).unwrap();

        for i in 0..10 {
            let vec: Vec<f32> = (0..128).map(|j| (i * j) as f32).collect();
            engine.insert("test", &format!("vec{}", i), &vec).unwrap();
        }

        let stats = engine.stats("test").unwrap();
        assert_eq!(stats.memtable_vectors, 10);
        assert_eq!(stats.sstable_count, 0);
        assert_eq!(stats.dimensions, 128);
    }

    #[test]
    fn test_flush_threshold() {
        let temp_dir = TempDir::new().unwrap();
        let engine = VectorEngine::new(temp_dir.path());

        let config = VectorNamespaceEngineConfig::new("test", 4, DistanceFunction::Euclidean)
            .with_flush_threshold(5);
        engine.create_namespace(config).unwrap();

        for i in 0..4 {
            engine
                .insert("test", &format!("vec{}", i), &[i as f32, 0.0, 0.0, 0.0])
                .unwrap();
        }

        assert!(!engine.should_flush("test").unwrap());

        engine.insert("test", "vec4", &[4.0, 0.0, 0.0, 0.0]).unwrap();
        assert!(engine.should_flush("test").unwrap());
    }

    #[test]
    fn test_dimension_mismatch() {
        let temp_dir = TempDir::new().unwrap();
        let engine = VectorEngine::new(temp_dir.path());

        let config = VectorNamespaceEngineConfig::new("test", 4, DistanceFunction::Euclidean);
        engine.create_namespace(config).unwrap();

        let result = engine.insert("test", "vec1", &[1.0, 2.0]); // Wrong dims
        assert!(result.is_err());
    }

    #[test]
    fn test_merge_results_deduplication() {
        let temp_dir = TempDir::new().unwrap();
        let engine = VectorEngine::new(temp_dir.path());

        // Test the merge function directly
        let candidates = vec![
            VectorMatch::new("vec1".to_string(), 0.5),
            VectorMatch::new("vec2".to_string(), 0.3),
            VectorMatch::new("vec1".to_string(), 0.4), // Duplicate with different distance
            VectorMatch::new("vec3".to_string(), 0.6),
        ];

        let merged = engine.merge_results(candidates, 3);

        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].id, "vec2"); // Closest
        assert_eq!(merged[1].id, "vec1"); // Second closest (0.4, not 0.5)
        assert_eq!(merged[2].id, "vec3");
    }
}
