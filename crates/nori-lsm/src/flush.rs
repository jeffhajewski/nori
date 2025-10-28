/// Memtable flush and L0 admission logic.
///
/// Based on `lsm_atll_design.yaml` spec lines 176-182 (l0_admission algorithm).
///
/// # Flush Protocol
/// 1. Freeze active memtable → immutable memtable
/// 2. Write immutable memtable to SSTable file
/// 3. Register SSTable in MANIFEST as L0 file
/// 4. Delete WAL segment
/// 5. Trigger L0→L1 admission if L0 file count exceeds threshold
///
/// # L0→L1 Admission
/// 1. For each L0 file:
///    - Find overlapping L1 guards
///    - Split file on guard boundaries (block-aligned)
///    - Place fragments into corresponding L1 slots
/// 2. Update MANIFEST atomically
/// 3. Delete source L0 files
use crate::config::ATLLConfig;
use crate::error::{Error, Result};
use crate::guards::GuardManager;
use crate::manifest::{ManifestEdit, ManifestLog, RunMeta};
use crate::memtable::{Memtable, MemtableEntry};
use bytes::Bytes;
use nori_sstable::{Compression, Entry, SSTableBuilder, SSTableConfig};
use std::path::{Path, PathBuf};

/// Flusher handles memtable → SSTable conversion.
pub struct Flusher {
    /// Data directory for SST files
    sst_dir: PathBuf,

    /// ATLL configuration
    config: ATLLConfig,
}

impl Flusher {
    /// Creates a new flusher.
    pub fn new(sst_dir: impl AsRef<Path>, config: ATLLConfig) -> Result<Self> {
        let sst_dir = sst_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&sst_dir)?;

        Ok(Self { sst_dir, config })
    }

    /// Flushes a memtable to an SSTable file in L0.
    ///
    /// # Returns
    /// RunMeta describing the created SSTable.
    ///
    /// # Process
    /// 1. Allocate file number from MANIFEST
    /// 2. Write entries to SSTable (sorted order)
    /// 3. Build bloom filter
    /// 4. Sync file to disk
    /// 5. Return metadata
    pub async fn flush_to_l0(&self, memtable: &Memtable, file_number: u64) -> Result<RunMeta> {
        if memtable.is_empty() {
            return Err(Error::Internal("Cannot flush empty memtable".to_string()));
        }

        let sst_path = self.sst_path(file_number);

        // Create SSTable builder with config
        let config = SSTableConfig {
            path: sst_path,
            estimated_entries: memtable.len(),
            block_size: 4096,
            restart_interval: 16,
            compression: Compression::None,
            bloom_bits_per_key: 10,
            block_cache_mb: 64,
        };

        let mut builder = SSTableBuilder::new(config)
            .await
            .map_err(|e| Error::Internal(format!("Failed to create SSTable builder: {}", e)))?;

        let mut min_key: Option<Bytes> = None;
        let mut max_key: Option<Bytes> = None;
        let mut tombstone_count = 0u32;

        // Write all entries in sorted order
        let mut entry_count = 0;
        for (key, entry) in memtable.iter() {
            let sst_entry = match entry {
                MemtableEntry::Put { value, seqno } => {
                    Entry::put_with_seqno(key.clone(), value.clone(), seqno)
                }
                MemtableEntry::Delete { seqno } => {
                    tombstone_count += 1;
                    Entry::delete_with_seqno(key.clone(), seqno)
                }
            };

            // Log first and last few keys
            if entry_count < 3 || entry_count >= memtable.len() - 3 {
                println!("  FLUSH entry {}: {}", entry_count, String::from_utf8_lossy(&key));
            }

            builder
                .add(&sst_entry)
                .await
                .map_err(|e| Error::Internal(format!("Failed to add entry to SSTable: {}", e)))?;

            // Track key range
            if min_key.is_none() {
                min_key = Some(key.clone());
            }
            max_key = Some(key);
            entry_count += 1;
        }

        println!("=== FLUSH: Flushed {} entries to SSTable {} (from memtable with {} total) ===",
            entry_count, file_number, memtable.len());

        // Finalize SSTable
        let metadata = builder
            .finish()
            .await
            .map_err(|e| Error::Internal(format!("Failed to finish SSTable: {}", e)))?;

        let run_meta = RunMeta {
            file_number,
            size: metadata.file_size,
            min_key: min_key.ok_or_else(|| Error::Internal("No min key".to_string()))?,
            max_key: max_key.ok_or_else(|| Error::Internal("No max key".to_string()))?,
            min_seqno: memtable.min_seqno(),
            max_seqno: memtable.max_seqno(),
            tombstone_count,
            filter_fp: 0.001, // Default, actual FP rate from SSTable metadata
            heat_hint: 0.0,   // Will be updated by heat tracker
            value_log_segment_id: None,
        };

        Ok(run_meta)
    }

    /// Returns the SSTable file path for a given file number.
    fn sst_path(&self, file_number: u64) -> PathBuf {
        self.sst_dir.join(format!("sst-{:06}.sst", file_number))
    }
}

/// L0 admitter handles L0→L1 admission with guard splitting.
pub struct L0Admitter {
    /// Data directory for SST files
    sst_dir: PathBuf,

    /// Configuration
    config: ATLLConfig,
}

impl L0Admitter {
    /// Creates a new L0 admitter.
    pub fn new(sst_dir: impl AsRef<Path>, config: ATLLConfig) -> Result<Self> {
        let sst_dir = sst_dir.as_ref().to_path_buf();

        Ok(Self { sst_dir, config })
    }

    /// Admits L0 files to L1 by splitting on guard boundaries.
    ///
    /// # Algorithm (from spec lines 176-182)
    /// ```text
    /// for each flushed L0 run R:
    ///   for each (g_i, g_{i+1}) overlapping R:
    ///     split R on guard boundaries (block-aligned),
    ///     place fragment into L1 slot i; update bytes, runs.
    ///   if L0_files > max_files: raise scheduler priority for L0→L1 compactions.
    /// ```
    ///
    /// # Implementation Strategy
    /// Physical file splitting is expensive. We use a two-phase approach:
    /// 1. **Phase 3 (current)**: Assign L0 file to primary overlapping slot
    ///    - Quick admission to prevent L0 backlog
    ///    - File may span multiple slots temporarily
    /// 2. **Phase 4 (compaction)**: Physical splitting during tiering/promotion
    ///    - Split on guard boundaries when merging
    ///    - Amortize split cost across compaction work
    ///
    /// # Returns
    /// Vector of ManifestEdits: [DeleteFile(L0), AddFile(L1, slot), ...]
    pub async fn admit_to_l1(
        &self,
        l0_run: &RunMeta,
        guard_manager: &GuardManager,
        _manifest: &mut ManifestLog,
    ) -> Result<Vec<ManifestEdit>> {
        let mut edits = Vec::new();

        // Find which L1 slot(s) this run overlaps
        let overlapping_slots = guard_manager.overlapping_slots(&l0_run.min_key, &l0_run.max_key);

        if overlapping_slots.is_empty() {
            return Err(Error::Internal(
                "L0 admission: No overlapping L1 slots found".to_string(),
            ));
        }

        // Strategy: assign to first overlapping slot (primary slot)
        // Rationale:
        // - Fast admission (no file I/O needed)
        // - File will be split during compaction when merging with L1 runs
        // - Preserves guard invariants at L2+ (L1 is relaxed tiering layer)
        let target_slot = overlapping_slots[0];

        // If file spans multiple slots, log it for observability
        if overlapping_slots.len() > 1 {
            tracing::debug!(
                "L0 file {} spans {} slots [{:?}], assigning to slot {}",
                l0_run.file_number,
                overlapping_slots.len(),
                overlapping_slots,
                target_slot
            );
        }

        // Add to L1 at target slot (do this first so file is never orphaned)
        edits.push(ManifestEdit::AddFile {
            level: 1,
            slot_id: Some(target_slot),
            run: l0_run.clone(),
        });

        // Remove from L0 (do this second after it's safely in L1)
        edits.push(ManifestEdit::DeleteFile {
            level: 0,
            slot_id: None,
            file_number: l0_run.file_number,
        });

        Ok(edits)
    }

    /// Checks if L0 admission should be triggered.
    ///
    /// Triggers when L0 file count exceeds threshold.
    pub fn should_admit(&self, l0_file_count: usize) -> bool {
        l0_file_count > self.config.l0.max_files
    }
}

/// Compactor handles slot-local tiering compaction.
pub struct Compactor {
    /// Data directory for SST files
    sst_dir: PathBuf,

    /// Configuration
    config: ATLLConfig,
}

impl Compactor {
    /// Creates a new compactor.
    pub fn new(sst_dir: impl AsRef<Path>, config: ATLLConfig) -> Result<Self> {
        let sst_dir = sst_dir.as_ref().to_path_buf();

        Ok(Self { sst_dir, config })
    }

    /// Performs slot-local tiering compaction.
    ///
    /// # Algorithm (from spec lines 197-200)
    /// ```text
    /// # Trigger: slot runs > K_s OR bytes_s > slot_budget_s
    /// choose ~size-tiered group of oldest runs with similar size (e.g., 4),
    /// merge into one run within same slot; install; drop inputs.
    /// ```
    ///
    /// # Simplification for Phase 4
    /// Merges the oldest N runs in a slot into a single run.
    /// Physical merge implementation deferred to Phase 6.
    pub async fn compact_slot(
        &self,
        level: u8,
        slot_id: u32,
        runs: &[RunMeta],
        target_runs: usize,
    ) -> Result<Vec<ManifestEdit>> {
        if runs.len() <= target_runs {
            return Ok(Vec::new()); // No compaction needed
        }

        // For Phase 4: just plan the operation
        // Phase 6 will add physical merge logic

        let mut edits = Vec::new();

        // Mark old runs for deletion (simplified)
        for run in runs.iter().take(target_runs) {
            edits.push(ManifestEdit::DeleteFile {
                level,
                slot_id: Some(slot_id),
                file_number: run.file_number,
            });
        }

        Ok(edits)
    }

    /// Checks if compaction should be triggered for a slot.
    ///
    /// Triggers when:
    /// - Number of runs > K value for slot
    /// - Total bytes > slot budget
    pub fn should_compact(&self, slot_bytes: u64, run_count: usize, k: u8, level: u8) -> bool {
        let slot_budget = self.config.slot_budget_bytes(level);

        run_count > k as usize || slot_bytes > slot_budget
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memtable::Memtable;

    #[test]
    fn test_flusher_creation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");

        let config = ATLLConfig::default();
        let _flusher = Flusher::new(&sst_dir, config).unwrap();

        assert!(sst_dir.exists());
    }

    #[tokio::test]
    async fn test_flush_to_l0() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");

        let config = ATLLConfig::default();
        let flusher = Flusher::new(&sst_dir, config).unwrap();

        // Create memtable with some entries
        let mt = Memtable::new(1);
        mt.put(Bytes::from("key1"), Bytes::from("value1"), 1)
            .unwrap();
        mt.put(Bytes::from("key2"), Bytes::from("value2"), 2)
            .unwrap();
        mt.delete(Bytes::from("key3"), 3).unwrap();

        // Flush to L0
        let run = flusher.flush_to_l0(&mt, 1).await.unwrap();

        assert_eq!(run.file_number, 1);
        assert_eq!(run.min_key, Bytes::from("key1"));
        assert_eq!(run.max_key, Bytes::from("key3"));
        assert_eq!(run.min_seqno, 1);
        assert_eq!(run.max_seqno, 3);
        assert_eq!(run.tombstone_count, 1);

        // Verify file exists
        let sst_path = sst_dir.join("sst-000001.sst");
        assert!(sst_path.exists());
    }

    #[test]
    fn test_l0_admitter_should_admit() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");

        let config = ATLLConfig::default();
        let admitter = L0Admitter::new(&sst_dir, config).unwrap();

        assert!(!admitter.should_admit(5)); // Below threshold
        assert!(!admitter.should_admit(12)); // At threshold
        assert!(admitter.should_admit(13)); // Above threshold
    }

    #[tokio::test]
    async fn test_l0_admission_with_guards() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");
        let manifest_dir = temp_dir.path().join("manifest");

        let config = ATLLConfig::default();
        let admitter = L0Admitter::new(&sst_dir, config).unwrap();

        // Create guard manager with guards
        let guards = vec![Bytes::from("m")];
        let guard_mgr = GuardManager::with_guards(1, guards).unwrap();

        // Create L0 run that overlaps slot 0
        let l0_run = RunMeta {
            file_number: 1,
            size: 1024,
            min_key: Bytes::from("a"),
            max_key: Bytes::from("k"),
            min_seqno: 1,
            max_seqno: 100,
            tombstone_count: 0,
            filter_fp: 0.001,
            heat_hint: 0.0,
            value_log_segment_id: None,
        };

        let mut manifest = ManifestLog::open(&manifest_dir, 7).unwrap();

        let edits = admitter
            .admit_to_l1(&l0_run, &guard_mgr, &mut manifest)
            .await
            .unwrap();

        assert_eq!(edits.len(), 2); // AddFile + DeleteFile

        // First edit should be AddFile to L1
        match &edits[0] {
            ManifestEdit::AddFile {
                level,
                slot_id,
                run,
            } => {
                assert_eq!(*level, 1);
                assert_eq!(*slot_id, Some(0));
                assert_eq!(run.file_number, 1);
            }
            _ => panic!("Expected AddFile edit"),
        }

        // Second edit should be DeleteFile from L0
        match &edits[1] {
            ManifestEdit::DeleteFile {
                level,
                slot_id,
                file_number,
            } => {
                assert_eq!(*level, 0);
                assert_eq!(*slot_id, None);
                assert_eq!(*file_number, 1);
            }
            _ => panic!("Expected DeleteFile edit"),
        }
    }

    #[test]
    fn test_compactor_should_compact() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");

        let config = ATLLConfig::default();
        let compactor = Compactor::new(&sst_dir, config.clone()).unwrap();

        // Test run count trigger
        let slot_budget = config.slot_budget_bytes(1);
        assert!(compactor.should_compact(1024, 5, 3, 1)); // 5 runs > K=3

        // Test bytes trigger
        assert!(compactor.should_compact(slot_budget + 1, 2, 3, 1)); // Exceeds budget

        // No trigger
        assert!(!compactor.should_compact(1024, 2, 3, 1));
    }
}
