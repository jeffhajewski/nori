/**
 * Hashing utilities for NoriKV client.
 *
 * CRITICAL: These hash functions must produce identical results to the server's
 * Rust implementation. Any deviation will cause routing failures.
 *
 * - xxhash64: Fast non-cryptographic hash (seed=0)
 * - jumpConsistentHash: Minimal-disruption consistent hashing
 */

import xxhash from 'xxhash-wasm';
import type { XXHashAPI } from 'xxhash-wasm';

let xxhashInstance: XXHashAPI | null = null;

/**
 * Initialize the xxhash WASM module.
 * Must be called before using xxhash64().
 */
export async function initializeHasher(): Promise<void> {
  if (!xxhashInstance) {
    xxhashInstance = await xxhash();
  }
}

/**
 * Compute xxhash64 of a key with seed=0.
 *
 * @param key - The key to hash (Uint8Array or string)
 * @returns 64-bit hash as bigint
 *
 * @example
 * ```ts
 * await initializeHasher();
 * const hash = xxhash64('my-key');
 * // hash: 12345678901234567890n
 * ```
 */
export function xxhash64(key: Uint8Array | string): bigint {
  if (!xxhashInstance) {
    throw new Error('xxhash not initialized. Call initializeHasher() first.');
  }

  const keyBytes = typeof key === 'string'
    ? new TextEncoder().encode(key)
    : key;

  // Use seed=0 to match server implementation
  return xxhashInstance.h64Raw(keyBytes, 0n);
}

/**
 * Jump Consistent Hash - minimal-disruption consistent hashing.
 *
 * Maps a 64-bit hash to a bucket in [0, numBuckets).
 * When numBuckets changes, only ~(1/numBuckets) keys move.
 *
 * Reference: https://arxiv.org/abs/1406.2294
 *
 * This implementation MUST match the server's exactly:
 * ```rust
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
 * ```
 *
 * @param hash - 64-bit hash value
 * @param numBuckets - Number of buckets (must be > 0)
 * @returns Bucket index in [0, numBuckets)
 *
 * @example
 * ```ts
 * const hash = 12345678901234567890n;
 * const shard = jumpConsistentHash(hash, 1024);
 * // shard: 42
 * ```
 */
export function jumpConsistentHash(hash: bigint, numBuckets: number): number {
  if (numBuckets <= 0) {
    throw new Error(`numBuckets must be > 0, got ${numBuckets}`);
  }

  let key = BigInt.asUintN(64, hash);
  let b = -1;
  let j = 0;

  while (j < numBuckets) {
    b = j;

    // key = key * 2862933555777941757 + 1
    key = BigInt.asUintN(64,
      key * 2862933555777941757n + 1n
    );

    // j = (b + 1) * (2^31 / ((key >> 33) + 1))
    const divisor = Number((key >> 33n) + 1n);
    const numerator = (b + 1) * 2147483648; // 2^31
    j = Math.floor(numerator / divisor);
  }

  return b;
}

/**
 * Compute the shard for a given key.
 *
 * This combines xxhash64 and jumpConsistentHash to route a key to a shard.
 *
 * @param key - The key to route
 * @param totalShards - Total number of shards (default: 1024)
 * @returns Shard ID in [0, totalShards)
 *
 * @example
 * ```ts
 * await initializeHasher();
 * const shardId = getShardForKey('user:12345', 1024);
 * // shardId: 42
 * ```
 */
export function getShardForKey(
  key: Uint8Array | string,
  totalShards: number = 1024
): number {
  const hash = xxhash64(key);
  return jumpConsistentHash(hash, totalShards);
}

/**
 * Convert a key (string or Uint8Array) to bytes.
 */
export function keyToBytes(key: Uint8Array | string): Uint8Array {
  return typeof key === 'string'
    ? new TextEncoder().encode(key)
    : key;
}

/**
 * Convert a value (string or Uint8Array) to bytes.
 */
export function valueToBytes(value: Uint8Array | string): Uint8Array {
  return typeof value === 'string'
    ? new TextEncoder().encode(value)
    : value;
}

/**
 * Convert bytes to string (UTF-8 decoding).
 */
export function bytesToString(bytes: Uint8Array): string {
  return new TextDecoder().decode(bytes);
}
