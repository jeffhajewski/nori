/**
 * Integration tests for basic CRUD operations using mock server.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { NoriKVClient } from '@norikv/client/client';
import { MockNoriKVServer, createMockServer } from '../helpers/mock-server';
import {
  expectKeyExists,
  expectKeyNotExists,
  expectValidVersion,
  expectVersionsEqual,
  expectBufferEquals,
  randomKey,
  randomValue,
} from '../helpers/assertions';

describe('CRUD Operations (Mock)', () => {
  let server: MockNoriKVServer;
  let client: NoriKVClient;

  beforeEach(async () => {
    server = await createMockServer();
    client = new NoriKVClient({
      nodes: [server.getAddress()],
      watchCluster: false, // Disable for mock tests
    });
    await client.connect();
  });

  afterEach(async () => {
    await client.close();
    await server.stop();
  });

  describe('Put Operations', () => {
    it('should put a simple key-value pair', async () => {
      const key = randomKey('put');
      const value = randomValue();

      const version = await client.put(key, value);

      expectValidVersion(version);

      // Verify the value was stored
      const result = await expectKeyExists(client, key, value);
      expectVersionsEqual(result.version, version);
    });

    it('should put with Buffer key and value', async () => {
      const key = Buffer.from(randomKey('buffer'));
      const value = Buffer.from(randomValue());

      const version = await client.put(key, value);

      expectValidVersion(version);

      const result = await client.get(key);
      expect(result.value).toEqual(value);
      expectVersionsEqual(result.version, version);
    });

    it('should put with string key and Buffer value', async () => {
      const key = randomKey('mixed');
      const value = Buffer.from('test-buffer-value');

      const version = await client.put(key, value);

      expectValidVersion(version);
      await expectKeyExists(client, key, value);
    });

    it('should overwrite existing key', async () => {
      const key = randomKey('overwrite');
      const value1 = 'first-value';
      const value2 = 'second-value';

      const version1 = await client.put(key, value1);
      expectValidVersion(version1);

      const version2 = await client.put(key, value2);
      expectValidVersion(version2);

      // Version should be different
      expect(version2.term).toBeGreaterThanOrEqual(version1.term);
      expect(version2.index).toBeGreaterThan(version1.index);

      // Should get the latest value
      const result = await expectKeyExists(client, key, value2);
      expectVersionsEqual(result.version, version2);
    });

    it('should handle empty string value', async () => {
      const key = randomKey('empty');
      const value = '';

      const version = await client.put(key, value);
      expectValidVersion(version);

      await expectKeyExists(client, key, value);
    });

    it('should handle large values', async () => {
      const key = randomKey('large');
      const value = 'x'.repeat(10_000); // 10KB value

      const version = await client.put(key, value);
      expectValidVersion(version);

      await expectKeyExists(client, key, value);
    });

    it('should support idempotency key', async () => {
      const key = randomKey('idempotent');
      const value = randomValue();
      const idempotencyKey = 'test-idempotency-key';

      const version1 = await client.put(key, value, { idempotencyKey });
      const version2 = await client.put(key, value, { idempotencyKey });

      // With idempotency, versions should be the same
      expectVersionsEqual(version1, version2);
    });
  });

  describe('Get Operations', () => {
    it('should get an existing key', async () => {
      const key = randomKey('get');
      const value = randomValue();

      await client.put(key, value);
      const result = await client.get(key);

      expect(result.value).not.toBeNull();
      expectBufferEquals(result.value as Buffer, value);
      expectValidVersion(result.version);
    });

    it('should return null for non-existent key', async () => {
      const key = randomKey('nonexistent');

      await expectKeyNotExists(client, key);
    });

    it('should get with Buffer key', async () => {
      const key = Buffer.from(randomKey('buffer-get'));
      const value = randomValue();

      await client.put(key, value);
      const result = await client.get(key);

      expect(result.value).not.toBeNull();
      expectValidVersion(result.version);
    });

    it('should get latest version after multiple puts', async () => {
      const key = randomKey('multi-put');
      const values = ['v1', 'v2', 'v3'];

      let lastVersion;
      for (const value of values) {
        lastVersion = await client.put(key, value);
      }

      const result = await client.get(key);
      expectBufferEquals(result.value as Buffer, values[values.length - 1]);
      expectVersionsEqual(result.version, lastVersion!);
    });

    it('should support different consistency levels', async () => {
      const key = randomKey('consistency');
      const value = randomValue();

      await client.put(key, value);

      // Test all consistency levels
      const leaseResult = await client.get(key, { consistency: 'lease' });
      expect(leaseResult.value).not.toBeNull();

      const linearizableResult = await client.get(key, {
        consistency: 'linearizable',
      });
      expect(linearizableResult.value).not.toBeNull();

      const staleResult = await client.get(key, { consistency: 'stale_ok' });
      expect(staleResult.value).not.toBeNull();

      // All should return the same value
      expect(leaseResult.value).toEqual(linearizableResult.value);
      expect(leaseResult.value).toEqual(staleResult.value);
    });
  });

  describe('Delete Operations', () => {
    it('should delete an existing key', async () => {
      const key = randomKey('delete');
      const value = randomValue();

      await client.put(key, value);
      await expectKeyExists(client, key);

      const deleted = await client.delete(key);
      expect(deleted).toBe(true);

      await expectKeyNotExists(client, key);
    });

    it('should return false when deleting non-existent key', async () => {
      const key = randomKey('nonexistent-delete');

      const deleted = await client.delete(key);
      expect(deleted).toBe(false);
    });

    it('should delete with Buffer key', async () => {
      const key = Buffer.from(randomKey('buffer-delete'));
      const value = randomValue();

      await client.put(key, value);
      const deleted = await client.delete(key);

      expect(deleted).toBe(true);
      await expectKeyNotExists(client, key);
    });

    it('should allow re-putting after delete', async () => {
      const key = randomKey('delete-reput');
      const value1 = 'first';
      const value2 = 'second';

      await client.put(key, value1);
      await client.delete(key);
      await expectKeyNotExists(client, key);

      await client.put(key, value2);
      await expectKeyExists(client, key, value2);
    });

    it('should support idempotency key', async () => {
      const key = randomKey('delete-idempotent');
      const value = randomValue();
      const idempotencyKey = 'test-delete-idempotency';

      await client.put(key, value);

      const deleted1 = await client.delete(key, { idempotencyKey });
      const deleted2 = await client.delete(key, { idempotencyKey });

      expect(deleted1).toBe(true);
      // Second delete with same idempotency key should also succeed
      // (idempotent behavior)
      expect(deleted2).toBe(true);
    });

    it('should handle multiple deletes gracefully', async () => {
      const key = randomKey('multi-delete');
      const value = randomValue();

      await client.put(key, value);

      const deleted1 = await client.delete(key);
      expect(deleted1).toBe(true);

      const deleted2 = await client.delete(key);
      expect(deleted2).toBe(false); // Already deleted
    });
  });

  describe('Version Tracking', () => {
    it('should track versions across operations', async () => {
      const key = randomKey('version-track');
      const versions: typeof client extends NoriKVClient
        ? Awaited<ReturnType<typeof client.put>>[]
        : never = [];

      // Put multiple times
      for (let i = 0; i < 5; i++) {
        const version = await client.put(key, `value-${i}`);
        versions.push(version);
      }

      // Versions should be monotonically increasing
      for (let i = 1; i < versions.length; i++) {
        const prev = versions[i - 1];
        const curr = versions[i];

        expect(curr.term).toBeGreaterThanOrEqual(prev.term);
        if (curr.term === prev.term) {
          expect(curr.index).toBeGreaterThan(prev.index);
        }
      }
    });

    it('should return correct version on get', async () => {
      const key = randomKey('version-get');
      const value = randomValue();

      const putVersion = await client.put(key, value);
      const result = await client.get(key);

      expectVersionsEqual(result.version, putVersion);
    });
  });

  describe('Batch Operations', () => {
    it('should handle concurrent puts to different keys', async () => {
      const operations = Array.from({ length: 10 }, (_, i) => ({
        key: randomKey(`concurrent-${i}`),
        value: randomValue(),
      }));

      const versions = await Promise.all(
        operations.map((op) => client.put(op.key, op.value))
      );

      expect(versions).toHaveLength(10);
      versions.forEach((v) => expectValidVersion(v));

      // Verify all keys exist
      for (const op of operations) {
        await expectKeyExists(client, op.key, op.value);
      }
    });

    it('should handle concurrent gets', async () => {
      const key = randomKey('concurrent-get');
      const value = randomValue();

      await client.put(key, value);

      // Multiple concurrent gets
      const results = await Promise.all(
        Array.from({ length: 10 }, () => client.get(key))
      );

      expect(results).toHaveLength(10);
      results.forEach((result) => {
        expect(result.value).not.toBeNull();
        expectBufferEquals(result.value as Buffer, value);
      });
    });

    it('should handle mixed operations', async () => {
      const keys = Array.from({ length: 5 }, (_, i) =>
        randomKey(`mixed-${i}`)
      );

      // Put initial values
      await Promise.all(
        keys.map((key, i) => client.put(key, `value-${i}`))
      );

      // Mix of operations
      await Promise.all([
        client.get(keys[0]),
        client.put(keys[1], 'updated'),
        client.delete(keys[2]),
        client.get(keys[3]),
        client.put(keys[4], 'new-value'),
      ]);

      // Verify results
      await expectKeyExists(client, keys[0]);
      await expectKeyExists(client, keys[1], 'updated');
      await expectKeyNotExists(client, keys[2]);
      await expectKeyExists(client, keys[3]);
      await expectKeyExists(client, keys[4], 'new-value');
    });
  });

  describe('Edge Cases', () => {
    it('should handle special characters in keys', async () => {
      const specialKeys = [
        'key-with-dashes',
        'key.with.dots',
        'key_with_underscores',
        'key:with:colons',
        'key/with/slashes',
        'key with spaces',
      ];

      for (const key of specialKeys) {
        const value = randomValue();
        await client.put(key, value);
        await expectKeyExists(client, key, value);
      }
    });

    it('should handle Unicode in keys and values', async () => {
      const key = 'key-🔑-emoji';
      const value = 'value-✨-unicode-文字';

      await client.put(key, value);
      await expectKeyExists(client, key, value);
    });

    it('should handle binary data', async () => {
      const key = randomKey('binary');
      const value = Buffer.from([0x00, 0x01, 0xff, 0xfe, 0xaa, 0xbb]);

      await client.put(key, value);

      const result = await client.get(key);
      expect(result.value).toEqual(value);
    });
  });
});
