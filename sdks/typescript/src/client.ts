/**
 * Main NoriKV client implementation.
 *
 * This is the primary entry point for interacting with a NoriKV cluster.
 */

import * as grpc from '@grpc/grpc-js';
import * as protoLoader from '@grpc/proto-loader';
import { initializeHasher, keyToBytes, valueToBytes, bytesToString } from './hash.js';
import { ConnectionPool } from './connection.js';
import { TopologyManager } from './topology.js';
import { Router } from './router.js';
import { withRetry, DEFAULT_RETRY_CONFIG } from './retry.js';
import {
  fromProtoVersion,
  toProtoVersion,
  fromProtoClusterView,
  type ProtoPutRequest,
  type ProtoPutResponse,
  type ProtoGetRequest,
  type ProtoGetResponse,
  type ProtoDeleteRequest,
  type ProtoDeleteResponse,
} from './proto-types.js';
import {
  NoriKVError,
  NotLeaderError,
  fromGrpcError,
  InvalidArgumentError,
  NoNodesAvailableError,
} from './errors.js';
import type {
  Key,
  Value,
  ClientConfig,
  PutOptions,
  GetOptions,
  DeleteOptions,
  GetResult,
  ClusterView,
  Version,
  ConsistencyLevel,
} from './types.js';

/**
 * Main NoriKV client.
 *
 * @example
 * ```ts
 * const client = new NoriKVClient({
 *   nodes: ['localhost:50051', 'localhost:50052', 'localhost:50053'],
 *   totalShards: 1024,
 *   timeout: 5000,
 * });
 *
 * await client.connect();
 *
 * // Put a value
 * await client.put('user:123', 'Alice');
 *
 * // Get a value
 * const result = await client.get('user:123');
 * console.log(bytesToString(result.value)); // 'Alice'
 *
 * // Delete a value
 * await client.delete('user:123');
 *
 * await client.close();
 * ```
 */
export class NoriKVClient {
  private config: Required<ClientConfig>;
  private connectionPool: ConnectionPool;
  private topology: TopologyManager;
  private router: Router;
  private initialized = false;
  private closed = false;
  private clusterWatchStream?: grpc.ClientReadableStream<any>;

  constructor(config: ClientConfig) {
    if (!config.nodes || config.nodes.length === 0) {
      throw new InvalidArgumentError('At least one node address is required');
    }

    this.config = {
      nodes: config.nodes,
      totalShards: config.totalShards ?? 1024,
      timeout: config.timeout ?? 5000,
      retry: {
        ...DEFAULT_RETRY_CONFIG,
        ...config.retry,
      },
      watchCluster: config.watchCluster ?? true,
      maxConnectionsPerNode: config.maxConnectionsPerNode ?? 10,
    };

    this.connectionPool = new ConnectionPool({
      maxConnectionsPerNode: this.config.maxConnectionsPerNode,
      connectTimeout: this.config.timeout,
    });

    this.topology = new TopologyManager();
    this.router = new Router(this.topology, this.config.totalShards);

    // Initialize topology with seed nodes
    const seedView: ClusterView = {
      epoch: 0n,
      nodes: this.config.nodes.map((addr, idx) => ({
        id: `seed-${idx}`,
        addr,
        role: 'unknown',
      })),
      shards: [],
    };
    this.topology.updateView(seedView);
  }

  /**
   * Connect to the cluster and initialize the client.
   *
   * This will:
   * - Initialize the hash function (xxhash WASM module)
   * - Connect to seed nodes
   * - Fetch initial cluster view
   * - Start watching cluster changes (if enabled)
   */
  async connect(): Promise<void> {
    if (this.initialized) {
      return;
    }

    // Initialize hash function
    await initializeHasher();

    // Try to connect to any seed node
    const errors: Error[] = [];
    for (const addr of this.config.nodes) {
      try {
        await this.connectionPool.getConnection(addr);
        break; // Successfully connected
      } catch (err) {
        errors.push(err instanceof Error ? err : new Error(String(err)));
      }
    }

    if (errors.length === this.config.nodes.length) {
      throw new NoNodesAvailableError(
        `Failed to connect to any seed node: ${errors.map((e) => e.message).join(', ')}`
      );
    }

    // TODO: Fetch initial cluster view via gRPC Meta.WatchCluster
    // For now, we'll use the seed view

    // Start watching cluster changes
    if (this.config.watchCluster) {
      this.startClusterWatch();
    }

    this.initialized = true;
  }

  /**
   * Put a key-value pair.
   *
   * @param key - The key
   * @param value - The value
   * @param options - Put options
   * @returns Version of the written value
   *
   * @example
   * ```ts
   * // Simple put
   * const version = await client.put('key', 'value');
   *
   * // Put with TTL (10 seconds)
   * await client.put('key', 'value', { ttlMs: 10000 });
   *
   * // Conditional put (only if not exists)
   * await client.put('key', 'value', { ifNotExists: true });
   *
   * // Optimistic locking (CAS)
   * const result = await client.get('key');
   * await client.put('key', 'new-value', { ifMatchVersion: result.version });
   * ```
   */
  async put(key: Key, value: Value, options: PutOptions = {}): Promise<Version> {
    this.ensureInitialized();

    const keyBytes = keyToBytes(key);
    const valueBytes = valueToBytes(value);

    const request: ProtoPutRequest = {
      key: keyBytes,
      value: valueBytes,
      ttl_ms: options.ttlMs?.toString(),
      idempotency_key: options.idempotencyKey,
      if_match: toProtoVersion(options.ifMatchVersion),
    };

    // Route to the correct shard leader
    const routing = this.router.route(key, true);

    const operation = async () => {
      // Try primary target
      try {
        const response = await this.putInternal(routing.primaryAddr, request);
        return response;
      } catch (err) {
        // Handle NOT_LEADER redirect
        if (err instanceof NotLeaderError && err.leaderHint) {
          this.router.handleNotLeader(routing.shardId, err.leaderHint);
          // Retry on the correct leader
          const response = await this.putInternal(err.leaderHint, request);
          return response;
        }
        throw err;
      }
    };

    return await withRetry(
      async () => operation(),
      this.config.retry,
      {
        isIdempotent: !!options.idempotencyKey,
        idempotencyKey: options.idempotencyKey,
      }
    );
  }

  /**
   * Internal put operation without retry logic.
   */
  private async putInternal(address: string, request: ProtoPutRequest): Promise<Version> {
    const channel = await this.connectionPool.getConnection(address);

    // TODO: Use actual gRPC client stub
    // For now, throw not implemented
    throw new Error('gRPC stub not yet implemented - pending proto generation');

    // This is what it will look like once we have generated stubs:
    // const client = new KvClient(address, grpc.credentials.createInsecure());
    // const response: ProtoPutResponse = await new Promise((resolve, reject) => {
    //   client.put(request, (err, response) => {
    //     if (err) reject(fromGrpcError(err));
    //     else resolve(response);
    //   });
    // });
    // return fromProtoVersion(response.version)!;
  }

  /**
   * Get a value by key.
   *
   * @param key - The key
   * @param options - Get options
   * @returns Get result with value and version
   *
   * @example
   * ```ts
   * // Default read (lease-based linearizable)
   * const result = await client.get('key');
   * if (result.value) {
   *   console.log(bytesToString(result.value));
   * }
   *
   * // Stale read (fastest, may be stale)
   * const result = await client.get('key', { consistency: 'stale_ok' });
   *
   * // Strict linearizable read
   * const result = await client.get('key', { consistency: 'linearizable' });
   * ```
   */
  async get(key: Key, options: GetOptions = {}): Promise<GetResult> {
    this.ensureInitialized();

    const keyBytes = keyToBytes(key);
    const consistency = options.consistency ?? 'lease';

    const request: ProtoGetRequest = {
      key: keyBytes,
      consistency,
    };

    // Route to appropriate node
    const preferLeader = consistency === 'lease' || consistency === 'linearizable';
    const routing = this.router.route(key, preferLeader);

    const operation = async () => {
      try {
        const response = await this.getInternal(routing.primaryAddr, request);
        return response;
      } catch (err) {
        // For stale reads, try fallback replicas
        if (!preferLeader && routing.fallbackAddrs.length > 0) {
          for (const fallbackAddr of routing.fallbackAddrs) {
            try {
              const response = await this.getInternal(fallbackAddr, request);
              return response;
            } catch (fallbackErr) {
              // Continue to next fallback
            }
          }
        }
        throw err;
      }
    };

    return await withRetry(
      async () => operation(),
      this.config.retry,
      {
        isIdempotent: true, // Reads are always idempotent
      }
    );
  }

  /**
   * Internal get operation without retry logic.
   */
  private async getInternal(address: string, request: ProtoGetRequest): Promise<GetResult> {
    const channel = await this.connectionPool.getConnection(address);

    // TODO: Use actual gRPC client stub
    throw new Error('gRPC stub not yet implemented - pending proto generation');

    // This is what it will look like:
    // const client = new KvClient(address, grpc.credentials.createInsecure());
    // const response: ProtoGetResponse = await new Promise((resolve, reject) => {
    //   client.get(request, (err, response) => {
    //     if (err) reject(fromGrpcError(err));
    //     else resolve(response);
    //   });
    // });
    // return {
    //   value: response.value ?? null,
    //   version: fromProtoVersion(response.version),
    //   metadata: response.meta,
    // };
  }

  /**
   * Delete a key.
   *
   * @param key - The key to delete
   * @param options - Delete options
   * @returns Whether the key was deleted (true) or didn't exist (false)
   *
   * @example
   * ```ts
   * // Simple delete
   * await client.delete('key');
   *
   * // Conditional delete (optimistic locking)
   * const result = await client.get('key');
   * await client.delete('key', { ifMatchVersion: result.version });
   * ```
   */
  async delete(key: Key, options: DeleteOptions = {}): Promise<boolean> {
    this.ensureInitialized();

    const keyBytes = keyToBytes(key);

    const request: ProtoDeleteRequest = {
      key: keyBytes,
      idempotency_key: options.idempotencyKey,
      if_match: toProtoVersion(options.ifMatchVersion),
    };

    // Route to the correct shard leader
    const routing = this.router.route(key, true);

    const operation = async () => {
      try {
        const response = await this.deleteInternal(routing.primaryAddr, request);
        return response;
      } catch (err) {
        if (err instanceof NotLeaderError && err.leaderHint) {
          this.router.handleNotLeader(routing.shardId, err.leaderHint);
          const response = await this.deleteInternal(err.leaderHint, request);
          return response;
        }
        throw err;
      }
    };

    return await withRetry(
      async () => operation(),
      this.config.retry,
      {
        isIdempotent: !!options.idempotencyKey,
        idempotencyKey: options.idempotencyKey,
      }
    );
  }

  /**
   * Internal delete operation without retry logic.
   */
  private async deleteInternal(address: string, request: ProtoDeleteRequest): Promise<boolean> {
    const channel = await this.connectionPool.getConnection(address);

    // TODO: Use actual gRPC client stub
    throw new Error('gRPC stub not yet implemented - pending proto generation');

    // This is what it will look like:
    // const client = new KvClient(address, grpc.credentials.createInsecure());
    // const response: ProtoDeleteResponse = await new Promise((resolve, reject) => {
    //   client.delete(request, (err, response) => {
    //     if (err) reject(fromGrpcError(err));
    //     else resolve(response);
    //   });
    // });
    // return response.tombstoned;
  }

  /**
   * Get current cluster view.
   */
  getClusterView(): ClusterView | null {
    return this.topology.getView();
  }

  /**
   * Register a listener for cluster topology changes.
   *
   * @param listener - Callback to invoke on topology changes
   * @returns Unsubscribe function
   */
  onTopologyChange(listener: (event: any) => void): () => void {
    return this.topology.onTopologyChange(listener);
  }

  /**
   * Close the client and release all resources.
   */
  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;

    // Stop cluster watch
    if (this.clusterWatchStream) {
      this.clusterWatchStream.cancel();
      this.clusterWatchStream = undefined;
    }

    // Close connection pool
    await this.connectionPool.close();

    // Clear topology
    this.topology.clear();

    this.initialized = false;
  }

  /**
   * Check if client is connected.
   */
  isConnected(): boolean {
    return this.initialized && !this.closed;
  }

  /**
   * Start watching cluster topology changes.
   */
  private startClusterWatch(): void {
    // TODO: Implement cluster watch via gRPC Meta.WatchCluster
    // For now, this is a placeholder
    // const randomNode = this.router.getRandomNode();
    // const channel = await this.connectionPool.getConnection(randomNode);
    // const client = new MetaClient(randomNode, grpc.credentials.createInsecure());
    // this.clusterWatchStream = client.watchCluster({});
    // this.clusterWatchStream.on('data', (view: ProtoClusterView) => {
    //   const clusterView = fromProtoClusterView(view);
    //   this.topology.updateView(clusterView);
    // });
    // this.clusterWatchStream.on('error', (err) => {
    //   console.error('Cluster watch error:', err);
    //   // Try to reconnect
    //   setTimeout(() => this.startClusterWatch(), 5000);
    // });
  }

  /**
   * Ensure the client is initialized.
   */
  private ensureInitialized(): void {
    if (!this.initialized) {
      throw new Error('Client not initialized. Call connect() first.');
    }
    if (this.closed) {
      throw new Error('Client is closed.');
    }
  }
}

/**
 * Create a new NoriKV client.
 *
 * @param config - Client configuration
 * @returns NoriKVClient instance
 */
export function createClient(config: ClientConfig): NoriKVClient {
  return new NoriKVClient(config);
}
