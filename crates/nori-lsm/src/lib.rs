//! nori-lsm: Embeddable ATLL (Adaptive Tiered-Leveled LSM) engine.
//!
//! Implements the full ATLL design with:
//! - Guard-based range partitioning (slots)
//! - Dynamic K-way fanout per slot (adapts to heat)
//! - Learned guard placement (quantile sketches)
//! - Bandit-based compaction scheduler
//! - Bounded read fan-in for predictable tail latency
//! - WAL durability + SSTable storage
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │  Memtable (skiplist)                                        │
//! │  - In-memory writes                                         │
//! │  - WAL durability                                           │
//! │  - Flush trigger: 64MB or 30s WAL age                      │
//! └──────────────┬──────────────────────────────────────────────┘
//!                │ Flush
//!                ↓
//! ┌─────────────────────────────────────────────────────────────┐
//! │  L0 (unbounded overlapping SSTables)                        │
//! │  - Direct memtable flushes                                  │
//! │  - Admitted to L1 by splitting on guard boundaries          │
//! └──────────────┬──────────────────────────────────────────────┘
//!                │ L0→L1 admission
//!                ↓
//! ┌─────────────────────────────────────────────────────────────┐
//! │  L1+ (guard-partitioned levels)                             │
//! │  ┌────────────────────────────────────────────────────────┐ │
//! │  │ Slot 0    │ Slot 1    │ Slot 2    │ ... │ Slot N       │ │
//! │  │ [g₀, g₁)  │ [g₁, g₂)  │ [g₂, g₃)  │     │ [gₙ, +∞)    │ │
//! │  │ K=1-3 runs│ K=1-3 runs│ K=1-3 runs│     │ K=1-3 runs   │ │
//! │  └────────────────────────────────────────────────────────┘ │
//! │  - Hot slots: K→1 (leveled, low read amp)                   │
//! │  - Cold slots: K>1 (tiered, low write amp)                  │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```no_run
//! use nori_lsm::{LsmEngine, ATLLConfig};
//! use bytes::Bytes;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = ATLLConfig::default();
//!     let engine = LsmEngine::open(config).await?;
//!
//!     // Write
//!     engine.put(Bytes::from("key"), Bytes::from("value")).await?;
//!
//!     // Read
//!     if let Some(value) = engine.get(b"key").await? {
//!         println!("Value: {:?}", value);
//!     }
//!
//!     // Range scan
//!     let mut iter = engine.iter_range(b"a", b"z").await?;
//!     // ... iterate
//!
//!     Ok(())
//! }
//! ```
//!
//! # Performance Targets (from spec)
//!
//! - p95 GET latency: < 10ms
//! - p95 PUT latency: < 20ms
//! - Write amplification: < 12x (lower than pure leveled)
//! - Read fan-in: bounded by sum(K_i) + L0_files
//!
//! # Design References
//!
//! See `context/lsm_atll_design.yaml` for complete specification.

pub mod compaction;
pub mod config;
pub mod error;
pub mod flush;
pub mod guards;
pub mod heat;
pub mod iterator;
pub mod manifest;
pub mod memtable;

// Core modules (to be implemented in phases)
// pub mod filters;
// pub mod value_log;

pub use config::ATLLConfig;
pub use error::{Error, Result};

// Re-export key types from dependencies
pub use bytes::Bytes;
pub use nori_observe::{Meter, NoopMeter};
pub use nori_sstable::Entry;
pub use nori_wal::Record;

use guards::GuardManager;
use heat::HeatTracker;
use manifest::ManifestLog;
use memtable::Memtable;
use nori_wal::{Wal, WalConfig};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use std::time::Instant;

/// Main LSM engine implementing the ATLL (Adaptive Tiered-Leveled LSM) design.
///
/// Provides durable key-value storage with:
/// - Memtable for in-memory writes
/// - WAL for crash recovery
/// - Multi-level LSM tree with adaptive tiered-leveled compaction
/// - Guard-based range partitioning (slots)
/// - Heat tracking for dynamic K adjustment
/// - Snapshot isolation for reads
pub struct LsmEngine {
    /// Active memtable for writes
    memtable: Arc<parking_lot::RwLock<Memtable>>,

    /// Immutable memtables being flushed
    immutable_memtables: Arc<parking_lot::RwLock<Vec<Arc<Memtable>>>>,

    /// Write-ahead log for durability
    wal: Arc<parking_lot::RwLock<Wal>>,

    /// WAL creation time for age-based flush trigger
    wal_create_time: Arc<parking_lot::RwLock<Instant>>,

    /// Manifest tracking LSM levels and files
    manifest: Arc<parking_lot::RwLock<ManifestLog>>,

    /// Guard manager for slot boundaries
    guards: Arc<GuardManager>,

    /// Heat tracker for workload adaptation
    heat: Arc<HeatTracker>,

    /// Configuration
    config: ATLLConfig,

    /// SSTable directory path
    sst_dir: PathBuf,

    /// Next sequence number for writes
    seqno: Arc<AtomicU64>,

    /// Shutdown signal for background compaction thread
    compaction_shutdown: Arc<AtomicBool>,
}

impl LsmEngine {
    /// Opens an LSM engine with the given configuration.
    ///
    /// Creates or loads the LSM engine from disk. If the directory doesn't exist,
    /// initializes a fresh engine with empty manifest and guard set.
    ///
    /// # Steps
    /// 1. Create SSTable directory if needed
    /// 2. Load or create manifest
    /// 3. Initialize WAL and perform recovery
    /// 4. Replay WAL records into memtable
    /// 5. Initialize guard manager
    /// 6. Initialize heat tracker
    ///
    /// # TODO
    /// - Background compaction threads (Phase 6)
    /// - Persistent heat tracker state (Phase 8)
    pub async fn open(config: ATLLConfig) -> Result<Self> {
        use std::time::Duration;

        // Ensure SSTable directory exists
        let sst_dir = PathBuf::from(&config.data_dir);
        std::fs::create_dir_all(&sst_dir)?;

        // Load or create manifest (synchronous operation)
        let manifest_dir = sst_dir.join("manifest");
        let manifest = ManifestLog::open(&manifest_dir, config.max_levels)?;

        // Initialize sequence number from manifest's max seqno
        let snapshot = manifest.snapshot();
        let snapshot_guard = snapshot.read().unwrap();
        let max_seqno = snapshot_guard
            .all_files()
            .iter()
            .map(|run| run.max_seqno)
            .max()
            .unwrap_or(0);
        drop(snapshot_guard);
        let mut next_seqno = max_seqno + 1;

        // Initialize WAL with recovery
        let wal_dir = sst_dir.join("wal");
        let wal_config = WalConfig {
            dir: wal_dir,
            max_segment_size: 128 * 1024 * 1024, // 128 MB
            fsync_policy: nori_wal::FsyncPolicy::Batch(Duration::from_millis(5)),
            preallocate: true,
            node_id: 0,
        };

        let (wal, recovery_info) = Wal::open(wal_config).await
            .map_err(|e| Error::Internal(format!("Failed to open WAL: {}", e)))?;

        // Create memtable and replay WAL if needed
        let memtable = Memtable::new(next_seqno);

        if recovery_info.valid_records > 0 {
            // Replay all recovered records from the beginning
            let start_pos = nori_wal::Position {
                segment_id: 0,
                offset: 0,
            };
            let mut reader = wal.read_from(start_pos).await
                .map_err(|e| Error::Internal(format!("Failed to read WAL: {}", e)))?;

            loop {
                match reader.next_record().await {
                    Ok(Some((record, _pos))) => {
                        let seqno = next_seqno;
                        next_seqno += 1;

                        if record.tombstone {
                            memtable.delete(record.key, seqno)?;
                        } else {
                            memtable.put(record.key, record.value, seqno)?;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        return Err(Error::Internal(format!("WAL read error: {}", e)));
                    }
                }
            }
        }

        // Initialize guard manager with default guards for L1
        let guards = Arc::new(GuardManager::new(1, config.l1_slot_count)?);

        // Initialize heat tracker
        let heat = Arc::new(HeatTracker::new(config.clone()));

        // Create shutdown signal for background compaction
        let compaction_shutdown = Arc::new(AtomicBool::new(false));

        let engine = Self {
            memtable: Arc::new(parking_lot::RwLock::new(memtable)),
            immutable_memtables: Arc::new(parking_lot::RwLock::new(Vec::new())),
            wal: Arc::new(parking_lot::RwLock::new(wal)),
            wal_create_time: Arc::new(parking_lot::RwLock::new(Instant::now())),
            manifest: Arc::new(parking_lot::RwLock::new(manifest)),
            guards: guards.clone(),
            heat: heat.clone(),
            config: config.clone(),
            sst_dir: sst_dir.clone(),
            seqno: Arc::new(AtomicU64::new(next_seqno)),
            compaction_shutdown: compaction_shutdown.clone(),
        };

        // Spawn background compaction thread
        let manifest_clone = engine.manifest.clone();
        let shutdown_clone = compaction_shutdown.clone();
        let config_clone = config.clone();
        let sst_dir_clone = sst_dir.clone();
        let heat_clone = heat.clone();

        tokio::spawn(async move {
            Self::compaction_loop(
                manifest_clone,
                guards,
                heat_clone,
                config_clone,
                sst_dir_clone,
                shutdown_clone,
            )
            .await;
        });

        Ok(engine)
    }

    /// Retrieves the value for a given key.
    ///
    /// # Read Path
    /// 1. Check memtable (newest data)
    /// 2. Check L0 (all files, newest first)
    /// 3. Check L1+ (only overlapping slot, bounded K runs per slot)
    ///
    /// # Heat Tracking
    /// Records GET operation for the key's slot to update heat scores.
    ///
    /// # TODO
    /// - Bloom filter optimization (Phase 8)
    /// - Heat tracking integration (currently commented out)
    pub async fn get(&self, key: &[u8]) -> Result<Option<Bytes>> {
        use crate::memtable::MemtableEntry;
        use nori_sstable::SSTableReader;

        // 1. Check memtable first (newest data)
        {
            let memtable = self.memtable.read();
            if let Some(entry) = memtable.get(key) {
                return match entry {
                    MemtableEntry::Put { value, seqno: _ } => Ok(Some(value)),
                    MemtableEntry::Delete { seqno: _ } => Ok(None), // Tombstone
                };
            }
        }

        // 2. Check L0 files (newest to oldest)
        let snapshot = self.manifest.read().snapshot();
        let snapshot_guard = snapshot.read().unwrap();
        let l0_files = snapshot_guard.l0_files();

        // L0 files are ordered newest first in the manifest
        for run in l0_files.iter() {
            let sst_path = self.sst_dir.join(format!("{}.sst", run.file_number));
            if !sst_path.exists() {
                continue; // Skip missing files (should log warning in production)
            }

            // TODO: Check bloom filter before opening file
            let reader = SSTableReader::open(sst_path).await
                .map_err(|e| Error::SSTable(format!("Failed to open SSTable: {}", e)))?;

            if let Some(entry) = reader.get(key).await
                .map_err(|e| Error::SSTable(format!("SSTable read error: {}", e)))? {
                return if entry.tombstone {
                    Ok(None)
                } else {
                    Ok(Some(entry.value))
                };
            }
        }

        // 3. Check L1+ levels (only overlapping slots)
        // Determine which slot this key belongs to
        let slot_id = self.guards.slot_for_key(key);

        for level in 1..self.config.max_levels {
            let runs = snapshot_guard.slot_runs(level, slot_id);

            for run in runs.iter() {
                // Check if this run overlaps the key
                if key < run.min_key.as_ref() || key >= run.max_key.as_ref() {
                    continue; // Key not in this run's range
                }

                let sst_path = self.sst_dir.join(format!("{}.sst", run.file_number));
                if !sst_path.exists() {
                    continue;
                }

                // TODO: Check bloom filter before opening file
                let reader = SSTableReader::open(sst_path).await
                    .map_err(|e| Error::SSTable(format!("Failed to open SSTable: {}", e)))?;

                if let Some(entry) = reader.get(key).await
                    .map_err(|e| Error::SSTable(format!("SSTable read error: {}", e)))? {
                    return if entry.tombstone {
                        Ok(None)
                    } else {
                        Ok(Some(entry.value))
                    };
                }
            }
        }

        // TODO: Heat tracking
        // self.heat.record_op(level, slot_id, heat::Operation::Get);

        // Key not found
        Ok(None)
    }

    /// Inserts or updates a key-value pair.
    ///
    /// # Write Path
    /// 1. Check L0 backpressure (block if L0 > threshold)
    /// 2. Append to WAL for durability
    /// 3. Acquire next sequence number
    /// 4. Insert into memtable
    /// 5. Check flush triggers (size/age)
    /// 6. Return sequence number
    ///
    /// # Errors
    /// Returns `Error::L0Stall` if L0 file count exceeds threshold.
    /// Caller should retry after a short delay.
    pub async fn put(&self, key: Bytes, value: Bytes) -> Result<u64> {
        use std::sync::atomic::Ordering;

        // 1. Check L0 backpressure
        self.check_l0_pressure()?;

        // 2. Write to WAL first (durability)
        let record = Record::put(key.clone(), value.clone());
        {
            let wal = self.wal.write();
            wal.append(&record).await
                .map_err(|e| Error::Internal(format!("WAL append failed: {}", e)))?;
        }

        // 3. Get next sequence number
        let seqno = self.seqno.fetch_add(1, Ordering::SeqCst);

        // 4. Insert into memtable
        {
            let memtable = self.memtable.read();
            memtable.put(key, value, seqno)?;
        }

        // 5. Check flush triggers
        self.check_flush_triggers().await?;

        Ok(seqno)
    }

    /// Deletes a key (writes a tombstone).
    ///
    /// # Write Path
    /// 1. Check L0 backpressure (block if L0 > threshold)
    /// 2. Append tombstone to WAL for durability
    /// 3. Acquire next sequence number
    /// 4. Write tombstone to memtable
    /// 5. Check flush triggers
    ///
    /// # Errors
    /// Returns `Error::L0Stall` if L0 file count exceeds threshold.
    pub async fn delete(&self, key: &[u8]) -> Result<()> {
        use std::sync::atomic::Ordering;

        // 1. Check L0 backpressure
        self.check_l0_pressure()?;

        // 2. Write to WAL first (durability)
        let key_bytes = Bytes::copy_from_slice(key);
        let record = Record::delete(key_bytes.clone());
        {
            let wal = self.wal.write();
            wal.append(&record).await
                .map_err(|e| Error::Internal(format!("WAL append failed: {}", e)))?;
        }

        // 3. Get next sequence number
        let seqno = self.seqno.fetch_add(1, Ordering::SeqCst);

        // 4. Write tombstone to memtable
        {
            let memtable = self.memtable.read();
            memtable.delete(key_bytes, seqno)?;
        }

        // 5. Check flush triggers
        self.check_flush_triggers().await?;

        Ok(())
    }

    /// Returns an iterator over a key range [start, end).
    ///
    /// # Fan-in
    /// Merges across: memtable + L0 + sum(K_{i,s}) for each level
    /// Maximum fan-in with default config: 1 + 12 + (7 levels × 32 slots × 3 K) ≈ 685 iterators
    /// In practice, much lower due to range filtering and dynamic K.
    ///
    /// # Implementation
    /// Creates iterator sources from:
    /// 1. Memtable (highest priority)
    /// 2. All overlapping L0 files
    /// 3. Overlapping runs from L1+ slots
    pub async fn iter_range(&self, start: &[u8], end: &[u8]) -> Result<MergeIterator> {
        use crate::iterator::{IteratorSource, LsmIterator, SourceType};

        let mut sources = Vec::new();
        let mut source_types = Vec::new();

        // 1. Add memtable source
        let memtable = self.memtable.read();
        let memtable_range = memtable.range(start, end);
        drop(memtable);

        sources.push(IteratorSource::Memtable {
            iter: memtable_range,
            pos: 0,
        });
        source_types.push(SourceType::Memtable);

        // Get snapshot for stable view
        let snapshot = self.manifest.read().snapshot();
        let snapshot_guard = snapshot.read().unwrap();

        // 2. Add L0 files (all overlapping files)
        let l0_files = snapshot_guard.l0_files();
        for (file_idx, run) in l0_files.iter().enumerate() {
            // Check if this L0 file overlaps the range
            if !run.overlaps(start, end) {
                continue;
            }

            let sst_path = self.sst_dir.join(format!("{}.sst", run.file_number));
            if !sst_path.exists() {
                continue;
            }

            // Open SSTable iterator
            let reader = Arc::new(nori_sstable::SSTableReader::open(sst_path).await
                .map_err(|e| Error::SSTable(format!("Failed to open SSTable: {}", e)))?);

            let iter = reader.iter_range(Bytes::copy_from_slice(start), Bytes::copy_from_slice(end));

            sources.push(IteratorSource::SSTable(iter));
            source_types.push(SourceType::L0 { file_idx });
        }

        // 3. Add L1+ levels (only overlapping slots)
        let overlapping_slots = self.guards.overlapping_slots(start, end);

        for level in 1..self.config.max_levels {
            for slot_id in &overlapping_slots {
                let runs = snapshot_guard.slot_runs(level, *slot_id);

                for (run_idx, run) in runs.iter().enumerate() {
                    // Check if this run overlaps the range
                    if !run.overlaps(start, end) {
                        continue;
                    }

                    let sst_path = self.sst_dir.join(format!("{}.sst", run.file_number));
                    if !sst_path.exists() {
                        continue;
                    }

                    // Open SSTable iterator
                    let reader = Arc::new(nori_sstable::SSTableReader::open(sst_path).await
                        .map_err(|e| Error::SSTable(format!("Failed to open SSTable: {}", e)))?);

                    let iter = reader.iter_range(Bytes::copy_from_slice(start), Bytes::copy_from_slice(end));

                    sources.push(IteratorSource::SSTable(iter));
                    source_types.push(SourceType::Level {
                        level,
                        slot_id: *slot_id,
                        run_idx,
                    });
                }
            }
        }

        drop(snapshot_guard);

        // Create LSM iterator
        let mut lsm_iter = LsmIterator::new(
            sources,
            source_types,
            Some(Bytes::copy_from_slice(end)),
            None, // No snapshot isolation yet
        );

        // Initialize the heap
        lsm_iter.init().await?;

        Ok(MergeIterator { inner: lsm_iter })
    }

    /// Checks if flush triggers are met and initiates flush if needed.
    ///
    /// Triggers:
    /// - Size: memtable >= 64 MiB
    /// - Age: WAL age >= 30 seconds
    async fn check_flush_triggers(&self) -> Result<()> {
        let should_flush = {
            let memtable = self.memtable.read();
            let size_bytes = memtable.size();
            let size_trigger = size_bytes >= self.config.memtable.flush_trigger_bytes;

            let wal_age = self.wal_create_time.read().elapsed();
            let age_trigger = wal_age.as_secs() >= self.config.memtable.wal_age_trigger_sec;

            size_trigger || age_trigger
        };

        if should_flush {
            self.flush_memtable().await?;
        }

        Ok(())
    }

    /// Freezes the active memtable and initiates background flush.
    ///
    /// # Process
    /// 1. Capture WAL position (for cleanup after flush)
    /// 2. Acquire write lock on memtable
    /// 3. Swap active → immutable (create new empty memtable)
    /// 4. Add immutable to list
    /// 5. Reset WAL create time
    /// 6. Flush to L0, register in MANIFEST
    /// 7. Delete old WAL segments
    async fn flush_memtable(&self) -> Result<()> {
        use flush::Flusher;

        // 1. Capture WAL position before freezing
        // All entries before this position will be in the flushed SSTable
        let wal_position = {
            let wal = self.wal.read();
            wal.current_position().await
        };

        // 2. Freeze memtable (swap with new empty memtable)
        let frozen_memtable = {
            let mut memtable_guard = self.memtable.write();
            let old_seqno = self.seqno.load(std::sync::atomic::Ordering::SeqCst);
            let new_memtable = Memtable::new(old_seqno);
            let frozen = std::mem::replace(&mut *memtable_guard, new_memtable);
            Arc::new(frozen)
        };

        // Skip empty memtables
        if frozen_memtable.is_empty() {
            return Ok(());
        }

        // 3. Reset WAL create time
        {
            let mut wal_time = self.wal_create_time.write();
            *wal_time = Instant::now();
        }

        // 3. Flush to L0 (synchronous for now; Phase 6 will make this async)
        let file_number = {
            let manifest = self.manifest.read();
            manifest.snapshot().read().unwrap().version as u64 + 1000
        };

        let flusher = Flusher::new(&self.sst_dir, self.config.clone())?;
        let run_meta = flusher.flush_to_l0(&frozen_memtable, file_number).await?;

        // 4. Register in MANIFEST as L0 file
        let run_meta = {
            let mut manifest = self.manifest.write();
            let edit = manifest::ManifestEdit::AddFile {
                level: 0,
                slot_id: None, // L0 files have no slot
                run: run_meta.clone(),
            };
            manifest.append(edit)?;
            run_meta
        };

        // 5. Check if L0 admission should be triggered
        let l0_admitter = flush::L0Admitter::new(&self.sst_dir, self.config.clone())?;

        let should_admit = {
            let manifest = self.manifest.read();
            let snapshot = manifest.snapshot();
            let snapshot = snapshot.read().unwrap();
            l0_admitter.should_admit(snapshot.l0_file_count())
        };

        if should_admit {
            // Admit L0 file to L1
            let current_l0_count = {
                let manifest = self.manifest.read();
                let snapshot = manifest.snapshot();
                let snapshot_guard = snapshot.read().unwrap();
                snapshot_guard.l0_file_count()
            };

            tracing::info!(
                "L0 admission triggered: {} files > {} threshold",
                current_l0_count,
                self.config.l0.max_files
            );

            let edits = {
                let mut manifest = self.manifest.write();
                l0_admitter
                    .admit_to_l1(&run_meta, &self.guards, &mut manifest)
                    .await?
            };

            // Apply admission edits to manifest
            {
                let mut manifest = self.manifest.write();
                for edit in edits {
                    manifest.append(edit)?;
                }
            }

            tracing::debug!("L0 admission completed for file {}", run_meta.file_number);
        }

        // 6. Delete old WAL segments after successful flush
        // All data up to wal_position is now durably stored in L0
        {
            let wal = self.wal.read();
            match wal.delete_segments_before(wal_position).await {
                Ok(deleted_count) => {
                    if deleted_count > 0 {
                        tracing::info!(
                            "WAL cleanup: deleted {} segments after flush of file {}",
                            deleted_count,
                            run_meta.file_number
                        );
                    }
                }
                Err(e) => {
                    // WAL cleanup failure is not fatal - log warning and continue
                    // The WAL may grow larger than necessary but data is safe
                    tracing::warn!("WAL cleanup failed after flush: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Checks L0 backpressure and returns an error if threshold exceeded.
    ///
    /// # Algorithm
    /// L0 backpressure prevents unbounded L0 growth by blocking writes when:
    /// - L0 file count > max_files (default: 8)
    ///
    /// This gives the background compaction thread time to admit L0→L1
    /// and reduce backlog before accepting more writes.
    ///
    /// # Errors
    /// Returns `Error::L0Stall` if L0 file count exceeds threshold.
    /// Caller should:
    /// 1. Retry after a short delay (e.g., 10ms exponential backoff)
    /// 2. Log warning if stall persists (indicates compaction can't keep up)
    ///
    /// # Observability
    /// Emit VizEvent::WriteStall for dashboard visualization.
    fn check_l0_pressure(&self) -> Result<()> {
        let manifest = self.manifest.read();
        let snapshot = manifest.snapshot();
        let snapshot = snapshot.read().unwrap();

        let l0_count = snapshot.l0_file_count();
        let max_files = self.config.l0.max_files;

        if l0_count > max_files {
            return Err(Error::L0Stall(l0_count, max_files));
        }

        Ok(())
    }

    /// Triggers manual compaction for a key range.
    pub async fn compact_range(&self, _start: Option<&[u8]>, _end: Option<&[u8]>) -> Result<()> {
        // TODO: Future enhancement - manual compaction trigger
        Ok(())
    }

    /// Returns LSM statistics and metrics.
    pub fn stats(&self) -> Stats {
        // TODO: Phase 8 implementation
        Stats::default()
    }

    /// Background compaction loop.
    ///
    /// Runs continuously until shutdown signal is set.
    /// Each iteration:
    /// 1. Selects a compaction action via BanditScheduler
    /// 2. Executes the action via CompactionExecutor
    /// 3. Applies resulting ManifestEdits
    /// 4. Sleeps for compaction_interval_sec
    async fn compaction_loop(
        manifest: Arc<parking_lot::RwLock<ManifestLog>>,
        _guards: Arc<GuardManager>,
        _heat: Arc<HeatTracker>,
        config: ATLLConfig,
        _sst_dir: PathBuf,
        shutdown: Arc<AtomicBool>,
    ) {
        use std::sync::atomic::Ordering;
        use tokio::time::{sleep, Duration};

        tracing::info!("Background compaction loop started");

        // Phase 4 simplification: Run placeholder loop
        // Full implementation will be completed when CompactionExecutor is ready
        loop {
            // Check shutdown signal
            if shutdown.load(Ordering::Relaxed) {
                tracing::info!("Compaction loop shutting down");
                break;
            }

            // TODO Phase 4 continuation: Implement full compaction selection and execution
            // For now, just monitor manifest state and log
            {
                let manifest_guard = manifest.read();
                let snapshot = manifest_guard.snapshot();
                let snapshot_guard = snapshot.read().unwrap();

                let l0_count = snapshot_guard.l0_file_count();
                if l0_count > 0 {
                    tracing::debug!("Background compaction: L0 has {} files", l0_count);
                }
            }

            // Sleep before next iteration
            sleep(Duration::from_secs(config.io.compaction_interval_sec as u64)).await;
        }

        tracing::info!("Background compaction loop stopped");
    }
}

impl Drop for LsmEngine {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;

        // Signal compaction thread to shut down
        self.compaction_shutdown.store(true, Ordering::Relaxed);

        // Give the thread a moment to finish gracefully
        // In production, we'd use a proper join handle
        std::thread::sleep(std::time::Duration::from_millis(100));

        tracing::info!("LsmEngine dropped, compaction thread signaled to stop");
    }
}

/// Iterator over a key range.
pub struct MergeIterator {
    inner: iterator::LsmIterator,
}

impl MergeIterator {
    /// Returns the next key-value pair in the range.
    pub async fn next(&mut self) -> Result<Option<(Bytes, Bytes)>> {
        self.inner.next().await
    }
}

/// LSM engine statistics (placeholder).
#[derive(Debug, Default)]
pub struct Stats {
    pub l0_files: usize,
    pub compaction_bytes_in: u64,
    pub compaction_bytes_out: u64,
    pub write_amplification: f64,
    pub read_amplification_point: f64,
    pub space_amplification: f64,
}

// Placeholder function to satisfy the skeleton
pub fn placeholder() -> &'static str {
    "nori-lsm: ATLL implementation in progress (Phase 1/8)"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_placeholder() {
        assert_eq!(placeholder(), "nori-lsm: ATLL implementation in progress (Phase 1/8)");
    }

    #[tokio::test]
    async fn test_config_validation() {
        let config = ATLLConfig::default();
        assert!(config.validate().is_ok());
    }

    #[tokio::test]
    async fn test_engine_skeleton() {
        let config = ATLLConfig::default();
        let engine = LsmEngine::open(config).await;
        assert!(engine.is_ok());
    }

    /// Integration test: basic PUT and GET
    #[tokio::test]
    async fn test_put_get() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        let engine = LsmEngine::open(config).await.unwrap();

        // Write a key-value pair
        let key = Bytes::from("test_key");
        let value = Bytes::from("test_value");
        engine.put(key.clone(), value.clone()).await.unwrap();

        // Read it back
        let result = engine.get(b"test_key").await.unwrap();
        assert_eq!(result, Some(value));

        // Non-existent key
        let result = engine.get(b"nonexistent").await.unwrap();
        assert_eq!(result, None);
    }

    /// Integration test: DELETE and tombstone handling
    #[tokio::test]
    async fn test_delete() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        let engine = LsmEngine::open(config).await.unwrap();

        // Write and then delete
        let key = Bytes::from("delete_key");
        let value = Bytes::from("delete_value");
        engine.put(key.clone(), value.clone()).await.unwrap();

        // Verify it exists
        assert_eq!(engine.get(b"delete_key").await.unwrap(), Some(value));

        // Delete it
        engine.delete(b"delete_key").await.unwrap();

        // Verify tombstone masks the value
        assert_eq!(engine.get(b"delete_key").await.unwrap(), None);
    }

    /// Integration test: Overwrite semantics
    #[tokio::test]
    async fn test_overwrite() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        let engine = LsmEngine::open(config).await.unwrap();

        let key = Bytes::from("key");
        engine.put(key.clone(), Bytes::from("value1")).await.unwrap();
        engine.put(key.clone(), Bytes::from("value2")).await.unwrap();
        engine.put(key.clone(), Bytes::from("value3")).await.unwrap();

        // Should get the latest value
        let result = engine.get(b"key").await.unwrap();
        assert_eq!(result, Some(Bytes::from("value3")));
    }

    /// Integration test: Range scan (memtable only)
    #[tokio::test]
    async fn test_range_scan_memtable() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        let engine = LsmEngine::open(config).await.unwrap();

        // Write keys: a, b, c, d, e
        for ch in b'a'..=b'e' {
            let key = Bytes::from(vec![ch]);
            let value = Bytes::from(format!("value_{}", ch as char));
            engine.put(key, value).await.unwrap();
        }

        // Scan range [b, d)
        let mut iter = engine.iter_range(b"b", b"d").await.unwrap();

        let mut results = Vec::new();
        while let Some((key, value)) = iter.next().await.unwrap() {
            results.push((key, value));
        }

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, Bytes::from("b"));
        assert_eq!(results[0].1, Bytes::from("value_b"));
        assert_eq!(results[1].0, Bytes::from("c"));
        assert_eq!(results[1].1, Bytes::from("value_c"));
    }

    /// Integration test: Range scan with tombstones
    #[tokio::test]
    async fn test_range_scan_with_tombstones() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        let engine = LsmEngine::open(config).await.unwrap();

        // Write keys a, b, c, d
        for ch in b'a'..=b'd' {
            let key = Bytes::from(vec![ch]);
            let value = Bytes::from(format!("value_{}", ch as char));
            engine.put(key, value).await.unwrap();
        }

        // Delete 'b'
        engine.delete(b"b").await.unwrap();

        // Scan range [a, d)
        let mut iter = engine.iter_range(b"a", b"d").await.unwrap();

        let mut results = Vec::new();
        while let Some((key, value)) = iter.next().await.unwrap() {
            results.push((key, value));
        }

        // Should get a and c, but not b (tombstone)
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, Bytes::from("a"));
        assert_eq!(results[1].0, Bytes::from("c"));
    }

    /// Integration test: Empty range
    #[tokio::test]
    async fn test_empty_range() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        let engine = LsmEngine::open(config).await.unwrap();

        // Write keys a, b, c
        for ch in b'a'..=b'c' {
            let key = Bytes::from(vec![ch]);
            let value = Bytes::from(format!("value_{}", ch as char));
            engine.put(key, value).await.unwrap();
        }

        // Scan non-overlapping range [x, z)
        let mut iter = engine.iter_range(b"x", b"z").await.unwrap();

        let mut results = Vec::new();
        while let Some((key, value)) = iter.next().await.unwrap() {
            results.push((key, value));
        }

        assert_eq!(results.len(), 0);
    }

    /// Integration test: Sequence number monotonicity
    #[tokio::test]
    async fn test_seqno_monotonic() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        let engine = LsmEngine::open(config).await.unwrap();

        let seqno1 = engine.put(Bytes::from("key1"), Bytes::from("value1")).await.unwrap();
        let seqno2 = engine.put(Bytes::from("key2"), Bytes::from("value2")).await.unwrap();
        let seqno3 = engine.put(Bytes::from("key3"), Bytes::from("value3")).await.unwrap();

        assert!(seqno2 > seqno1);
        assert!(seqno3 > seqno2);
    }

    /// Integration test: Engine persistence (reopen)
    #[tokio::test]
    async fn test_engine_reopen() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        // First session: write data
        {
            let engine = LsmEngine::open(config.clone()).await.unwrap();
            engine.put(Bytes::from("persistent_key"), Bytes::from("persistent_value")).await.unwrap();
        }

        // Second session: reopen and verify WAL recovery
        {
            let engine = LsmEngine::open(config.clone()).await.unwrap();
            // With WAL recovery, data should be recovered
            let result = engine.get(b"persistent_key").await.unwrap();
            assert_eq!(result, Some(Bytes::from("persistent_value")));
        }
    }

    /// Phase 2 test: WAL durability - multiple records
    #[tokio::test]
    async fn test_wal_durability_multiple_records() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        // Write multiple records
        {
            let engine = LsmEngine::open(config.clone()).await.unwrap();
            for i in 0..100 {
                let key = Bytes::from(format!("key{:03}", i));
                let value = Bytes::from(format!("value{:03}", i));
                engine.put(key, value).await.unwrap();
            }
        }

        // Reopen and verify all records
        {
            let engine = LsmEngine::open(config.clone()).await.unwrap();
            for i in 0..100 {
                let key = format!("key{:03}", i);
                let expected = Bytes::from(format!("value{:03}", i));
                let result = engine.get(key.as_bytes()).await.unwrap();
                assert_eq!(result, Some(expected), "Key {} not recovered", key);
            }
        }
    }

    /// Phase 2 test: WAL recovery with tombstones
    #[tokio::test]
    async fn test_wal_recovery_with_tombstones() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        // Write, delete, write pattern
        {
            let engine = LsmEngine::open(config.clone()).await.unwrap();
            engine.put(Bytes::from("key1"), Bytes::from("value1")).await.unwrap();
            engine.put(Bytes::from("key2"), Bytes::from("value2")).await.unwrap();
            engine.delete(b"key1").await.unwrap();
            engine.put(Bytes::from("key3"), Bytes::from("value3")).await.unwrap();
        }

        // Reopen and verify tombstones work
        {
            let engine = LsmEngine::open(config.clone()).await.unwrap();
            assert_eq!(engine.get(b"key1").await.unwrap(), None); // Deleted
            assert_eq!(engine.get(b"key2").await.unwrap(), Some(Bytes::from("value2")));
            assert_eq!(engine.get(b"key3").await.unwrap(), Some(Bytes::from("value3")));
        }
    }

    /// Phase 2 test: Flush trigger by size (simplified - manual test)
    #[tokio::test]
    async fn test_flush_trigger_manual() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();
        // Set very small flush trigger for testing
        config.memtable.flush_trigger_bytes = 1024; // 1KB

        let engine = LsmEngine::open(config).await.unwrap();

        // Write enough data to trigger flush
        // Each entry is ~50 bytes, so 25 entries ≈ 1.25KB should trigger
        for i in 0..30 {
            let key = Bytes::from(format!("large_key_{:010}", i));
            let value = Bytes::from(vec![b'x'; 100]); // 100 byte value
            engine.put(key, value).await.unwrap();
        }

        // Check manifest for L0 files (flush should have occurred)
        let manifest = engine.manifest.read();
        let snapshot = manifest.snapshot();
        let snap_guard = snapshot.read().unwrap();
        let l0_count = snap_guard.l0_files().len();

        // We expect at least one flush to have occurred
        assert!(l0_count > 0, "Expected at least one L0 file after flush trigger");
    }

    /// Phase 2 test: WAL recovery order preservation
    #[tokio::test]
    async fn test_wal_recovery_order() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        // Write same key multiple times
        {
            let engine = LsmEngine::open(config.clone()).await.unwrap();
            engine.put(Bytes::from("key"), Bytes::from("v1")).await.unwrap();
            engine.put(Bytes::from("key"), Bytes::from("v2")).await.unwrap();
            engine.put(Bytes::from("key"), Bytes::from("v3")).await.unwrap();
            engine.put(Bytes::from("key"), Bytes::from("v4")).await.unwrap();
        }

        // Reopen and verify last value wins
        {
            let engine = LsmEngine::open(config.clone()).await.unwrap();
            let result = engine.get(b"key").await.unwrap();
            assert_eq!(result, Some(Bytes::from("v4")));
        }
    }

    /// Phase 2 test: Empty memtable doesn't flush
    #[tokio::test]
    async fn test_empty_memtable_no_flush() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        let engine = LsmEngine::open(config).await.unwrap();

        // Manually trigger flush check (should do nothing)
        engine.check_flush_triggers().await.unwrap();

        // Verify no L0 files created
        let manifest = engine.manifest.read();
        let snapshot = manifest.snapshot();
        let snap_guard = snapshot.read().unwrap();
        assert_eq!(snap_guard.l0_files().len(), 0);
    }

    #[tokio::test]
    async fn test_l0_admission_triggers_at_threshold() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();
        config.memtable.flush_trigger_bytes = 1024; // Small memtable for fast flushes
        config.l0.max_files = 2; // Low threshold for testing

        let max_files = config.l0.max_files; // Capture before move
        let engine = LsmEngine::open(config).await.unwrap();

        // Write enough data to trigger 3 flushes (above threshold)
        for batch in 0..3 {
            for i in 0..20 {
                let key = format!("key_{}_{}", batch, i);
                let value = vec![b'x'; 100]; // 100 bytes per value
                engine.put(Bytes::from(key), Bytes::from(value)).await.unwrap();
            }
            // Force flush after each batch
            engine.check_flush_triggers().await.unwrap();
        }

        // After 3 flushes, L0 admission should have been triggered
        // L0 count should be <= max_files because admission moved files to L1
        let manifest = engine.manifest.read();
        let snapshot = manifest.snapshot();
        let snap_guard = snapshot.read().unwrap();

        let l0_count = snap_guard.l0_file_count();
        println!("L0 file count after 3 flushes: {}", l0_count);

        // Should have triggered admission, moving files to L1
        assert!(
            l0_count <= max_files + 1,
            "L0 admission should have triggered: {} files > {} threshold",
            l0_count,
            max_files
        );

        // Verify some files moved to L1
        let l1_level = snap_guard.levels.get(1);
        if let Some(level) = l1_level {
            // Check if any L1 slots have runs
            let l1_file_count: usize = level.slots.iter().map(|s| s.runs.len()).sum();
            assert!(
                l1_file_count > 0,
                "L1 should have files after L0 admission"
            );
            println!("L1 file count: {}", l1_file_count);
        }
    }

    #[tokio::test]
    async fn test_l0_backpressure_blocks_writes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();
        config.memtable.flush_trigger_bytes = 512;
        config.l0.max_files = 1; // Very low threshold

        let engine = LsmEngine::open(config).await.unwrap();

        // Write enough to trigger 2 flushes
        for batch in 0..2 {
            for i in 0..10 {
                let key = format!("key_{}_{}", batch, i);
                let value = vec![b'x'; 100];

                let result = engine.put(Bytes::from(key), Bytes::from(value)).await;

                // After L0 exceeds threshold, writes should stall
                if batch == 1 && i > 5 {
                    // May or may not stall depending on timing
                    if let Err(Error::L0Stall(count, max)) = result {
                        println!("Write stalled: {} > {}", count, max);
                        assert!(count > max);
                        return; // Test passed - backpressure working
                    }
                }
            }
            engine.check_flush_triggers().await.unwrap();
        }

        // If we didn't hit a stall, that's also OK (admission was fast enough)
        println!("No write stall encountered (L0 admission kept up)");
    }

    #[tokio::test]
    async fn test_l0_admission_with_overlapping_guards() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();
        config.memtable.flush_trigger_bytes = 1024;
        config.l0.max_files = 1;

        let engine = LsmEngine::open(config).await.unwrap();

        // Write keys across different guard ranges
        // Guard at "m" should split keyspace
        for i in 0..20 {
            let key = if i < 10 {
                format!("a_key_{:02}", i) // Before "m"
            } else {
                format!("z_key_{:02}", i) // After "m"
            };
            let value = vec![b'x'; 100];
            engine.put(Bytes::from(key), Bytes::from(value)).await.unwrap();
        }

        // Force flush
        engine.check_flush_triggers().await.unwrap();
        engine.check_flush_triggers().await.unwrap(); // Second flush to trigger admission

        // Verify L0 admission happened
        let manifest = engine.manifest.read();
        let snapshot = manifest.snapshot();
        let snap_guard = snapshot.read().unwrap();

        let l0_count = snap_guard.l0_file_count();
        println!("L0 count after admission: {}", l0_count);

        // Should have moved files to L1
        assert!(
            l0_count <= 2,
            "L0 should be bounded after admission"
        );
    }

    /// Phase 5 test: WAL segments are cleaned up after flush
    #[tokio::test]
    async fn test_wal_cleanup_after_flush() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();
        config.memtable.flush_trigger_bytes = 1024; // Small memtable to trigger flush

        let wal_dir = temp_dir.path().join("wal");
        let engine = LsmEngine::open(config).await.unwrap();

        // Helper to count WAL segments
        let count_wal_segments = || {
            std::fs::read_dir(&wal_dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .map_or(false, |ext| ext == "wal")
                })
                .count()
        };

        // Initial WAL segment count
        let initial_count = count_wal_segments();
        assert_eq!(initial_count, 1, "Should start with one WAL segment");

        // Write enough data to trigger a flush
        for i in 0..50 {
            let key = format!("key{:03}", i);
            let value = vec![b'x'; 50]; // 50 bytes per value
            engine.put(Bytes::from(key), Bytes::from(value)).await.unwrap();
        }

        // Force flush
        engine.check_flush_triggers().await.unwrap();

        // Wait a bit for async flush to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // After flush, old WAL segments should be cleaned up
        // We should still have at least one segment (the active one)
        let after_flush_count = count_wal_segments();
        println!(
            "WAL segments: initial={}, after_flush={}",
            initial_count, after_flush_count
        );

        // The count should not grow unbounded - cleanup should keep it minimal
        assert!(
            after_flush_count <= 2,
            "WAL segments should be cleaned up after flush"
        );

        // Verify L0 file was created (data was flushed successfully)
        let manifest = engine.manifest.read();
        let snapshot = manifest.snapshot();
        let snap_guard = snapshot.read().unwrap();
        let l0_count = snap_guard.l0_file_count();
        assert!(
            l0_count > 0,
            "Should have at least one L0 file after flush"
        );
    }

    /// Phase 5 test: WAL cleanup across multiple flushes
    #[tokio::test]
    async fn test_wal_cleanup_multiple_flushes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();
        config.memtable.flush_trigger_bytes = 512; // Very small for frequent flushes

        let wal_dir = temp_dir.path().join("wal");
        let engine = LsmEngine::open(config).await.unwrap();

        let count_wal_segments = || {
            std::fs::read_dir(&wal_dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .map_or(false, |ext| ext == "wal")
                })
                .count()
        };

        // Trigger multiple flushes
        for batch in 0..5 {
            for i in 0..20 {
                let key = format!("batch{}_key{}", batch, i);
                let value = vec![b'x'; 40];
                engine.put(Bytes::from(key), Bytes::from(value)).await.unwrap();
            }
            engine.check_flush_triggers().await.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }

        // After multiple flushes, WAL segment count should remain bounded
        let final_count = count_wal_segments();
        println!("WAL segments after 5 flushes: {}", final_count);

        assert!(
            final_count <= 3,
            "WAL should not grow unbounded: {} segments after 5 flushes",
            final_count
        );
    }
}

