//! Block encoding/decoding with restart points for prefix compression.
//!
//! Block structure:
//! ```text
//! [Entries with prefix compression]
//! [Restart points array: u32...]
//! [Restart count: u32]
//! ```
//!
//! Prefix compression:
//! Every `restart_interval` entries, we store the full key. Between restart points,
//! we store only the suffix that differs from the previous key.
//!
//! Entry format in block:
//! - shared_len: varint (bytes shared with previous key)
//! - unshared_len: varint (bytes not shared)
//! - value_len: varint
//! - unshared_key: bytes[unshared_len]
//! - value: bytes[value_len]

use crate::entry::Entry;
use crate::error::{Result, SSTableError};
use bytes::{Buf, BufMut, Bytes, BytesMut};

/// A block of entries with prefix compression.
#[derive(Debug, Clone)]
pub struct Block {
    data: Bytes,
    restart_points: Vec<u32>,
}

impl Block {
    /// Decodes a block from bytes.
    pub fn decode(data: Bytes) -> Result<Self> {
        if data.len() < 4 {
            return Err(SSTableError::Incomplete);
        }

        // Read restart count from last 4 bytes
        let restart_count = u32::from_le_bytes([
            data[data.len() - 4],
            data[data.len() - 3],
            data[data.len() - 2],
            data[data.len() - 1],
        ]);

        let restart_points_size = restart_count as usize * 4;
        if data.len() < restart_points_size + 4 {
            return Err(SSTableError::Incomplete);
        }

        // Read restart points
        let restart_points_start = data.len() - 4 - restart_points_size;
        let mut restart_points = Vec::with_capacity(restart_count as usize);

        for i in 0..restart_count as usize {
            let offset = restart_points_start + i * 4;
            let point = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            restart_points.push(point);
        }

        Ok(Block {
            data,
            restart_points,
        })
    }

    /// Returns an iterator over entries in this block.
    pub fn iter(&self) -> BlockIterator {
        BlockIterator {
            data: self.data.clone(),
            restart_points: self.restart_points.clone(),
            offset: 0,
            last_key: Bytes::new(),
        }
    }

    /// Searches for a key in the block.
    pub fn get(&self, key: &[u8]) -> Result<Option<Entry>> {
        // Binary search over restart points
        let mut left = 0;
        let mut right = self.restart_points.len();

        while left < right {
            let mid = (left + right) / 2;
            let restart_offset = self.restart_points[mid] as usize;

            // Decode the key at this restart point
            let mut buf = &self.data[restart_offset..];
            let shared_len = decode_varint(&mut buf)? as usize;
            let unshared_len = decode_varint(&mut buf)? as usize;
            let _value_len_encoded = decode_varint(&mut buf)?;

            if shared_len != 0 {
                return Err(SSTableError::InvalidFormat(
                    "restart point must have shared_len=0".to_string(),
                ));
            }

            if buf.len() < unshared_len {
                return Err(SSTableError::Incomplete);
            }

            let restart_key = &buf[..unshared_len];

            match key.cmp(restart_key) {
                std::cmp::Ordering::Less => right = mid,
                std::cmp::Ordering::Equal => {
                    // Found exact match at restart point
                    return self.decode_entry_at(restart_offset, &Bytes::new());
                }
                std::cmp::Ordering::Greater => left = mid + 1,
            }
        }

        // Now scan linearly from the restart point
        if left == 0 {
            // No suitable restart point, start from beginning
            let mut iter = self.iter();
            while let Some(entry) = iter.try_next()? {
                match entry.key.as_ref().cmp(key) {
                    std::cmp::Ordering::Equal => return Ok(Some(entry)),
                    std::cmp::Ordering::Greater => return Ok(None),
                    std::cmp::Ordering::Less => continue,
                }
            }
            return Ok(None);
        }

        // Start from the restart point before where the key would be
        let restart_offset = self.restart_points[left - 1] as usize;

        let mut iter = BlockIterator {
            data: self.data.clone(),
            restart_points: self.restart_points.clone(),
            offset: restart_offset,
            last_key: Bytes::new(),
        };

        while let Some(entry) = iter.try_next()? {
            match entry.key.as_ref().cmp(key) {
                std::cmp::Ordering::Equal => return Ok(Some(entry)),
                std::cmp::Ordering::Greater => return Ok(None),
                std::cmp::Ordering::Less => continue,
            }
        }

        Ok(None)
    }

    fn decode_entry_at(&self, offset: usize, last_key: &Bytes) -> Result<Option<Entry>> {
        let data_end = self.data.len() - 4 - (self.restart_points.len() * 4);
        if offset >= data_end {
            return Ok(None);
        }

        let mut buf = &self.data[offset..data_end];

        let shared_len = decode_varint(&mut buf)? as usize;
        let unshared_len = decode_varint(&mut buf)? as usize;
        let value_len_encoded = decode_varint(&mut buf)?;

        // Extract tombstone flag from high bit
        let tombstone = (value_len_encoded & (1u64 << 63)) != 0;
        let value_len = (value_len_encoded & !(1u64 << 63)) as usize;

        if buf.len() < unshared_len + value_len {
            return Err(SSTableError::Incomplete);
        }

        // Reconstruct full key
        let mut key = BytesMut::with_capacity(shared_len + unshared_len);
        if shared_len > last_key.len() {
            return Err(SSTableError::InvalidFormat(format!(
                "shared_len {} exceeds last_key len {}",
                shared_len,
                last_key.len()
            )));
        }
        key.put_slice(&last_key[..shared_len]);
        key.put_slice(&buf[..unshared_len]);
        let key = key.freeze();

        buf.advance(unshared_len);

        let value = Bytes::copy_from_slice(&buf[..value_len]);

        Ok(Some(Entry {
            key,
            value,
            tombstone,
        }))
    }
}

/// Iterator over entries in a block.
pub struct BlockIterator {
    data: Bytes,
    restart_points: Vec<u32>,
    offset: usize,
    last_key: Bytes,
}

impl BlockIterator {
    /// Returns the next entry in the block.
    ///
    /// Returns `Ok(Some(entry))` if there is another entry,
    /// `Ok(None)` if iteration is complete, or `Err` on decoding errors.
    pub fn try_next(&mut self) -> Result<Option<Entry>> {
        let data_end = self.data.len() - 4 - (self.restart_points.len() * 4);
        if self.offset >= data_end {
            return Ok(None);
        }

        let start_offset = self.offset;
        let mut buf = &self.data[self.offset..data_end];
        let initial_buf_len = buf.len();

        let shared_len = decode_varint(&mut buf)? as usize;
        let unshared_len = decode_varint(&mut buf)? as usize;
        let value_len_encoded = decode_varint(&mut buf)?;

        // Extract tombstone flag from high bit
        let tombstone = (value_len_encoded & (1u64 << 63)) != 0;
        let value_len = (value_len_encoded & !(1u64 << 63)) as usize;

        if buf.len() < unshared_len + value_len {
            return Err(SSTableError::Incomplete);
        }

        // Reconstruct full key
        // Optimization: Only allocate if we have a shared prefix
        let key = if shared_len > 0 {
            if shared_len > self.last_key.len() {
                return Err(SSTableError::InvalidFormat(format!(
                    "shared_len {} exceeds last_key len {}",
                    shared_len,
                    self.last_key.len()
                )));
            }
            // Need to combine shared prefix with unshared suffix
            let mut key_buf = BytesMut::with_capacity(shared_len + unshared_len);
            key_buf.put_slice(&self.last_key[..shared_len]);
            key_buf.put_slice(&buf[..unshared_len]);
            key_buf.freeze()
        } else {
            // No shared prefix - can create Bytes directly from slice
            // Calculate the absolute offset in self.data
            let varint_bytes = initial_buf_len - buf.len();
            let unshared_start = self.offset + varint_bytes;
            self.data
                .slice(unshared_start..unshared_start + unshared_len)
        };

        buf.advance(unshared_len);

        // Optimization: Use slice instead of copy for value
        let value_offset_in_buf = data_end - buf.len();
        let value = self
            .data
            .slice(value_offset_in_buf..value_offset_in_buf + value_len);

        // Update offset: advance by how many bytes we consumed
        let bytes_consumed = initial_buf_len - buf.len() + value_len;
        self.offset = start_offset + bytes_consumed;

        // Update last_key (Bytes::clone is cheap - just increments refcount)
        self.last_key = key.clone();

        Ok(Some(Entry {
            key,
            value,
            tombstone,
        }))
    }
}

/// Builder for creating blocks with prefix compression.
pub struct BlockBuilder {
    buffer: BytesMut,
    restart_points: Vec<u32>,
    restart_interval: usize,
    counter: usize,
    last_key: Bytes,
    entry_count: usize,
}

impl BlockBuilder {
    /// Creates a new block builder.
    pub fn new(restart_interval: usize) -> Self {
        let mut builder = Self {
            buffer: BytesMut::new(),
            restart_points: Vec::new(),
            restart_interval,
            counter: 0,
            last_key: Bytes::new(),
            entry_count: 0,
        };
        builder.restart_points.push(0);
        builder
    }

    /// Adds an entry to the block.
    pub fn add(&mut self, entry: &Entry) -> Result<()> {
        // Check key ordering
        if !self.last_key.is_empty() && entry.key <= self.last_key {
            return Err(SSTableError::KeysNotSorted(
                self.last_key.to_vec(),
                entry.key.to_vec(),
            ));
        }

        // Determine if this is a restart point
        let use_restart = self.counter >= self.restart_interval;

        let shared_len = if use_restart {
            0
        } else {
            common_prefix_len(&self.last_key, &entry.key)
        };

        let unshared_len = entry.key.len() - shared_len;

        // If this is a restart point, record the offset
        if use_restart {
            self.restart_points.push(self.buffer.len() as u32);
            self.counter = 0;
        }

        // Encode entry with prefix compression
        encode_varint(&mut self.buffer, shared_len as u64);
        encode_varint(&mut self.buffer, unshared_len as u64);
        // Use high bit of value_len to encode tombstone flag
        let value_len_encoded = if entry.tombstone {
            (entry.value.len() as u64) | (1u64 << 63)
        } else {
            entry.value.len() as u64
        };
        encode_varint(&mut self.buffer, value_len_encoded);
        self.buffer.put_slice(&entry.key[shared_len..]);
        self.buffer.put_slice(&entry.value);

        self.last_key = entry.key.clone();
        self.counter += 1;
        self.entry_count += 1;

        Ok(())
    }

    /// Returns the current size of the block in bytes.
    pub fn current_size(&self) -> usize {
        self.buffer.len() + (self.restart_points.len() * 4) + 4
    }

    /// Returns the number of entries added so far.
    pub fn entry_count(&self) -> usize {
        self.entry_count
    }

    /// Resets the builder for reuse.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.restart_points.clear();
        self.restart_points.push(0);
        self.counter = 0;
        self.last_key = Bytes::new();
        self.entry_count = 0;
    }

    /// Finishes building the block and returns the encoded bytes.
    pub fn finish(&mut self) -> Bytes {
        // Append restart points
        for &point in &self.restart_points {
            self.buffer.put_u32_le(point);
        }

        // Append restart count
        self.buffer.put_u32_le(self.restart_points.len() as u32);

        self.buffer.clone().freeze()
    }
}

/// Returns the length of the common prefix between two byte slices.
fn common_prefix_len(a: &[u8], b: &[u8]) -> usize {
    let min_len = a.len().min(b.len());
    for i in 0..min_len {
        if a[i] != b[i] {
            return i;
        }
    }
    min_len
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_builder_single_entry() {
        let mut builder = BlockBuilder::new(16); // DEFAULT_RESTART_INTERVAL
        let entry = Entry::put(&b"key1"[..], &b"value1"[..]);

        builder.add(&entry).unwrap();
        let block_data = builder.finish();

        let block = Block::decode(block_data).unwrap();
        let found = block.get(b"key1").unwrap().unwrap();

        assert_eq!(found.key, entry.key);
        assert_eq!(found.value, entry.value);
    }

    #[test]
    fn test_block_builder_multiple_entries() {
        let mut builder = BlockBuilder::new(16); // DEFAULT_RESTART_INTERVAL

        for i in 0..100 {
            let key = format!("key{:03}", i);
            let value = format!("value{:03}", i);
            builder
                .add(&Entry::put(key.clone(), value.clone()))
                .unwrap();
        }

        let block_data = builder.finish();
        let block = Block::decode(block_data).unwrap();

        // Test lookups
        for i in 0..100 {
            let key = format!("key{:03}", i);
            let expected_value = format!("value{:03}", i);

            let found = block.get(key.as_bytes()).unwrap().unwrap();
            assert_eq!(found.key.as_ref(), key.as_bytes());
            assert_eq!(found.value.as_ref(), expected_value.as_bytes());
        }

        // Test non-existent key
        let not_found = block.get(b"key999").unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_block_iterator() {
        let mut builder = BlockBuilder::new(16); // DEFAULT_RESTART_INTERVAL

        let entries = vec![
            Entry::put(&b"apple"[..], &b"red"[..]),
            Entry::put(&b"banana"[..], &b"yellow"[..]),
            Entry::put(&b"cherry"[..], &b"red"[..]),
        ];

        for entry in &entries {
            builder.add(entry).unwrap();
        }

        let block_data = builder.finish();
        let block = Block::decode(block_data).unwrap();

        let mut iter = block.iter();
        for expected in &entries {
            let entry = iter.try_next().unwrap().unwrap();
            assert_eq!(entry.key, expected.key);
            assert_eq!(entry.value, expected.value);
        }

        assert!(iter.try_next().unwrap().is_none());
    }

    #[test]
    fn test_block_prefix_compression() {
        let mut builder = BlockBuilder::new(2); // Small restart interval for testing

        // Add keys with common prefixes
        builder.add(&Entry::put(&b"apple"[..], &b"v1"[..])).unwrap();
        builder
            .add(&Entry::put(&b"application"[..], &b"v2"[..]))
            .unwrap();
        builder.add(&Entry::put(&b"apply"[..], &b"v3"[..])).unwrap();

        let size_with_compression = builder.current_size();

        // Build without compression (restart_interval = 1)
        let mut builder_no_compression = BlockBuilder::new(1);
        builder_no_compression
            .add(&Entry::put(&b"apple"[..], &b"v1"[..]))
            .unwrap();
        builder_no_compression
            .add(&Entry::put(&b"application"[..], &b"v2"[..]))
            .unwrap();
        builder_no_compression
            .add(&Entry::put(&b"apply"[..], &b"v3"[..]))
            .unwrap();

        let size_without_compression = builder_no_compression.current_size();

        // Compression should save space
        assert!(size_with_compression < size_without_compression);
    }

    #[test]
    fn test_keys_not_sorted() {
        let mut builder = BlockBuilder::new(16); // DEFAULT_RESTART_INTERVAL
        builder.add(&Entry::put(&b"zebra"[..], &b"v1"[..])).unwrap();

        let result = builder.add(&Entry::put(&b"apple"[..], &b"v2"[..]));
        assert!(matches!(result, Err(SSTableError::KeysNotSorted(_, _))));
    }

    #[test]
    fn test_common_prefix_len() {
        assert_eq!(common_prefix_len(b"apple", b"application"), 4);
        assert_eq!(common_prefix_len(b"apple", b"banana"), 0);
        assert_eq!(common_prefix_len(b"test", b"test"), 4);
        assert_eq!(common_prefix_len(b"", b"test"), 0);
    }
}
