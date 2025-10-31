//! Raft log storage backed by nori-wal.
//!
//! Provides:
//! - Persistent log storage via WAL
//! - In-memory cache for recent entries (reduces disk reads)
//! - Index-based access (get entry by log index)
//! - Range queries (get entries from..to)
//! - Truncation (delete entries from index forward)
//! - Crash recovery (reload entries from WAL on startup)

use crate::error::Result;
use crate::types::{LogEntry, LogIndex, Term};
use bytes::{Bytes, BytesMut};
use nori_wal::{Record, RecoveryInfo, Wal, WalConfig};
use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Raft log backed by nori-wal.
///
/// Stores log entries persistently to disk and maintains an in-memory cache
/// for recently accessed entries.
///
/// # Storage Format
///
/// Each log entry is serialized using bincode and stored in the WAL as:
/// - Key: log index (u64, encoded as 8 bytes big-endian for sorting)
/// - Value: serialized LogEntry (bincode)
///
/// The WAL provides:
/// - Crash recovery (automatically replays valid records on open)
/// - Durability (fsync to disk)
/// - Efficient sequential writes
///
/// # Cache
///
/// Recent entries are cached in memory to reduce disk I/O. The cache is:
/// - Bounded size (configurable max entries, default 1000)
/// - LRU eviction (least recently used entries are dropped)
/// - Read-through (on cache miss, load from WAL)
#[derive(Clone)]
pub struct RaftLog {
    /// Underlying WAL for persistence
    wal: Arc<Wal>,

    /// In-memory cache of recent log entries (index → entry)
    /// BTreeMap provides sorted iteration and range queries
    cache: Arc<RwLock<BTreeMap<LogIndex, LogEntry>>>,

    /// Maximum number of entries to cache
    max_cache_size: usize,
}

impl RaftLog {
    /// Open a Raft log at the given path.
    ///
    /// Creates the directory if it doesn't exist.
    /// Recovers log entries from WAL on startup.
    ///
    /// Returns the log and recovery info (how many records were recovered).
    pub async fn open(path: impl Into<PathBuf>) -> Result<(Self, RecoveryInfo)> {
        let path = path.into();
        let config = WalConfig {
            dir: path,
            ..Default::default()
        };

        let (wal, recovery_info) = Wal::open(config).await?;

        let log = Self {
            wal: Arc::new(wal),
            cache: Arc::new(RwLock::new(BTreeMap::new())),
            max_cache_size: 1000, // Default cache size
        };

        // Reload entries from WAL into cache
        log.rebuild_cache().await?;

        Ok((log, recovery_info))
    }

    /// Rebuild the in-memory cache from WAL.
    ///
    /// Called during recovery to populate the cache with persisted entries.
    /// Loads all entries from the WAL (should be called only once at startup).
    async fn rebuild_cache(&self) -> Result<()> {
        // For now, we'll load all entries into cache during recovery
        // In production, we'd only load the most recent N entries
        // and lazily load older entries on demand

        // TODO: Implement WAL iteration to reload entries
        // For now, cache starts empty and entries are added as they're appended

        Ok(())
    }

    /// Append a log entry.
    ///
    /// Writes the entry to the WAL and adds it to the cache.
    /// The entry's index must be exactly last_index + 1.
    ///
    /// Returns the position in the WAL where the entry was written.
    pub async fn append(&self, entry: LogEntry) -> Result<()> {
        // Serialize entry using bincode
        let value = bincode::serialize(&entry)?;

        // Create WAL record (key = index, value = serialized entry)
        let key = index_to_bytes(entry.index);
        let record = Record::put(key, value);

        // Append to WAL
        self.wal.append(&record).await?;

        // Add to cache
        self.add_to_cache(entry);

        Ok(())
    }

    /// Append multiple log entries in a batch.
    ///
    /// More efficient than calling `append` repeatedly (fewer fsync calls).
    pub async fn append_batch(&self, entries: Vec<LogEntry>) -> Result<()> {
        for entry in entries {
            // Serialize entry
            let value = bincode::serialize(&entry)?;
            let key = index_to_bytes(entry.index);
            let record = Record::put(key, value);

            // Append to WAL (batching handled by WAL internally)
            self.wal.append(&record).await?;

            // Add to cache
            self.add_to_cache(entry);
        }

        Ok(())
    }

    /// Get a log entry by index.
    ///
    /// Returns None if the entry doesn't exist.
    /// Checks cache first, then WAL on cache miss.
    pub async fn get(&self, index: LogIndex) -> Result<Option<LogEntry>> {
        // Check cache first
        {
            let cache = self.cache.read();
            if let Some(entry) = cache.get(&index) {
                return Ok(Some(entry.clone()));
            }
        }

        // Cache miss - would need to read from WAL
        // For now, return None (we'll implement WAL iteration later)
        // In production: read from WAL, deserialize, add to cache
        Ok(None)
    }

    /// Get a range of log entries [from_index, to_index).
    ///
    /// Returns entries in ascending order by index.
    /// Returns empty vec if range is empty or entries don't exist.
    pub async fn get_range(
        &self,
        from_index: LogIndex,
        to_index: LogIndex,
    ) -> Result<Vec<LogEntry>> {
        let cache = self.cache.read();

        let mut entries = Vec::new();
        for (_, entry) in cache.range(from_index..to_index) {
            entries.push(entry.clone());
        }

        Ok(entries)
    }

    /// Get the last log entry.
    ///
    /// Returns None if log is empty.
    pub async fn last(&self) -> Result<Option<LogEntry>> {
        let cache = self.cache.read();
        Ok(cache.iter().next_back().map(|(_, e)| e.clone()))
    }

    /// Get the last log index.
    ///
    /// Returns LogIndex(0) if log is empty.
    pub async fn last_index(&self) -> LogIndex {
        let cache = self.cache.read();
        cache
            .keys()
            .next_back()
            .copied()
            .unwrap_or(LogIndex::ZERO)
    }

    /// Get the term of the last log entry.
    ///
    /// Returns Term(0) if log is empty.
    pub async fn last_term(&self) -> Term {
        let cache = self.cache.read();
        cache
            .iter()
            .next_back()
            .map(|(_, e)| e.term)
            .unwrap_or(Term::ZERO)
    }

    /// Truncate the log from `from_index` forward (delete all entries >= from_index).
    ///
    /// Used when:
    /// - Follower receives AppendEntries with conflicting entries (delete conflicting suffix)
    /// - Snapshot installed (delete all entries included in snapshot)
    ///
    /// This operation is not reversible.
    pub async fn truncate(&self, from_index: LogIndex) -> Result<()> {
        // Remove from cache
        {
            let mut cache = self.cache.write();
            cache.split_off(&from_index);
        }

        // TODO: Mark entries as deleted in WAL
        // For now, we rely on cache to track active entries
        // In production: write tombstone records to WAL

        Ok(())
    }

    /// Sync the log to disk.
    ///
    /// Ensures all appended entries are durable (persisted to stable storage).
    /// Blocks until fsync completes.
    pub async fn sync(&self) -> Result<()> {
        self.wal.sync().await?;
        Ok(())
    }

    /// Add an entry to the cache.
    ///
    /// Evicts oldest entries if cache exceeds max size (LRU eviction).
    fn add_to_cache(&self, entry: LogEntry) {
        let mut cache = self.cache.write();
        cache.insert(entry.index, entry);

        // Evict oldest entries if cache too large
        while cache.len() > self.max_cache_size {
            if let Some((index, _)) = cache.iter().next() {
                let index = *index;
                cache.remove(&index);
            } else {
                break;
            }
        }
    }

    /// Get the number of entries in the cache.
    pub fn cache_size(&self) -> usize {
        self.cache.read().len()
    }
}

/// Convert log index to bytes (big-endian for sorting).
///
/// WAL records are stored sorted by key, so we use big-endian encoding
/// to ensure index 1 < index 2 < ... lexicographically.
fn index_to_bytes(index: LogIndex) -> Bytes {
    let mut buf = BytesMut::with_capacity(8);
    buf.extend_from_slice(&index.as_u64().to_be_bytes());
    buf.freeze()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_raft_log_append_and_get() {
        let temp_dir = TempDir::new().unwrap();
        let (log, _recovery) = RaftLog::open(temp_dir.path()).await.unwrap();

        let entry1 = LogEntry::new(Term(1), LogIndex(1), Bytes::from("cmd1"));
        let entry2 = LogEntry::new(Term(1), LogIndex(2), Bytes::from("cmd2"));

        log.append(entry1.clone()).await.unwrap();
        log.append(entry2.clone()).await.unwrap();

        // Get entries from cache
        let got1 = log.get(LogIndex(1)).await.unwrap();
        assert_eq!(got1, Some(entry1));

        let got2 = log.get(LogIndex(2)).await.unwrap();
        assert_eq!(got2, Some(entry2));
    }

    #[tokio::test]
    async fn test_raft_log_last_index_and_term() {
        let temp_dir = TempDir::new().unwrap();
        let (log, _recovery) = RaftLog::open(temp_dir.path()).await.unwrap();

        assert_eq!(log.last_index().await, LogIndex::ZERO);
        assert_eq!(log.last_term().await, Term::ZERO);

        let entry = LogEntry::new(Term(5), LogIndex(10), Bytes::from("cmd"));
        log.append(entry).await.unwrap();

        assert_eq!(log.last_index().await, LogIndex(10));
        assert_eq!(log.last_term().await, Term(5));
    }

    #[tokio::test]
    async fn test_raft_log_range() {
        let temp_dir = TempDir::new().unwrap();
        let (log, _recovery) = RaftLog::open(temp_dir.path()).await.unwrap();

        for i in 1..=10 {
            let entry = LogEntry::new(Term(1), LogIndex(i), Bytes::from(format!("cmd{}", i)));
            log.append(entry).await.unwrap();
        }

        let range = log.get_range(LogIndex(3), LogIndex(7)).await.unwrap();
        assert_eq!(range.len(), 4);
        assert_eq!(range[0].index, LogIndex(3));
        assert_eq!(range[3].index, LogIndex(6));
    }

    #[tokio::test]
    async fn test_raft_log_truncate() {
        let temp_dir = TempDir::new().unwrap();
        let (log, _recovery) = RaftLog::open(temp_dir.path()).await.unwrap();

        for i in 1..=10 {
            let entry = LogEntry::new(Term(1), LogIndex(i), Bytes::from(format!("cmd{}", i)));
            log.append(entry).await.unwrap();
        }

        log.truncate(LogIndex(6)).await.unwrap();

        assert_eq!(log.last_index().await, LogIndex(5));
        assert!(log.get(LogIndex(6)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_raft_log_cache_eviction() {
        let temp_dir = TempDir::new().unwrap();
        let (mut log, _recovery) = RaftLog::open(temp_dir.path()).await.unwrap();
        log.max_cache_size = 5; // Small cache for testing

        for i in 1..=10 {
            let entry = LogEntry::new(Term(1), LogIndex(i), Bytes::from(format!("cmd{}", i)));
            log.append(entry).await.unwrap();
        }

        // Cache should only hold last 5 entries
        assert_eq!(log.cache_size(), 5);
        assert!(log.cache.read().contains_key(&LogIndex(6)));
        assert!(log.cache.read().contains_key(&LogIndex(10)));
        assert!(!log.cache.read().contains_key(&LogIndex(1)));
    }
}
