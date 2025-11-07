//! Shard placement and consistent hashing for NoriKV.
//!
//! This module implements:
//! - xxhash64 with seed=0 for key hashing
//! - Jump Consistent Hash for minimal-disruption shard assignment
//!
//! CRITICAL: These implementations must match the client SDKs exactly!

use std::hash::Hasher;
use twox_hash::XxHash64;

/// Compute xxhash64 of a key with seed=0.
///
/// This MUST produce identical results to the TypeScript SDK implementation.
///
/// # Examples
///
/// ```
/// use norikv_placement::xxhash64;
///
/// let hash = xxhash64(b"hello");
/// assert_eq!(hash, xxhash64(b"hello")); // Deterministic
/// ```
pub fn xxhash64(key: &[u8]) -> u64 {
    let mut hasher = XxHash64::with_seed(0);
    hasher.write(key);
    hasher.finish()
}

/// Jump Consistent Hash - minimal-disruption consistent hashing.
///
/// Maps a 64-bit hash to a bucket in [0, num_buckets).
/// When num_buckets changes, only ~(1/num_buckets) keys move.
///
/// Reference: https://arxiv.org/abs/1406.2294
///
/// This MUST match the TypeScript implementation exactly!
///
/// # Algorithm
///
/// ```text
/// let mut b = -1;
/// let mut j = 0;
/// while j < num_buckets {
///     b = j;
///     key = key * 2862933555777941757 + 1;
///     j = (b + 1) * (2^31 / ((key >> 33) + 1))
/// }
/// return b;
/// ```
///
/// # Examples
///
/// ```
/// use norikv_placement::jump_consistent_hash;
///
/// let hash = 12345678901234567890u64;
/// let shard = jump_consistent_hash(hash, 1024);
/// assert!(shard < 1024);
/// ```
pub fn jump_consistent_hash(mut key: u64, num_buckets: u32) -> u32 {
    let mut b: i64 = -1;
    let mut j: i64 = 0;

    while j < num_buckets as i64 {
        b = j;
        key = key.wrapping_mul(2862933555777941757).wrapping_add(1);
        j = ((b + 1) as f64 * (f64::from(1u32 << 31) / ((key >> 33) + 1) as f64)) as i64;
    }

    b as u32
}

/// Compute the shard for a given key.
///
/// This combines xxhash64 and jump_consistent_hash to route a key to a shard.
///
/// # Examples
///
/// ```
/// use norikv_placement::get_shard_for_key;
///
/// let shard = get_shard_for_key(b"user:12345", 1024);
/// assert!(shard < 1024);
/// ```
pub fn get_shard_for_key(key: &[u8], total_shards: u32) -> u32 {
    let hash = xxhash64(key);
    jump_consistent_hash(hash, total_shards)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xxhash64_consistency() {
        let key = b"hello";
        let hash1 = xxhash64(key);
        let hash2 = xxhash64(key);
        assert_eq!(hash1, hash2, "xxhash64 should be deterministic");
    }

    #[test]
    fn test_xxhash64_different_keys() {
        let hash1 = xxhash64(b"hello");
        let hash2 = xxhash64(b"world");
        assert_ne!(hash1, hash2, "Different keys should have different hashes");
    }

    #[test]
    fn test_jump_consistent_hash_in_range() {
        let hash = 12345678901234567890u64;
        let num_buckets = 1024;
        let bucket = jump_consistent_hash(hash, num_buckets);
        assert!(bucket < num_buckets, "Bucket should be in valid range");
    }

    #[test]
    fn test_jump_consistent_hash_consistency() {
        let hash = 12345678901234567890u64;
        let num_buckets = 1024;
        let bucket1 = jump_consistent_hash(hash, num_buckets);
        let bucket2 = jump_consistent_hash(hash, num_buckets);
        assert_eq!(bucket1, bucket2, "Jump hash should be deterministic");
    }

    #[test]
    fn test_jump_consistent_hash_distribution() {
        use std::collections::HashSet;
        let mut buckets = HashSet::new();
        let num_buckets = 100;

        for i in 0u64..1000 {
            let hash = xxhash64(&i.to_le_bytes());
            let bucket = jump_consistent_hash(hash, num_buckets);
            buckets.insert(bucket);
        }

        // With 1000 keys and 100 buckets, we should hit most buckets
        assert!(
            buckets.len() > 90,
            "Expected > 90 unique buckets, got {}",
            buckets.len()
        );
    }

    #[test]
    fn test_jump_consistent_hash_minimal_move() {
        // When adding one bucket, only ~1% of keys should move
        let num_keys = 1000;
        let old_buckets = 100;
        let new_buckets = 101;

        let mut moved = 0;
        for i in 0u64..num_keys {
            let hash = xxhash64(&i.to_le_bytes());
            let old_bucket = jump_consistent_hash(hash, old_buckets);
            let new_bucket = jump_consistent_hash(hash, new_buckets);
            if old_bucket != new_bucket {
                moved += 1;
            }
        }

        // Should be roughly 1/101 ≈ 10 keys, allow some variance
        assert!(
            moved > 0 && moved < 20,
            "Expected 1-20 keys to move, got {}",
            moved
        );
    }

    #[test]
    fn test_get_shard_for_key() {
        let shard1 = get_shard_for_key(b"user:123", 1024);
        let shard2 = get_shard_for_key(b"user:123", 1024);
        assert_eq!(shard1, shard2, "Same key should map to same shard");
        assert!(shard1 < 1024, "Shard should be in valid range");
    }
}
