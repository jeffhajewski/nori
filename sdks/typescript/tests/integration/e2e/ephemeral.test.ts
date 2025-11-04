/**
 * Ephemeral server tests.
 *
 * These tests verify the ephemeral (in-memory) server functionality.
 * Requires norikv-server binary in PATH or NORIKV_SERVER_PATH.
 */

import { describe, it, expect, beforeAll, afterAll, beforeEach } from 'vitest';
import { createEphemeral, EphemeralNoriKV, EphemeralServerError } from '@norikv/client/ephemeral';
import { NoriKVClient } from '@norikv/client/client';
import { bytesToString } from '@norikv/client/hash';
import { exec } from 'child_process';
import { promisify } from 'util';

const execAsync = promisify(exec);

// Check if norikv-server is available
let serverAvailable = false;

beforeAll(async () => {
  try {
    const binaryPath = process.env.NORIKV_SERVER_PATH || 'norikv-server';
    await execAsync(`${binaryPath} --version 2>/dev/null || ${binaryPath} --help 2>/dev/null || echo "ok"`);
    serverAvailable = true;
  } catch {
    serverAvailable = false;
  }
});

describe.skipIf(!serverAvailable)('Ephemeral Server', () => {
  describe('createEphemeral', () => {
    it('should create and start an ephemeral server', async () => {
      const cluster = await createEphemeral();

      try {
        expect(cluster).toBeDefined();
        expect(cluster['port']).toBeGreaterThan(0);

        // Should be able to get a client
        const client = cluster.getClient();
        expect(client).toBeInstanceOf(NoriKVClient);
      } finally {
        await cluster.stop();
      }
    }, 30000);

    it('should perform CRUD operations', async () => {
      const cluster = await createEphemeral();

      try {
        const client = cluster.getClient();
        await client.connect();

        // Put
        const version = await client.put('test-key', 'test-value');
        expect(version).toBeDefined();
        expect(version.term).toBeGreaterThan(0);
        expect(version.index).toBeGreaterThan(0);

        // Get
        const result = await client.get('test-key');
        expect(result.value).toBeDefined();
        expect(bytesToString(result.value!)).toBe('test-value');
        expect(result.version).toEqual(version);

        // Update
        const version2 = await client.put('test-key', 'updated-value');
        expect(version2.index).toBeGreaterThan(version.index);

        // Get updated
        const result2 = await client.get('test-key');
        expect(bytesToString(result2.value!)).toBe('updated-value');

        // Delete
        const deleted = await client.delete('test-key');
        expect(deleted).toBe(true);

        // Verify deletion
        const result3 = await client.get('test-key');
        expect(result3.value).toBeNull();

        await client.close();
      } finally {
        await cluster.stop();
      }
    }, 30000);

    it('should support multiple concurrent ephemeral instances', async () => {
      const cluster1 = await createEphemeral();
      const cluster2 = await createEphemeral();

      try {
        // Different ports
        expect(cluster1['port']).not.toBe(cluster2['port']);

        const client1 = cluster1.getClient();
        const client2 = cluster2.getClient();

        await client1.connect();
        await client2.connect();

        // Write to cluster 1
        await client1.put('key', 'value1');

        // Write to cluster 2
        await client2.put('key', 'value2');

        // Values should be independent
        const result1 = await client1.get('key');
        const result2 = await client2.get('key');

        expect(bytesToString(result1.value!)).toBe('value1');
        expect(bytesToString(result2.value!)).toBe('value2');

        await client1.close();
        await client2.close();
      } finally {
        await cluster1.stop();
        await cluster2.stop();
      }
    }, 30000);

    it('should clean up temp directories on stop', async () => {
      const cluster = await createEphemeral();
      const tempDir = cluster['tempDir'];

      await cluster.stop();

      // Temp dir should be removed
      if (tempDir) {
        const fs = await import('fs');
        expect(fs.existsSync(tempDir)).toBe(false);
      }
    }, 30000);
  });

  describe('EphemeralNoriKV class', () => {
    it('should support manual start/stop lifecycle', async () => {
      const cluster = new EphemeralNoriKV();

      try {
        await cluster.start();

        const client = cluster.getClient();
        await client.connect();

        await client.put('key1', 'value1');
        const result = await client.get('key1');
        expect(bytesToString(result.value!)).toBe('value1');

        await client.close();
      } finally {
        await cluster.stop();
      }
    }, 30000);

    it('should support custom shard count', async () => {
      const cluster = new EphemeralNoriKV({ totalShards: 128 });

      try {
        await cluster.start();

        const client = cluster.getClient();
        await client.connect();

        await client.put('test', 'value');
        const result = await client.get('test');
        expect(bytesToString(result.value!)).toBe('value');

        await client.close();
      } finally {
        await cluster.stop();
      }
    }, 30000);

    it('should throw error if getClient() called before start()', () => {
      const cluster = new EphemeralNoriKV();

      expect(() => cluster.getClient()).toThrow(EphemeralServerError);
      expect(() => cluster.getClient()).toThrow(/not started/);
    });

    it('should throw error if start() called twice', async () => {
      const cluster = new EphemeralNoriKV();

      try {
        await cluster.start();
        await expect(cluster.start()).rejects.toThrow(EphemeralServerError);
        await expect(cluster.start()).rejects.toThrow(/already started/);
      } finally {
        await cluster.stop();
      }
    }, 30000);

    it('should handle binary operations', async () => {
      const cluster = await createEphemeral();

      try {
        const client = cluster.getClient();
        await client.connect();

        const binaryKey = Buffer.from([0x01, 0x02, 0x03, 0x04]);
        const binaryValue = Buffer.from([0xaa, 0xbb, 0xcc, 0xdd]);

        await client.put(binaryKey, binaryValue);
        const result = await client.get(binaryKey);

        expect(result.value).toEqual(binaryValue);

        await client.close();
      } finally {
        await cluster.stop();
      }
    }, 30000);

    it('should support custom port', async () => {
      // Find a free port (simplified - just use a high number)
      const customPort = 50000 + Math.floor(Math.random() * 10000);

      const cluster = new EphemeralNoriKV({ port: customPort });

      try {
        await cluster.start();
        expect(cluster['port']).toBe(customPort);

        const client = cluster.getClient();
        await client.connect();
        await client.put('test', 'value');

        await client.close();
      } finally {
        await cluster.stop();
      }
    }, 30000);

    it('should handle multiple operations in sequence', async () => {
      const cluster = await createEphemeral();

      try {
        const client = cluster.getClient();
        await client.connect();

        // Perform multiple operations
        for (let i = 0; i < 10; i++) {
          await client.put(`key${i}`, `value${i}`);
        }

        // Verify all values
        for (let i = 0; i < 10; i++) {
          const result = await client.get(`key${i}`);
          expect(bytesToString(result.value!)).toBe(`value${i}`);
        }

        // Delete all
        for (let i = 0; i < 10; i++) {
          const deleted = await client.delete(`key${i}`);
          expect(deleted).toBe(true);
        }

        // Verify deletions
        for (let i = 0; i < 10; i++) {
          const result = await client.get(`key${i}`);
          expect(result.value).toBeNull();
        }

        await client.close();
      } finally {
        await cluster.stop();
      }
    }, 30000);
  });

  describe('Error Handling', () => {
    it('should handle server startup timeout gracefully', async () => {
      // Create a cluster with very short timeout
      const cluster = new EphemeralNoriKV({ startupTimeout: 0.001 });

      await expect(cluster.start()).rejects.toThrow(EphemeralServerError);
      await expect(cluster.start()).rejects.toThrow(/did not become healthy/);
    }, 10000);
  });
});

// Separate describe block for tests that don't require the server
describe('Binary Discovery', () => {
  it('should throw helpful error when binary not found', async () => {
    // Temporarily modify PATH and env var
    const oldPath = process.env.PATH;
    const oldServerPath = process.env.NORIKV_SERVER_PATH;

    try {
      process.env.PATH = '';
      delete process.env.NORIKV_SERVER_PATH;

      const cluster = new EphemeralNoriKV();

      await expect(cluster.start()).rejects.toThrow(EphemeralServerError);
      await expect(cluster.start()).rejects.toThrow(/binary not found/);
    } finally {
      // Restore environment
      process.env.PATH = oldPath || '';
      if (oldServerPath) {
        process.env.NORIKV_SERVER_PATH = oldServerPath;
      }
    }
  }, 5000);
});
