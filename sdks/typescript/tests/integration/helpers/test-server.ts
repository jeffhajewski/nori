/**
 * Test server harness for E2E integration tests.
 *
 * Manages lifecycle of real NoriKV server instances for testing.
 * Uses child_process to spawn server binaries.
 */

import { spawn, type ChildProcess } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import { sleep } from './assertions';

export interface TestServerConfig {
  /** Server binary path (defaults to cargo-built binary) */
  serverBinary?: string;

  /** gRPC port (0 = random available port) */
  grpcPort?: number;

  /** HTTP port (0 = random available port) */
  httpPort?: number;

  /** Data directory (if not provided, creates temp dir) */
  dataDir?: string;

  /** Node ID */
  nodeId?: string;

  /** Seed nodes for cluster formation */
  seedNodes?: string[];

  /** Total shards in cluster */
  totalShards?: number;

  /** Enable verbose logging */
  verbose?: boolean;

  /** Additional environment variables */
  env?: Record<string, string>;
}

export interface TestServerInstance {
  /** Process handle */
  process: ChildProcess;

  /** Node ID */
  nodeId: string;

  /** gRPC address (host:port) */
  grpcAddr: string;

  /** HTTP address (host:port) */
  httpAddr: string;

  /** Data directory */
  dataDir: string;

  /** Cleanup function */
  cleanup: () => Promise<void>;
}

export interface TestClusterConfig {
  /** Number of nodes */
  nodeCount: number;

  /** Total shards */
  totalShards?: number;

  /** Server binary path */
  serverBinary?: string;

  /** Enable verbose logging */
  verbose?: boolean;

  /** Base port for allocation (default: 50051) */
  basePort?: number;
}

export interface TestCluster {
  /** All server instances */
  nodes: TestServerInstance[];

  /** Get addresses of all nodes */
  getAddresses: () => string[];

  /** Stop all nodes */
  stop: () => Promise<void>;

  /** Stop a specific node */
  stopNode: (nodeId: string) => Promise<void>;

  /** Restart a specific node */
  restartNode: (nodeId: string) => Promise<void>;

  /** Get node by ID */
  getNode: (nodeId: string) => TestServerInstance | undefined;
}

/**
 * Find the server binary path.
 * Looks for cargo-built binary in workspace.
 */
function findServerBinary(): string {
  // Look for debug build first
  const debugPath = path.join(
    __dirname,
    '../../../..',
    'target/debug/norikv-server'
  );
  if (fs.existsSync(debugPath)) {
    return debugPath;
  }

  // Try release build
  const releasePath = path.join(
    __dirname,
    '../../../..',
    'target/release/norikv-server'
  );
  if (fs.existsSync(releasePath)) {
    return releasePath;
  }

  throw new Error(
    'Server binary not found. Run "cargo build -p norikv-server" first.'
  );
}

/**
 * Create a temporary directory for test data.
 */
function createTempDir(prefix: string = 'norikv-test-'): string {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), prefix));
  return tmpDir;
}

/**
 * Wait for server to be ready by polling health endpoint.
 */
async function waitForServerReady(
  httpAddr: string,
  timeoutMs: number = 10000
): Promise<void> {
  const startTime = Date.now();
  const [host, port] = httpAddr.split(':');
  const healthUrl = `http://${host}:${port}/health`;

  while (Date.now() - startTime < timeoutMs) {
    try {
      // Try to fetch health endpoint
      const response = await fetch(healthUrl);
      if (response.ok) {
        return; // Server is ready
      }
    } catch (err) {
      // Server not ready yet, continue polling
    }

    await sleep(100);
  }

  throw new Error(
    `Server at ${httpAddr} did not become ready within ${timeoutMs}ms`
  );
}

/**
 * Start a single test server instance.
 *
 * @param config - Server configuration
 * @returns Server instance handle
 */
export async function startTestServer(
  config: TestServerConfig = {}
): Promise<TestServerInstance> {
  const serverBinary = config.serverBinary || findServerBinary();
  const nodeId = config.nodeId || `test-node-${Date.now()}`;
  const grpcPort = config.grpcPort || 0; // 0 = random port
  const httpPort = config.httpPort || 0;
  const dataDir = config.dataDir || createTempDir(`norikv-${nodeId}-`);
  const totalShards = config.totalShards || 1024;
  const verbose = config.verbose ?? false;

  // Ensure data directory exists
  if (!fs.existsSync(dataDir)) {
    fs.mkdirSync(dataDir, { recursive: true });
  }

  // Build command line args
  const args: string[] = [
    '--node-id',
    nodeId,
    '--grpc-addr',
    `127.0.0.1:${grpcPort}`,
    '--http-addr',
    `127.0.0.1:${httpPort}`,
    '--data-dir',
    dataDir,
    '--total-shards',
    totalShards.toString(),
  ];

  if (config.seedNodes && config.seedNodes.length > 0) {
    args.push('--seed-nodes', config.seedNodes.join(','));
  }

  // Build environment
  const env: Record<string, string> = {
    ...process.env,
    RUST_LOG: verbose ? 'debug' : 'info',
    ...config.env,
  };

  // Spawn server process
  const serverProcess = spawn(serverBinary, args, {
    env: env as NodeJS.ProcessEnv,
    stdio: verbose ? 'inherit' : 'ignore',
  });

  // Handle process errors
  serverProcess.on('error', (err) => {
    console.error(`Server process error (${nodeId}):`, err);
  });

  // Wait for server to output its addresses
  // For now, we'll use a simple approach: parse from config
  // In a real implementation, we'd parse stdout for actual bound addresses
  const grpcAddr = grpcPort === 0 ? `127.0.0.1:50051` : `127.0.0.1:${grpcPort}`;
  const httpAddr = httpPort === 0 ? `127.0.0.1:8080` : `127.0.0.1:${httpPort}`;

  // Wait for server to be ready
  try {
    await waitForServerReady(httpAddr);
  } catch (err) {
    // Kill process if startup failed
    serverProcess.kill('SIGTERM');
    throw err;
  }

  // Cleanup function
  const cleanup = async () => {
    // Graceful shutdown
    serverProcess.kill('SIGTERM');

    // Wait for process to exit
    await new Promise<void>((resolve) => {
      const timeout = setTimeout(() => {
        // Force kill if graceful shutdown takes too long
        serverProcess.kill('SIGKILL');
        resolve();
      }, 5000);

      serverProcess.on('exit', () => {
        clearTimeout(timeout);
        resolve();
      });
    });

    // Clean up data directory if we created it
    if (!config.dataDir) {
      try {
        fs.rmSync(dataDir, { recursive: true, force: true });
      } catch (err) {
        console.warn(`Failed to clean up data dir ${dataDir}:`, err);
      }
    }
  };

  return {
    process: serverProcess,
    nodeId,
    grpcAddr,
    httpAddr,
    dataDir,
    cleanup,
  };
}

/**
 * Start a test cluster with multiple nodes.
 *
 * @param config - Cluster configuration
 * @returns Cluster handle
 */
export async function startTestCluster(
  config: TestClusterConfig
): Promise<TestCluster> {
  const { nodeCount, totalShards = 1024, serverBinary, verbose = false } = config;
  const basePort = config.basePort || 50051;

  if (nodeCount < 1) {
    throw new Error('Node count must be at least 1');
  }

  const nodes: TestServerInstance[] = [];
  const nodeMap = new Map<string, TestServerInstance>();

  try {
    // Start first node (seed node)
    const firstNode = await startTestServer({
      serverBinary,
      nodeId: `node-0`,
      grpcPort: basePort,
      httpPort: basePort + 1000,
      totalShards,
      verbose,
    });
    nodes.push(firstNode);
    nodeMap.set(firstNode.nodeId, firstNode);

    // Start remaining nodes, pointing to first node as seed
    const seedNodes = [firstNode.grpcAddr];

    for (let i = 1; i < nodeCount; i++) {
      const node = await startTestServer({
        serverBinary,
        nodeId: `node-${i}`,
        grpcPort: basePort + i,
        httpPort: basePort + 1000 + i,
        seedNodes,
        totalShards,
        verbose,
      });
      nodes.push(node);
      nodeMap.set(node.nodeId, node);

      // Wait a bit between node starts for cluster formation
      await sleep(500);
    }

    // Wait for cluster to stabilize
    await sleep(2000);

    return {
      nodes,

      getAddresses: () => nodes.map((n) => n.grpcAddr),

      stop: async () => {
        // Stop all nodes in parallel
        await Promise.all(nodes.map((n) => n.cleanup()));
        nodes.length = 0;
        nodeMap.clear();
      },

      stopNode: async (nodeId: string) => {
        const node = nodeMap.get(nodeId);
        if (!node) {
          throw new Error(`Node ${nodeId} not found`);
        }

        await node.cleanup();
        nodeMap.delete(nodeId);

        const idx = nodes.findIndex((n) => n.nodeId === nodeId);
        if (idx >= 0) {
          nodes.splice(idx, 1);
        }
      },

      restartNode: async (nodeId: string) => {
        // Find original node config
        const originalIdx = nodes.findIndex((n) => n.nodeId === nodeId);
        if (originalIdx < 0) {
          throw new Error(`Node ${nodeId} not found`);
        }

        const original = nodes[originalIdx];
        const seedNodes = nodes
          .filter((n) => n.nodeId !== nodeId)
          .map((n) => n.grpcAddr);

        // Stop node
        await original.cleanup();

        // Restart with same config
        const restarted = await startTestServer({
          serverBinary,
          nodeId,
          grpcPort: parseInt(original.grpcAddr.split(':')[1]),
          httpPort: parseInt(original.httpAddr.split(':')[1]),
          dataDir: original.dataDir,
          seedNodes,
          totalShards,
          verbose,
        });

        nodes[originalIdx] = restarted;
        nodeMap.set(nodeId, restarted);
      },

      getNode: (nodeId: string) => nodeMap.get(nodeId),
    };
  } catch (err) {
    // Clean up any started nodes on error
    await Promise.all(nodes.map((n) => n.cleanup()));
    throw err;
  }
}

/**
 * Helper to run a test with a single server instance.
 */
export async function withTestServer<T>(
  config: TestServerConfig,
  fn: (server: TestServerInstance) => Promise<T>
): Promise<T> {
  const server = await startTestServer(config);
  try {
    return await fn(server);
  } finally {
    await server.cleanup();
  }
}

/**
 * Helper to run a test with a cluster.
 */
export async function withTestCluster<T>(
  config: TestClusterConfig,
  fn: (cluster: TestCluster) => Promise<T>
): Promise<T> {
  const cluster = await startTestCluster(config);
  try {
    return await fn(cluster);
  } finally {
    await cluster.stop();
  }
}
