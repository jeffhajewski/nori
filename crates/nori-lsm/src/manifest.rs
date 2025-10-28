/// MANIFEST system for tracking LSM tree structure with crash safety.
///
/// Based on `lsm_atll_design.yaml` spec lines 139-172.
///
/// The MANIFEST is a versioned edit log that tracks:
/// - Level structure and slots
/// - SSTable file metadata (runs)
/// - Guard keys for range partitioning
/// - Heat scores and compaction state
///
/// # File Layout
/// ```text
/// data_dir/manifest/
///   ├── CURRENT           # Points to active MANIFEST file
///   ├── MANIFEST-000001   # Incremental edit log
///   ├── MANIFEST-000002   # Snapshot + subsequent edits
///   └── ...
/// ```
///
/// # Recovery Protocol
/// 1. Read CURRENT to get active MANIFEST file
/// 2. Load latest snapshot (if exists)
/// 3. Replay incremental edits
/// 4. Verify referenced SSTable files exist
/// 5. Reconstruct in-memory level/slot map
use crate::error::{Error, Result};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// Complete snapshot of the LSM tree structure.
///
/// Periodically written to reduce recovery time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestSnapshot {
    /// Manifest version number (monotonically increasing)
    pub version: u64,

    /// Next available SSTable file number
    pub next_file_number: u64,

    /// Last allocated sequence number
    pub last_seqno: u64,

    /// All levels (L0 through max_levels)
    pub levels: Vec<LevelMeta>,
}

/// Metadata for a single level in the LSM tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelMeta {
    /// Level number (0 = L0, 1 = L1, etc.)
    pub level: u8,

    /// Guard keys defining slot boundaries (empty for L0)
    /// Slot i covers range [guards[i], guards[i+1])
    /// Spec name: `guards`
    pub guards: Vec<Bytes>,

    /// Slots for this level (empty for L0)
    pub slots: Vec<SlotMeta>,

    /// L0 files (unsorted, overlapping)
    /// Only populated for level 0
    pub l0_files: Vec<RunMeta>,
}

/// Metadata for a single slot (range partition) within a level.
///
/// Spec: lines 146-164 of lsm_atll_design.yaml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotMeta {
    /// Slot identifier within this level
    pub slot_id: u32,

    /// Range boundaries [start, end)
    pub range: SlotRange,

    /// Total bytes in this slot across all runs
    pub bytes: u64,

    /// Sorted runs (SSTables) in this slot
    /// Bounded by K_{level,slot}
    pub runs: Vec<RunMeta>,

    /// Current K value for this slot (dynamic, adapts to heat)
    /// Spec name: `K`
    pub k: u8,

    /// Heat score [0.0, 1.0] from EWMA of operations
    pub heat_score: f32,

    /// Age histogram for temporal locality tracking
    /// Spec name: `age_histogram`
    pub age_histogram: Vec<u32>,

    /// Tombstone density [0.0, 1.0]
    pub tombstone_density: f32,

    /// Timestamp of last compaction (Unix epoch seconds)
    pub last_compaction_ts: u64,

    /// ZNS zone IDs if ZNS is enabled (empty otherwise)
    /// Spec name: `zns_zone_ids`
    pub zns_zone_ids: Vec<u32>,
}

/// Range boundaries for a slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotRange {
    /// Start key (inclusive)
    pub start: Bytes,
    /// End key (exclusive)
    pub end: Bytes,
}

/// Metadata for a single SSTable run.
///
/// Corresponds to one .sst file on disk.
/// Spec: lines 150-158 of lsm_atll_design.yaml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMeta {
    /// Unique file number (maps to sst-{file_number}.sst)
    pub file_number: u64,

    /// File size in bytes
    pub size: u64,

    /// Minimum key in this run
    pub min_key: Bytes,

    /// Maximum key in this run
    pub max_key: Bytes,

    /// Minimum sequence number
    /// Spec name: `seqno_min`
    pub min_seqno: u64,

    /// Maximum sequence number
    /// Spec name: `seqno_max`
    pub max_seqno: u64,

    /// Number of tombstones in this run
    pub tombstone_count: u32,

    /// Filter false positive rate achieved
    /// Spec name: `filter_fp`
    pub filter_fp: f64,

    /// Heat hint at time of creation
    pub heat_hint: f32,

    /// Value log segment ID (if value log enabled)
    /// Spec name: `value_log_segment_id`
    pub value_log_segment_id: Option<u32>,
}

impl RunMeta {
    /// Returns true if this run overlaps the given key range [start, end).
    pub fn overlaps(&self, start: &[u8], end: &[u8]) -> bool {
        // Run overlaps if: run.max_key >= start AND run.min_key < end
        self.max_key.as_ref() >= start && self.min_key.as_ref() < end
    }
}

/// Incremental edit to the MANIFEST.
///
/// Edits are appended to the MANIFEST log and applied atomically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ManifestEdit {
    /// Add a new SSTable file to a level/slot
    AddFile {
        level: u8,
        slot_id: Option<u32>, // None for L0
        run: RunMeta,
    },

    /// Remove an SSTable file (after compaction)
    DeleteFile {
        level: u8,
        slot_id: Option<u32>,
        file_number: u64,
    },

    /// Update slot metadata (K, heat, tombstone density, etc.)
    UpdateSlot {
        level: u8,
        slot_id: u32,
        slot: SlotMeta,
    },

    /// Update guard keys for a level
    UpdateGuards {
        level: u8,
        guards: Vec<Bytes>,
    },

    /// Advance the sequence number watermark
    SetLastSeqno { seqno: u64 },

    /// Advance the next file number
    SetNextFileNumber { file_number: u64 },

    /// Complete snapshot (replaces all previous state)
    Snapshot(Box<ManifestSnapshot>),
}

impl ManifestSnapshot {
    /// Returns all L0 files.
    pub fn l0_files(&self) -> &[RunMeta] {
        if self.levels.is_empty() {
            &[]
        } else {
            &self.levels[0].l0_files
        }
    }

    /// Returns all runs for a specific slot in a level.
    pub fn slot_runs(&self, level: u8, slot_id: u32) -> Vec<RunMeta> {
        if level == 0 || level as usize >= self.levels.len() {
            return vec![];
        }

        let level_meta = &self.levels[level as usize];
        level_meta
            .slots
            .iter()
            .find(|s| s.slot_id == slot_id)
            .map(|s| s.runs.clone())
            .unwrap_or_default()
    }

    /// Returns all files across all levels (for debugging/stats).
    pub fn all_files(&self) -> Vec<&RunMeta> {
        let mut files = Vec::new();

        // L0 files
        for file in self.l0_files() {
            files.push(file);
        }

        // L1+ files
        for level_meta in self.levels.iter().skip(1) {
            for slot in &level_meta.slots {
                for run in &slot.runs {
                    files.push(run);
                }
            }
        }

        files
    }

    /// Creates a new empty manifest snapshot.
    pub fn new() -> Self {
        Self {
            version: 0,
            next_file_number: 1,
            last_seqno: 0,
            levels: Vec::new(),
        }
    }

    /// Initializes a manifest with the given number of levels.
    pub fn with_levels(max_levels: u8) -> Self {
        let mut levels = Vec::new();
        for level in 0..=max_levels {
            levels.push(LevelMeta {
                level,
                guards: Vec::new(),
                slots: Vec::new(),
                l0_files: Vec::new(),
            });
        }

        Self {
            version: 0,
            next_file_number: 1,
            last_seqno: 0,
            levels,
        }
    }

    /// Applies an edit to this snapshot, returning the new version.
    pub fn apply_edit(&mut self, edit: ManifestEdit) -> Result<()> {
        match edit {
            ManifestEdit::AddFile {
                level,
                slot_id,
                run,
            } => {
                let level_meta = self
                    .levels
                    .get_mut(level as usize)
                    .ok_or_else(|| Error::Manifest(format!("Level {} does not exist", level)))?;

                if level == 0 {
                    // L0: add to unsorted file list
                    level_meta.l0_files.push(run);
                } else {
                    // L1+: add to slot
                    let slot_id = slot_id.ok_or_else(|| {
                        Error::Manifest("slot_id required for L1+".to_string())
                    })?;

                    // Find existing slot or create new one
                    let slot = level_meta
                        .slots
                        .iter_mut()
                        .find(|s| s.slot_id == slot_id);

                    match slot {
                        Some(slot) => {
                            slot.bytes += run.size;
                            slot.runs.push(run);
                        }
                        None => {
                            // Create new slot on demand
                            // Range will be set based on guards during L0 admission
                            level_meta.slots.push(SlotMeta {
                                slot_id,
                                range: SlotRange {
                                    start: run.min_key.clone(),
                                    end: run.max_key.clone(),
                                },
                                bytes: run.size,
                                runs: vec![run],
                                k: 3, // Default K value
                                heat_score: 0.0,
                                age_histogram: Vec::new(),
                                tombstone_density: 0.0,
                                last_compaction_ts: 0,
                                zns_zone_ids: Vec::new(),
                            });
                            // Keep slots sorted by slot_id
                            level_meta.slots.sort_by_key(|s| s.slot_id);
                        }
                    }
                }
            }

            ManifestEdit::DeleteFile {
                level,
                slot_id,
                file_number,
            } => {
                let level_meta = self
                    .levels
                    .get_mut(level as usize)
                    .ok_or_else(|| Error::Manifest(format!("Level {} does not exist", level)))?;

                if level == 0 {
                    level_meta.l0_files.retain(|r| r.file_number != file_number);
                } else {
                    let slot_id = slot_id.ok_or_else(|| {
                        Error::Manifest("slot_id required for L1+".to_string())
                    })?;

                    let slot = level_meta
                        .slots
                        .iter_mut()
                        .find(|s| s.slot_id == slot_id)
                        .ok_or_else(|| Error::SlotNotFound(level, slot_id))?;

                    if let Some(run) = slot.runs.iter().find(|r| r.file_number == file_number) {
                        slot.bytes = slot.bytes.saturating_sub(run.size);
                    }

                    slot.runs.retain(|r| r.file_number != file_number);
                }
            }

            ManifestEdit::UpdateSlot {
                level,
                slot_id,
                slot,
            } => {
                let level_meta = self
                    .levels
                    .get_mut(level as usize)
                    .ok_or_else(|| Error::Manifest(format!("Level {} does not exist", level)))?;

                if let Some(existing) = level_meta.slots.iter_mut().find(|s| s.slot_id == slot_id)
                {
                    *existing = slot;
                } else {
                    level_meta.slots.push(slot);
                }
            }

            ManifestEdit::UpdateGuards { level, guards } => {
                let level_meta = self
                    .levels
                    .get_mut(level as usize)
                    .ok_or_else(|| Error::Manifest(format!("Level {} does not exist", level)))?;

                level_meta.guards = guards;
            }

            ManifestEdit::SetLastSeqno { seqno } => {
                self.last_seqno = seqno;
            }

            ManifestEdit::SetNextFileNumber { file_number } => {
                self.next_file_number = file_number;
            }

            ManifestEdit::Snapshot(snapshot) => {
                *self = *snapshot;
            }
        }

        self.version += 1;
        Ok(())
    }

    /// Allocates a new SSTable file number.
    pub fn alloc_file_number(&mut self) -> u64 {
        let file_number = self.next_file_number;
        self.next_file_number += 1;
        file_number
    }

    /// Allocates a new sequence number.
    pub fn alloc_seqno(&mut self) -> u64 {
        self.last_seqno += 1;
        self.last_seqno
    }

    /// Returns the L0 file count.
    pub fn l0_file_count(&self) -> usize {
        self.levels
            .first()
            .map(|l| l.l0_files.len())
            .unwrap_or(0)
    }

    /// Returns metadata for a specific level.
    pub fn level(&self, level: u8) -> Option<&LevelMeta> {
        self.levels.get(level as usize)
    }

    /// Returns mutable metadata for a specific level.
    pub fn level_mut(&mut self, level: u8) -> Option<&mut LevelMeta> {
        self.levels.get_mut(level as usize)
    }
}

impl Default for ManifestSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

/// MANIFEST log manager for atomic updates and crash recovery.
///
/// # File Format
/// - Each edit is length-prefixed and bincode-encoded
/// - Format: [length: u32][edit: bincode bytes][crc32: u32]
/// - CURRENT file contains the name of the active MANIFEST file
///
/// # Crash Safety
/// - Edits are fsynced before updating CURRENT
/// - Partial edits at end are truncated during recovery
/// - Atomic CURRENT update via write + rename
pub struct ManifestLog {
    /// Directory containing MANIFEST files
    manifest_dir: PathBuf,

    /// Current in-memory snapshot
    snapshot: Arc<RwLock<ManifestSnapshot>>,

    /// Active MANIFEST file writer
    writer: Option<BufWriter<File>>,

    /// Current MANIFEST file number
    current_manifest_number: u64,

    /// Edit counter for triggering snapshots
    edits_since_snapshot: usize,

    /// Trigger snapshot after this many edits
    snapshot_threshold: usize,
}

impl ManifestLog {
    /// Opens or creates a MANIFEST log.
    ///
    /// # Recovery Protocol
    /// 1. Read CURRENT file to get active MANIFEST
    /// 2. Load snapshot if exists
    /// 3. Replay edits with CRC validation
    /// 4. Truncate partial tail if needed
    pub fn open(manifest_dir: impl AsRef<Path>, max_levels: u8) -> Result<Self> {
        let manifest_dir = manifest_dir.as_ref();
        fs::create_dir_all(manifest_dir)?;

        let current_path = manifest_dir.join("CURRENT");

        let (snapshot, manifest_number) = if current_path.exists() {
            Self::recover(manifest_dir, max_levels)?
        } else {
            // New MANIFEST
            let snapshot = ManifestSnapshot::with_levels(max_levels);
            (snapshot, 1)
        };

        let manifest_path = Self::manifest_path(manifest_dir, manifest_number);
        let writer = BufWriter::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&manifest_path)?,
        );

        // Write CURRENT file
        Self::write_current_file(manifest_dir, manifest_number)?;

        Ok(Self {
            manifest_dir: manifest_dir.to_path_buf(),
            snapshot: Arc::new(RwLock::new(snapshot)),
            writer: Some(writer),
            current_manifest_number: manifest_number,
            edits_since_snapshot: 0,
            snapshot_threshold: 100, // Write snapshot every 100 edits
        })
    }

    /// Appends an edit to the log and applies it to the in-memory snapshot.
    ///
    /// # Durability
    /// - Edit is serialized with CRC
    /// - Written to MANIFEST file
    /// - Fsynced to disk
    /// - Applied to in-memory snapshot
    pub fn append(&mut self, edit: ManifestEdit) -> Result<()> {
        // Serialize edit with length prefix and CRC
        let encoded = bincode::serialize(&edit)
            .map_err(|e| Error::Manifest(format!("Failed to serialize edit: {}", e)))?;

        let length = encoded.len() as u32;
        let crc = crc32c::crc32c(&encoded);

        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| Error::Manifest("Writer not initialized".to_string()))?;

        // Write: [length][data][crc]
        writer.write_all(&length.to_le_bytes())?;
        writer.write_all(&encoded)?;
        writer.write_all(&crc.to_le_bytes())?;
        writer.flush()?;

        // Fsync for durability
        writer.get_ref().sync_all()?;

        // Apply to in-memory snapshot
        {
            let mut snapshot = self.snapshot.write().unwrap();
            snapshot.apply_edit(edit)?;
        }

        self.edits_since_snapshot += 1;

        // Trigger snapshot if threshold reached
        if self.edits_since_snapshot >= self.snapshot_threshold {
            self.write_snapshot()?;
        }

        Ok(())
    }

    /// Writes a complete snapshot and starts a new MANIFEST file.
    pub fn write_snapshot(&mut self) -> Result<()> {
        let snapshot = self.snapshot.read().unwrap().clone();

        // Start new MANIFEST file
        let new_manifest_number = self.current_manifest_number + 1;
        let new_manifest_path = Self::manifest_path(&self.manifest_dir, new_manifest_number);

        let mut new_writer = BufWriter::new(File::create(&new_manifest_path)?);

        // Write snapshot as first edit
        let snapshot_edit = ManifestEdit::Snapshot(Box::new(snapshot));
        let encoded = bincode::serialize(&snapshot_edit)
            .map_err(|e| Error::Manifest(format!("Failed to serialize snapshot: {}", e)))?;

        let length = encoded.len() as u32;
        let crc = crc32c::crc32c(&encoded);

        new_writer.write_all(&length.to_le_bytes())?;
        new_writer.write_all(&encoded)?;
        new_writer.write_all(&crc.to_le_bytes())?;
        new_writer.flush()?;
        new_writer.get_ref().sync_all()?;

        // Update CURRENT file
        Self::write_current_file(&self.manifest_dir, new_manifest_number)?;

        // Switch to new writer
        self.writer = Some(new_writer);
        self.current_manifest_number = new_manifest_number;
        self.edits_since_snapshot = 0;

        // Clean up old MANIFEST files (keep last 2)
        self.cleanup_old_manifests()?;

        Ok(())
    }

    /// Returns a read-only view of the current snapshot.
    pub fn snapshot(&self) -> Arc<RwLock<ManifestSnapshot>> {
        Arc::clone(&self.snapshot)
    }

    /// Recovers the MANIFEST from disk.
    fn recover(manifest_dir: &Path, max_levels: u8) -> Result<(ManifestSnapshot, u64)> {
        let current_path = manifest_dir.join("CURRENT");
        let mut current_file = File::open(current_path)?;
        let mut manifest_name = String::new();
        current_file.read_to_string(&mut manifest_name)?;
        let manifest_name = manifest_name.trim();

        // Parse manifest number from "MANIFEST-NNNNNN"
        let manifest_number = manifest_name
            .strip_prefix("MANIFEST-")
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| Error::Manifest("Invalid CURRENT file format".to_string()))?;

        let manifest_path = manifest_dir.join(manifest_name);
        let mut reader = BufReader::new(File::open(manifest_path)?);

        let mut snapshot = ManifestSnapshot::with_levels(max_levels);

        loop {
            // Read length
            let mut length_buf = [0u8; 4];
            if reader.read_exact(&mut length_buf).is_err() {
                break; // EOF or partial write
            }
            let length = u32::from_le_bytes(length_buf) as usize;

            // Read edit data
            let mut data = vec![0u8; length];
            if reader.read_exact(&mut data).is_err() {
                break; // Partial write
            }

            // Read CRC
            let mut crc_buf = [0u8; 4];
            if reader.read_exact(&mut crc_buf).is_err() {
                break; // Partial write
            }
            let stored_crc = u32::from_le_bytes(crc_buf);
            let computed_crc = crc32c::crc32c(&data);

            if stored_crc != computed_crc {
                // CRC mismatch: truncate here
                break;
            }

            // Deserialize and apply edit
            let edit: ManifestEdit = bincode::deserialize(&data).map_err(|e| {
                Error::Manifest(format!("Failed to deserialize edit: {}", e))
            })?;

            snapshot.apply_edit(edit)?;
        }

        Ok((snapshot, manifest_number))
    }

    /// Returns the path to a MANIFEST file.
    fn manifest_path(manifest_dir: &Path, number: u64) -> PathBuf {
        manifest_dir.join(format!("MANIFEST-{:06}", number))
    }

    /// Writes the CURRENT file atomically.
    fn write_current_file(manifest_dir: &Path, manifest_number: u64) -> Result<()> {
        let current_path = manifest_dir.join("CURRENT");
        let temp_path = manifest_dir.join("CURRENT.tmp");

        let manifest_name = format!("MANIFEST-{:06}", manifest_number);
        fs::write(&temp_path, &manifest_name)?;

        // Atomic rename
        fs::rename(&temp_path, &current_path)?;

        // Fsync directory to ensure rename is durable
        File::open(manifest_dir)?.sync_all()?;

        Ok(())
    }

    /// Cleans up old MANIFEST files, keeping only the last 2.
    fn cleanup_old_manifests(&self) -> Result<()> {
        let mut manifest_files: Vec<(u64, PathBuf)> = Vec::new();

        for entry in fs::read_dir(&self.manifest_dir)? {
            let entry = entry?;
            let path = entry.path();

            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if let Some(num_str) = name.strip_prefix("MANIFEST-") {
                    if let Ok(num) = num_str.parse::<u64>() {
                        manifest_files.push((num, path));
                    }
                }
            }
        }

        // Sort by number descending
        manifest_files.sort_by(|a, b| b.0.cmp(&a.0));

        // Keep last 2, delete the rest
        for (_, path) in manifest_files.iter().skip(2) {
            let _ = fs::remove_file(path);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_snapshot_creation() {
        let manifest = ManifestSnapshot::with_levels(7);
        assert_eq!(manifest.levels.len(), 8); // L0-L7
        assert_eq!(manifest.version, 0);
        assert_eq!(manifest.next_file_number, 1);
        assert_eq!(manifest.last_seqno, 0);
    }

    #[test]
    fn test_alloc_file_number() {
        let mut manifest = ManifestSnapshot::new();
        assert_eq!(manifest.alloc_file_number(), 1);
        assert_eq!(manifest.alloc_file_number(), 2);
        assert_eq!(manifest.alloc_file_number(), 3);
    }

    #[test]
    fn test_alloc_seqno() {
        let mut manifest = ManifestSnapshot::new();
        assert_eq!(manifest.alloc_seqno(), 1);
        assert_eq!(manifest.alloc_seqno(), 2);
        assert_eq!(manifest.alloc_seqno(), 3);
    }

    #[test]
    fn test_add_l0_file() {
        let mut manifest = ManifestSnapshot::with_levels(7);

        let run = RunMeta {
            file_number: 1,
            size: 1024,
            min_key: Bytes::from("a"),
            max_key: Bytes::from("z"),
            min_seqno: 1,
            max_seqno: 100,
            tombstone_count: 0,
            filter_fp: 0.001,
            heat_hint: 0.5,
            value_log_segment_id: None,
        };

        manifest
            .apply_edit(ManifestEdit::AddFile {
                level: 0,
                slot_id: None,
                run,
            })
            .unwrap();

        assert_eq!(manifest.l0_file_count(), 1);
        assert_eq!(manifest.version, 1);
    }

    #[test]
    fn test_delete_l0_file() {
        let mut manifest = ManifestSnapshot::with_levels(7);

        let run = RunMeta {
            file_number: 1,
            size: 1024,
            min_key: Bytes::from("a"),
            max_key: Bytes::from("z"),
            min_seqno: 1,
            max_seqno: 100,
            tombstone_count: 0,
            filter_fp: 0.001,
            heat_hint: 0.5,
            value_log_segment_id: None,
        };

        manifest
            .apply_edit(ManifestEdit::AddFile {
                level: 0,
                slot_id: None,
                run,
            })
            .unwrap();

        assert_eq!(manifest.l0_file_count(), 1);

        manifest
            .apply_edit(ManifestEdit::DeleteFile {
                level: 0,
                slot_id: None,
                file_number: 1,
            })
            .unwrap();

        assert_eq!(manifest.l0_file_count(), 0);
    }

    #[test]
    fn test_update_guards() {
        let mut manifest = ManifestSnapshot::with_levels(7);

        let guards = vec![
            Bytes::from("a"),
            Bytes::from("m"),
            Bytes::from("z"),
        ];

        manifest
            .apply_edit(ManifestEdit::UpdateGuards {
                level: 1,
                guards: guards.clone(),
            })
            .unwrap();

        let level1 = manifest.level(1).unwrap();
        assert_eq!(level1.guards.len(), 3);
        assert_eq!(level1.guards[0], Bytes::from("a"));
    }

    #[test]
    fn test_add_file_to_slot() {
        let mut manifest = ManifestSnapshot::with_levels(7);

        // First, create a slot
        let slot = SlotMeta {
            slot_id: 0,
            range: SlotRange {
                start: Bytes::from("a"),
                end: Bytes::from("m"),
            },
            bytes: 0,
            runs: Vec::new(),
            k: 3,
            heat_score: 0.5,
            age_histogram: Vec::new(),
            tombstone_density: 0.0,
            last_compaction_ts: 0,
            zns_zone_ids: Vec::new(),
        };

        manifest
            .apply_edit(ManifestEdit::UpdateSlot {
                level: 1,
                slot_id: 0,
                slot,
            })
            .unwrap();

        // Now add a file to the slot
        let run = RunMeta {
            file_number: 1,
            size: 1024,
            min_key: Bytes::from("a"),
            max_key: Bytes::from("k"),
            min_seqno: 1,
            max_seqno: 100,
            tombstone_count: 0,
            filter_fp: 0.001,
            heat_hint: 0.5,
            value_log_segment_id: None,
        };

        manifest
            .apply_edit(ManifestEdit::AddFile {
                level: 1,
                slot_id: Some(0),
                run,
            })
            .unwrap();

        let level1 = manifest.level(1).unwrap();
        let slot = &level1.slots[0];
        assert_eq!(slot.runs.len(), 1);
        assert_eq!(slot.bytes, 1024);
    }

    #[test]
    fn test_set_seqno_and_file_number() {
        let mut manifest = ManifestSnapshot::new();

        manifest
            .apply_edit(ManifestEdit::SetLastSeqno { seqno: 100 })
            .unwrap();
        assert_eq!(manifest.last_seqno, 100);

        manifest
            .apply_edit(ManifestEdit::SetNextFileNumber { file_number: 50 })
            .unwrap();
        assert_eq!(manifest.next_file_number, 50);
    }

    #[test]
    fn test_manifest_log_create() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manifest_dir = temp_dir.path().join("manifest");

        let log = ManifestLog::open(&manifest_dir, 7).unwrap();

        // Verify CURRENT file exists
        assert!(manifest_dir.join("CURRENT").exists());

        // Verify MANIFEST-000001 exists
        assert!(manifest_dir.join("MANIFEST-000001").exists());

        let snapshot = log.snapshot();
        let snapshot = snapshot.read().unwrap();
        assert_eq!(snapshot.levels.len(), 8); // L0-L7
    }

    #[test]
    fn test_manifest_log_append_and_recover() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manifest_dir = temp_dir.path().join("manifest");

        {
            let mut log = ManifestLog::open(&manifest_dir, 7).unwrap();

            // Add L0 file
            let run = RunMeta {
                file_number: 1,
                size: 1024,
                min_key: Bytes::from("a"),
                max_key: Bytes::from("z"),
                min_seqno: 1,
                max_seqno: 100,
                tombstone_count: 0,
                filter_fp: 0.001,
                heat_hint: 0.5,
                value_log_segment_id: None,
            };

            log.append(ManifestEdit::AddFile {
                level: 0,
                slot_id: None,
                run,
            })
            .unwrap();

            log.append(ManifestEdit::SetLastSeqno { seqno: 200 })
                .unwrap();

            let snapshot = log.snapshot();
            let snapshot = snapshot.read().unwrap();
            assert_eq!(snapshot.l0_file_count(), 1);
            assert_eq!(snapshot.last_seqno, 200);
        }

        // Reopen and verify recovery
        {
            let log = ManifestLog::open(&manifest_dir, 7).unwrap();
            let snapshot = log.snapshot();
            let snapshot = snapshot.read().unwrap();

            assert_eq!(snapshot.l0_file_count(), 1);
            assert_eq!(snapshot.last_seqno, 200);
        }
    }

    #[test]
    fn test_manifest_log_snapshot_trigger() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manifest_dir = temp_dir.path().join("manifest");

        let mut log = ManifestLog::open(&manifest_dir, 7).unwrap();
        log.snapshot_threshold = 5; // Trigger after 5 edits

        // Add 6 edits to trigger snapshot
        for i in 1..=6 {
            log.append(ManifestEdit::SetLastSeqno { seqno: i * 10 })
                .unwrap();
        }

        // Should have created MANIFEST-000002
        assert!(manifest_dir.join("MANIFEST-000002").exists());

        let snapshot = log.snapshot();
        let snapshot = snapshot.read().unwrap();
        assert_eq!(snapshot.last_seqno, 60);
    }
}
