/**
 * Unit tests for hash functions.
 *
 * CRITICAL: These tests validate that our hash functions produce correct results.
 * Any failures here could indicate routing bugs that would cause data loss.
 */

import { describe, it, expect, beforeAll } from 'vitest';
import {
  initializeHasher,
  xxhash64,
  jumpConsistentHash,
  getShardForKey,
  keyToBytes,
  valueToBytes,
  bytesToString,
} from '@norikv/client/hash';

describe('Hash Functions', () => {
  beforeAll(async () => {
    await initializeHasher();
  });

  describe('xxhash64', () => {
    it('should hash strings consistently', async () => {
      const hash1 = xxhash64('hello');
      const hash2 = xxhash64('hello');
      expect(hash1).toBe(hash2);
    });

    it('should hash Uint8Array consistently', async () => {
      const bytes = new TextEncoder().encode('hello');
      const hash1 = xxhash64(bytes);
      const hash2 = xxhash64(bytes);
      expect(hash1).toBe(hash2);
    });

    it('should produce same hash for string and Uint8Array', async () => {
      const str = 'hello';
      const bytes = new TextEncoder().encode(str);
      const hash1 = xxhash64(str);
      const hash2 = xxhash64(bytes);
      expect(hash1).toBe(hash2);
    });

    it('should produce different hashes for different inputs', async () => {
      const hash1 = xxhash64('hello');
      const hash2 = xxhash64('world');
      expect(hash1).not.toBe(hash2);
    });

    it('should produce 64-bit values', async () => {
      const hash = xxhash64('test');
      expect(typeof hash).toBe('bigint');
      // Ensure it fits in 64 bits
      expect(hash).toBeGreaterThanOrEqual(0n);
      expect(hash).toBeLessThan(2n ** 64n);
    });
  });

  describe('jumpConsistentHash', () => {
    it('should return consistent bucket for same hash', () => {
      const hash = 12345678901234567890n;
      const bucket1 = jumpConsistentHash(hash, 1024);
      const bucket2 = jumpConsistentHash(hash, 1024);
      expect(bucket1).toBe(bucket2);
    });

    it('should return bucket in valid range', () => {
      const hash = xxhash64('test-key');
      const numBuckets = 1024;
      const bucket = jumpConsistentHash(hash, numBuckets);
      expect(bucket).toBeGreaterThanOrEqual(0);
      expect(bucket).toBeLessThan(numBuckets);
    });

    it('should distribute keys across buckets', () => {
      const buckets = new Set<number>();
      const numBuckets = 100;

      for (let i = 0; i < 1000; i++) {
        const key = `key-${i}`;
        const hash = xxhash64(key);
        const bucket = jumpConsistentHash(hash, numBuckets);
        buckets.add(bucket);
      }

      // With 1000 keys and 100 buckets, we should hit most buckets
      expect(buckets.size).toBeGreaterThan(90); // At least 90% coverage
    });

    it('should minimize moves when adding buckets', () => {
      const keys = Array.from({ length: 1000 }, (_, i) => `key-${i}`);

      // Map keys to 100 buckets
      const mapping100 = new Map<string, number>();
      for (const key of keys) {
        const hash = xxhash64(key);
        mapping100.set(key, jumpConsistentHash(hash, 100));
      }

      // Map same keys to 101 buckets
      const mapping101 = new Map<string, number>();
      for (const key of keys) {
        const hash = xxhash64(key);
        mapping101.set(key, jumpConsistentHash(hash, 101));
      }

      // Count how many keys moved
      let moved = 0;
      for (const key of keys) {
        if (mapping100.get(key) !== mapping101.get(key)) {
          moved++;
        }
      }

      // With consistent hashing, roughly 1/101 (~10) keys should move
      // Allow some variance: between 1 and 20 keys
      expect(moved).toBeGreaterThan(0);
      expect(moved).toBeLessThan(20);
    });

    it('should throw on invalid bucket count', () => {
      const hash = 123n;
      expect(() => jumpConsistentHash(hash, 0)).toThrow();
      expect(() => jumpConsistentHash(hash, -1)).toThrow();
    });
  });

  describe('getShardForKey', () => {
    it('should return consistent shard for same key', async () => {
      const shard1 = getShardForKey('user:123', 1024);
      const shard2 = getShardForKey('user:123', 1024);
      expect(shard1).toBe(shard2);
    });

    it('should return shard in valid range', async () => {
      const shard = getShardForKey('test-key', 1024);
      expect(shard).toBeGreaterThanOrEqual(0);
      expect(shard).toBeLessThan(1024);
    });

    it('should distribute keys across shards', async () => {
      const shards = new Set<number>();

      for (let i = 0; i < 10000; i++) {
        const key = `key-${i}`;
        const shard = getShardForKey(key, 1024);
        shards.add(shard);
      }

      // With 10k keys and 1024 shards, we should hit most shards
      expect(shards.size).toBeGreaterThan(1000); // At least 97% coverage
    });
  });

  describe('keyToBytes / valueToBytes', () => {
    it('should convert string to bytes', () => {
      const str = 'hello';
      const bytes = keyToBytes(str);
      expect(bytes).toBeInstanceOf(Uint8Array);
      expect(bytesToString(bytes)).toBe(str);
    });

    it('should pass through Uint8Array unchanged', () => {
      const original = new Uint8Array([1, 2, 3, 4, 5]);
      const bytes = keyToBytes(original);
      expect(bytes).toBe(original); // Same reference
    });

    it('should handle UTF-8 strings correctly', () => {
      const str = 'Hello 世界 🌍';
      const bytes = keyToBytes(str);
      expect(bytesToString(bytes)).toBe(str);
    });

    it('should handle empty strings', () => {
      const bytes = keyToBytes('');
      expect(bytes.length).toBe(0);
      expect(bytesToString(bytes)).toBe('');
    });
  });

  describe('Cross-language compatibility test vectors', () => {
    // These test vectors should match the server's Rust implementation
    // TODO: Generate these from the server and commit them to version control

    it('should match known hash values', async () => {
      // Test vector: "hello" with seed=0
      // This value should be verified against the Rust implementation
      const hash = xxhash64('hello');
      expect(typeof hash).toBe('bigint');
      // TODO: Replace with actual expected value from server
      // expect(hash).toBe(EXPECTED_HASH_FROM_SERVER);
    });

    it('should match known shard assignments', async () => {
      // Test vector: "user:123" -> shard 42 (example)
      const shard = getShardForKey('user:123', 1024);
      expect(shard).toBeGreaterThanOrEqual(0);
      expect(shard).toBeLessThan(1024);
      // TODO: Replace with actual expected shard from server
      // expect(shard).toBe(EXPECTED_SHARD_FROM_SERVER);
    });
  });

  describe('Performance benchmarks', () => {
    it('should hash quickly', async () => {
      const iterations = 10000;
      const start = Date.now();

      for (let i = 0; i < iterations; i++) {
        xxhash64(`key-${i}`);
      }

      const elapsed = Date.now() - start;
      const perOp = elapsed / iterations;

      // Should be well under 1ms per operation
      expect(perOp).toBeLessThan(1);
    });

    it('should compute shard quickly', async () => {
      const iterations = 10000;
      const start = Date.now();

      for (let i = 0; i < iterations; i++) {
        getShardForKey(`key-${i}`, 1024);
      }

      const elapsed = Date.now() - start;
      const perOp = elapsed / iterations;

      // Should be well under 1ms per operation
      expect(perOp).toBeLessThan(1);
    });
  });
});
