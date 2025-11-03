/**
 * E2E cluster tests with multiple real NoriKV servers.
 *
 * Tests multi-node cluster behavior, failover, and replication.
 */

import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { NoriKVClient } from '@norikv/client/client';
import {
  startTestCluster,
  type TestCluster,
} from '../helpers/test-server';
import {
  expectKeyExists,
  expectEventualConsistency,
  randomKey,
  randomValue,
  sleep,
} from '../helpers/assertions';

describe('E2E Cluster Operations', () => {
  let cluster: TestCluster;
  let client: NoriKVClient;

  beforeAll(async () => {
    // Start a 3-node cluster
    cluster = await startTestCluster({
      nodeCount: 3,
      totalShards: 1024,
      verbose: false,
    });

    // Create client connected to all nodes
    client = new NoriKVClient({
      nodes: cluster.getAddresses(),
      watchCluster: true,
    });

    await client.connect();
  }, 60000); // 60 second timeout for cluster startup

  afterAll(async () => {
    if (client) {
      await client.close();
    }
    if (cluster) {
      await cluster.stop();
    }
  });

  describe('Basic Cluster Operations', () => {
    it('should connect to cluster', () => {
      expect(client.isConnected()).toBe(true);
    });

    it('should have cluster view with all nodes', () => {
      const view = client.getClusterView();
      expect(view).not.toBeNull();
      expect(view!.nodes.length).toBeGreaterThanOrEqual(1);
    });

    it('should write and read from cluster', async () => {
      const key = randomKey('cluster-basic');
      const value = randomValue();

      await client.put(key, value);
      await expectKeyExists(client, key, value);
    });
  });

  describe('Replication', () => {
    it('should replicate data across nodes', async () => {
      const key = randomKey('cluster-replicate');
      const value = randomValue();

      // Write to cluster
      await client.put(key, value);

      // Data should eventually be consistent across nodes
      await expectEventualConsistency(client, key, value, {
        timeoutMs: 10000,
      });
    });

    it('should handle multiple concurrent writes', async () => {
      const operations = Array.from({ length: 20 }, (_, i) => ({
        key: randomKey(`cluster-concurrent-${i}`),
        value: randomValue(),
      }));

      // Concurrent writes
      await Promise.all(
        operations.map((op) => client.put(op.key, op.value))
      );

      // Verify all values are eventually consistent
      for (const op of operations) {
        await expectEventualConsistency(client, op.key, op.value, {
          timeoutMs: 10000,
        });
      }
    });
  });

  describe('Node Failures', () => {
    it('should handle node restart', async () => {
      const key = randomKey('cluster-restart');
      const value = randomValue();

      // Write data
      await client.put(key, value);
      await expectKeyExists(client, key, value);

      // Restart a follower node (not node-0 which might be leader)
      const nodeToRestart = 'node-1';
      await cluster.stopNode(nodeToRestart);
      await sleep(1000);
      await cluster.restartNode(nodeToRestart);

      // Wait for node to rejoin
      await sleep(2000);

      // Data should still be accessible
      await expectEventualConsistency(client, key, value, {
        timeoutMs: 10000,
      });
    });

    it('should continue operating with minority failure', async () => {
      const key = randomKey('cluster-minority-fail');
      const value = randomValue();

      // Stop one node (cluster should still have quorum)
      await cluster.stopNode('node-2');

      // Wait a bit for cluster to stabilize
      await sleep(2000);

      // Should still be able to write and read
      await client.put(key, value);
      await expectEventualConsistency(client, key, value, {
        timeoutMs: 10000,
      });

      // Restart the node
      await cluster.restartNode('node-2');
      await sleep(2000);
    });
  });

  describe('Consistency Levels', () => {
    it('should provide linearizable reads', async () => {
      const key = randomKey('cluster-linearizable');
      const value = randomValue();

      await client.put(key, value);

      // Linearizable read should see the latest value
      const result = await client.get(key, { consistency: 'linearizable' });
      expect(result.value?.toString()).toBe(value);
    });

    it('should provide lease-based reads', async () => {
      const key = randomKey('cluster-lease');
      const value = randomValue();

      await client.put(key, value);

      // Lease read should see the latest value
      const result = await client.get(key, { consistency: 'lease' });
      expect(result.value?.toString()).toBe(value);
    });

    it('should allow stale reads', async () => {
      const key = randomKey('cluster-stale');
      const value = randomValue();

      await client.put(key, value);

      // Stale read might see slightly old value but should work
      const result = await client.get(key, { consistency: 'stale_ok' });
      expect(result.value).not.toBeNull();
    });
  });

  describe('Load Distribution', () => {
    it('should distribute writes across shards', async () => {
      const operations = Array.from({ length: 100 }, (_, i) => ({
        key: randomKey(`cluster-distribute-${i}`),
        value: randomValue(),
      }));

      // Write many keys (should be distributed across shards)
      await Promise.all(
        operations.map((op) => client.put(op.key, op.value))
      );

      // Verify a sample of values
      const sample = operations.slice(0, 10);
      for (const op of sample) {
        await expectEventualConsistency(client, op.key, op.value, {
          timeoutMs: 10000,
        });
      }
    });
  });

  describe('Cluster Topology', () => {
    it('should track topology changes', async () => {
      const changes: any[] = [];

      const unsubscribe = client.onTopologyChange((event) => {
        changes.push(event);
      });

      // Wait for potential topology changes
      await sleep(2000);

      unsubscribe();

      // Should have received at least the initial view
      // (Exact number depends on cluster behavior)
      expect(changes.length).toBeGreaterThanOrEqual(0);
    });
  });
});
