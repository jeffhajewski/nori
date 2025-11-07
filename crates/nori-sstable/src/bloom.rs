//! Bloom filter for reducing disk I/O on non-existent key lookups.
//!
//! A Bloom filter is a space-efficient probabilistic data structure that tests
//! whether an element is a member of a set. False positives are possible (~0.9%
//! with 10 bits/key), but false negatives are not.
//!
//! # Hash Functions
//!
//! We use xxhash64 with seed variation to generate multiple hash values:
//! - h1 = xxhash64(key, seed=0)
//! - h2 = xxhash64(key, seed=1)
//! - h_i = h1 + i * h2  (double hashing)

use crate::error::{Result, SSTableError};
use bytes::{BufMut, Bytes, BytesMut};

/// Bloom filter for fast membership testing.
#[derive(Debug, Clone)]
pub struct BloomFilter {
    /// Bit array stored as bytes
    bits: Vec<u8>,
    /// Number of hash functions (k)
    num_hash_functions: u32,
    /// Total number of bits (m)
    num_bits: usize,
}

impl BloomFilter {
    /// Creates a new Bloom filter sized for the expected number of keys.
    ///
    /// # Parameters
    /// - `num_keys`: Expected number of keys to be added
    /// - `bits_per_key`: Bits allocated per key (default: 10 for ~0.9% FP rate)
    pub fn new(num_keys: usize, bits_per_key: usize) -> Self {
        // Calculate optimal number of bits: m = n * bits_per_key
        let target_bits = num_keys * bits_per_key;

        // Round up to next power of 2 for fast modulo via bitwise AND
        // This trades ~50% memory overhead for 30-60% performance improvement
        let num_bits = if target_bits == 0 {
            0
        } else {
            target_bits.next_power_of_two()
        };

        // Calculate optimal number of hash functions: k = ln(2) * m/n ≈ 0.693 * bits_per_key
        // For bits_per_key=10: k ≈ 7
        let num_hash_functions = ((bits_per_key as f64 * 0.693).ceil() as u32).max(1);

        // Allocate bit array (round up to byte boundary)
        let num_bytes = num_bits.div_ceil(8);
        let bits = vec![0u8; num_bytes];

        Self {
            bits,
            num_hash_functions,
            num_bits,
        }
    }

    /// Creates an empty Bloom filter (for testing or empty SSTables).
    pub fn empty() -> Self {
        Self {
            bits: vec![],
            num_hash_functions: 0,
            num_bits: 0,
        }
    }

    /// Adds a key to the Bloom filter.
    pub fn add(&mut self, key: &[u8]) {
        if self.num_bits == 0 {
            return; // Empty filter
        }

        let (h1, h2) = self.hash(key);

        for i in 0..self.num_hash_functions {
            let bit_pos = self.get_bit_position(h1, h2, i);
            self.set_bit(bit_pos);
        }
    }

    /// Tests whether a key might be in the set.
    ///
    /// Returns `true` if the key *might* be present (could be false positive).
    /// Returns `false` if the key is *definitely not* present (no false negatives).
    pub fn contains(&self, key: &[u8]) -> bool {
        if self.num_bits == 0 {
            return false; // Empty filter contains nothing
        }

        let (h1, h2) = self.hash(key);

        for i in 0..self.num_hash_functions {
            let bit_pos = self.get_bit_position(h1, h2, i);
            if !self.get_bit(bit_pos) {
                return false; // Definitely not present
            }
        }

        true // Might be present (could be false positive)
    }

    /// Computes two hash values using xxhash64 with different seeds.
    #[inline]
    fn hash(&self, key: &[u8]) -> (u64, u64) {
        let h1 = xxhash_rust::xxh64::xxh64(key, 0);
        let h2 = xxhash_rust::xxh64::xxh64(key, 1);
        (h1, h2)
    }

    /// Computes bit position using double hashing: (h1 + i * h2) mod m
    ///
    /// Uses bitwise AND for fast modulo (requires num_bits to be power of 2).
    #[inline]
    fn get_bit_position(&self, h1: u64, h2: u64, i: u32) -> usize {
        let hash = h1.wrapping_add((i as u64).wrapping_mul(h2));
        // Fast modulo via bitwise AND (num_bits is always power of 2)
        (hash & (self.num_bits as u64 - 1)) as usize
    }

    /// Sets a bit at the given position.
    #[inline]
    fn set_bit(&mut self, pos: usize) {
        let byte_index = pos >> 3; // Fast divide by 8
        let bit_index = pos & 7;   // Fast modulo 8
        self.bits[byte_index] |= 1 << bit_index;
    }

    /// Gets a bit at the given position.
    #[inline]
    fn get_bit(&self, pos: usize) -> bool {
        let byte_index = pos >> 3; // Fast divide by 8
        let bit_index = pos & 7;   // Fast modulo 8
        (self.bits[byte_index] & (1 << bit_index)) != 0
    }

    /// Encodes the Bloom filter to bytes for storage.
    ///
    /// Format:
    /// - num_hash_functions: u32
    /// - num_bits: u64
    /// - bit array: [u8]
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::new();

        buf.put_u32_le(self.num_hash_functions);
        buf.put_u64_le(self.num_bits as u64);
        buf.put_slice(&self.bits);

        buf.freeze()
    }

    /// Decodes a Bloom filter from bytes.
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() < 12 {
            return Err(SSTableError::Incomplete);
        }

        let num_hash_functions = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let num_bits = u64::from_le_bytes([
            data[4], data[5], data[6], data[7], data[8], data[9], data[10], data[11],
        ]) as usize;

        let expected_bytes = num_bits.div_ceil(8);
        if data.len() < 12 + expected_bytes {
            return Err(SSTableError::Incomplete);
        }

        let bits = data[12..12 + expected_bytes].to_vec();

        Ok(Self {
            bits,
            num_hash_functions,
            num_bits,
        })
    }

    /// Returns the size of the encoded Bloom filter in bytes.
    pub fn size(&self) -> usize {
        12 + self.bits.len()
    }

    /// Returns the number of bits in the filter.
    pub fn num_bits(&self) -> usize {
        self.num_bits
    }

    /// Returns the number of hash functions used.
    pub fn num_hash_functions(&self) -> u32 {
        self.num_hash_functions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{BLOOM_BITS_PER_KEY, BLOOM_FP_RATE};

    #[test]
    fn test_bloom_filter_basic() {
        let mut bloom = BloomFilter::new(100, BLOOM_BITS_PER_KEY);

        // Add some keys
        bloom.add(b"key1");
        bloom.add(b"key2");
        bloom.add(b"key3");

        // Keys that were added should be found
        assert!(bloom.contains(b"key1"));
        assert!(bloom.contains(b"key2"));
        assert!(bloom.contains(b"key3"));

        // Key that wasn't added should not be found (with high probability)
        // Note: There's a small chance of false positive
        assert!(!bloom.contains(b"key999"));
    }

    #[test]
    fn test_bloom_filter_false_positive_rate() {
        let num_keys = 1000;
        let mut bloom = BloomFilter::new(num_keys, BLOOM_BITS_PER_KEY);

        // Add keys
        for i in 0..num_keys {
            let key = format!("key{:04}", i);
            bloom.add(key.as_bytes());
        }

        // Verify all added keys are found (no false negatives)
        for i in 0..num_keys {
            let key = format!("key{:04}", i);
            assert!(
                bloom.contains(key.as_bytes()),
                "False negative for key{:04}",
                i
            );
        }

        // Test false positive rate with keys not in the set
        let test_count = 10000;
        let mut false_positives = 0;

        for i in num_keys..(num_keys + test_count) {
            let key = format!("key{:04}", i);
            if bloom.contains(key.as_bytes()) {
                false_positives += 1;
            }
        }

        let fp_rate = false_positives as f64 / test_count as f64;
        println!(
            "False positive rate: {:.2}% (target: {:.2}%)",
            fp_rate * 100.0,
            BLOOM_FP_RATE * 100.0
        );

        // False positive rate should be less than 2% (target is ~0.9%)
        assert!(
            fp_rate < 0.02,
            "False positive rate too high: {:.2}%",
            fp_rate * 100.0
        );
    }

    #[test]
    fn test_bloom_filter_empty() {
        let bloom = BloomFilter::empty();

        assert!(!bloom.contains(b"any_key"));
        assert_eq!(bloom.num_bits(), 0);
        assert_eq!(bloom.num_hash_functions(), 0);
    }

    #[test]
    fn test_bloom_filter_single_key() {
        let mut bloom = BloomFilter::new(1, BLOOM_BITS_PER_KEY);

        bloom.add(b"only_key");

        assert!(bloom.contains(b"only_key"));
    }

    #[test]
    fn test_bloom_filter_encode_decode() {
        let mut bloom = BloomFilter::new(100, BLOOM_BITS_PER_KEY);

        // Add some keys
        for i in 0..100 {
            let key = format!("key{}", i);
            bloom.add(key.as_bytes());
        }

        // Encode
        let encoded = bloom.encode();

        // Decode
        let decoded = BloomFilter::decode(&encoded).unwrap();

        // Verify properties match
        assert_eq!(decoded.num_bits(), bloom.num_bits());
        assert_eq!(decoded.num_hash_functions(), bloom.num_hash_functions());

        // Verify all keys are still found
        for i in 0..100 {
            let key = format!("key{}", i);
            assert!(decoded.contains(key.as_bytes()));
        }
    }

    #[test]
    fn test_bloom_filter_large_dataset() {
        let num_keys = 100_000;
        let mut bloom = BloomFilter::new(num_keys, BLOOM_BITS_PER_KEY);

        // Add many keys
        for i in 0..num_keys {
            let key = format!("key{:06}", i);
            bloom.add(key.as_bytes());
        }

        // Spot check some keys
        assert!(bloom.contains(b"key000000"));
        assert!(bloom.contains(b"key050000"));
        assert!(bloom.contains(b"key099999"));

        // Encode/decode still works
        let encoded = bloom.encode();
        let decoded = BloomFilter::decode(&encoded).unwrap();
        assert!(decoded.contains(b"key050000"));
    }

    #[test]
    fn test_bloom_filter_decode_incomplete() {
        let data = vec![0u8; 5]; // Too short
        let result = BloomFilter::decode(&data);
        assert!(matches!(result, Err(SSTableError::Incomplete)));
    }

    #[test]
    fn test_bloom_filter_optimal_hash_functions() {
        // For 10 bits per key, optimal k ≈ 7
        let bloom = BloomFilter::new(100, 10);
        assert_eq!(bloom.num_hash_functions(), 7);

        // For 20 bits per key, optimal k ≈ 14
        let bloom = BloomFilter::new(100, 20);
        assert_eq!(bloom.num_hash_functions(), 14);
    }
}
