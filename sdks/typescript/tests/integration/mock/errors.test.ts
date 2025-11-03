/**
 * Integration tests for error handling using mock server.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import * as grpc from '@grpc/grpc-js';
import { NoriKVClient } from '@norikv/client/client';
import { MockNoriKVServer, createMockServer } from '../helpers/mock-server';
import {
  NotLeaderError,
  InvalidArgumentError,
  NoNodesAvailableError,
} from '@norikv/client/errors';
import {
  expectKeyExists,
  expectToRejectWith,
  randomKey,
  randomValue,
} from '../helpers/assertions';

describe('Error Handling (Mock)', () => {
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

  describe('NOT_LEADER Errors', () => {
    it('should handle NOT_LEADER error on put', async () => {
      const key = randomKey('not-leader-put');
      const value = randomValue();

      // Configure server to return NOT_LEADER
      server.setNotLeader('127.0.0.1:50052');

      // Client should automatically retry with leader hint
      await expectToRejectWith(
        client.put(key, value),
        /Not the leader/
      );
    });

    it('should handle NOT_LEADER error on delete', async () => {
      const key = randomKey('not-leader-delete');

      server.setNotLeader('127.0.0.1:50052');

      await expectToRejectWith(
        client.delete(key),
        /Not the leader/
      );
    });

    it('should recover after leader changes', async () => {
      const key = randomKey('leader-recovery');
      const value = randomValue();

      // First put succeeds
      await client.put(key, value);

      // Leader changes
      server.setNotLeader('127.0.0.1:50052');

      // Operations fail during transition
      await expect(client.get(key)).rejects.toThrow();

      // Leader stabilizes
      server.clearNotLeader();

      // Operations succeed again
      await expectKeyExists(client, key);
    });
  });

  describe('Network Errors', () => {
    it('should handle connection timeout', async () => {
      const client = new NoriKVClient({
        nodes: ['127.0.0.1:1'], // Invalid port
        timeout: 1000,
        watchCluster: false,
      });

      await expect(client.connect()).rejects.toThrow(NoNodesAvailableError);
    });

    it('should handle request timeout', async () => {
      const key = randomKey('timeout');
      const value = randomValue();

      // Create client with very short timeout
      const timeoutClient = new NoriKVClient({
        nodes: [server.getAddress()],
        timeout: 1, // 1ms timeout
        watchCluster: false,
      });

      await timeoutClient.connect();

      // Request should timeout
      await expect(timeoutClient.put(key, value)).rejects.toThrow();

      await timeoutClient.close();
    });

    it('should handle server unavailable', async () => {
      const key = randomKey('unavailable');

      // Stop server
      await server.stop();

      // Operations should fail
      await expect(client.get(key)).rejects.toThrow();
    });

    it('should handle UNAVAILABLE status', async () => {
      const key = randomKey('unavailable-status');

      const error: grpc.ServiceError = {
        name: 'UNAVAILABLE',
        message: 'Server unavailable',
        code: grpc.status.UNAVAILABLE,
      };

      server.setError(error);

      await expect(client.get(key)).rejects.toThrow();
    });
  });

  describe('Invalid Arguments', () => {
    it('should reject empty node list', () => {
      expect(() => {
        new NoriKVClient({
          nodes: [],
        });
      }).toThrow(InvalidArgumentError);
    });

    it('should handle INVALID_ARGUMENT status', async () => {
      const key = randomKey('invalid-arg');

      const error: grpc.ServiceError = {
        name: 'INVALID_ARGUMENT',
        message: 'Invalid request',
        code: grpc.status.INVALID_ARGUMENT,
      };

      server.setError(error);

      await expect(client.get(key)).rejects.toThrow();
    });
  });

  describe('Conditional Operation Failures', () => {
    it('should handle ifNotExists conflict', async () => {
      const key = randomKey('if-not-exists');
      const value1 = 'first';
      const value2 = 'second';

      // First put succeeds
      await client.put(key, value1);

      // Second put with ifNotExists should fail
      // Note: Mock server doesn't implement this yet, so we'll test the API
      await expect(
        client.put(key, value2, { ifNotExists: true })
      ).rejects.toThrow();
    });

    it('should handle ifMatchVersion mismatch', async () => {
      const key = randomKey('if-match');
      const value1 = 'first';
      const value2 = 'second';

      const version1 = await client.put(key, value1);

      // Update value
      await client.put(key, value2);

      // Try to update with old version should fail
      await expect(
        client.put(key, 'third', { ifMatchVersion: version1 })
      ).rejects.toThrow();
    });

    it('should handle delete with wrong version', async () => {
      const key = randomKey('delete-wrong-version');
      const value1 = 'first';
      const value2 = 'second';

      const version1 = await client.put(key, value1);
      await client.put(key, value2);

      // Try to delete with old version should fail
      await expect(
        client.delete(key, { ifMatchVersion: version1 })
      ).rejects.toThrow();
    });
  });

  describe('Resource Exhaustion', () => {
    it('should handle RESOURCE_EXHAUSTED status', async () => {
      const key = randomKey('exhausted');

      const error: grpc.ServiceError = {
        name: 'RESOURCE_EXHAUSTED',
        message: 'Out of memory',
        code: grpc.status.RESOURCE_EXHAUSTED,
      };

      server.setError(error);

      await expect(client.get(key)).rejects.toThrow();
    });
  });

  describe('Retry Logic', () => {
    it('should retry on transient errors', async () => {
      const key = randomKey('retry');
      const value = randomValue();

      // Configure retry policy
      const retryClient = new NoriKVClient({
        nodes: [server.getAddress()],
        watchCluster: false,
        retry: {
          maxAttempts: 3,
          initialDelayMs: 10,
          maxDelayMs: 100,
        },
      });

      await retryClient.connect();

      // First request fails
      server.setError({
        name: 'UNAVAILABLE',
        message: 'Temporary failure',
        code: grpc.status.UNAVAILABLE,
      });

      // This should retry and eventually fail
      await expect(retryClient.put(key, value)).rejects.toThrow();

      await retryClient.close();
    });

    it('should respect max retry attempts', async () => {
      const key = randomKey('max-retries');

      const retryClient = new NoriKVClient({
        nodes: [server.getAddress()],
        watchCluster: false,
        retry: {
          maxAttempts: 2,
          initialDelayMs: 10,
        },
      });

      await retryClient.connect();

      server.setError({
        name: 'UNAVAILABLE',
        message: 'Persistent failure',
        code: grpc.status.UNAVAILABLE,
      });

      const startTime = Date.now();
      await expect(retryClient.get(key)).rejects.toThrow();
      const duration = Date.now() - startTime;

      // Should fail relatively quickly (not infinite retries)
      expect(duration).toBeLessThan(1000);

      await retryClient.close();
    });

    it('should not retry on non-retryable errors', async () => {
      const key = randomKey('no-retry');

      const retryClient = new NoriKVClient({
        nodes: [server.getAddress()],
        watchCluster: false,
        retry: {
          maxAttempts: 5,
        },
      });

      await retryClient.connect();

      // INVALID_ARGUMENT should not be retried
      server.setError({
        name: 'INVALID_ARGUMENT',
        message: 'Invalid request',
        code: grpc.status.INVALID_ARGUMENT,
      });

      const startTime = Date.now();
      await expect(retryClient.get(key)).rejects.toThrow();
      const duration = Date.now() - startTime;

      // Should fail immediately (no retries)
      expect(duration).toBeLessThan(100);

      await retryClient.close();
    });
  });

  describe('Connection Pool Errors', () => {
    it('should handle connection failures gracefully', async () => {
      const multiNodeClient = new NoriKVClient({
        nodes: [
          '127.0.0.1:1', // Invalid
          server.getAddress(), // Valid
        ],
        watchCluster: false,
      });

      // Should still connect to valid node
      await multiNodeClient.connect();

      const key = randomKey('multi-node');
      const value = randomValue();

      // Should work with at least one valid node
      await multiNodeClient.put(key, value);
      await expectKeyExists(multiNodeClient, key, value);

      await multiNodeClient.close();
    });

    it('should fail if all nodes are unavailable', async () => {
      const allBadClient = new NoriKVClient({
        nodes: ['127.0.0.1:1', '127.0.0.1:2', '127.0.0.1:3'],
        timeout: 100,
        watchCluster: false,
      });

      await expect(allBadClient.connect()).rejects.toThrow(
        NoNodesAvailableError
      );
    });
  });

  describe('Concurrent Operations with Errors', () => {
    it('should handle partial failures in batch operations', async () => {
      const operations = Array.from({ length: 5 }, (_, i) => ({
        key: randomKey(`batch-${i}`),
        value: randomValue(),
      }));

      // Put some values
      await Promise.all(
        operations.slice(0, 3).map((op) => client.put(op.key, op.value))
      );

      // Inject error
      server.setError({
        name: 'UNAVAILABLE',
        message: 'Temporary failure',
        code: grpc.status.UNAVAILABLE,
      });

      // Batch operation should partially fail
      const results = await Promise.allSettled(
        operations.map((op) => client.put(op.key, op.value))
      );

      const failures = results.filter((r) => r.status === 'rejected');
      expect(failures.length).toBeGreaterThan(0);

      // Clear error
      server.clearError();

      // First 3 should still exist
      await expectKeyExists(client, operations[0].key);
      await expectKeyExists(client, operations[1].key);
      await expectKeyExists(client, operations[2].key);
    });
  });

  describe('Client State Errors', () => {
    it('should reject operations when not connected', async () => {
      const disconnectedClient = new NoriKVClient({
        nodes: [server.getAddress()],
        watchCluster: false,
      });

      // Don't connect
      await expect(disconnectedClient.get('key')).rejects.toThrow(
        /not initialized/i
      );
    });

    it('should reject operations after close', async () => {
      const key = randomKey('after-close');

      await client.close();

      await expect(client.get(key)).rejects.toThrow(/closed/i);
    });

    it('should handle double close gracefully', async () => {
      await client.close();
      await expect(client.close()).resolves.not.toThrow();
    });

    it('should handle double connect gracefully', async () => {
      // Already connected in beforeEach
      await expect(client.connect()).resolves.not.toThrow();
    });
  });

  describe('Error Recovery', () => {
    it('should recover from temporary errors', async () => {
      const key = randomKey('recovery');
      const value = randomValue();

      // Inject temporary error
      server.setError({
        name: 'UNAVAILABLE',
        message: 'Temporary failure',
        code: grpc.status.UNAVAILABLE,
      });

      await expect(client.put(key, value)).rejects.toThrow();

      // Clear error
      server.clearError();

      // Should work now
      await client.put(key, value);
      await expectKeyExists(client, key, value);
    });

    it('should maintain connection pool after errors', async () => {
      const key1 = randomKey('pool-1');
      const key2 = randomKey('pool-2');
      const value = randomValue();

      // First operation succeeds
      await client.put(key1, value);

      // Inject error
      server.setError({
        name: 'INTERNAL',
        message: 'Internal error',
        code: grpc.status.INTERNAL,
      });

      await expect(client.get(key1)).rejects.toThrow();

      // Clear error
      server.clearError();

      // Pool should still be usable
      await client.put(key2, value);
      await expectKeyExists(client, key2, value);
    });
  });
});
