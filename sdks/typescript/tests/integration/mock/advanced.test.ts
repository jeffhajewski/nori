/**
 * Integration tests for advanced features using mock server.
 *
 * Tests TTL, conditional operations, consistency levels, and other advanced functionality.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { NoriKVClient } from '@norikv/client/client';
import { MockNoriKVServer, createMockServer } from '../helpers/mock-server';
import {
  expectKeyExists,
  expectKeyNotExists,
  expectValidVersion,
  expectVersionsEqual,
  sleep,
  waitFor,
  randomKey,
  randomValue,
} from '../helpers/assertions';

describe('Advanced Features (Mock)', () => {
  let server: MockNoriKVServer;
  let client: NoriKVClient;

  beforeEach(async () => {
    server = await createMockServer();
    client = new NoriKVClient({
      nodes: [server.getAddress()],
      watchCluster: false,
    });
    await client.connect();
  });

  afterEach(async () => {
    await client.close();
    await server.stop();
  });

  describe('TTL (Time-To-Live)', () => {
    it('should expire keys after TTL', async () => {
      const key = randomKey('ttl');
      const value = randomValue();
      const ttlMs = 500; // 500ms TTL

      await client.put(key, value, { ttlMs });

      // Should exist immediately
      await expectKeyExists(client, key, value);

      // Wait for expiration
      await sleep(ttlMs + 100);

      // Should be expired
      await expectKeyNotExists(client, key);
    });

    it('should support zero TTL (no expiration)', async () => {
      const key = randomKey('no-ttl');
      const value = randomValue();

      await client.put(key, value, { ttlMs: 0 });

      await expectKeyExists(client, key, value);

      // Wait and verify still exists
      await sleep(1000);
      await expectKeyExists(client, key, value);
    });

    it('should support long TTL values', async () => {
      const key = randomKey('long-ttl');
      const value = randomValue();
      const ttlMs = 3600000; // 1 hour

      await client.put(key, value, { ttlMs });

      // Should exist
      await expectKeyExists(client, key, value);
    });

    it('should handle TTL on updated keys', async () => {
      const key = randomKey('ttl-update');
      const value1 = 'first';
      const value2 = 'second';

      // Put with short TTL
      await client.put(key, value1, { ttlMs: 5000 });

      // Update with new TTL
      await client.put(key, value2, { ttlMs: 10000 });

      // Should have new value and new TTL
      await expectKeyExists(client, key, value2);
    });

    it('should not affect non-expired keys', async () => {
      const expireKey = randomKey('expire');
      const keepKey = randomKey('keep');
      const value = randomValue();

      await client.put(expireKey, value, { ttlMs: 500 });
      await client.put(keepKey, value); // No TTL

      await sleep(600);

      await expectKeyNotExists(client, expireKey);
      await expectKeyExists(client, keepKey, value);
    });
  });

  describe('Conditional Put (ifNotExists)', () => {
    it('should succeed when key does not exist', async () => {
      const key = randomKey('if-not-exists');
      const value = randomValue();

      const version = await client.put(key, value, { ifNotExists: true });

      expectValidVersion(version);
      await expectKeyExists(client, key, value);
    });

    it('should fail when key already exists', async () => {
      const key = randomKey('exists');
      const value1 = 'first';
      const value2 = 'second';

      // First put succeeds
      await client.put(key, value1);

      // Second put with ifNotExists should fail
      await expect(
        client.put(key, value2, { ifNotExists: true })
      ).rejects.toThrow();

      // Original value should remain
      await expectKeyExists(client, key, value1);
    });

    it('should succeed after delete', async () => {
      const key = randomKey('delete-recreate');
      const value1 = 'first';
      const value2 = 'second';

      await client.put(key, value1);
      await client.delete(key);

      // Should succeed since key was deleted
      await client.put(key, value2, { ifNotExists: true });

      await expectKeyExists(client, key, value2);
    });
  });

  describe('Conditional Put (ifMatchVersion)', () => {
    it('should succeed when version matches', async () => {
      const key = randomKey('version-match');
      const value1 = 'first';
      const value2 = 'second';

      const version1 = await client.put(key, value1);

      // Update with correct version
      const version2 = await client.put(key, value2, {
        ifMatchVersion: version1,
      });

      expectValidVersion(version2);
      expect(version2.index).toBeGreaterThan(version1.index);

      await expectKeyExists(client, key, value2);
    });

    it('should fail when version does not match', async () => {
      const key = randomKey('version-mismatch');
      const value1 = 'first';
      const value2 = 'second';
      const value3 = 'third';

      const version1 = await client.put(key, value1);
      await client.put(key, value2); // Update without condition

      // Try to update with old version
      await expect(
        client.put(key, value3, { ifMatchVersion: version1 })
      ).rejects.toThrow();

      // Should have value2
      await expectKeyExists(client, key, value2);
    });

    it('should support optimistic locking pattern', async () => {
      const key = randomKey('optimistic-lock');
      const initialValue = 'initial';

      await client.put(key, initialValue);

      // Multiple clients trying to update
      const update1 = async () => {
        const result = await client.get(key);
        await sleep(50);
        return client.put(key, 'update-1', {
          ifMatchVersion: result.version!,
        });
      };

      const update2 = async () => {
        const result = await client.get(key);
        await sleep(50);
        return client.put(key, 'update-2', {
          ifMatchVersion: result.version!,
        });
      };

      // One should succeed, one should fail
      const results = await Promise.allSettled([update1(), update2()]);

      const succeeded = results.filter((r) => r.status === 'fulfilled');
      const failed = results.filter((r) => r.status === 'rejected');

      expect(succeeded.length).toBe(1);
      expect(failed.length).toBe(1);
    });
  });

  describe('Conditional Delete (ifMatchVersion)', () => {
    it('should delete when version matches', async () => {
      const key = randomKey('delete-match');
      const value = randomValue();

      const version = await client.put(key, value);

      const deleted = await client.delete(key, { ifMatchVersion: version });

      expect(deleted).toBe(true);
      await expectKeyNotExists(client, key);
    });

    it('should fail when version does not match', async () => {
      const key = randomKey('delete-mismatch');
      const value1 = 'first';
      const value2 = 'second';

      const version1 = await client.put(key, value1);
      await client.put(key, value2);

      await expect(
        client.delete(key, { ifMatchVersion: version1 })
      ).rejects.toThrow();

      // Should still exist
      await expectKeyExists(client, key, value2);
    });
  });

  describe('Consistency Levels', () => {
    it('should support lease consistency (default)', async () => {
      const key = randomKey('lease');
      const value = randomValue();

      await client.put(key, value);

      const result = await client.get(key, { consistency: 'lease' });

      expect(result.value).not.toBeNull();
      expectValidVersion(result.version);
    });

    it('should support linearizable consistency', async () => {
      const key = randomKey('linearizable');
      const value = randomValue();

      await client.put(key, value);

      const result = await client.get(key, { consistency: 'linearizable' });

      expect(result.value).not.toBeNull();
      expectValidVersion(result.version);
    });

    it('should support stale_ok consistency', async () => {
      const key = randomKey('stale');
      const value = randomValue();

      await client.put(key, value);

      const result = await client.get(key, { consistency: 'stale_ok' });

      expect(result.value).not.toBeNull();
      expectValidVersion(result.version);
    });

    it('should return same value across consistency levels', async () => {
      const key = randomKey('consistency-compare');
      const value = randomValue();

      await client.put(key, value);

      const leaseResult = await client.get(key, { consistency: 'lease' });
      const linearizableResult = await client.get(key, {
        consistency: 'linearizable',
      });
      const staleResult = await client.get(key, { consistency: 'stale_ok' });

      // All should return the same value
      expect(leaseResult.value).toEqual(linearizableResult.value);
      expect(leaseResult.value).toEqual(staleResult.value);
    });

    it('should use lease consistency by default', async () => {
      const key = randomKey('default-consistency');
      const value = randomValue();

      await client.put(key, value);

      const result = await client.get(key); // No consistency specified

      expect(result.value).not.toBeNull();
    });
  });

  describe('Idempotency Keys', () => {
    it('should support idempotent puts', async () => {
      const key = randomKey('idempotent-put');
      const value = randomValue();
      const idempotencyKey = 'test-idempotency-1';

      const version1 = await client.put(key, value, { idempotencyKey });
      const version2 = await client.put(key, value, { idempotencyKey });

      // Versions should be identical
      expectVersionsEqual(version1, version2);
    });

    it('should support idempotent deletes', async () => {
      const key = randomKey('idempotent-delete');
      const value = randomValue();
      const idempotencyKey = 'test-delete-idempotency-1';

      await client.put(key, value);

      const deleted1 = await client.delete(key, { idempotencyKey });
      const deleted2 = await client.delete(key, { idempotencyKey });

      expect(deleted1).toBe(true);
      expect(deleted2).toBe(true);
    });

    it('should differentiate different idempotency keys', async () => {
      const key = randomKey('different-idempotency');
      const value = randomValue();

      const version1 = await client.put(key, value, {
        idempotencyKey: 'key-1',
      });
      const version2 = await client.put(key, value, {
        idempotencyKey: 'key-2',
      });

      // Different idempotency keys should produce different versions
      expect(version2.index).toBeGreaterThan(version1.index);
    });
  });

  describe('Metadata', () => {
    it('should return metadata with get results', async () => {
      const key = randomKey('metadata');
      const value = randomValue();

      await client.put(key, value);

      const result = await client.get(key);

      expect(result.metadata).toBeDefined();
    });

    it('should include server metadata', async () => {
      const key = randomKey('server-metadata');
      const value = randomValue();

      await client.put(key, value);

      const result = await client.get(key);

      // Metadata object should exist (even if empty)
      expect(typeof result.metadata).toBe('object');
    });
  });

  describe('Large Scale Operations', () => {
    it('should handle many concurrent operations', async () => {
      const operations = Array.from({ length: 100 }, (_, i) => ({
        key: randomKey(`concurrent-${i}`),
        value: randomValue(),
      }));

      // Concurrent puts
      const versions = await Promise.all(
        operations.map((op) => client.put(op.key, op.value))
      );

      expect(versions).toHaveLength(100);
      versions.forEach((v) => expectValidVersion(v));

      // Verify all values
      const results = await Promise.all(
        operations.map((op) => client.get(op.key))
      );

      results.forEach((result, idx) => {
        expect(result.value).not.toBeNull();
      });
    });

    it('should handle rapid updates to same key', async () => {
      const key = randomKey('rapid-updates');
      const iterations = 50;

      const versions = [];
      for (let i = 0; i < iterations; i++) {
        const version = await client.put(key, `value-${i}`);
        versions.push(version);
      }

      // Versions should be monotonically increasing
      for (let i = 1; i < versions.length; i++) {
        expect(versions[i].index).toBeGreaterThan(versions[i - 1].index);
      }

      // Should have latest value
      const result = await client.get(key);
      expect(result.value?.toString()).toBe(`value-${iterations - 1}`);
    });
  });

  describe('Complex Workflows', () => {
    it('should support read-modify-write pattern', async () => {
      const key = randomKey('rmw');
      const initialValue = '0';

      await client.put(key, initialValue);

      // Read-modify-write loop
      for (let i = 1; i <= 5; i++) {
        const result = await client.get(key);
        const currentValue = parseInt(result.value!.toString(), 10);
        const newValue = (currentValue + 1).toString();

        await client.put(key, newValue, {
          ifMatchVersion: result.version!,
        });
      }

      const final = await client.get(key);
      expect(final.value?.toString()).toBe('5');
    });

    it('should support transaction-like patterns', async () => {
      const key1 = randomKey('txn-1');
      const key2 = randomKey('txn-2');

      // Prepare both keys
      await client.put(key1, 'value-1');
      await client.put(key2, 'value-2');

      // Read both
      const result1 = await client.get(key1);
      const result2 = await client.get(key2);

      // Update both with version check
      await Promise.all([
        client.put(key1, 'updated-1', { ifMatchVersion: result1.version! }),
        client.put(key2, 'updated-2', { ifMatchVersion: result2.version! }),
      ]);

      // Verify updates
      await expectKeyExists(client, key1, 'updated-1');
      await expectKeyExists(client, key2, 'updated-2');
    });

    it('should support caching patterns with TTL', async () => {
      const cacheKey = randomKey('cache');
      const cacheValue = 'cached-data';
      const cacheTTL = 1000; // 1 second

      // Simulate cache miss -> fetch -> store
      let result = await client.get(cacheKey);
      if (result.value === null) {
        await client.put(cacheKey, cacheValue, { ttlMs: cacheTTL });
      }

      // Should be cached
      result = await client.get(cacheKey);
      expect(result.value?.toString()).toBe(cacheValue);

      // Wait for expiration
      await sleep(cacheTTL + 100);

      // Cache miss again
      result = await client.get(cacheKey);
      expect(result.value).toBeNull();
    });

    it('should support distributed counter pattern', async () => {
      const counterKey = randomKey('counter');

      await client.put(counterKey, '0');

      // Multiple concurrent increments
      const increments = Array.from({ length: 10 }, async () => {
        let success = false;
        while (!success) {
          try {
            const result = await client.get(counterKey);
            const current = parseInt(result.value!.toString(), 10);
            await client.put(counterKey, (current + 1).toString(), {
              ifMatchVersion: result.version!,
            });
            success = true;
          } catch (err) {
            // Conflict, retry
            await sleep(10);
          }
        }
      });

      await Promise.all(increments);

      const final = await client.get(counterKey);
      expect(parseInt(final.value!.toString(), 10)).toBe(10);
    });
  });

  describe('Performance Characteristics', () => {
    it('should handle burst traffic', async () => {
      const startTime = Date.now();
      const operations = Array.from({ length: 50 }, (_, i) =>
        client.put(randomKey(`burst-${i}`), randomValue())
      );

      await Promise.all(operations);

      const duration = Date.now() - startTime;

      // Should complete reasonably quickly (mock server is fast)
      expect(duration).toBeLessThan(5000);
    });

    it('should handle sequential operations efficiently', async () => {
      const key = randomKey('sequential');

      const startTime = Date.now();

      for (let i = 0; i < 20; i++) {
        await client.put(key, `value-${i}`);
      }

      const duration = Date.now() - startTime;

      // Sequential should still be reasonably fast
      expect(duration).toBeLessThan(2000);
    });
  });
});
