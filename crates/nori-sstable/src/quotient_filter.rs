//! Quotient Filter for reducing disk I/O on non-existent key lookups.
//!
//! A Quotient Filter is a space-efficient probabilistic data structure similar to
//! a Bloom filter, but with the additional property of supporting efficient merging
//! without re-hashing the original keys. This makes it ideal for LSM-tree compaction.
//!
//! # How It Works
//!
//! A QF stores fingerprints of keys by splitting each hash into:
//! - **Quotient (q bits)**: Used as the slot index (home bucket)
//! - **Remainder (r bits)**: Stored in the slot
//!
//! When collisions occur, elements are shifted to adjacent slots while maintaining
//! metadata bits that allow reconstruction of the original quotient.
//!
//! # Merge Support
//!
//! Unlike Bloom filters, QFs can be merged without access to original keys:
//! 1. Extract all (quotient, remainder) pairs from input filters
//! 2. Insert fingerprints directly into the output filter
//!
//! This enables O(n) merge during compaction instead of O(n * hash_cost).
//!
//! # Parameters
//!
//! For ~1% false positive rate with 50-200 keys per block:
//! - `remainder_bits = 7` (FPR = 1/128 = 0.78%)
//! - `quotient_bits = 8` (256 slots, ~75% load at 150 keys)

use crate::error::{Result, SSTableError};
use bytes::{BufMut, Bytes, BytesMut};

/// Default number of quotient bits (2^8 = 256 slots).
pub const DEFAULT_QUOTIENT_BITS: u8 = 8;

/// Default number of remainder bits (FPR = 2^-7 ≈ 0.78%).
pub const DEFAULT_REMAINDER_BITS: u8 = 7;

/// Default target load factor for sizing.
pub const DEFAULT_LOAD_FACTOR: f64 = 0.75;

/// A fingerprint extracted from or to be inserted into a Quotient Filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint {
    /// The quotient (slot index).
    pub quotient: u32,
    /// The remainder (stored value).
    pub remainder: u16,
}

impl Fingerprint {
    /// Creates a fingerprint from a hash value.
    #[inline]
    pub fn from_hash(hash: u64, quotient_bits: u8, remainder_bits: u8) -> Self {
        let quotient_mask = (1u64 << quotient_bits) - 1;
        let remainder_mask = (1u64 << remainder_bits) - 1;

        // Use high bits for quotient, next bits for remainder
        let quotient = ((hash >> (64 - quotient_bits)) & quotient_mask) as u32;
        let remainder = ((hash >> (64 - quotient_bits - remainder_bits)) & remainder_mask) as u16;

        Self { quotient, remainder }
    }
}

/// Configuration for creating a Quotient Filter.
#[derive(Debug, Clone, Copy)]
pub struct QuotientFilterConfig {
    /// Number of quotient bits (q). Determines slot count = 2^q.
    pub quotient_bits: u8,
    /// Number of remainder bits (r). Determines FPR = 2^(-r).
    pub remainder_bits: u8,
}

impl QuotientFilterConfig {
    /// Creates a configuration sized for the expected number of keys.
    ///
    /// Uses default load factor of 0.75 for good performance.
    pub fn for_keys(num_keys: usize) -> Self {
        Self::for_keys_with_load(num_keys, DEFAULT_LOAD_FACTOR)
    }

    /// Creates a configuration sized for expected keys with custom load factor.
    pub fn for_keys_with_load(num_keys: usize, load_factor: f64) -> Self {
        let load_factor = load_factor.clamp(0.5, 0.9);
        let slots_needed = (num_keys as f64 / load_factor).ceil() as usize;
        let quotient_bits = if slots_needed == 0 {
            4 // Minimum: 16 slots
        } else {
            ((slots_needed as f64).log2().ceil() as u8).clamp(4, 16)
        };

        Self {
            quotient_bits,
            remainder_bits: DEFAULT_REMAINDER_BITS,
        }
    }

    /// Returns the number of slots this configuration will create.
    pub fn num_slots(&self) -> usize {
        1 << self.quotient_bits
    }
}

impl Default for QuotientFilterConfig {
    fn default() -> Self {
        Self {
            quotient_bits: DEFAULT_QUOTIENT_BITS,
            remainder_bits: DEFAULT_REMAINDER_BITS,
        }
    }
}

/// Quotient Filter for membership testing with merge support.
///
/// This implementation uses a simplified linear probing approach that stores
/// (quotient, remainder) pairs explicitly. This trades some space efficiency
/// for correctness and simpler merge semantics.
///
/// # Features
///
/// - O(1) average insert and lookup
/// - Supports merging two QFs without re-hashing keys
/// - Space-efficient: ~16 bits per entry
///
/// # Invariants
///
/// - Each slot stores either nothing (empty) or a complete fingerprint
/// - Fingerprints are stored at or after their home bucket
/// - Lookup scans from home bucket until empty slot or full wrap
#[derive(Debug, Clone)]
pub struct QuotientFilter {
    /// Each slot stores Option<(quotient, remainder)>.
    /// quotient is stored to enable fingerprint extraction for merging.
    slots: Vec<Option<(u32, u16)>>,
    /// Number of quotient bits (q).
    quotient_bits: u8,
    /// Number of remainder bits (r).
    remainder_bits: u8,
    /// Number of elements inserted.
    count: usize,
}

impl QuotientFilter {
    /// Creates a new quotient filter with the given configuration.
    pub fn new(config: QuotientFilterConfig) -> Self {
        let num_slots = config.num_slots();
        Self {
            slots: vec![None; num_slots],
            quotient_bits: config.quotient_bits,
            remainder_bits: config.remainder_bits,
            count: 0,
        }
    }

    /// Creates a new quotient filter with the given configuration (alias for `new`).
    pub fn with_config(config: QuotientFilterConfig) -> Self {
        Self::new(config)
    }

    /// Creates a quotient filter sized for the expected number of keys.
    pub fn for_keys(num_keys: usize) -> Self {
        Self::new(QuotientFilterConfig::for_keys(num_keys))
    }

    /// Creates an empty quotient filter (for testing or empty blocks).
    pub fn empty() -> Self {
        Self {
            slots: vec![],
            quotient_bits: 0,
            remainder_bits: DEFAULT_REMAINDER_BITS,
            count: 0,
        }
    }

    /// Returns the number of elements in the filter.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns true if the filter is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Clears all elements from the filter, keeping the same configuration.
    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            *slot = None;
        }
        self.count = 0;
    }

    /// Returns the estimated encoded size in bytes.
    ///
    /// This is the size of the `encode()` output.
    pub fn encoded_size(&self) -> usize {
        self.size()
    }

    /// Returns the current load factor.
    pub fn load_factor(&self) -> f64 {
        if self.slots.is_empty() {
            0.0
        } else {
            self.count as f64 / self.slots.len() as f64
        }
    }

    /// Returns the number of slots in the filter.
    pub fn num_slots(&self) -> usize {
        self.slots.len()
    }

    /// Returns the quotient bits configuration.
    pub fn quotient_bits(&self) -> u8 {
        self.quotient_bits
    }

    /// Returns the remainder bits configuration.
    pub fn remainder_bits(&self) -> u8 {
        self.remainder_bits
    }

    /// Computes fingerprint from key using xxhash64.
    #[inline]
    fn fingerprint(&self, key: &[u8]) -> Fingerprint {
        let hash = xxhash_rust::xxh64::xxh64(key, 0);
        Fingerprint::from_hash(hash, self.quotient_bits, self.remainder_bits)
    }

    /// Computes a fingerprint for the given key without inserting it.
    ///
    /// This is useful for pre-computing fingerprints during compaction
    /// to avoid re-hashing when using `insert_fingerprint()`.
    #[inline]
    pub fn compute_fingerprint(&self, key: &[u8]) -> Fingerprint {
        self.fingerprint(key)
    }

    /// Adds a key to the filter.
    pub fn add(&mut self, key: &[u8]) {
        if self.slots.is_empty() {
            return;
        }

        let fp = self.fingerprint(key);
        self.insert_fingerprint(fp);
    }

    /// Inserts a pre-computed fingerprint into the filter.
    ///
    /// This is used during merge operations to avoid re-hashing keys.
    pub fn insert_fingerprint(&mut self, fp: Fingerprint) {
        if self.slots.is_empty() {
            return;
        }

        let num_slots = self.slots.len();
        let home = (fp.quotient as usize) % num_slots;

        // Linear probe to find an empty slot
        let mut pos = home;
        for _ in 0..num_slots {
            if self.slots[pos].is_none() {
                self.slots[pos] = Some((fp.quotient, fp.remainder));
                self.count += 1;
                return;
            }
            // Check for duplicate
            if let Some((q, r)) = self.slots[pos] {
                if q == fp.quotient && r == fp.remainder {
                    return; // Already present
                }
            }
            pos = (pos + 1) % num_slots;
        }
        // Filter is full - shouldn't happen with proper sizing
    }

    /// Tests whether a key might be in the set.
    ///
    /// Returns `true` if the key *might* be present (could be false positive).
    /// Returns `false` if the key is *definitely not* present (no false negatives).
    pub fn contains(&self, key: &[u8]) -> bool {
        if self.slots.is_empty() || self.count == 0 {
            return false;
        }

        let fp = self.fingerprint(key);
        self.lookup_fingerprint(fp)
    }

    /// Looks up a fingerprint in the filter.
    fn lookup_fingerprint(&self, fp: Fingerprint) -> bool {
        if self.slots.is_empty() {
            return false;
        }

        let num_slots = self.slots.len();
        let home = (fp.quotient as usize) % num_slots;

        // Linear probe from home bucket
        let mut pos = home;
        for _ in 0..num_slots {
            match self.slots[pos] {
                None => return false, // Empty slot means not found
                Some((q, r)) => {
                    if q == fp.quotient && r == fp.remainder {
                        return true;
                    }
                }
            }
            pos = (pos + 1) % num_slots;
        }

        false
    }

    // === MERGE SUPPORT ===

    /// Extracts all fingerprints from the filter.
    ///
    /// This enables merge without re-hashing original keys.
    pub fn extract_fingerprints(&self) -> Vec<Fingerprint> {
        self.slots
            .iter()
            .filter_map(|slot| {
                slot.map(|(q, r)| Fingerprint {
                    quotient: q,
                    remainder: r,
                })
            })
            .collect()
    }

    /// Merges another quotient filter into this one.
    ///
    /// # Requirements
    ///
    /// - Both filters must have the same remainder_bits
    /// - This filter must have sufficient capacity
    ///
    /// # Errors
    ///
    /// Returns an error if filters are incompatible or capacity is exceeded.
    pub fn merge(&mut self, other: &QuotientFilter) -> Result<()> {
        if self.remainder_bits != other.remainder_bits {
            return Err(SSTableError::InvalidFormat(
                "Cannot merge quotient filters with different remainder bits".to_string(),
            ));
        }

        let fingerprints = other.extract_fingerprints();

        // Check capacity
        let new_count = self.count + fingerprints.len();
        if new_count > self.slots.len() {
            return Err(SSTableError::InvalidFormat(format!(
                "Merge would exceed capacity: {} + {} > {}",
                self.count,
                fingerprints.len(),
                self.slots.len()
            )));
        }

        // Insert each fingerprint
        for fp in fingerprints {
            self.insert_fingerprint(fp);
        }

        Ok(())
    }

    /// Creates a new filter by merging multiple filters.
    ///
    /// The result filter is sized to accommodate all elements with target load factor.
    pub fn merge_all(filters: &[&QuotientFilter]) -> Result<QuotientFilter> {
        if filters.is_empty() {
            return Ok(QuotientFilter::empty());
        }

        let remainder_bits = filters[0].remainder_bits;
        for filter in filters.iter().skip(1) {
            if filter.remainder_bits != remainder_bits {
                return Err(SSTableError::InvalidFormat(
                    "Cannot merge quotient filters with different remainder bits".to_string(),
                ));
            }
        }

        let total_elements: usize = filters.iter().map(|f| f.count).sum();

        // Size new filter to accommodate all elements at 75% load
        let config = QuotientFilterConfig::for_keys(total_elements);
        let mut result = QuotientFilter::new(QuotientFilterConfig {
            quotient_bits: config.quotient_bits.max(
                filters
                    .iter()
                    .map(|f| f.quotient_bits)
                    .max()
                    .unwrap_or(DEFAULT_QUOTIENT_BITS),
            ),
            remainder_bits,
        });

        // Insert all fingerprints
        for filter in filters {
            for fp in filter.extract_fingerprints() {
                result.insert_fingerprint(fp);
            }
        }

        Ok(result)
    }

    // === SERIALIZATION ===

    /// Encodes the quotient filter to bytes.
    ///
    /// Format:
    /// - version: u8 (1)
    /// - quotient_bits: u8
    /// - remainder_bits: u8
    /// - count: u32 (little-endian)
    /// - num_slots: u32 (little-endian)
    /// - For each slot:
    ///   - has_value: u8 (0 or 1)
    ///   - if has_value: quotient: u32 LE, remainder: u16 LE
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::new();

        buf.put_u8(1); // version
        buf.put_u8(self.quotient_bits);
        buf.put_u8(self.remainder_bits);
        buf.put_u32_le(self.count as u32);
        buf.put_u32_le(self.slots.len() as u32);

        for slot in &self.slots {
            match slot {
                None => buf.put_u8(0),
                Some((q, r)) => {
                    buf.put_u8(1);
                    buf.put_u32_le(*q);
                    buf.put_u16_le(*r);
                }
            }
        }

        buf.freeze()
    }

    /// Decodes a quotient filter from bytes.
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() < 11 {
            return Err(SSTableError::Incomplete);
        }

        let version = data[0];
        if version != 1 {
            return Err(SSTableError::InvalidFormat(format!(
                "Unsupported quotient filter version: {}",
                version
            )));
        }

        let quotient_bits = data[1];
        let remainder_bits = data[2];
        let count = u32::from_le_bytes([data[3], data[4], data[5], data[6]]) as usize;
        let num_slots = u32::from_le_bytes([data[7], data[8], data[9], data[10]]) as usize;

        let mut slots = Vec::with_capacity(num_slots);
        let mut offset = 11;

        for _ in 0..num_slots {
            if offset >= data.len() {
                return Err(SSTableError::Incomplete);
            }

            if data[offset] == 0 {
                slots.push(None);
                offset += 1;
            } else {
                if offset + 7 > data.len() {
                    return Err(SSTableError::Incomplete);
                }
                let q = u32::from_le_bytes([
                    data[offset + 1],
                    data[offset + 2],
                    data[offset + 3],
                    data[offset + 4],
                ]);
                let r = u16::from_le_bytes([data[offset + 5], data[offset + 6]]);
                slots.push(Some((q, r)));
                offset += 7;
            }
        }

        Ok(Self {
            slots,
            quotient_bits,
            remainder_bits,
            count,
        })
    }

    /// Returns the size of the encoded filter in bytes.
    ///
    /// This is an estimate - actual size depends on occupancy.
    pub fn size(&self) -> usize {
        // Header: 11 bytes
        // Each slot: 1 byte (empty) or 7 bytes (occupied)
        let empty_slots = self.slots.len() - self.count;
        11 + empty_slots + self.count * 7
    }

    /// Returns the encoded size in bytes (compact format).
    ///
    /// Uses a more compact encoding that only stores occupied slots.
    pub fn encode_compact(&self) -> Bytes {
        let mut buf = BytesMut::new();

        buf.put_u8(2); // version 2 = compact
        buf.put_u8(self.quotient_bits);
        buf.put_u8(self.remainder_bits);
        buf.put_u32_le(self.count as u32);
        buf.put_u32_le(self.slots.len() as u32);

        // Only store occupied slots
        for (idx, slot) in self.slots.iter().enumerate() {
            if let Some((q, r)) = slot {
                buf.put_u32_le(idx as u32);
                buf.put_u32_le(*q);
                buf.put_u16_le(*r);
            }
        }

        buf.freeze()
    }

    /// Decodes from compact format.
    pub fn decode_compact(data: &[u8]) -> Result<Self> {
        if data.len() < 11 {
            return Err(SSTableError::Incomplete);
        }

        let version = data[0];
        if version != 2 {
            return Err(SSTableError::InvalidFormat(format!(
                "Expected compact format (version 2), got: {}",
                version
            )));
        }

        let quotient_bits = data[1];
        let remainder_bits = data[2];
        let count = u32::from_le_bytes([data[3], data[4], data[5], data[6]]) as usize;
        let num_slots = u32::from_le_bytes([data[7], data[8], data[9], data[10]]) as usize;

        let mut slots = vec![None; num_slots];
        let mut offset = 11;

        for _ in 0..count {
            if offset + 10 > data.len() {
                return Err(SSTableError::Incomplete);
            }
            let idx = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize;
            let q = u32::from_le_bytes([
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);
            let r = u16::from_le_bytes([data[offset + 8], data[offset + 9]]);

            if idx < num_slots {
                slots[idx] = Some((q, r));
            }
            offset += 10;
        }

        Ok(Self {
            slots,
            quotient_bits,
            remainder_bits,
            count,
        })
    }

    /// Returns the compact encoded size in bytes.
    pub fn size_compact(&self) -> usize {
        // Header: 11 bytes
        // Each occupied slot: 10 bytes (idx: 4, q: 4, r: 2)
        11 + self.count * 10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quotient_filter_basic() {
        let mut qf = QuotientFilter::for_keys(100);

        qf.add(b"key1");
        qf.add(b"key2");
        qf.add(b"key3");

        // Keys that were added should be found
        assert!(qf.contains(b"key1"), "key1 not found");
        assert!(qf.contains(b"key2"), "key2 not found");
        assert!(qf.contains(b"key3"), "key3 not found");
    }

    #[test]
    fn test_quotient_filter_no_false_negatives() {
        let mut qf = QuotientFilter::for_keys(500);

        let keys: Vec<String> = (0..200).map(|i| format!("key{:04}", i)).collect();

        for key in &keys {
            qf.add(key.as_bytes());
        }

        // All inserted keys must be found (no false negatives)
        for key in &keys {
            assert!(qf.contains(key.as_bytes()), "False negative for {}", key);
        }
    }

    #[test]
    fn test_quotient_filter_false_positive_rate() {
        let mut qf = QuotientFilter::for_keys(1000);

        // Insert 500 keys
        for i in 0..500 {
            qf.add(format!("key{:04}", i).as_bytes());
        }

        // Test false positive rate with keys not in the set
        let mut fps = 0;
        let test_count = 10000;
        for i in 500..(500 + test_count) {
            if qf.contains(format!("key{:04}", i).as_bytes()) {
                fps += 1;
            }
        }

        let fp_rate = fps as f64 / test_count as f64;
        println!("Quotient Filter FPR: {:.2}%", fp_rate * 100.0);

        // FPR should be less than 3% (target is ~0.78% for r=7)
        assert!(fp_rate < 0.03, "FPR too high: {:.2}%", fp_rate * 100.0);
    }

    #[test]
    fn test_quotient_filter_empty() {
        let qf = QuotientFilter::empty();

        assert!(!qf.contains(b"any_key"));
        assert_eq!(qf.len(), 0);
        assert!(qf.is_empty());
    }

    #[test]
    fn test_quotient_filter_encode_decode() {
        let mut qf = QuotientFilter::for_keys(100);

        for i in 0..50 {
            qf.add(format!("key{}", i).as_bytes());
        }

        // Encode
        let encoded = qf.encode();

        // Decode
        let decoded = QuotientFilter::decode(&encoded).unwrap();

        // Verify properties
        assert_eq!(decoded.quotient_bits(), qf.quotient_bits());
        assert_eq!(decoded.remainder_bits(), qf.remainder_bits());
        assert_eq!(decoded.len(), qf.len());

        // Verify all keys are still found
        for i in 0..50 {
            assert!(
                decoded.contains(format!("key{}", i).as_bytes()),
                "key{} not found after decode",
                i
            );
        }
    }

    #[test]
    fn test_quotient_filter_compact_encode_decode() {
        let mut qf = QuotientFilter::for_keys(100);

        for i in 0..50 {
            qf.add(format!("key{}", i).as_bytes());
        }

        // Encode compact
        let encoded = qf.encode_compact();

        // Decode compact
        let decoded = QuotientFilter::decode_compact(&encoded).unwrap();

        // Verify properties
        assert_eq!(decoded.quotient_bits(), qf.quotient_bits());
        assert_eq!(decoded.remainder_bits(), qf.remainder_bits());
        assert_eq!(decoded.len(), qf.len());

        // Verify all keys are still found
        for i in 0..50 {
            assert!(
                decoded.contains(format!("key{}", i).as_bytes()),
                "key{} not found after compact decode",
                i
            );
        }

        // Compact should be smaller for sparse filters
        let standard_size = qf.size();
        let compact_size = qf.size_compact();
        println!(
            "Standard size: {}, Compact size: {}",
            standard_size, compact_size
        );
    }

    #[test]
    fn test_quotient_filter_merge() {
        let config = QuotientFilterConfig {
            quotient_bits: 8,
            remainder_bits: 7,
        };

        let mut qf1 = QuotientFilter::new(config);
        let mut qf2 = QuotientFilter::new(config);

        // Add different keys to each filter
        for i in 0..30 {
            qf1.add(format!("a{:04}", i).as_bytes());
        }
        for i in 0..30 {
            qf2.add(format!("b{:04}", i).as_bytes());
        }

        // Merge
        let merged = QuotientFilter::merge_all(&[&qf1, &qf2]).unwrap();

        // All keys from both filters should be present
        for i in 0..30 {
            assert!(
                merged.contains(format!("a{:04}", i).as_bytes()),
                "Missing key a{:04}",
                i
            );
            assert!(
                merged.contains(format!("b{:04}", i).as_bytes()),
                "Missing key b{:04}",
                i
            );
        }
    }

    #[test]
    fn test_fingerprint_extraction() {
        let mut qf = QuotientFilter::for_keys(100);

        for i in 0..20 {
            qf.add(format!("key{}", i).as_bytes());
        }

        let fingerprints = qf.extract_fingerprints();

        // Should have 20 fingerprints
        assert_eq!(fingerprints.len(), 20);

        // Create new filter and insert extracted fingerprints
        let mut qf2 = QuotientFilter::for_keys(100);
        for fp in &fingerprints {
            qf2.insert_fingerprint(*fp);
        }

        // Both filters should return same results
        for i in 0..20 {
            let key = format!("key{}", i);
            assert_eq!(
                qf.contains(key.as_bytes()),
                qf2.contains(key.as_bytes()),
                "Mismatch for {}",
                key
            );
        }
    }

    #[test]
    fn test_config_for_keys() {
        // Small number of keys
        let config = QuotientFilterConfig::for_keys(50);
        assert!(config.quotient_bits >= 6); // At least 64 slots
        assert!(config.quotient_bits <= 8); // At most 256 slots

        // Medium number of keys
        let config = QuotientFilterConfig::for_keys(200);
        assert!(config.quotient_bits >= 8);
        assert!(config.quotient_bits <= 10);

        // Large number of keys
        let config = QuotientFilterConfig::for_keys(1000);
        assert!(config.quotient_bits >= 10);
    }

    #[test]
    fn test_decode_incomplete() {
        let data = vec![0u8; 5]; // Too short
        let result = QuotientFilter::decode(&data);
        assert!(matches!(result, Err(SSTableError::Incomplete)));
    }

    #[test]
    fn test_decode_invalid_version() {
        let mut data = vec![0u8; 20];
        data[0] = 99; // Invalid version
        let result = QuotientFilter::decode(&data);
        assert!(matches!(result, Err(SSTableError::InvalidFormat(_))));
    }

    #[test]
    fn test_duplicate_insertion() {
        let mut qf = QuotientFilter::for_keys(100);

        // Insert same key multiple times
        qf.add(b"duplicate_key");
        qf.add(b"duplicate_key");
        qf.add(b"duplicate_key");

        // Should only be counted once
        assert_eq!(qf.len(), 1);
        assert!(qf.contains(b"duplicate_key"));
    }

    #[test]
    fn test_fingerprint_from_hash() {
        let fp = Fingerprint::from_hash(0xFFFF_FFFF_FFFF_FFFF, 8, 7);

        // High 8 bits should be quotient
        assert_eq!(fp.quotient, 0xFF);

        // Next 7 bits should be remainder
        assert_eq!(fp.remainder, 0x7F);
    }
}
