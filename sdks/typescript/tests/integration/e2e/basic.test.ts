/**
 * Basic E2E integration tests with real NoriKV server.
 *
 * These tests require a compiled norikv-server binary.
 * Run `cargo build -p norikv-server` from the repository root before running these tests.
 */

import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { NoriKVClient } from '@norikv/client/client';
import {
  startTestServer,
  type TestServerInstance,
} from '../helpers/test-server';
import {
  expectKeyExists,
  expectKeyNotExists,
  expectValidVersion,
  randomKey,
  randomValue,
} from '../helpers/assertions';

describe('E2E Basic Operations', () => {
  let server: TestServerInstance;
  let client: NoriKVClient;

  beforeAll(async () => {
    // Start a single test server
    server = await startTestServer({
      verbose: false,
      totalShards: 1024,
    });

    // Create client
    client = new NoriKVClient({
      nodes: [server.grpcAddr],
      watchCluster: true,
    });

    await client.connect();
  }, 30000); // 30 second timeout for server startup

  afterAll(async () => {
    if (client) {
      await client.close();
    }
    if (server) {
      await server.cleanup();
    }
  });

  describe('Basic CRUD', () => {
    it('should put and get a value', async () => {
      const key = randomKey('e2e-basic');
      const value = randomValue();

      const version = await client.put(key, value);
      expectValidVersion(version);

      const result = await expectKeyExists(client, key, value);
      expect(result.version).toEqual(version);
    });

    it('should delete a value', async () => {
      const key = randomKey('e2e-delete');
      const value = randomValue();

      await client.put(key, value);
      await expectKeyExists(client, key);

      const deleted = await client.delete(key);
      expect(deleted).toBe(true);

      await expectKeyNotExists(client, key);
    });

    it('should handle non-existent keys', async () => {
      const key = randomKey('e2e-nonexistent');

      await expectKeyNotExists(client, key);
    });

    it('should overwrite existing keys', async () => {
      const key = randomKey('e2e-overwrite');
      const value1 = 'first-value';
      const value2 = 'second-value';

      await client.put(key, value1);
      await expectKeyExists(client, key, value1);

      await client.put(key, value2);
      await expectKeyExists(client, key, value2);
    });
  });

  describe('Consistency', () => {
    it('should support lease consistency', async () => {
      const key = randomKey('e2e-lease');
      const value = randomValue();

      await client.put(key, value);

      const result = await client.get(key, { consistency: 'lease' });
      expect(result.value).not.toBeNull();
      expectValidVersion(result.version);
    });

    it('should support linearizable reads', async () => {
      const key = randomKey('e2e-linearizable');
      const value = randomValue();

      await client.put(key, value);

      const result = await client.get(key, { consistency: 'linearizable' });
      expect(result.value).not.toBeNull();
      expectValidVersion(result.version);
    });

    it('should support stale reads', async () => {
      const key = randomKey('e2e-stale');
      const value = randomValue();

      await client.put(key, value);

      const result = await client.get(key, { consistency: 'stale_ok' });
      expect(result.value).not.toBeNull();
      expectValidVersion(result.version);
    });
  });

  describe('Concurrent Operations', () => {
    it('should handle concurrent puts', async () => {
      const operations = Array.from({ length: 10 }, (_, i) => ({
        key: randomKey(`e2e-concurrent-${i}`),
        value: randomValue(),
      }));

      // Concurrent puts
      await Promise.all(
        operations.map((op) => client.put(op.key, op.value))
      );

      // Verify all values
      for (const op of operations) {
        await expectKeyExists(client, op.key, op.value);
      }
    });

    it('should handle concurrent gets', async () => {
      const key = randomKey('e2e-concurrent-get');
      const value = randomValue();

      await client.put(key, value);

      // Multiple concurrent gets
      const results = await Promise.all(
        Array.from({ length: 10 }, () => client.get(key))
      );

      results.forEach((result) => {
        expect(result.value).not.toBeNull();
        expectValidVersion(result.version);
      });
    });
  });

  describe('Version Tracking', () => {
    it('should track versions correctly', async () => {
      const key = randomKey('e2e-version');

      const versions = [];
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
  });

  describe('Client State', () => {
    it('should report connected state', () => {
      expect(client.isConnected()).toBe(true);
    });

    it('should have cluster view', () => {
      const view = client.getClusterView();
      expect(view).not.toBeNull();
      expect(view!.nodes.length).toBeGreaterThan(0);
    });
  });
});
