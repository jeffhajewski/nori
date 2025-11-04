// Package hash provides hashing utilities for NoriKV client.
//
// CRITICAL: These hash functions must produce identical results to the server's
// Rust implementation and other SDKs (Python, TypeScript). Any deviation will
// cause routing failures and data loss.
//
// - xxhash64: Fast non-cryptographic hash (seed=0)
// - JumpConsistentHash: Minimal-disruption consistent hashing
//
// Reference implementations:
// - Rust server: uses xxhash crate + jump_consistent_hash
// - Python SDK: xxhash + jump_consistent_hash
// - TypeScript SDK: xxhash-wasm + jumpConsistentHash
package hash

import (
	"github.com/cespare/xxhash/v2"
)

// XXHash64 computes xxhash64 of a key with seed=0.
//
// This function MUST use seed=0 to match the server implementation.
//
// Example:
//
//	hashValue := XXHash64([]byte("my-key"))
func XXHash64(key []byte) uint64 {
	// Use seed=0 to match server implementation
	return xxhash.Sum64(key)
}

// JumpConsistentHash maps a 64-bit hash to a bucket in [0, numBuckets).
//
// When numBuckets changes, only ~(1/numBuckets) keys move to different buckets.
// This is the Jump Consistent Hash algorithm from the reference paper:
// https://arxiv.org/abs/1406.2294
//
// This implementation MUST match the server's Rust implementation exactly:
//
//	pub fn jump_consistent_hash(key: u64, num_buckets: u32) -> u32 {
//	    let mut b = -1i64;
//	    let mut j = 0i64;
//	    while j < num_buckets as i64 {
//	        b = j;
//	        let key = key.wrapping_mul(2862933555777941757u64).wrapping_add(1);
//	        j = ((b + 1) as f64 * (f64::from(1u32 << 31) / ((key >> 33) + 1) as f64)) as i64;
//	    }
//	    b as u32
//	}
//
// Example:
//
//	hash := uint64(12345678901234567890)
//	bucket := JumpConsistentHash(hash, 1024)
func JumpConsistentHash(key uint64, numBuckets int) int {
	if numBuckets <= 0 {
		panic("numBuckets must be > 0")
	}

	var b int64 = -1
	var j int64 = 0

	for j < int64(numBuckets) {
		b = j

		// key = key * 2862933555777941757 + 1 (wrapping)
		key = key*2862933555777941757 + 1

		// j = (b + 1) * (2^31 / ((key >> 33) + 1))
		divisor := float64((key >> 33) + 1)
		numerator := float64(b+1) * 2147483648.0 // 2^31
		j = int64(numerator / divisor)
	}

	return int(b)
}

// GetShardForKey computes the shard for a given key.
//
// This combines XXHash64 and JumpConsistentHash to route a key to a shard.
//
// Example:
//
//	shardID := GetShardForKey([]byte("user:12345"), 1024)
func GetShardForKey(key []byte, totalShards int) int {
	hash := XXHash64(key)
	return JumpConsistentHash(hash, totalShards)
}

// KeyToBytes converts a string key to bytes.
// This is a convenience function for consistency with other SDKs.
func KeyToBytes(key string) []byte {
	return []byte(key)
}

// ValueToBytes converts a string value to bytes.
// This is a convenience function for consistency with other SDKs.
func ValueToBytes(value string) []byte {
	return []byte(value)
}

// BytesToString converts bytes to a string.
// This is a convenience function for consistency with other SDKs.
func BytesToString(data []byte) string {
	return string(data)
}
