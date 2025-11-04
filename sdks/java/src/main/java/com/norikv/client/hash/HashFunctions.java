package com.norikv.client.hash;

import net.jpountz.xxhash.XXHash64;
import net.jpountz.xxhash.XXHashFactory;

/**
 * Hashing utilities for NoriKV client.
 *
 * <p>CRITICAL: These hash functions must produce identical results to the server's
 * Rust implementation and other SDKs (Go, Python, TypeScript). Any deviation will
 * cause routing failures and data loss.
 *
 * <ul>
 * <li>xxhash64: Fast non-cryptographic hash (seed=0)</li>
 * <li>JumpConsistentHash: Minimal-disruption consistent hashing</li>
 * </ul>
 *
 * <p>Reference implementations:
 * <ul>
 * <li>Rust server: uses xxhash crate + jump_consistent_hash</li>
 * <li>Go SDK: github.com/cespare/xxhash/v2</li>
 * <li>Python SDK: xxhash + jump_consistent_hash</li>
 * <li>TypeScript SDK: xxhash-wasm + jumpConsistentHash</li>
 * </ul>
 */
public final class HashFunctions {

    private static final XXHash64 HASHER = XXHashFactory.fastestInstance().hash64();
    private static final long SEED = 0L;

    // Constants for Jump Consistent Hash
    private static final long JUMP_MULTIPLIER = 2862933555777941757L;
    private static final double TWO_POW_31 = 2147483648.0; // 2^31

    private HashFunctions() {
        // Utility class - prevent instantiation
    }

    /**
     * Computes xxhash64 of a key with seed=0.
     *
     * <p>This function MUST use seed=0 to match the server implementation.
     *
     * @param key the key to hash
     * @return the 64-bit hash value
     */
    public static long xxhash64(byte[] key) {
        return HASHER.hash(key, 0, key.length, SEED);
    }

    /**
     * Maps a 64-bit hash to a bucket in [0, numBuckets).
     *
     * <p>When numBuckets changes, only ~(1/numBuckets) keys move to different buckets.
     * This is the Jump Consistent Hash algorithm from the reference paper:
     * <a href="https://arxiv.org/abs/1406.2294">https://arxiv.org/abs/1406.2294</a>
     *
     * <p>This implementation MUST match the server's Rust implementation exactly:
     * <pre>
     * pub fn jump_consistent_hash(key: u64, num_buckets: u32) -> u32 {
     *     let mut b = -1i64;
     *     let mut j = 0i64;
     *     while j < num_buckets as i64 {
     *         b = j;
     *         let key = key.wrapping_mul(2862933555777941757u64).wrapping_add(1);
     *         j = ((b + 1) as f64 * (f64::from(1u32 << 31) / ((key >> 33) + 1) as f64)) as i64;
     *     }
     *     b as u32
     * }
     * </pre>
     *
     * @param key the hash value
     * @param numBuckets the number of buckets (must be > 0)
     * @return the bucket index in range [0, numBuckets)
     * @throws IllegalArgumentException if numBuckets <= 0
     */
    public static int jumpConsistentHash(long key, int numBuckets) {
        if (numBuckets <= 0) {
            throw new IllegalArgumentException("numBuckets must be > 0, got: " + numBuckets);
        }

        long b = -1;
        long j = 0;

        while (j < numBuckets) {
            b = j;

            // key = key * 2862933555777941757 + 1 (wrapping arithmetic)
            key = key * JUMP_MULTIPLIER + 1;

            // j = (b + 1) * (2^31 / ((key >> 33) + 1))
            // Note: >>> is unsigned right shift in Java
            long divisor = (key >>> 33) + 1;
            double numerator = (b + 1) * TWO_POW_31;
            j = (long) (numerator / divisor);
        }

        return (int) b;
    }

    /**
     * Computes the shard for a given key.
     *
     * <p>This combines xxhash64 and jumpConsistentHash to route a key to a shard.
     *
     * @param key the key to route
     * @param totalShards the total number of shards
     * @return the shard ID in range [0, totalShards)
     */
    public static int getShardForKey(byte[] key, int totalShards) {
        long hash = xxhash64(key);
        return jumpConsistentHash(hash, totalShards);
    }

    /**
     * Converts a string key to bytes using UTF-8 encoding.
     *
     * <p>This is a convenience function for consistency with other SDKs.
     *
     * @param key the string key
     * @return the UTF-8 bytes
     */
    public static byte[] keyToBytes(String key) {
        return key.getBytes(java.nio.charset.StandardCharsets.UTF_8);
    }

    /**
     * Converts a string value to bytes using UTF-8 encoding.
     *
     * <p>This is a convenience function for consistency with other SDKs.
     *
     * @param value the string value
     * @return the UTF-8 bytes
     */
    public static byte[] valueToBytes(String value) {
        return value.getBytes(java.nio.charset.StandardCharsets.UTF_8);
    }

    /**
     * Converts bytes to a string using UTF-8 encoding.
     *
     * <p>This is a convenience function for consistency with other SDKs.
     *
     * @param data the bytes to convert
     * @return the UTF-8 string
     */
    public static String bytesToString(byte[] data) {
        return new String(data, java.nio.charset.StandardCharsets.UTF_8);
    }
}
