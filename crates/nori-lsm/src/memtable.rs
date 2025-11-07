use crate::error::Result;
use bytes::Bytes;
use crossbeam_skiplist::SkipMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Entry in the memtable representing a single key-value operation.
/// Each entry is tagged with a sequence number for MVCC ordering.
#[derive(Debug, Clone)]
pub enum MemtableEntry {
    /// A put operation: key → value
    Put {
        value: Bytes,
        seqno: u64,
        /// Optional expiration time (absolute timestamp)
        /// If present, this entry expires after this time
        expires_at: Option<SystemTime>,
    },
    /// A delete operation (tombstone)
    Delete { seqno: u64 },
}

/// In-memory write buffer backed by a concurrent skiplist.
///
/// The memtable stores all recent writes with sequence numbers for ordering.
/// When the memtable reaches the size threshold, it's frozen and flushed to L0.
///
/// # Concurrency
/// Uses `crossbeam_skiplist::SkipMap` for lock-free concurrent reads and writes.
/// Multiple readers and one writer can operate simultaneously.
///
/// # Memory Layout
/// ```text
/// SkipMap<Bytes, MemtableEntry>
///   key1 → Put { value: v1, seqno: 100 }
///   key2 → Delete { seqno: 101 }
///   key3 → Put { value: v3, seqno: 102 }
/// ```
pub struct Memtable {
    /// Concurrent skiplist storing entries
    data: Arc<SkipMap<Bytes, MemtableEntry>>,

    /// Approximate size in bytes (keys + values + overhead)
    /// Updated on each write; not exact due to lock-free design
    approx_size: Arc<std::sync::atomic::AtomicUsize>,

    /// Minimum sequence number in this memtable
    min_seqno: u64,

    /// Maximum sequence number in this memtable
    max_seqno: Arc<std::sync::atomic::AtomicU64>,
}

impl Memtable {
    /// Creates a new empty memtable starting at the given sequence number.
    pub fn new(min_seqno: u64) -> Self {
        Self {
            data: Arc::new(SkipMap::new()),
            approx_size: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            min_seqno,
            max_seqno: Arc::new(std::sync::atomic::AtomicU64::new(min_seqno)),
        }
    }

    /// Inserts a key-value pair with the given sequence number.
    ///
    /// # Arguments
    /// * `key` - The key to insert
    /// * `value` - The value to associate with the key
    /// * `seqno` - Sequence number for this operation (must be monotonically increasing)
    /// * `ttl` - Optional time-to-live duration; entry expires after this duration
    ///
    /// # Returns
    /// The approximate size of the memtable after insertion
    pub fn put(&self, key: Bytes, value: Bytes, seqno: u64, ttl: Option<Duration>) -> Result<usize> {
        let entry_size = key.len() + value.len() + std::mem::size_of::<MemtableEntry>();

        // Calculate expiration time if TTL is provided
        let expires_at = ttl.map(|duration| SystemTime::now() + duration);

        let entry = MemtableEntry::Put {
            value: value.clone(),
            seqno,
            expires_at,
        };

        self.data.insert(key, entry);

        // Update sequence number
        self.max_seqno
            .fetch_max(seqno, std::sync::atomic::Ordering::Release);

        // Update approximate size
        let new_size = self
            .approx_size
            .fetch_add(entry_size, std::sync::atomic::Ordering::Relaxed)
            + entry_size;

        Ok(new_size)
    }

    /// Deletes a key by inserting a tombstone.
    ///
    /// # Arguments
    /// * `key` - The key to delete
    /// * `seqno` - Sequence number for this operation
    ///
    /// # Returns
    /// The approximate size of the memtable after deletion
    pub fn delete(&self, key: Bytes, seqno: u64) -> Result<usize> {
        let entry_size = key.len() + std::mem::size_of::<MemtableEntry>();

        let entry = MemtableEntry::Delete { seqno };

        self.data.insert(key, entry);

        // Update sequence number
        self.max_seqno
            .fetch_max(seqno, std::sync::atomic::Ordering::Release);

        // Update approximate size
        let new_size = self
            .approx_size
            .fetch_add(entry_size, std::sync::atomic::Ordering::Relaxed)
            + entry_size;

        Ok(new_size)
    }

    /// Retrieves the value for a key, if present.
    ///
    /// Returns the most recent entry for the key. The caller is responsible
    /// for interpreting tombstones.
    ///
    /// # TTL Handling
    /// If the entry has expired (TTL), returns None as if the key doesn't exist.
    pub fn get(&self, key: &[u8]) -> Option<MemtableEntry> {
        self.data.get(key).and_then(|entry| {
            let entry = entry.value().clone();

            // Check if entry has expired
            match &entry {
                MemtableEntry::Put { expires_at, .. } => {
                    if let Some(expiration) = expires_at {
                        if SystemTime::now() >= *expiration {
                            // Entry has expired, treat as not found
                            return None;
                        }
                    }
                    Some(entry)
                }
                MemtableEntry::Delete { .. } => Some(entry),
            }
        })
    }

    /// Returns the approximate size of this memtable in bytes.
    pub fn size(&self) -> usize {
        self.approx_size.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Returns the minimum sequence number in this memtable.
    pub fn min_seqno(&self) -> u64 {
        self.min_seqno
    }

    /// Returns the maximum sequence number in this memtable.
    pub fn max_seqno(&self) -> u64 {
        self.max_seqno.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Returns an iterator over all entries in sorted key order.
    ///
    /// This is used during flush to write entries to an SSTable.
    pub fn iter(&self) -> MemtableIterator<'_> {
        MemtableIterator {
            inner: self.data.iter(),
        }
    }

    /// Returns entries in a key range [start, end).
    ///
    /// Used for range scans in the read path.
    pub fn range(&self, start: &[u8], end: &[u8]) -> Vec<(Bytes, MemtableEntry)> {
        let start_key = Bytes::copy_from_slice(start);
        let end_key = Bytes::copy_from_slice(end);

        self.data
            .iter()
            .skip_while(|entry| entry.key() < &start_key)
            .take_while(|entry| entry.key() < &end_key)
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Returns true if the memtable is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns the number of entries in the memtable.
    pub fn len(&self) -> usize {
        self.data.len()
    }
}

/// Iterator over memtable entries in sorted key order.
pub struct MemtableIterator<'a> {
    inner: crossbeam_skiplist::map::Iter<'a, Bytes, MemtableEntry>,
}

impl<'a> Iterator for MemtableIterator<'a> {
    type Item = (Bytes, MemtableEntry);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|entry| {
            let key = entry.key().clone();
            let value = entry.value().clone();
            (key, value)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memtable_put_get() {
        let mt = Memtable::new(0);

        let key = Bytes::from("key1");
        let value = Bytes::from("value1");

        mt.put(key.clone(), value.clone(), 1, None).unwrap();

        match mt.get(&key) {
            Some(MemtableEntry::Put { value: v, seqno, expires_at }) => {
                assert_eq!(v, value);
                assert_eq!(seqno, 1);
                assert_eq!(expires_at, None);
            }
            _ => panic!("Expected Put entry"),
        }
    }

    #[test]
    fn test_memtable_delete() {
        let mt = Memtable::new(0);

        let key = Bytes::from("key1");
        mt.delete(key.clone(), 1).unwrap();

        match mt.get(&key) {
            Some(MemtableEntry::Delete { seqno }) => {
                assert_eq!(seqno, 1);
            }
            _ => panic!("Expected Delete entry"),
        }
    }

    #[test]
    fn test_memtable_overwrite() {
        let mt = Memtable::new(0);

        let key = Bytes::from("key1");
        mt.put(key.clone(), Bytes::from("value1"), 1, None).unwrap();
        mt.put(key.clone(), Bytes::from("value2"), 2, None).unwrap();

        // Should get the most recent write (seqno 2)
        match mt.get(&key) {
            Some(MemtableEntry::Put { value: v, seqno, .. }) => {
                assert_eq!(v, Bytes::from("value2"));
                assert_eq!(seqno, 2);
            }
            _ => panic!("Expected Put entry"),
        }
    }

    #[test]
    fn test_memtable_size() {
        let mt = Memtable::new(0);

        let initial_size = mt.size();
        assert_eq!(initial_size, 0);

        let key = Bytes::from("key1");
        let value = Bytes::from("value1");

        let size_after_put = mt.put(key.clone(), value.clone(), 1, None).unwrap();
        assert!(size_after_put > 0);
        assert_eq!(mt.size(), size_after_put);
    }

    #[test]
    fn test_memtable_seqno_tracking() {
        let mt = Memtable::new(100);

        assert_eq!(mt.min_seqno(), 100);
        assert_eq!(mt.max_seqno(), 100);

        mt.put(Bytes::from("key1"), Bytes::from("value1"), 101, None)
            .unwrap();
        assert_eq!(mt.max_seqno(), 101);

        mt.put(Bytes::from("key2"), Bytes::from("value2"), 105, None)
            .unwrap();
        assert_eq!(mt.max_seqno(), 105);

        assert_eq!(mt.min_seqno(), 100);
    }

    #[test]
    fn test_memtable_iterator() {
        let mt = Memtable::new(0);

        mt.put(Bytes::from("c"), Bytes::from("value_c"), 3, None).unwrap();
        mt.put(Bytes::from("a"), Bytes::from("value_a"), 1, None).unwrap();
        mt.put(Bytes::from("b"), Bytes::from("value_b"), 2, None).unwrap();

        let entries: Vec<_> = mt.iter().collect();

        // Should be sorted by key
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].0, Bytes::from("a"));
        assert_eq!(entries[1].0, Bytes::from("b"));
        assert_eq!(entries[2].0, Bytes::from("c"));
    }

    #[test]
    fn test_memtable_empty() {
        let mt = Memtable::new(0);
        assert!(mt.is_empty());
        assert_eq!(mt.len(), 0);

        mt.put(Bytes::from("key"), Bytes::from("value"), 1, None).unwrap();
        assert!(!mt.is_empty());
        assert_eq!(mt.len(), 1);
    }
}
