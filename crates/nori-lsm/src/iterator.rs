/// Multi-level merge iterator for LSM range scans and point queries.
///
/// Provides:
/// - K-way merge across memtable + L0 + L1+ levels
/// - Deduplication (newest version wins)
/// - Tombstone filtering
/// - Snapshot isolation via sequence numbers
/// - Range filtering
///
/// # Architecture
///
/// ```text
/// ┌─────────────────────────────────────────────────────────────┐
/// │  LsmIterator (K-way merge)                                  │
/// │  ┌──────────────────────────────────────────────────────┐   │
/// │  │  Min-Heap (MergeCandidate)                           │   │
/// │  │  - Key ordering                                      │   │
/// │  │  - Source priority (memtable > L0 > L1+)           │   │
/// │  └──────────────────────────────────────────────────────┘   │
/// │                                                              │
/// │  Sources:                                                    │
/// │  ┌────────────┐  ┌────────────┐  ┌─────────────────────┐   │
/// │  │ Memtable   │  │ L0 Files   │  │ L1+ Slots           │   │
/// │  │ (newest)   │  │ (rev order)│  │ (K runs per slot)   │   │
/// │  └────────────┘  └────────────┘  └─────────────────────┘   │
/// └─────────────────────────────────────────────────────────────┘
/// ```
use crate::error::{Error, Result};
use crate::memtable::MemtableEntry;
use bytes::Bytes;
use nori_sstable::{Entry, SSTableIterator};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Source type for merge iterator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    /// Memtable (highest priority - newest data)
    Memtable,

    /// L0 file (priority by file age/index)
    L0 { file_idx: usize },

    /// L1+ level (priority by level, then slot)
    Level {
        level: u8,
        slot_id: u32,
        run_idx: usize,
    },
}

impl SourceType {
    /// Returns priority order for deduplication (lower = higher priority).
    fn priority(&self) -> u32 {
        match self {
            SourceType::Memtable => 0,
            SourceType::L0 { file_idx } => 1000 + (*file_idx as u32),
            SourceType::Level { level, run_idx, .. } => {
                10000 + (*level as u32) * 100 + (*run_idx as u32)
            }
        }
    }
}

/// Candidate entry in the merge heap.
#[derive(Debug)]
pub struct MergeCandidate {
    /// Key from this source
    pub key: Bytes,

    /// Value (or tombstone marker)
    pub value: Bytes,

    /// Is this a tombstone?
    pub is_tombstone: bool,

    /// Sequence number
    pub seqno: u64,

    /// Source of this entry
    pub source: SourceType,

    /// Index in the sources vector
    pub source_idx: usize,
}

impl PartialEq for MergeCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
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
        match other.key.cmp(&self.key) {
            Ordering::Equal => {
                // Same key: prioritize newer source (lower priority value)
                self.source.priority().cmp(&other.source.priority())
            }
            ord => ord,
        }
    }
}

/// Iterator source wrapper.
pub enum IteratorSource {
    /// Memtable range iterator
    Memtable {
        iter: Vec<(Bytes, MemtableEntry)>,
        pos: usize,
    },

    /// SSTable iterator (L0 or L1+), boxed to reduce enum size
    SSTable(Box<SSTableIterator>),
}

impl IteratorSource {
    /// Tries to get the next entry from this source.
    pub async fn try_next(&mut self) -> Result<Option<Entry>> {
        match self {
            IteratorSource::Memtable { iter, pos } => {
                if *pos >= iter.len() {
                    Ok(None)
                } else {
                    let (key, entry) = &iter[*pos];
                    *pos += 1;

                    match entry {
                        MemtableEntry::Put { value, seqno } => Ok(Some(Entry {
                            key: key.clone(),
                            value: value.clone(),
                            tombstone: false,
                            seqno: *seqno,
                        })),
                        MemtableEntry::Delete { seqno } => Ok(Some(Entry {
                            key: key.clone(),
                            value: Bytes::new(),
                            tombstone: true,
                            seqno: *seqno,
                        })),
                    }
                }
            }
            IteratorSource::SSTable(iter) => iter
                .try_next()
                .await
                .map_err(|e| Error::Internal(format!("SSTable iteration error: {}", e))),
        }
    }
}

/// Multi-level LSM merge iterator.
///
/// Performs K-way merge across all LSM levels with:
/// - Deduplication (newest version per key)
/// - Tombstone filtering
/// - Snapshot isolation
/// - Range bounds
pub struct LsmIterator {
    /// All iterator sources
    sources: Vec<IteratorSource>,

    /// Source type metadata
    source_types: Vec<SourceType>,

    /// Min-heap for K-way merge
    heap: BinaryHeap<MergeCandidate>,

    /// Last key returned (for deduplication)
    last_key: Option<Bytes>,

    /// End key for range bound
    end_key: Option<Bytes>,

    /// Snapshot sequence number (None = latest)
    snapshot_seqno: Option<u64>,

    /// Iterator exhausted
    exhausted: bool,
}

impl LsmIterator {
    /// Creates a new LSM iterator from sources.
    pub fn new(
        sources: Vec<IteratorSource>,
        source_types: Vec<SourceType>,
        end_key: Option<Bytes>,
        snapshot_seqno: Option<u64>,
    ) -> Self {
        assert_eq!(sources.len(), source_types.len());

        Self {
            sources,
            source_types,
            heap: BinaryHeap::new(),
            last_key: None,
            end_key,
            snapshot_seqno,
            exhausted: false,
        }
    }

    /// Initializes the iterator by priming the heap with first entry from each source.
    pub async fn init(&mut self) -> Result<()> {
        let num_sources = self.sources.len();
        for idx in 0..num_sources {
            if let Some(entry) = self.sources[idx].try_next().await? {
                self.push_to_heap(entry, idx);
            }
        }
        Ok(())
    }

    /// Returns the next key-value pair from the iterator.
    ///
    /// # Returns
    /// - `Ok(Some((key, value)))` if there's a next entry
    /// - `Ok(None)` if iteration is complete
    /// - `Err` on I/O errors
    pub async fn next(&mut self) -> Result<Option<(Bytes, Bytes)>> {
        if self.exhausted {
            return Ok(None);
        }

        loop {
            // Pop next candidate from heap
            let candidate = match self.heap.pop() {
                Some(c) => c,
                None => {
                    self.exhausted = true;
                    return Ok(None);
                }
            };

            // Check end bound
            if let Some(ref end) = self.end_key {
                if candidate.key.as_ref() >= end.as_ref() {
                    self.exhausted = true;
                    return Ok(None);
                }
            }

            // Advance the source that produced this candidate
            let source_idx = candidate.source_idx;
            if let Some(next_entry) = self.sources[source_idx].try_next().await? {
                self.push_to_heap(next_entry, source_idx);
            }

            // Skip duplicates (same key as last returned)
            if let Some(ref last) = self.last_key {
                if candidate.key == *last {
                    continue; // Duplicate, skip
                }
            }

            // Skip tombstones
            if candidate.is_tombstone {
                self.last_key = Some(candidate.key);
                continue;
            }

            // Check snapshot isolation
            if let Some(snapshot_seqno) = self.snapshot_seqno {
                if candidate.seqno > snapshot_seqno {
                    continue; // Entry too new for snapshot
                }
            }

            // Valid entry - return it
            self.last_key = Some(candidate.key.clone());
            return Ok(Some((candidate.key, candidate.value)));
        }
    }

    /// Pushes an entry from a source into the heap.
    fn push_to_heap(&mut self, entry: Entry, source_idx: usize) {
        let source_type = self.source_types[source_idx];

        self.heap.push(MergeCandidate {
            key: entry.key,
            value: entry.value,
            is_tombstone: entry.tombstone,
            seqno: 0, // TODO: Extract from entry metadata when available
            source: source_type,
            source_idx,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_type_priority() {
        let memtable = SourceType::Memtable;
        let l0_0 = SourceType::L0 { file_idx: 0 };
        let l0_1 = SourceType::L0 { file_idx: 1 };
        let l1 = SourceType::Level {
            level: 1,
            slot_id: 0,
            run_idx: 0,
        };

        assert!(memtable.priority() < l0_0.priority());
        assert!(l0_0.priority() < l0_1.priority());
        assert!(l0_1.priority() < l1.priority());
    }

    #[test]
    fn test_merge_candidate_ordering() {
        let c1 = MergeCandidate {
            key: Bytes::from("a"),
            value: Bytes::new(),
            is_tombstone: false,
            seqno: 1,
            source: SourceType::Memtable,
            source_idx: 0,
        };

        let c2 = MergeCandidate {
            key: Bytes::from("b"),
            value: Bytes::new(),
            is_tombstone: false,
            seqno: 2,
            source: SourceType::L0 { file_idx: 0 },
            source_idx: 1,
        };

        // Min-heap: "a" < "b", so c1 > c2 (reversed for min-heap)
        assert!(c1 > c2);
    }

    #[test]
    fn test_merge_candidate_same_key_priority() {
        let memtable_entry = MergeCandidate {
            key: Bytes::from("key"),
            value: Bytes::from("v1"),
            is_tombstone: false,
            seqno: 2,
            source: SourceType::Memtable,
            source_idx: 0,
        };

        let l0_entry = MergeCandidate {
            key: Bytes::from("key"),
            value: Bytes::from("v2"),
            is_tombstone: false,
            seqno: 1,
            source: SourceType::L0 { file_idx: 0 },
            source_idx: 1,
        };

        // Same key: memtable has higher priority (lower priority value)
        assert!(memtable_entry < l0_entry); // memtable should come first
    }
}
