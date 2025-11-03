/**
 * Ephemeral (in-memory) NoriKV instance for testing and development.
 *
 * This module provides utilities for spawning a temporary NoriKV server process
 * configured for single-node, ephemeral storage. The server automatically cleans
 * up when stopped.
 *
 * @example
 * ```typescript
 * import { createEphemeral } from '@norikv/client';
 *
 * async function main() {
 *   const cluster = await createEphemeral();
 *   const client = cluster.getClient();
 *
 *   await client.connect();
 *   await client.put('key', 'value');
 *   const result = await client.get('key');
 *   console.log(result.value?.toString()); // 'value'
 *   await client.close();
 *   await cluster.stop();
 * }
 * ```
 */

import { spawn, ChildProcess } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as net from 'net';
import { promisify } from 'util';
import * as yaml from 'yaml';
import { NoriKVClient } from './client';
import type { ClientConfig } from './types';

const mkdir = promisify(fs.mkdir);
const writeFile = promisify(fs.writeFile);
const unlink = promisify(fs.unlink);
const rmdir = promisify(fs.rmdir);

/**
 * Error thrown when ephemeral server operations fail.
 */
export class EphemeralServerError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'EphemeralServerError';
    Object.setPrototypeOf(this, EphemeralServerError.prototype);
  }
}

/**
 * Find an available port on localhost.
 */
async function findFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();

    server.unref();
    server.on('error', reject);

    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      if (address && typeof address === 'object') {
        const port = address.port;
        server.close(() => {
          resolve(port);
        });
      } else {
        reject(new Error('Failed to get port from server'));
      }
    });
  });
}

/**
 * Find the norikv-server binary.
 *
 * Searches in the following order:
 * 1. NORIKV_SERVER_PATH environment variable
 * 2. Same directory as this npm package
 * 3. System PATH
 *
 * @throws {EphemeralServerError} If binary not found
 */
function findNorikvServerBinary(): string {
  // Check environment variable
  const envPath = process.env.NORIKV_SERVER_PATH;
  if (envPath && fs.existsSync(envPath)) {
    try {
      fs.accessSync(envPath, fs.constants.X_OK);
      return envPath;
    } catch {
      // Not executable, continue searching
    }
  }

  // Check package directory (for bundled binaries)
  const packageDir = path.join(__dirname, '..');
  const bundledBinary = path.join(packageDir, 'bin', 'norikv-server');
  if (fs.existsSync(bundledBinary)) {
    try {
      fs.accessSync(bundledBinary, fs.constants.X_OK);
      return bundledBinary;
    } catch {
      // Not executable
    }
  }

  // Check system PATH
  const pathEnv = process.env.PATH || '';
  const pathDirs = pathEnv.split(path.delimiter);

  for (const dir of pathDirs) {
    const binaryPath = path.join(dir, 'norikv-server');
    if (fs.existsSync(binaryPath)) {
      try {
        fs.accessSync(binaryPath, fs.constants.X_OK);
        return binaryPath;
      } catch {
        continue;
      }
    }
  }

  // Not found - raise helpful error
  throw new EphemeralServerError(
    'norikv-server binary not found. Please install it or set NORIKV_SERVER_PATH.\n' +
      '\n' +
      'Installation options:\n' +
      '  1. Build from source: cargo build --release -p norikv-server\n' +
      '  2. Set NORIKV_SERVER_PATH to point to the binary\n' +
      '  3. Add the binary to your system PATH'
  );
}

/**
 * Generate a random hex string for node IDs.
 */
function randomHex(length: number): string {
  const bytes = Math.ceil(length / 2);
  const buffer = Buffer.alloc(bytes);
  for (let i = 0; i < bytes; i++) {
    buffer[i] = Math.floor(Math.random() * 256);
  }
  return buffer.toString('hex').slice(0, length);
}

/**
 * Wait for a condition to be true with timeout.
 */
async function waitFor(
  condition: () => Promise<boolean>,
  timeoutMs: number,
  intervalMs: number = 100
): Promise<void> {
  const startTime = Date.now();

  while (Date.now() - startTime < timeoutMs) {
    if (await condition()) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }

  throw new Error(`Timeout after ${timeoutMs}ms`);
}

/**
 * Options for creating an ephemeral server.
 */
export interface EphemeralOptions {
  /** Optional port number. If not specified, an available port is auto-assigned. */
  port?: number;

  /** Optional data directory. If not specified, a temp directory is created. */
  dataDir?: string;

  /** Number of virtual shards (default: 256 for faster startup). */
  totalShards?: number;

  /** Optional path to norikv-server binary. */
  serverBinary?: string;

  /** Optional path to log file. If not specified, logs go to temp file. */
  logFile?: string;

  /** Seconds to wait for server to become ready (default: 10). */
  startupTimeout?: number;
}

/**
 * Manages an ephemeral NoriKV server instance.
 *
 * This class spawns a local norikv-server process configured for single-node,
 * ephemeral storage. The server runs on an automatically assigned port and
 * stores data in a temporary directory that is cleaned up on exit.
 *
 * @example
 * ```typescript
 * const cluster = new EphemeralNoriKV();
 * await cluster.start();
 * const client = cluster.getClient();
 * await client.connect();
 * await client.put('key', 'value');
 * await client.close();
 * await cluster.stop();
 * ```
 */
export class EphemeralNoriKV {
  private port?: number;
  private dataDir?: string;
  private totalShards: number;
  private serverBinary?: string;
  private logFile?: string;
  private startupTimeout: number;

  // Runtime state
  private process?: ChildProcess;
  private tempDir?: string;
  private tempLog?: string;
  private configFile?: string;
  private client?: NoriKVClient;
  private started = false;

  constructor(options: EphemeralOptions = {}) {
    this.port = options.port;
    this.dataDir = options.dataDir;
    this.totalShards = options.totalShards ?? 256;
    this.serverBinary = options.serverBinary;
    this.logFile = options.logFile;
    this.startupTimeout = (options.startupTimeout ?? 10) * 1000; // Convert to ms
  }

  /**
   * Start the ephemeral server.
   *
   * This method:
   * 1. Creates a temporary directory for data
   * 2. Generates server configuration
   * 3. Spawns the norikv-server process
   * 4. Waits for the server to become healthy
   *
   * @throws {EphemeralServerError} If server fails to start or become healthy
   */
  async start(): Promise<void> {
    if (this.started) {
      throw new EphemeralServerError('Server already started');
    }

    // Find server binary
    const binary = this.serverBinary || findNorikvServerBinary();

    // Assign port if not specified
    if (!this.port) {
      this.port = await findFreePort();
    }

    // Create temp directory if needed
    let actualDataDir: string;
    if (!this.dataDir) {
      this.tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'norikv-ephemeral-'));
      actualDataDir = this.tempDir;
    } else {
      actualDataDir = this.dataDir;
      await mkdir(actualDataDir, { recursive: true });
    }

    // Create temp log file if needed
    let actualLogFile: string;
    if (!this.logFile) {
      this.tempLog = path.join(os.tmpdir(), `norikv-ephemeral-${randomHex(8)}.log`);
      actualLogFile = this.tempLog;
    } else {
      actualLogFile = this.logFile;
    }

    // Generate node configuration
    const nodeId = `ephemeral-${randomHex(8)}`;
    const config = {
      node_id: nodeId,
      rpc_addr: `127.0.0.1:${this.port}`,
      data_dir: actualDataDir,
      cluster: {
        seed_nodes: [], // Single node, no cluster
        total_shards: this.totalShards,
        replication_factor: 1,
      },
      telemetry: {
        prometheus: {
          enabled: false, // Disable metrics for ephemeral mode
        },
        otlp: {
          enabled: false,
        },
      },
    };

    // Write config to temp file
    this.configFile = path.join(os.tmpdir(), `norikv-config-${randomHex(8)}.yaml`);
    await writeFile(this.configFile, yaml.stringify(config));

    // Spawn server process
    try {
      const logStream = fs.createWriteStream(actualLogFile, { flags: 'w' });

      this.process = spawn(binary, [this.configFile], {
        env: { ...process.env, RUST_LOG: 'info' },
        stdio: ['ignore', logStream, logStream],
      });

      this.process.on('error', (err) => {
        throw new EphemeralServerError(`Failed to start server: ${err.message}`);
      });
    } catch (err) {
      await this.cleanup();
      throw new EphemeralServerError(`Failed to start server: ${(err as Error).message}`);
    }

    // Wait for server to become healthy
    try {
      await this.waitForHealth();
    } catch (err) {
      await this.cleanup();
      throw err;
    }

    this.started = true;
  }

  /**
   * Wait for server to become healthy.
   *
   * Polls the server by attempting to connect via gRPC until it responds
   * or the timeout is reached.
   *
   * @throws {EphemeralServerError} If server doesn't become healthy in time
   */
  private async waitForHealth(): Promise<void> {
    let lastError: Error | undefined;

    try {
      await waitFor(
        async () => {
          // Check if process has died
          if (this.process?.exitCode !== null) {
            throw new EphemeralServerError(
              `Server process exited with code ${this.process?.exitCode}. ` +
                `Check logs at: ${this.logFile || this.tempLog}`
            );
          }

          // Try to connect
          try {
            const testClient = new NoriKVClient({
              nodes: [`localhost:${this.port}`],
              totalShards: this.totalShards,
            });

            await testClient.connect();
            await testClient.close();

            // Success!
            return true;
          } catch (err) {
            lastError = err as Error;
            return false;
          }
        },
        this.startupTimeout
      );
    } catch {
      // Timeout reached
      throw new EphemeralServerError(
        `Server did not become healthy within ${this.startupTimeout / 1000}s. ` +
          `Last error: ${lastError?.message}. ` +
          `Check logs at: ${this.logFile || this.tempLog}`
      );
    }
  }

  /**
   * Get a client configured to connect to this ephemeral server.
   *
   * @throws {EphemeralServerError} If server is not started
   */
  getClient(): NoriKVClient {
    if (!this.started) {
      throw new EphemeralServerError('Server not started. Call start() first.');
    }

    if (!this.client) {
      this.client = new NoriKVClient({
        nodes: [`localhost:${this.port}`],
        totalShards: this.totalShards,
      });
    }

    return this.client;
  }

  /**
   * Stop the ephemeral server and clean up resources.
   *
   * This method:
   * 1. Sends SIGTERM to the server process
   * 2. Waits up to 5 seconds for graceful shutdown
   * 3. Sends SIGKILL if necessary
   * 4. Cleans up temporary files and directories
   */
  async stop(): Promise<void> {
    if (!this.started) {
      return;
    }

    // Close client if we created one
    if (this.client) {
      try {
        await this.client.close();
      } catch {
        // Ignore errors during client shutdown
      }
      this.client = undefined;
    }

    // Stop server process
    if (this.process) {
      try {
        this.process.kill('SIGTERM');

        // Wait up to 5 seconds for graceful shutdown
        await new Promise<void>((resolve) => {
          const timeout = setTimeout(() => {
            // Force kill if not terminated
            this.process?.kill('SIGKILL');
            resolve();
          }, 5000);

          this.process?.on('exit', () => {
            clearTimeout(timeout);
            resolve();
          });
        });
      } catch {
        // Ignore errors during process cleanup
      }

      this.process = undefined;
    }

    // Clean up temp resources
    await this.cleanup();
    this.started = false;
  }

  /**
   * Clean up temporary files and directories.
   */
  private async cleanup(): Promise<void> {
    if (this.configFile && fs.existsSync(this.configFile)) {
      try {
        await unlink(this.configFile);
      } catch {
        // Ignore errors
      }
      this.configFile = undefined;
    }

    if (this.tempLog && fs.existsSync(this.tempLog)) {
      try {
        await unlink(this.tempLog);
      } catch {
        // Ignore errors
      }
      this.tempLog = undefined;
    }

    if (this.tempDir && fs.existsSync(this.tempDir)) {
      try {
        // Remove directory recursively
        fs.rmSync(this.tempDir, { recursive: true, force: true });
      } catch {
        // Ignore errors
      }
      this.tempDir = undefined;
    }
  }
}

/**
 * Create and start an ephemeral NoriKV server.
 *
 * This is a convenience function that creates an EphemeralNoriKV instance
 * and starts it.
 *
 * @param options - Configuration options for the ephemeral server
 * @returns EphemeralNoriKV instance that is started and ready to use
 *
 * @example
 * ```typescript
 * const cluster = await createEphemeral();
 * const client = cluster.getClient();
 *
 * await client.connect();
 * await client.put('key', 'value');
 * const result = await client.get('key');
 * console.log(result.value?.toString());
 * await client.close();
 * await cluster.stop();
 * ```
 */
export async function createEphemeral(options: EphemeralOptions = {}): Promise<EphemeralNoriKV> {
  const cluster = new EphemeralNoriKV(options);
  await cluster.start();
  return cluster;
}
