//! Block encoding/decoding with restart points for prefix compression.
//!
//! # Block Structure
//!
//! ## Version 1 (legacy): No inline filter
//! ```text
//! [Entries with prefix compression]
//! [Restart points array: u32...]
//! [Restart count: u32]
//! ```
//!
//! ## Version 2: With inline Quotient Filter
//! ```text
//! [Entries with prefix compression]
//! [Restart points array: u32...]
//! [Restart count: u32]
//! [Quotient Filter data]
//! [QF size: u16]
//! ```
//!
//! # Prefix Compression
//!
//! Every `restart_interval` entries, we store the full key. Between restart points,
//! we store only the suffix that differs from the previous key.
//!
//! # Entry Format
//!
//! - shared_len: varint (bytes shared with previous key)
//! - unshared_len: varint (bytes not shared)
//! - value_len: varint (high bit = tombstone flag)
//! - term: varint
//! - index: varint
//! - unshared_key: bytes[unshared_len]
//! - value: bytes[value_len]

use crate::entry::Entry;
use crate::error::{Result, SSTableError};
use crate::quotient_filter::{Fingerprint, QuotientFilter, QuotientFilterConfig};
use bytes::{Buf, BufMut, Bytes, BytesMut};

/// Result of a per-block filter check during lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterCheckResult {
    /// Quotient Filter rejected the key (definitely not present).
    Skipped,
    /// Quotient Filter passed (key might be present, needs binary search).
    Passed,
    /// No filter present (v1 format block).
    NoFilter,
}

/// A block of entries with prefix compression and optional Quotient Filter.
#[derive(Debug, Clone)]
pub struct Block {
    data: Bytes,
    restart_points: Vec<u32>,
    /// Optional Quotient Filter for fast negative lookups (v2 format).
    quotient_filter: Option<QuotientFilter>,
    /// End offset of entry data (before restart points).
    data_end: usize,
}

impl Block {
    /// Decodes a block from bytes (v1 format, no inline filter).
    pub fn decode(data: Bytes) -> Result<Self> {
        Self::decode_with_filter(data, false)
    }

    /// Decodes a block from bytes, optionally parsing an inline Quotient Filter.
    ///
    /// Use `has_inline_filter=true` for v2 format blocks that include a QF.
    pub fn decode_with_filter(data: Bytes, has_inline_filter: bool) -> Result<Self> {
        if data.len() < 4 {
            return Err(SSTableError::Incomplete);
        }

        // For v2 format, parse QF from the end first
        let (qf, qf_total_size) = if has_inline_filter {
            // Read QF size from last 2 bytes
            if data.len() < 6 {
                return Err(SSTableError::Incomplete);
            }
            let qf_size = u16::from_le_bytes([data[data.len() - 2], data[data.len() - 1]]) as usize;

            if qf_size == 0 {
                (None, 2) // Just the size field, no QF data
            } else {
                if data.len() < qf_size + 2 {
                    return Err(SSTableError::Incomplete);
                }
                let qf_start = data.len() - 2 - qf_size;
                let qf_data = &data[qf_start..data.len() - 2];
                let qf = QuotientFilter::decode(qf_data)?;
                (Some(qf), qf_size + 2)
            }
        } else {
            (None, 0)
        };

        // Remaining data after removing QF
        let remaining_len = data.len() - qf_total_size;

        if remaining_len < 4 {
            return Err(SSTableError::Incomplete);
        }

        // Read restart count
        let restart_count = u32::from_le_bytes([
            data[remaining_len - 4],
            data[remaining_len - 3],
            data[remaining_len - 2],
            data[remaining_len - 1],
        ]);

        let restart_points_size = restart_count as usize * 4;
        if remaining_len < restart_points_size + 4 {
            return Err(SSTableError::Incomplete);
        }

        // Read restart points
        let restart_points_start = remaining_len - 4 - restart_points_size;
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

        let data_end = restart_points_start;

        Ok(Block {
            data,
            restart_points,
            quotient_filter: qf,
            data_end,
        })
    }

    /// Returns true if this block has an inline Quotient Filter.
    pub fn has_quotient_filter(&self) -> bool {
        self.quotient_filter.is_some()
    }

    /// Fast check if a key might be in this block.
    ///
    /// Returns `true` if the key might be present (requires full lookup).
    /// Returns `false` if the key is definitely not present (no false negatives).
    ///
    /// If no Quotient Filter is present, always returns `true`.
    pub fn may_contain(&self, key: &[u8]) -> bool {
        match &self.quotient_filter {
            Some(qf) => qf.contains(key),
            None => true, // No filter, must do full lookup
        }
    }

    /// Returns a reference to the Quotient Filter, if present.
    pub fn quotient_filter(&self) -> Option<&QuotientFilter> {
        self.quotient_filter.as_ref()
    }

    /// Returns an iterator over entries in this block.
    pub fn iter(&self) -> BlockIterator {
        BlockIterator {
            data: self.data.clone(),
            data_end: self.data_end,
            offset: 0,
            last_key: Bytes::new(),
        }
    }

    /// Searches for a key in the block.
    ///
    /// If a Quotient Filter is present, it is checked first for fast rejection.
    pub fn get(&self, key: &[u8]) -> Result<Option<Entry>> {
        self.get_with_filter_result(key).map(|(entry, _)| entry)
    }

    /// Searches for a key in the block, returning filter check result.
    ///
    /// Returns `(Option<Entry>, FilterCheckResult)` where:
    /// - `FilterCheckResult::Skipped` - QF rejected the key (fast path)
    /// - `FilterCheckResult::Passed` - QF passed or no QF present
    ///
    /// This is useful for tracking per-block QF metrics.
    pub fn get_with_filter_result(&self, key: &[u8]) -> Result<(Option<Entry>, FilterCheckResult)> {
        // Fast path: check Quotient Filter first
        if let Some(ref qf) = self.quotient_filter {
            if !qf.contains(key) {
                return Ok((None, FilterCheckResult::Skipped)); // Definitely not present
            }
            // QF passed, continue to binary search
            let entry = self.search_binary(key)?;
            return Ok((entry, FilterCheckResult::Passed));
        }

        // No QF present
        let entry = self.search_binary(key)?;
        Ok((entry, FilterCheckResult::NoFilter))
    }

    /// Binary search for a key in the block (internal helper).
    fn search_binary(&self, key: &[u8]) -> Result<Option<Entry>> {
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
            // Skip version (term, index) before reading key
            let _term = decode_varint(&mut buf)?;
            let _index = decode_varint(&mut buf)?;

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
            data_end: self.data_end,
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
        if offset >= self.data_end {
            return Ok(None);
        }

        let mut buf = &self.data[offset..self.data_end];

        let shared_len = decode_varint(&mut buf)? as usize;
        let unshared_len = decode_varint(&mut buf)? as usize;
        let value_len_encoded = decode_varint(&mut buf)?;

        // Extract tombstone flag from high bit
        let tombstone = (value_len_encoded & (1u64 << 63)) != 0;
        let value_len = (value_len_encoded & !(1u64 << 63)) as usize;

        // Decode version (term, index)
        let term = decode_varint(&mut buf)?;
        let index = decode_varint(&mut buf)?;

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
            term,
            index,
        }))
    }
}

/// Iterator over entries in a block.
pub struct BlockIterator {
    data: Bytes,
    data_end: usize,
    offset: usize,
    last_key: Bytes,
}

impl BlockIterator {
    /// Returns the next entry in the block.
    ///
    /// Returns `Ok(Some(entry))` if there is another entry,
    /// `Ok(None)` if iteration is complete, or `Err` on decoding errors.
    pub fn try_next(&mut self) -> Result<Option<Entry>> {
        if self.offset >= self.data_end {
            return Ok(None);
        }

        let start_offset = self.offset;
        let mut buf = &self.data[self.offset..self.data_end];
        let initial_buf_len = buf.len();

        let shared_len = decode_varint(&mut buf)? as usize;
        let unshared_len = decode_varint(&mut buf)? as usize;
        let value_len_encoded = decode_varint(&mut buf)?;

        // Extract tombstone flag from high bit
        let tombstone = (value_len_encoded & (1u64 << 63)) != 0;
        let value_len = (value_len_encoded & !(1u64 << 63)) as usize;

        // Decode version (term, index)
        let term = decode_varint(&mut buf)?;
        let index = decode_varint(&mut buf)?;

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
        let value_offset_in_buf = self.data_end - buf.len();
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
            term,
            index,
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
    /// Optional Quotient Filter for v2 format blocks.
    quotient_filter: Option<QuotientFilter>,
}

impl BlockBuilder {
    /// Creates a new block builder (v1 format, no inline filter).
    pub fn new(restart_interval: usize) -> Self {
        let mut builder = Self {
            buffer: BytesMut::new(),
            restart_points: Vec::new(),
            restart_interval,
            counter: 0,
            last_key: Bytes::new(),
            entry_count: 0,
            quotient_filter: None,
        };
        builder.restart_points.push(0);
        builder
    }

    /// Creates a new block builder with Quotient Filter support (v2 format).
    ///
    /// The `config` specifies the QF parameters. Use `QuotientFilterConfig::default()`
    /// for sensible defaults (~0.78% FPR with ~75% load factor).
    pub fn new_with_filter(restart_interval: usize, config: QuotientFilterConfig) -> Self {
        let mut builder = Self {
            buffer: BytesMut::new(),
            restart_points: Vec::new(),
            restart_interval,
            counter: 0,
            last_key: Bytes::new(),
            entry_count: 0,
            quotient_filter: Some(QuotientFilter::with_config(config)),
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

        // Add to Quotient Filter if present (for v2 format)
        if let Some(ref mut qf) = self.quotient_filter {
            qf.add(&entry.key);
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
        // Encode version (term, index) for MVCC
        encode_varint(&mut self.buffer, entry.term);
        encode_varint(&mut self.buffer, entry.index);
        self.buffer.put_slice(&entry.key[shared_len..]);
        self.buffer.put_slice(&entry.value);

        self.last_key = entry.key.clone();
        self.counter += 1;
        self.entry_count += 1;

        Ok(())
    }

    /// Adds an entry to the block with a pre-computed fingerprint.
    ///
    /// This method is used during compaction to avoid re-hashing keys.
    /// Instead of hashing the key to compute the fingerprint, the caller
    /// provides a pre-computed fingerprint that is inserted directly into
    /// the Quotient Filter.
    ///
    /// If this builder doesn't have a Quotient Filter (v1 format), the
    /// fingerprint is ignored and this behaves like `add()`.
    pub fn add_with_fingerprint(&mut self, entry: &Entry, fp: Fingerprint) -> Result<()> {
        // Check key ordering
        if !self.last_key.is_empty() && entry.key <= self.last_key {
            return Err(SSTableError::KeysNotSorted(
                self.last_key.to_vec(),
                entry.key.to_vec(),
            ));
        }

        // Insert fingerprint directly (skip hashing)
        if let Some(ref mut qf) = self.quotient_filter {
            qf.insert_fingerprint(fp);
        }

        // Rest is identical to add()
        let use_restart = self.counter >= self.restart_interval;

        let shared_len = if use_restart {
            0
        } else {
            common_prefix_len(&self.last_key, &entry.key)
        };

        let unshared_len = entry.key.len() - shared_len;

        if use_restart {
            self.restart_points.push(self.buffer.len() as u32);
            self.counter = 0;
        }

        encode_varint(&mut self.buffer, shared_len as u64);
        encode_varint(&mut self.buffer, unshared_len as u64);
        let value_len_encoded = if entry.tombstone {
            (entry.value.len() as u64) | (1u64 << 63)
        } else {
            entry.value.len() as u64
        };
        encode_varint(&mut self.buffer, value_len_encoded);
        encode_varint(&mut self.buffer, entry.term);
        encode_varint(&mut self.buffer, entry.index);
        self.buffer.put_slice(&entry.key[shared_len..]);
        self.buffer.put_slice(&entry.value);

        self.last_key = entry.key.clone();
        self.counter += 1;
        self.entry_count += 1;

        Ok(())
    }

    /// Computes the fingerprint for a key using this builder's QF configuration.
    ///
    /// Returns `None` if this builder doesn't have a Quotient Filter.
    /// This is useful when the caller needs to pre-compute fingerprints
    /// for later use with `add_with_fingerprint()`.
    pub fn compute_fingerprint(&self, key: &[u8]) -> Option<Fingerprint> {
        self.quotient_filter.as_ref().map(|qf| qf.compute_fingerprint(key))
    }

    /// Returns the current size of the block in bytes.
    ///
    /// For v2 format blocks, this includes the estimated Quotient Filter size.
    pub fn current_size(&self) -> usize {
        let base_size = self.buffer.len() + (self.restart_points.len() * 4) + 4;
        match &self.quotient_filter {
            Some(qf) => base_size + qf.encoded_size() + 2, // +2 for QF size trailer
            None => base_size,
        }
    }

    /// Returns the number of entries added so far.
    pub fn entry_count(&self) -> usize {
        self.entry_count
    }

    /// Resets the builder for reuse.
    ///
    /// If the builder was created with a Quotient Filter, a new empty QF
    /// with the same configuration will be created.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.restart_points.clear();
        self.restart_points.push(0);
        self.counter = 0;
        self.last_key = Bytes::new();
        self.entry_count = 0;
        // Reset QF with same config if present
        if let Some(ref mut qf) = self.quotient_filter {
            qf.clear();
        }
    }

    /// Finishes building the block and returns the encoded bytes.
    ///
    /// For v2 format blocks with a Quotient Filter, the layout is:
    /// `[entries][restart points][restart count][QF data][QF size: u16]`
    pub fn finish(&mut self) -> Bytes {
        // Append restart points
        for &point in &self.restart_points {
            self.buffer.put_u32_le(point);
        }

        // Append restart count
        self.buffer.put_u32_le(self.restart_points.len() as u32);

        // Append Quotient Filter if present (v2 format)
        if let Some(ref qf) = self.quotient_filter {
            let qf_data = qf.encode();
            let qf_size = qf_data.len() as u16;
            self.buffer.put_slice(&qf_data);
            self.buffer.put_u16_le(qf_size);
        }

        self.buffer.clone().freeze()
    }

    /// Returns true if this builder produces v2 format blocks with inline QF.
    pub fn has_quotient_filter(&self) -> bool {
        self.quotient_filter.is_some()
    }

    /// Returns a reference to the Quotient Filter, if present.
    pub fn quotient_filter(&self) -> Option<&QuotientFilter> {
        self.quotient_filter.as_ref()
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

    // === V2 FORMAT TESTS (with Quotient Filter) ===

    #[test]
    fn test_block_builder_with_filter() {
        let config = QuotientFilterConfig::for_keys(100);
        let mut builder = BlockBuilder::new_with_filter(16, config);

        assert!(builder.has_quotient_filter());

        // Add entries
        for i in 0..50 {
            let key = format!("key{:03}", i);
            let value = format!("value{:03}", i);
            builder.add(&Entry::put(key, value)).unwrap();
        }

        // QF should have entries
        assert_eq!(builder.quotient_filter().unwrap().len(), 50);

        let block_data = builder.finish();

        // Decode as v2 format
        let block = Block::decode_with_filter(block_data, true).unwrap();

        assert!(block.has_quotient_filter());
        assert_eq!(block.quotient_filter().unwrap().len(), 50);
    }

    #[test]
    fn test_block_v2_roundtrip() {
        let config = QuotientFilterConfig::for_keys(100);
        let mut builder = BlockBuilder::new_with_filter(16, config);

        let entries: Vec<_> = (0..30)
            .map(|i| Entry::put(format!("key{:03}", i), format!("val{:03}", i)))
            .collect();

        for entry in &entries {
            builder.add(entry).unwrap();
        }

        let block_data = builder.finish();
        let block = Block::decode_with_filter(block_data, true).unwrap();

        // Verify all entries via iteration
        let mut iter = block.iter();
        for expected in &entries {
            let entry = iter.try_next().unwrap().unwrap();
            assert_eq!(entry.key, expected.key);
            assert_eq!(entry.value, expected.value);
        }
        assert!(iter.try_next().unwrap().is_none());

        // Verify lookups
        for entry in &entries {
            let found = block.get(&entry.key).unwrap().unwrap();
            assert_eq!(found.key, entry.key);
            assert_eq!(found.value, entry.value);
        }
    }

    #[test]
    fn test_block_qf_fast_rejection() {
        let config = QuotientFilterConfig::for_keys(100);
        let mut builder = BlockBuilder::new_with_filter(16, config);

        // Add known keys
        for i in 0..20 {
            builder
                .add(&Entry::put(format!("exists{:03}", i), "value"))
                .unwrap();
        }

        let block_data = builder.finish();
        let block = Block::decode_with_filter(block_data, true).unwrap();

        // Keys that exist should pass may_contain
        for i in 0..20 {
            assert!(
                block.may_contain(format!("exists{:03}", i).as_bytes()),
                "False negative for exists{:03}",
                i
            );
        }

        // Most non-existent keys should be rejected by QF (statistical)
        let mut rejected = 0;
        for i in 100..200 {
            if !block.may_contain(format!("notexists{:03}", i).as_bytes()) {
                rejected += 1;
            }
        }

        // With ~0.78% FPR, we expect ~97-99% rejection
        assert!(
            rejected > 90,
            "QF should reject most non-existent keys, but only rejected {}",
            rejected
        );
    }

    #[test]
    fn test_block_qf_get_uses_filter() {
        let config = QuotientFilterConfig::for_keys(100);
        let mut builder = BlockBuilder::new_with_filter(16, config);

        // Add a few keys
        builder.add(&Entry::put("aaa", "v1")).unwrap();
        builder.add(&Entry::put("bbb", "v2")).unwrap();
        builder.add(&Entry::put("ccc", "v3")).unwrap();

        let block_data = builder.finish();
        let block = Block::decode_with_filter(block_data, true).unwrap();

        // Existing keys found
        assert!(block.get(b"aaa").unwrap().is_some());
        assert!(block.get(b"bbb").unwrap().is_some());
        assert!(block.get(b"ccc").unwrap().is_some());

        // Non-existent key returns None (may skip binary search due to QF)
        assert!(block.get(b"zzz").unwrap().is_none());
    }

    #[test]
    fn test_block_builder_reset_with_filter() {
        let config = QuotientFilterConfig::for_keys(100);
        let mut builder = BlockBuilder::new_with_filter(16, config);

        // Add some entries
        builder.add(&Entry::put("key1", "v1")).unwrap();
        builder.add(&Entry::put("key2", "v2")).unwrap();
        assert_eq!(builder.entry_count(), 2);
        assert_eq!(builder.quotient_filter().unwrap().len(), 2);

        // Reset
        builder.reset();

        assert_eq!(builder.entry_count(), 0);
        assert_eq!(builder.quotient_filter().unwrap().len(), 0);
        assert!(builder.has_quotient_filter());

        // Can add new entries
        builder.add(&Entry::put("newkey", "newval")).unwrap();
        assert_eq!(builder.entry_count(), 1);
    }

    #[test]
    fn test_block_without_filter_decode_v1() {
        // v1 format blocks should still work with decode()
        let mut builder = BlockBuilder::new(16);

        for i in 0..20 {
            builder
                .add(&Entry::put(format!("key{:03}", i), format!("val{:03}", i)))
                .unwrap();
        }

        let block_data = builder.finish();
        let block = Block::decode(block_data).unwrap();

        assert!(!block.has_quotient_filter());
        assert!(block.may_contain(b"anything")); // No filter, always returns true

        // Lookups still work
        let found = block.get(b"key005").unwrap().unwrap();
        assert_eq!(found.value.as_ref(), b"val005");
    }

    #[test]
    fn test_add_with_fingerprint_produces_correct_block() {
        let config = QuotientFilterConfig::for_keys(100);
        let mut builder = BlockBuilder::new_with_filter(16, config);

        // Add entries using add_with_fingerprint
        for i in 0..30 {
            let key = format!("key{:03}", i);
            let entry = Entry::put(key.clone(), format!("value{:03}", i));
            let fp = builder.compute_fingerprint(key.as_bytes()).unwrap();
            builder.add_with_fingerprint(&entry, fp).unwrap();
        }

        let block_data = builder.finish();
        let block = Block::decode_with_filter(block_data, true).unwrap();

        // All keys should be findable
        for i in 0..30 {
            let key = format!("key{:03}", i);
            let found = block.get(key.as_bytes()).unwrap().unwrap();
            assert_eq!(found.value.as_ref(), format!("value{:03}", i).as_bytes());
        }

        // QF should reject most non-existent keys
        let mut rejected = 0;
        for i in 100..150 {
            if !block.may_contain(format!("missing{:03}", i).as_bytes()) {
                rejected += 1;
            }
        }
        assert!(rejected > 40, "Expected QF to reject most missing keys");
    }

    #[test]
    fn test_add_with_fingerprint_equivalent_to_add() {
        let config = QuotientFilterConfig::for_keys(100);

        // Build with add()
        let mut builder1 = BlockBuilder::new_with_filter(16, config);
        for i in 0..20 {
            let entry = Entry::put(format!("key{:03}", i), format!("val{:03}", i));
            builder1.add(&entry).unwrap();
        }
        let block1_data = builder1.finish();

        // Build with add_with_fingerprint()
        let mut builder2 = BlockBuilder::new_with_filter(16, config);
        for i in 0..20 {
            let key = format!("key{:03}", i);
            let entry = Entry::put(key.clone(), format!("val{:03}", i));
            let fp = builder2.compute_fingerprint(key.as_bytes()).unwrap();
            builder2.add_with_fingerprint(&entry, fp).unwrap();
        }
        let block2_data = builder2.finish();

        // Both blocks should have the same content
        let block1 = Block::decode_with_filter(block1_data, true).unwrap();
        let block2 = Block::decode_with_filter(block2_data, true).unwrap();

        // QF lengths should match
        assert_eq!(
            block1.quotient_filter().unwrap().len(),
            block2.quotient_filter().unwrap().len()
        );

        // Same entries should be found in both
        for i in 0..20 {
            let key = format!("key{:03}", i);
            let found1 = block1.get(key.as_bytes()).unwrap().unwrap();
            let found2 = block2.get(key.as_bytes()).unwrap().unwrap();
            assert_eq!(found1.key, found2.key);
            assert_eq!(found1.value, found2.value);
        }
    }

    #[test]
    fn test_compute_fingerprint_returns_none_for_v1() {
        let builder = BlockBuilder::new(16);
        assert!(builder.compute_fingerprint(b"any_key").is_none());
    }

    #[test]
    fn test_add_with_fingerprint_on_v1_builder() {
        // For v1 format, add_with_fingerprint should work but ignore the fingerprint
        let mut builder = BlockBuilder::new(16);

        // Create a dummy fingerprint (won't be used since v1 has no QF)
        let dummy_fp = Fingerprint {
            quotient: 0,
            remainder: 0,
        };

        builder
            .add_with_fingerprint(&Entry::put("key1", "val1"), dummy_fp)
            .unwrap();
        builder
            .add_with_fingerprint(&Entry::put("key2", "val2"), dummy_fp)
            .unwrap();

        let block_data = builder.finish();
        let block = Block::decode(block_data).unwrap();

        // Block should work normally without QF
        assert!(!block.has_quotient_filter());
        assert!(block.get(b"key1").unwrap().is_some());
        assert!(block.get(b"key2").unwrap().is_some());
    }
}
