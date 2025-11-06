//! Two-level index for fast block lookups in SSTables.
//!
//! The index maps key ranges to block offsets, enabling efficient binary search
//! to locate which block contains a given key without scanning the entire file.
//!
//! # Index Structure
//!
//! ```text
//! Top-level index (stored in SSTable):
//!   Block 0: first_key="apple"  → offset=0,    size=4096
//!   Block 1: first_key="cherry" → offset=4096, size=4096
//!   Block 2: first_key="fig"    → offset=8192, size=3000
//! ```
//!
//! To find key "banana":
//! 1. Binary search: "banana" is between "apple" and "cherry"
//! 2. Read Block 0 at offset 0
//! 3. Search within Block 0

use crate::error::{Result, SSTableError};
use bytes::{Buf, BufMut, Bytes, BytesMut};

/// Entry in the top-level index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    /// First key in the block (restart point key).
    pub first_key: Bytes,
    /// Byte offset of block in SSTable file.
    pub block_offset: u64,
    /// Size of block in bytes.
    pub block_size: u32,
}

/// Top-level index for an SSTable.
#[derive(Debug, Clone)]
pub struct Index {
    entries: Vec<IndexEntry>,
}

impl Index {
    /// Creates a new empty index.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Adds a block to the index.
    ///
    /// Blocks must be added in sorted order by first_key.
    pub fn add_block(&mut self, first_key: Bytes, block_offset: u64, block_size: u32) {
        // Validate sorted order
        if let Some(last) = self.entries.last() {
            if first_key <= last.first_key {
                panic!("Index entries must be added in sorted order");
            }
        }

        self.entries.push(IndexEntry {
            first_key,
            block_offset,
            block_size,
        });
    }

    /// Finds the index of the block that might contain the given key.
    ///
    /// Returns `None` if the key is definitely before the first block or after the last block.
    /// Returns `Some(index)` if the key might be in block at that index.
    pub fn find_block(&self, key: &[u8]) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }

        // Binary search to find the rightmost block where first_key <= key
        let mut left = 0;
        let mut right = self.entries.len();

        while left < right {
            let mid = (left + right) / 2;

            match key.cmp(&self.entries[mid].first_key) {
                std::cmp::Ordering::Less => {
                    // key < entries[mid].first_key
                    // Key might be in a previous block
                    right = mid;
                }
                std::cmp::Ordering::Equal => {
                    // Exact match - key is in this block
                    return Some(mid);
                }
                std::cmp::Ordering::Greater => {
                    // key > entries[mid].first_key
                    // Key might be in this block or a later one
                    left = mid + 1;
                }
            }
        }

        // If left == 0, key is before the first block
        if left == 0 {
            return None;
        }

        // Key might be in block at index (left - 1)
        Some(left - 1)
    }

    /// Returns the number of blocks in the index.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns an iterator over index entries.
    pub fn iter(&self) -> impl Iterator<Item = &IndexEntry> {
        self.entries.iter()
    }

    /// Gets an entry by index.
    pub fn get(&self, index: usize) -> Option<&IndexEntry> {
        self.entries.get(index)
    }

    /// Encodes the index to bytes for storage.
    ///
    /// Format:
    /// ```text
    /// [Entry 1]
    ///   - first_key_len: varint
    ///   - first_key: bytes
    ///   - block_offset: u64 (little-endian)
    ///   - block_size: u32 (little-endian)
    /// [Entry 2]
    /// ...
    /// [Entry count: u32 (little-endian)]
    /// ```
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::new();

        for entry in &self.entries {
            // Encode first_key length and data
            encode_varint(&mut buf, entry.first_key.len() as u64);
            buf.put_slice(&entry.first_key);

            // Encode offset and size
            buf.put_u64_le(entry.block_offset);
            buf.put_u32_le(entry.block_size);
        }

        // Append entry count
        buf.put_u32_le(self.entries.len() as u32);

        buf.freeze()
    }

    /// Decodes an index from bytes.
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() < 4 {
            return Err(SSTableError::Incomplete);
        }

        // Read entry count from end
        let entry_count = u32::from_le_bytes([
            data[data.len() - 4],
            data[data.len() - 3],
            data[data.len() - 2],
            data[data.len() - 1],
        ]) as usize;

        let mut entries = Vec::with_capacity(entry_count);
        let mut cursor = &data[..data.len() - 4];

        for _ in 0..entry_count {
            // Decode first_key length
            let key_len = decode_varint(&mut cursor)? as usize;

            if cursor.len() < key_len + 12 {
                return Err(SSTableError::Incomplete);
            }

            // Decode first_key
            let first_key = Bytes::copy_from_slice(&cursor[..key_len]);
            cursor.advance(key_len);

            // Decode offset and size
            let block_offset = u64::from_le_bytes([
                cursor[0], cursor[1], cursor[2], cursor[3], cursor[4], cursor[5], cursor[6],
                cursor[7],
            ]);
            cursor.advance(8);

            let block_size = u32::from_le_bytes([cursor[0], cursor[1], cursor[2], cursor[3]]);
            cursor.advance(4);

            entries.push(IndexEntry {
                first_key,
                block_offset,
                block_size,
            });
        }

        Ok(Self { entries })
    }

    /// Returns the size of the encoded index in bytes.
    pub fn size(&self) -> usize {
        let mut size = 4; // Entry count

        for entry in &self.entries {
            size += varint_size(entry.first_key.len() as u64);
            size += entry.first_key.len();
            size += 8; // block_offset
            size += 4; // block_size
        }

        size
    }
}

impl Default for Index {
    fn default() -> Self {
        Self::new()
    }
}

/// Encodes a u64 as a varint.
fn encode_varint(buf: &mut BytesMut, mut value: u64) {
    while value >= 0x80 {
        buf.put_u8((value as u8) | 0x80);
        value >>= 7;
    }
    buf.put_u8(value as u8);
}

/// Decodes a varint from a buffer.
fn decode_varint(buf: &mut &[u8]) -> Result<u64> {
    let mut value = 0u64;
    let mut shift = 0;

    loop {
        if shift >= 64 {
            return Err(SSTableError::InvalidFormat("varint overflow".to_string()));
        }
        if buf.is_empty() {
            return Err(SSTableError::Incomplete);
        }

        let byte = buf[0];
        buf.advance(1);
        value |= ((byte & 0x7F) as u64) << shift;

        if byte & 0x80 == 0 {
            return Ok(value);
        }

        shift += 7;
    }
}

/// Returns the size of a varint-encoded value in bytes.
fn varint_size(value: u64) -> usize {
    if value == 0 {
        1
    } else {
        ((63 - value.leading_zeros()) / 7 + 1) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_basic() {
        let mut index = Index::new();

        index.add_block(Bytes::from("apple"), 0, 4096);
        index.add_block(Bytes::from("cherry"), 4096, 4096);
        index.add_block(Bytes::from("fig"), 8192, 3000);

        assert_eq!(index.len(), 3);
        assert!(!index.is_empty());
    }

    #[test]
    fn test_index_find_block() {
        let mut index = Index::new();

        index.add_block(Bytes::from("apple"), 0, 4096);
        index.add_block(Bytes::from("cherry"), 4096, 4096);
        index.add_block(Bytes::from("fig"), 8192, 3000);

        // Exact matches
        assert_eq!(index.find_block(b"apple"), Some(0));
        assert_eq!(index.find_block(b"cherry"), Some(1));
        assert_eq!(index.find_block(b"fig"), Some(2));

        // Keys between blocks
        assert_eq!(index.find_block(b"banana"), Some(0)); // Between apple and cherry
        assert_eq!(index.find_block(b"date"), Some(1)); // Between cherry and fig
        assert_eq!(index.find_block(b"grape"), Some(2)); // After fig

        // Key before first block
        assert_eq!(index.find_block(b"aaa"), None);
    }

    #[test]
    fn test_index_empty() {
        let index = Index::new();

        assert_eq!(index.len(), 0);
        assert!(index.is_empty());
        assert_eq!(index.find_block(b"any_key"), None);
    }

    #[test]
    fn test_index_single_block() {
        let mut index = Index::new();
        index.add_block(Bytes::from("key"), 0, 4096);

        assert_eq!(index.find_block(b"key"), Some(0));
        assert_eq!(index.find_block(b"zzz"), Some(0)); // After first key
        assert_eq!(index.find_block(b"aaa"), None); // Before first key
    }

    #[test]
    fn test_index_encode_decode() {
        let mut index = Index::new();

        index.add_block(Bytes::from("apple"), 0, 4096);
        index.add_block(Bytes::from("banana"), 4096, 4096);
        index.add_block(Bytes::from("cherry"), 8192, 4096);

        // Encode
        let encoded = index.encode();

        // Decode
        let decoded = Index::decode(&encoded).unwrap();

        // Verify structure
        assert_eq!(decoded.len(), 3);

        let entries: Vec<&IndexEntry> = decoded.iter().collect();
        assert_eq!(entries[0].first_key, Bytes::from("apple"));
        assert_eq!(entries[0].block_offset, 0);
        assert_eq!(entries[0].block_size, 4096);

        assert_eq!(entries[1].first_key, Bytes::from("banana"));
        assert_eq!(entries[1].block_offset, 4096);

        assert_eq!(entries[2].first_key, Bytes::from("cherry"));
        assert_eq!(entries[2].block_offset, 8192);

        // Verify find_block still works
        assert_eq!(decoded.find_block(b"apple"), Some(0));
        assert_eq!(decoded.find_block(b"banana"), Some(1));
        assert_eq!(decoded.find_block(b"apricot"), Some(0));
    }

    #[test]
    fn test_index_large() {
        let mut index = Index::new();

        // Add 1000 blocks
        for i in 0..1000 {
            let key = format!("key{:04}", i);
            index.add_block(Bytes::from(key), i * 4096, 4096);
        }

        assert_eq!(index.len(), 1000);

        // Test binary search
        assert_eq!(index.find_block(b"key0000"), Some(0));
        assert_eq!(index.find_block(b"key0500"), Some(500));
        assert_eq!(index.find_block(b"key0999"), Some(999));

        // Test key between entries
        assert_eq!(index.find_block(b"key0250"), Some(250));

        // Encode/decode
        let encoded = index.encode();
        let decoded = Index::decode(&encoded).unwrap();
        assert_eq!(decoded.len(), 1000);
        assert_eq!(decoded.find_block(b"key0500"), Some(500));
    }

    #[test]
    fn test_index_decode_incomplete() {
        let data = vec![0u8; 2]; // Too short
        let result = Index::decode(&data);
        assert!(matches!(result, Err(SSTableError::Incomplete)));
    }

    #[test]
    #[should_panic(expected = "sorted order")]
    fn test_index_not_sorted() {
        let mut index = Index::new();
        index.add_block(Bytes::from("banana"), 0, 4096);
        index.add_block(Bytes::from("apple"), 4096, 4096); // Out of order!
    }

    #[test]
    fn test_index_get() {
        let mut index = Index::new();
        index.add_block(Bytes::from("apple"), 0, 4096);
        index.add_block(Bytes::from("banana"), 4096, 4096);

        let entry = index.get(0).unwrap();
        assert_eq!(entry.first_key, Bytes::from("apple"));

        let entry = index.get(1).unwrap();
        assert_eq!(entry.first_key, Bytes::from("banana"));

        assert!(index.get(2).is_none());
    }

    #[test]
    fn test_index_boundary_keys() {
        let mut index = Index::new();
        index.add_block(Bytes::from("b"), 0, 4096);
        index.add_block(Bytes::from("d"), 4096, 4096);
        index.add_block(Bytes::from("f"), 8192, 4096);

        // Test exact boundaries
        assert_eq!(index.find_block(b"b"), Some(0));
        assert_eq!(index.find_block(b"d"), Some(1));
        assert_eq!(index.find_block(b"f"), Some(2));

        // Test just before boundaries
        assert_eq!(index.find_block(b"a"), None);
        assert_eq!(index.find_block(b"c"), Some(0));
        assert_eq!(index.find_block(b"e"), Some(1));

        // Test just after boundaries
        assert_eq!(index.find_block(b"b\x00"), Some(0));
        assert_eq!(index.find_block(b"d\x00"), Some(1));
        assert_eq!(index.find_block(b"f\x00"), Some(2));
    }
}
