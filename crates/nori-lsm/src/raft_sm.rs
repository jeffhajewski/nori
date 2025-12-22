//! Raft state machine implementation for LsmEngine.
//!
//! Provides StateMachine trait implementation to integrate LSM with Raft consensus.
//! Commands are serialized as bincode and applied atomically to the LSM engine.
//!
//! Supports both key-value and vector operations:
//! - Key-value: Put, Delete
//! - Vector: VectorInsert, VectorDelete, VectorCreateIndex, VectorDropIndex

use crate::vector::{VectorIndexType, VectorNamespaceConfig, VectorStorage};
use crate::{Error, LsmEngine, Result};
use bytes::Bytes;
use nori_vector::DistanceFunction;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Command that can be replicated via Raft.
///
/// Represents atomic operations on the LSM engine and vector storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    // ========== Key-Value Commands ==========
    /// Put key-value pair with optional TTL
    Put {
        key: Bytes,
        value: Bytes,
        ttl: Option<Duration>,
    },
    /// Delete key
    Delete { key: Bytes },

    // ========== Vector Commands ==========
    /// Create a new vector index namespace
    VectorCreateIndex {
        /// Namespace name
        namespace: String,
        /// Vector dimensions
        dimensions: usize,
        /// Distance function
        distance: DistanceFunction,
        /// Index type (BruteForce or HNSW)
        index_type: VectorIndexType,
    },
    /// Drop a vector index namespace
    VectorDropIndex {
        /// Namespace name
        namespace: String,
    },
    /// Insert a vector into a namespace
    VectorInsert {
        /// Namespace name
        namespace: String,
        /// Vector ID
        id: String,
        /// Vector data
        vector: Vec<f32>,
    },
    /// Delete a vector from a namespace
    VectorDelete {
        /// Namespace name
        namespace: String,
        /// Vector ID
        id: String,
    },
}

impl Command {
    /// Serialize command to bytes using bincode.
    pub fn serialize(&self) -> Result<Bytes> {
        bincode::serialize(self)
            .map(Bytes::from)
            .map_err(|e| Error::Internal(format!("Failed to serialize command: {}", e)))
    }

    /// Deserialize command from bytes using bincode.
    pub fn deserialize(data: &[u8]) -> Result<Self> {
        bincode::deserialize(data)
            .map_err(|e| Error::Internal(format!("Failed to deserialize command: {}", e)))
    }
}

/// Wrapper for LsmEngine that implements Raft's StateMachine trait.
///
/// This adapter allows LSM to be used as a replicated state machine in Raft.
/// Commands are serialized and applied through the Raft log.
///
/// Manages both key-value storage (via LsmEngine) and vector storage.
pub struct LsmStateMachine {
    /// Key-value storage engine
    engine: Arc<LsmEngine>,
    /// Vector storage for vector indexes
    vectors: Arc<VectorStorage>,
}

impl LsmStateMachine {
    /// Create a new LSM state machine wrapper.
    pub fn new(engine: Arc<LsmEngine>) -> Self {
        Self {
            engine,
            vectors: Arc::new(VectorStorage::new()),
        }
    }

    /// Create with existing vector storage.
    pub fn with_vector_storage(engine: Arc<LsmEngine>, vectors: Arc<VectorStorage>) -> Self {
        Self { engine, vectors }
    }

    /// Get reference to the underlying LSM engine.
    pub fn engine(&self) -> &Arc<LsmEngine> {
        &self.engine
    }

    /// Get reference to the vector storage.
    pub fn vectors(&self) -> &Arc<VectorStorage> {
        &self.vectors
    }
}

// Note: We can't implement the Raft StateMachine trait directly here
// because it would create a circular dependency (nori-lsm -> nori-raft -> nori-lsm).
// Instead, we provide an async apply method that can be called from a Raft adapter.

impl LsmStateMachine {
    /// Apply a command to the LSM engine.
    ///
    /// Called by Raft when a log entry is committed.
    /// Commands are deserialized and applied atomically.
    pub async fn apply_command(&mut self, command: &[u8]) -> Result<()> {
        let cmd = Command::deserialize(command)?;

        match cmd {
            // Key-value commands
            Command::Put { key, value, ttl } => {
                self.engine.put(key, value, ttl).await?;
            }
            Command::Delete { key } => {
                self.engine.delete(&key).await?;
            }

            // Vector commands
            Command::VectorCreateIndex {
                namespace,
                dimensions,
                distance,
                index_type,
            } => {
                let config = VectorNamespaceConfig {
                    dimensions,
                    distance,
                    index_type,
                    hnsw_config: None,
                };
                self.vectors.create_namespace(&namespace, config)?;
            }
            Command::VectorDropIndex { namespace } => {
                self.vectors.drop_namespace(&namespace);
            }
            Command::VectorInsert {
                namespace,
                id,
                vector,
            } => {
                self.vectors.insert(&namespace, &id, &vector)?;
            }
            Command::VectorDelete { namespace, id } => {
                self.vectors.delete(&namespace, &id)?;
            }
        }

        Ok(())
    }

    /// Create a snapshot of the LSM state.
    ///
    /// Serializes the current manifest snapshot, which includes:
    /// - Version and sequence numbers
    /// - All SSTable file references and metadata
    /// - Level structure and key ranges
    ///
    /// The snapshot captures a consistent point-in-time view of the LSM tree.
    pub fn create_snapshot(&self) -> Result<Bytes> {
        // Get the current manifest snapshot (consistent view of all SSTables)
        let manifest_snapshot = {
            let manifest = self.engine.manifest.read();
            let snapshot_arc = manifest.snapshot();
            let snapshot_guard = snapshot_arc.read();
            (*snapshot_guard).clone()
        };

        // Serialize using bincode (same as used for commands)
        bincode::serialize(&manifest_snapshot)
            .map(Bytes::from)
            .map_err(|e| Error::Internal(format!("Failed to serialize snapshot: {}", e)))
    }

    /// Restore LSM state from a snapshot.
    ///
    /// Delegates to `LsmEngine::restore_from_snapshot` which:
    /// - Deserializes the manifest snapshot
    /// - Verifies all SSTable files exist locally
    /// - Pauses compaction, clears memtable, restores manifest, resumes compaction
    ///
    /// Returns error if any SSTable files are missing (full data transfer not yet supported).
    pub fn restore_snapshot(&mut self, snapshot: &[u8]) -> Result<()> {
        // Use tokio runtime to call the async restore method
        // This is safe because we're already in an async context (Raft apply loop)
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                self.engine.restore_from_snapshot(snapshot).await
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ATLLConfig;
    use crate::Version;
    use tempfile::TempDir;

    /// Helper to extract just the value from get() result, ignoring version
    fn get_value(result: Option<(Bytes, Version)>) -> Option<Bytes> {
        result.map(|(v, _)| v)
    }

    async fn create_test_engine() -> (Arc<LsmEngine>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        let engine = Arc::new(LsmEngine::open(config).await.unwrap());
        (engine, temp_dir)
    }

    #[tokio::test]
    async fn test_command_serialization() {
        let cmd = Command::Put {
            key: Bytes::from("test_key"),
            value: Bytes::from("test_value"),
            ttl: None,
        };

        let serialized = cmd.serialize().unwrap();
        let deserialized = Command::deserialize(&serialized).unwrap();

        match deserialized {
            Command::Put { key, value, ttl } => {
                assert_eq!(key, Bytes::from("test_key"));
                assert_eq!(value, Bytes::from("test_value"));
                assert_eq!(ttl, None);
            }
            _ => panic!("Expected Put command"),
        }
    }

    #[tokio::test]
    async fn test_apply_put_command() {
        let (engine, _temp) = create_test_engine().await;
        let mut sm = LsmStateMachine::new(engine.clone());

        let cmd = Command::Put {
            key: Bytes::from("key1"),
            value: Bytes::from("value1"),
            ttl: None,
        };

        let serialized = cmd.serialize().unwrap();
        sm.apply_command(&serialized).await.unwrap();

        // Verify the value was written
        let result = engine.get(b"key1").await.unwrap();
        assert_eq!(get_value(result), Some(Bytes::from("value1")));
    }

    #[tokio::test]
    async fn test_apply_delete_command() {
        let (engine, _temp) = create_test_engine().await;
        let mut sm = LsmStateMachine::new(engine.clone());

        // First write a value
        engine.put(Bytes::from("key1"), Bytes::from("value1"), None)
            .await
            .unwrap();

        // Then delete it via command
        let cmd = Command::Delete {
            key: Bytes::from("key1"),
        };

        let serialized = cmd.serialize().unwrap();
        sm.apply_command(&serialized).await.unwrap();

        // Verify the value was deleted
        let result = engine.get(b"key1").await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_snapshot_create_restore() {
        let (engine, _temp) = create_test_engine().await;
        let mut sm = LsmStateMachine::new(engine);

        // Create snapshot - should serialize manifest
        let snapshot = sm.create_snapshot().unwrap();
        assert!(!snapshot.is_empty(), "Snapshot should contain serialized manifest");

        // Restore snapshot - for an empty engine with no SST files, should succeed
        let result = sm.restore_snapshot(&snapshot);
        assert!(
            result.is_ok(),
            "Restore should succeed when no SST files are referenced: {:?}",
            result.err()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_snapshot_restore_missing_files() {
        let (engine, _temp) = create_test_engine().await;
        let mut sm = LsmStateMachine::new(engine);

        // Create a fake manifest snapshot that references non-existent SST files
        let mut manifest_snapshot = crate::manifest::ManifestSnapshot::with_levels(7);
        manifest_snapshot.levels[0].l0_files.push(crate::manifest::RunMeta {
            file_number: 999999,
            size: 1024,
            min_key: Bytes::from("a"),
            max_key: Bytes::from("z"),
            min_version: Version::new(0, 1),
            max_version: Version::new(0, 100),
            tombstone_count: 0,
            filter_fp: 0.01,
            heat_hint: 0.5,
            value_log_segment_id: None,
        });

        let snapshot_data = bincode::serialize(&manifest_snapshot).unwrap();

        // Restore should fail because SST file 999999 doesn't exist
        let result = sm.restore_snapshot(&snapshot_data);
        assert!(
            result.is_err(),
            "Restore should fail with missing SST files"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("SSTable files missing"),
            "Error should indicate missing files: {}",
            err_msg
        );
    }

    // ========== Vector Command Tests ==========

    #[tokio::test]
    async fn test_vector_create_index_command() {
        let (engine, _temp) = create_test_engine().await;
        let mut sm = LsmStateMachine::new(engine);

        let cmd = Command::VectorCreateIndex {
            namespace: "embeddings".to_string(),
            dimensions: 128,
            distance: DistanceFunction::Euclidean,
            index_type: VectorIndexType::Hnsw,
        };

        let serialized = cmd.serialize().unwrap();
        sm.apply_command(&serialized).await.unwrap();

        // Verify namespace was created
        assert!(sm.vectors().namespace_exists("embeddings"));
        assert_eq!(sm.vectors().dimensions("embeddings").unwrap(), 128);
    }

    #[tokio::test]
    async fn test_vector_drop_index_command() {
        let (engine, _temp) = create_test_engine().await;
        let mut sm = LsmStateMachine::new(engine);

        // Create index first
        let create_cmd = Command::VectorCreateIndex {
            namespace: "embeddings".to_string(),
            dimensions: 128,
            distance: DistanceFunction::Euclidean,
            index_type: VectorIndexType::Hnsw,
        };
        sm.apply_command(&create_cmd.serialize().unwrap()).await.unwrap();

        // Drop the index
        let drop_cmd = Command::VectorDropIndex {
            namespace: "embeddings".to_string(),
        };
        sm.apply_command(&drop_cmd.serialize().unwrap()).await.unwrap();

        // Verify namespace was dropped
        assert!(!sm.vectors().namespace_exists("embeddings"));
    }

    #[tokio::test]
    async fn test_vector_insert_command() {
        let (engine, _temp) = create_test_engine().await;
        let mut sm = LsmStateMachine::new(engine);

        // Create index first
        let create_cmd = Command::VectorCreateIndex {
            namespace: "embeddings".to_string(),
            dimensions: 4,
            distance: DistanceFunction::Euclidean,
            index_type: VectorIndexType::BruteForce,
        };
        sm.apply_command(&create_cmd.serialize().unwrap()).await.unwrap();

        // Insert vector
        let insert_cmd = Command::VectorInsert {
            namespace: "embeddings".to_string(),
            id: "vec1".to_string(),
            vector: vec![1.0, 2.0, 3.0, 4.0],
        };
        sm.apply_command(&insert_cmd.serialize().unwrap()).await.unwrap();

        // Verify vector was inserted
        assert_eq!(sm.vectors().len("embeddings").unwrap(), 1);
        let stored = sm.vectors().get("embeddings", "vec1").unwrap().unwrap();
        assert_eq!(stored, vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[tokio::test]
    async fn test_vector_delete_command() {
        let (engine, _temp) = create_test_engine().await;
        let mut sm = LsmStateMachine::new(engine);

        // Create index and insert vector
        let create_cmd = Command::VectorCreateIndex {
            namespace: "embeddings".to_string(),
            dimensions: 4,
            distance: DistanceFunction::Euclidean,
            index_type: VectorIndexType::BruteForce,
        };
        sm.apply_command(&create_cmd.serialize().unwrap()).await.unwrap();

        let insert_cmd = Command::VectorInsert {
            namespace: "embeddings".to_string(),
            id: "vec1".to_string(),
            vector: vec![1.0, 2.0, 3.0, 4.0],
        };
        sm.apply_command(&insert_cmd.serialize().unwrap()).await.unwrap();

        // Delete vector
        let delete_cmd = Command::VectorDelete {
            namespace: "embeddings".to_string(),
            id: "vec1".to_string(),
        };
        sm.apply_command(&delete_cmd.serialize().unwrap()).await.unwrap();

        // Verify vector was deleted
        assert_eq!(sm.vectors().len("embeddings").unwrap(), 0);
    }

    #[tokio::test]
    async fn test_vector_command_serialization() {
        // Test VectorCreateIndex serialization
        let cmd = Command::VectorCreateIndex {
            namespace: "test".to_string(),
            dimensions: 256,
            distance: DistanceFunction::Cosine,
            index_type: VectorIndexType::Hnsw,
        };
        let serialized = cmd.serialize().unwrap();
        let deserialized = Command::deserialize(&serialized).unwrap();

        match deserialized {
            Command::VectorCreateIndex { namespace, dimensions, distance, index_type } => {
                assert_eq!(namespace, "test");
                assert_eq!(dimensions, 256);
                assert_eq!(distance, DistanceFunction::Cosine);
                assert_eq!(index_type, VectorIndexType::Hnsw);
            }
            _ => panic!("Expected VectorCreateIndex command"),
        }

        // Test VectorInsert serialization
        let cmd = Command::VectorInsert {
            namespace: "embeddings".to_string(),
            id: "doc123".to_string(),
            vector: vec![0.1, 0.2, 0.3],
        };
        let serialized = cmd.serialize().unwrap();
        let deserialized = Command::deserialize(&serialized).unwrap();

        match deserialized {
            Command::VectorInsert { namespace, id, vector } => {
                assert_eq!(namespace, "embeddings");
                assert_eq!(id, "doc123");
                assert_eq!(vector, vec![0.1, 0.2, 0.3]);
            }
            _ => panic!("Expected VectorInsert command"),
        }
    }

    #[tokio::test]
    async fn test_vector_search_via_state_machine() {
        let (engine, _temp) = create_test_engine().await;
        let mut sm = LsmStateMachine::new(engine);

        // Create index
        let create_cmd = Command::VectorCreateIndex {
            namespace: "embeddings".to_string(),
            dimensions: 4,
            distance: DistanceFunction::Euclidean,
            index_type: VectorIndexType::BruteForce,
        };
        sm.apply_command(&create_cmd.serialize().unwrap()).await.unwrap();

        // Insert vectors via commands (simulating Raft log replay)
        for i in 0..5 {
            let insert_cmd = Command::VectorInsert {
                namespace: "embeddings".to_string(),
                id: format!("vec{}", i),
                vector: vec![i as f32, 0.0, 0.0, 0.0],
            };
            sm.apply_command(&insert_cmd.serialize().unwrap()).await.unwrap();
        }

        // Search (reads don't go through Raft, just use vector storage directly)
        let results = sm.vectors().search("embeddings", &[0.0, 0.0, 0.0, 0.0], 3).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].id, "vec0"); // Closest to origin
    }
}
