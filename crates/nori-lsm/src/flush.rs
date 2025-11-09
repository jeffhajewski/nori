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
    #[allow(dead_code)] // TODO: Will be used for configurable SSTable settings
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
                MemtableEntry::Put { value, seqno, .. } => {
                    Entry::put_with_seqno(key.clone(), value.clone(), seqno)
                }
                MemtableEntry::Delete { seqno } => {
                    tombstone_count += 1;
                    Entry::delete_with_seqno(key.clone(), seqno)
                }
            };

            // Trace logging (disabled in release builds)
            if entry_count < 3 || entry_count >= memtable.len() - 3 {
                tracing::trace!(
                    "Flushing entry {}: {}",
                    entry_count,
                    String::from_utf8_lossy(&key)
                );
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

        tracing::debug!(
            "Flushed {} entries to SSTable {} (from memtable with {} total)",
            entry_count,
            file_number,
            memtable.len()
        );

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
    #[allow(dead_code)] // TODO: Will be used for physical file splitting in Phase 4
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

    /// Admits L0 file to L1 with physical file splitting on guard boundaries.
    ///
    /// # Phase 6 Implementation
    /// Physically splits the L0 file into multiple L1 fragments, one per overlapping slot.
    /// Each fragment is written as a separate SSTable aligned to guard boundaries.
    ///
    /// # Arguments
    /// - `l0_run`: L0 run to admit
    /// - `guard_manager`: Guard manager for slot boundaries
    /// - `new_file_numbers`: Pre-allocated file numbers for output fragments (one per overlapping slot)
    ///
    /// # Returns
    /// Vector of ManifestEdits: [DeleteFile(L0), AddFile(L1, slot1), AddFile(L1, slot2), ...]
    pub async fn admit_to_l1_with_split(
        &self,
        l0_run: &RunMeta,
        guard_manager: &GuardManager,
        new_file_numbers: &[u64],
    ) -> Result<Vec<ManifestEdit>> {
        // Find which L1 slot(s) this run overlaps
        let overlapping_slots = guard_manager.overlapping_slots(&l0_run.min_key, &l0_run.max_key);

        if overlapping_slots.is_empty() {
            return Err(Error::Internal(
                "L0 admission: No overlapping L1 slots found".to_string(),
            ));
        }

        if overlapping_slots.len() != new_file_numbers.len() {
            return Err(Error::Internal(format!(
                "L0 admission: File number count mismatch (slots={}, file_numbers={})",
                overlapping_slots.len(),
                new_file_numbers.len()
            )));
        }

        println!(
            "=== L0→L1 SPLIT: Splitting L0 file {} into {} fragments across slots {:?} ===",
            l0_run.file_number,
            overlapping_slots.len(),
            overlapping_slots
        );

        // If file only overlaps one slot, no splitting needed - use fast path
        if overlapping_slots.len() == 1 {
            let mut edits = Vec::new();
            edits.push(ManifestEdit::AddFile {
                level: 1,
                slot_id: Some(overlapping_slots[0]),
                run: l0_run.clone(),
            });
            edits.push(ManifestEdit::DeleteFile {
                level: 0,
                slot_id: None,
                file_number: l0_run.file_number,
            });
            return Ok(edits);
        }

        // Physical splitting path: read L0 file and split on guard boundaries
        let mut edits = Vec::new();

        // Create one fragment per overlapping slot
        for (idx, &slot_id) in overlapping_slots.iter().enumerate() {
            let file_number = new_file_numbers[idx];
            let output_path = self.sst_path(file_number);

            // Get guard boundaries for this slot
            let (slot_min, slot_max) = guard_manager.slot_boundaries(slot_id);

            let config = nori_sstable::SSTableConfig {
                path: output_path,
                estimated_entries: 1000, // Rough estimate, will adjust
                block_size: 4096,
                restart_interval: 16,
                compression: nori_sstable::Compression::None,
                bloom_bits_per_key: 10,
                block_cache_mb: 64,
            };

            let mut builder = nori_sstable::SSTableBuilder::new(config)
                .await
                .map_err(|e| Error::Internal(format!("Failed to create builder: {}", e)))?;

            // Filter entries within this slot's guard range
            let mut min_key: Option<Bytes> = None;
            let mut max_key: Option<Bytes> = None;
            let mut tombstone_count = 0u32;
            let mut entry_count = 0usize;
            let mut min_seqno = u64::MAX;
            let mut max_seqno = 0u64;

            // Iterate through all entries and write those within slot bounds
            // Note: This requires re-reading the file for each slot (inefficient)
            // A better implementation would buffer entries or use range iterators
            let reader2 = nori_sstable::SSTableReader::open(self.sst_path(l0_run.file_number))
                .await
                .map_err(|e| Error::Internal(format!("Failed to re-open L0 file: {}", e)))?;
            let reader2_arc = std::sync::Arc::new(reader2);
            let mut slot_iter = reader2_arc.iter();

            while let Some(entry) = slot_iter.try_next()
                .await
                .map_err(|e| Error::Internal(format!("Iterator error: {}", e)))?
            {
                // Check if entry is within this slot's bounds
                let in_range = match (&slot_min, &slot_max) {
                    (Some(min), Some(max)) => {
                        entry.key.as_ref() >= min.as_ref() && entry.key.as_ref() < max.as_ref()
                    }
                    (Some(min), None) => entry.key.as_ref() >= min.as_ref(),
                    (None, Some(max)) => entry.key.as_ref() < max.as_ref(),
                    (None, None) => true,
                };

                if !in_range {
                    continue;
                }

                // Add entry to this slot's fragment
                builder.add(&entry)
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to add entry: {}", e)))?;

                if min_key.is_none() {
                    min_key = Some(entry.key.clone());
                }
                max_key = Some(entry.key.clone());

                if entry.tombstone {
                    tombstone_count += 1;
                }

                min_seqno = min_seqno.min(entry.seqno);
                max_seqno = max_seqno.max(entry.seqno);
                entry_count += 1;
            }

            // Skip empty fragments
            if entry_count == 0 {
                println!("  Slot {} fragment is empty, skipping", slot_id);
                continue;
            }

            // Finalize fragment
            let metadata = builder.finish()
                .await
                .map_err(|e| Error::Internal(format!("Failed to finish fragment: {}", e)))?;

            println!(
                "  Slot {} fragment: {} entries, {} bytes (file {})",
                slot_id, entry_count, metadata.file_size, file_number
            );

            // Create run metadata for this fragment
            let fragment_run = RunMeta {
                file_number,
                size: metadata.file_size,
                min_key: min_key.ok_or_else(|| Error::Internal("No min key".to_string()))?,
                max_key: max_key.ok_or_else(|| Error::Internal("No max key".to_string()))?,
                min_seqno,
                max_seqno,
                tombstone_count,
                filter_fp: 0.001,
                heat_hint: 0.0,
                value_log_segment_id: None,
            };

            edits.push(ManifestEdit::AddFile {
                level: 1,
                slot_id: Some(slot_id),
                run: fragment_run,
            });
        }

        // Delete original L0 file
        edits.push(ManifestEdit::DeleteFile {
            level: 0,
            slot_id: None,
            file_number: l0_run.file_number,
        });

        println!(
            "=== L0→L1 SPLIT COMPLETE: {} fragments created ===",
            overlapping_slots.len()
        );

        Ok(edits)
    }

    /// Helper to construct SSTable file path
    fn sst_path(&self, file_number: u64) -> PathBuf {
        self.sst_dir.join(format!("sst-{:06}.sst", file_number))
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
    #[allow(dead_code)] // TODO: Will be used for physical merge implementation in Phase 6
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
    /// # Phase 6 Implementation
    /// Physically merges the oldest N runs in a slot into a single run.
    /// Performs K-way merge, writes output SSTable, and updates manifest.
    ///
    /// # Arguments
    /// - `level`: Target level for compaction
    /// - `slot_id`: Slot identifier within the level
    /// - `runs`: All runs in this slot (sorted oldest to newest)
    /// - `target_runs`: Number of runs to merge
    /// - `new_file_number`: File number for the output SSTable
    ///
    /// # Returns
    /// ManifestEdits: AddFile for merged output + DeleteFile for inputs
    pub async fn compact_slot(
        &self,
        level: u8,
        slot_id: u32,
        runs: &[RunMeta],
        target_runs: usize,
        new_file_number: u64,
    ) -> Result<Vec<ManifestEdit>> {
        if runs.len() <= target_runs {
            return Ok(Vec::new()); // No compaction needed
        }

        // Select runs to merge (oldest N runs)
        let runs_to_merge = &runs[..target_runs.min(runs.len())];

        println!(
            "=== COMPACTION: Merging {} runs in L{} slot {} ===",
            runs_to_merge.len(),
            level,
            slot_id
        );

        // Phase 6: Physical merge implementation
        // Step 1: Open all input SSTables
        let mut iterators = Vec::new();
        for run in runs_to_merge {
            let sst_path = self.sst_path(run.file_number);

            // Open SSTable for reading
            let reader = nori_sstable::SSTableReader::open(sst_path)
                .await
                .map_err(|e| Error::Internal(format!("Failed to open SSTable {}: {}", run.file_number, e)))?;

            // Wrap in Arc and create iterator
            let reader_arc = std::sync::Arc::new(reader);
            let iter = reader_arc.iter();

            iterators.push(iter);
        }

        // Step 2: Perform K-way merge and write to new SSTable
        let output_path = self.sst_path(new_file_number);

        let config = nori_sstable::SSTableConfig {
            path: output_path,
            estimated_entries: runs_to_merge.iter().map(|r| r.size / 100).sum::<u64>() as usize, // Rough estimate
            block_size: 4096,
            restart_interval: 16,
            compression: nori_sstable::Compression::None,
            bloom_bits_per_key: 10,
            block_cache_mb: 64,
        };

        let mut builder = nori_sstable::SSTableBuilder::new(config)
            .await
            .map_err(|e| Error::Internal(format!("Failed to create SSTable builder: {}", e)))?;

        // Merge all entries using K-way merge with min-heap
        let mut min_key: Option<Bytes> = None;
        let mut max_key: Option<Bytes> = None;
        let mut tombstone_count = 0u32;
        let mut entry_count = 0usize;
        let mut min_seqno = u64::MAX;
        let mut max_seqno = 0u64;

        // Use K-way merge with deduplication
        use std::collections::BinaryHeap;
        use std::cmp::Ordering;

        #[derive(Debug)]
        struct MergeCandidate {
            entry: nori_sstable::Entry,
            source_idx: usize,
        }

        impl PartialEq for MergeCandidate {
            fn eq(&self, other: &Self) -> bool {
                self.entry.key == other.entry.key
            }
        }

        impl Eq for MergeCandidate {}

        impl PartialOrd for MergeCandidate {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        impl Ord for MergeCandidate {
            fn cmp(&self, other: &Self) -> Ordering {
                // Min-heap: reverse key comparison for ascending order
                match other.entry.key.cmp(&self.entry.key) {
                    Ordering::Equal => {
                        // Same key: prioritize higher seqno (newer wins)
                        // For compaction within same level, higher seqno = newer
                        other.entry.seqno.cmp(&self.entry.seqno)
                    }
                    ord => ord,
                }
            }
        }

        // Initialize heap with first entry from each iterator
        let mut heap: BinaryHeap<MergeCandidate> = BinaryHeap::new();
        for (idx, iter) in iterators.iter_mut().enumerate() {
            if let Some(entry) = iter.try_next()
                .await
                .map_err(|e| Error::Internal(format!("Iterator error: {}", e)))?
            {
                heap.push(MergeCandidate {
                    entry,
                    source_idx: idx,
                });
            }
        }

        let mut last_key: Option<Bytes> = None;

        while let Some(candidate) = heap.pop() {
            let source_idx = candidate.source_idx;
            let entry = candidate.entry;

            // Advance the iterator that produced this entry
            if let Some(next_entry) = iterators[source_idx].try_next()
                .await
                .map_err(|e| Error::Internal(format!("Iterator error: {}", e)))?
            {
                heap.push(MergeCandidate {
                    entry: next_entry,
                    source_idx,
                });
            }

            // Deduplicate: skip if same key as last emitted
            if let Some(ref last) = last_key {
                if entry.key == *last {
                    continue; // Duplicate - skip
                }
            }

            // Write entry to output SSTable
            builder.add(&entry)
                .await
                .map_err(|e| Error::Internal(format!("Failed to add entry: {}", e)))?;

            // Track metadata
            if min_key.is_none() {
                min_key = Some(entry.key.clone());
            }
            max_key = Some(entry.key.clone());

            if entry.tombstone {
                tombstone_count += 1;
            }

            min_seqno = min_seqno.min(entry.seqno);
            max_seqno = max_seqno.max(entry.seqno);
            entry_count += 1;

            last_key = Some(entry.key);
        }

        // Step 3: Finalize new SSTable
        let metadata = builder.finish()
            .await
            .map_err(|e| Error::Internal(format!("Failed to finish SSTable: {}", e)))?;

        println!(
            "=== COMPACTION: Wrote {} entries to SSTable {} ({} bytes) ===",
            entry_count, new_file_number, metadata.file_size
        );

        // Step 4: Create manifest edits
        let mut edits = Vec::new();

        // Add new merged file
        let new_run = RunMeta {
            file_number: new_file_number,
            size: metadata.file_size,
            min_key: min_key.ok_or_else(|| Error::Internal("No min key".to_string()))?,
            max_key: max_key.ok_or_else(|| Error::Internal("No max key".to_string()))?,
            min_seqno,
            max_seqno,
            tombstone_count,
            filter_fp: 0.001,
            heat_hint: 0.0,
            value_log_segment_id: None,
        };

        edits.push(ManifestEdit::AddFile {
            level,
            slot_id: Some(slot_id),
            run: new_run,
        });

        // Delete old input files
        for run in runs_to_merge {
            edits.push(ManifestEdit::DeleteFile {
                level,
                slot_id: Some(slot_id),
                file_number: run.file_number,
            });
        }

        Ok(edits)
    }

    /// Returns the SSTable file path for a given file number.
    fn sst_path(&self, file_number: u64) -> PathBuf {
        self.sst_dir.join(format!("sst-{:06}.sst", file_number))
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
        mt.put(Bytes::from("key1"), Bytes::from("value1"), 1, None)
            .unwrap();
        mt.put(Bytes::from("key2"), Bytes::from("value2"), 2, None)
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
