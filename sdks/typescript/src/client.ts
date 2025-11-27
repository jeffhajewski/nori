/**
 * Main NoriKV client implementation.
 *
 * This is the primary entry point for interacting with a NoriKV cluster.
 */

import * as grpc from '@grpc/grpc-js';
import { initializeHasher, keyToBytes, valueToBytes } from '@norikv/client/hash';
import { ConnectionPool } from '@norikv/client/connection';
import { TopologyManager } from '@norikv/client/topology';
import { Router } from '@norikv/client/router';
import { withRetry, DEFAULT_RETRY_CONFIG } from '@norikv/client/retry';
import {
  fromProtoVersion,
  toProtoVersion,
  fromProtoClusterView,
  toProtoClusterView,
  toProtoDistanceFunction,
  toProtoVectorIndexType,
  type ProtoPutRequest,
  type ProtoGetRequest,
  type ProtoDeleteRequest,
  type ProtoCreateVectorIndexRequest,
  type ProtoDropVectorIndexRequest,
  type ProtoVectorInsertRequest,
  type ProtoVectorDeleteRequest,
  type ProtoVectorSearchRequest,
  type ProtoVectorGetRequest,
} from '@norikv/client/proto-types';
import {
  NotLeaderError,
  InvalidArgumentError,
  NoNodesAvailableError,
  NotFoundError,
} from '@norikv/client/errors';
import {
  kvPut,
  kvGet,
  kvDelete,
  metaWatchCluster,
  vectorCreateIndex,
  vectorDropIndex,
  vectorInsert,
  vectorDelete,
  vectorSearch,
  vectorGet,
} from '@norikv/client/grpc-services';
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
  DistanceFunction,
  VectorIndexType,
  CreateVectorIndexOptions,
  DropVectorIndexOptions,
  VectorInsertOptions,
  VectorDeleteOptions,
  VectorSearchOptions,
  VectorSearchResult,
  VectorMatch,
} from '@norikv/client/types';

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
      ttlMs: options.ttlMs || 0,
      idempotencyKey: options.idempotencyKey || '',
      ifMatch: toProtoVersion(options.ifMatchVersion),
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
    const client = await this.connectionPool.getKvClient(address);
    const response = await kvPut(client, request, this.config.timeout);

    const version = fromProtoVersion(response.version);
    if (!version) {
      throw new Error('Server returned empty version');
    }

    return version;
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
    const client = await this.connectionPool.getKvClient(address);
    const response = await kvGet(client, request, this.config.timeout);

    return {
      value: response.value && response.value.length > 0 ? response.value : null,
      version: fromProtoVersion(response.version),
      metadata: response.meta,
    };
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
      idempotencyKey: options.idempotencyKey || '',
      ifMatch: toProtoVersion(options.ifMatchVersion),
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
    const client = await this.connectionPool.getKvClient(address);
    const response = await kvDelete(client, request, this.config.timeout);
    return response.tombstoned;
  }

  // ===========================================================================
  // Vector Operations
  // ===========================================================================

  /**
   * Create a vector index.
   *
   * @param namespace - The namespace/index name
   * @param dimensions - Number of dimensions for vectors
   * @param distance - Distance function (euclidean, cosine, inner_product)
   * @param indexType - Index type (brute_force, hnsw)
   * @param options - Create options
   * @returns true if created, false if already existed
   *
   * @example
   * ```ts
   * const created = await client.vectorCreateIndex(
   *   'embeddings',
   *   1536,
   *   'cosine',
   *   'hnsw'
   * );
   * ```
   */
  async vectorCreateIndex(
    namespace: string,
    dimensions: number,
    distance: DistanceFunction,
    indexType: VectorIndexType,
    options: CreateVectorIndexOptions = {}
  ): Promise<boolean> {
    this.ensureInitialized();

    if (!namespace) {
      throw new InvalidArgumentError('namespace cannot be empty');
    }
    if (dimensions <= 0) {
      throw new InvalidArgumentError('dimensions must be greater than 0');
    }

    const request: ProtoCreateVectorIndexRequest = {
      namespace,
      dimensions,
      distance: toProtoDistanceFunction(distance),
      indexType: toProtoVectorIndexType(indexType),
      idempotencyKey: options.idempotencyKey || '',
    };

    return await withRetry(
      async () => this.vectorCreateIndexInternal(request),
      this.config.retry,
      { isIdempotent: !!options.idempotencyKey }
    );
  }

  private async vectorCreateIndexInternal(request: ProtoCreateVectorIndexRequest): Promise<boolean> {
    const address = await this.getAnyAddress();
    const client = await this.connectionPool.getVectorClient(address);
    const response = await vectorCreateIndex(client, request, this.config.timeout);
    return response.created;
  }

  /**
   * Drop a vector index.
   *
   * @param namespace - The namespace/index name to drop
   * @param options - Drop options
   * @returns true if dropped, false if didn't exist
   *
   * @example
   * ```ts
   * const dropped = await client.vectorDropIndex('embeddings');
   * ```
   */
  async vectorDropIndex(
    namespace: string,
    options: DropVectorIndexOptions = {}
  ): Promise<boolean> {
    this.ensureInitialized();

    if (!namespace) {
      throw new InvalidArgumentError('namespace cannot be empty');
    }

    const request: ProtoDropVectorIndexRequest = {
      namespace,
      idempotencyKey: options.idempotencyKey || '',
    };

    return await withRetry(
      async () => this.vectorDropIndexInternal(request),
      this.config.retry,
      { isIdempotent: !!options.idempotencyKey }
    );
  }

  private async vectorDropIndexInternal(request: ProtoDropVectorIndexRequest): Promise<boolean> {
    const address = await this.getAnyAddress();
    const client = await this.connectionPool.getVectorClient(address);
    const response = await vectorDropIndex(client, request, this.config.timeout);
    return response.dropped;
  }

  /**
   * Insert a vector into an index.
   *
   * @param namespace - The namespace/index name
   * @param id - Unique ID for the vector
   * @param vector - The vector data
   * @param options - Insert options
   * @returns Version of the inserted vector
   *
   * @example
   * ```ts
   * const version = await client.vectorInsert(
   *   'embeddings',
   *   'doc-123',
   *   [0.1, 0.2, 0.3, ...]
   * );
   * ```
   */
  async vectorInsert(
    namespace: string,
    id: string,
    vector: number[],
    options: VectorInsertOptions = {}
  ): Promise<Version> {
    this.ensureInitialized();

    if (!namespace) {
      throw new InvalidArgumentError('namespace cannot be empty');
    }
    if (!id) {
      throw new InvalidArgumentError('id cannot be empty');
    }
    if (!vector || vector.length === 0) {
      throw new InvalidArgumentError('vector cannot be empty');
    }

    const request: ProtoVectorInsertRequest = {
      namespace,
      id,
      vector,
      idempotencyKey: options.idempotencyKey || '',
    };

    return await withRetry(
      async () => this.vectorInsertInternal(request),
      this.config.retry,
      { isIdempotent: !!options.idempotencyKey }
    );
  }

  private async vectorInsertInternal(request: ProtoVectorInsertRequest): Promise<Version> {
    const address = await this.getAnyAddress();
    const client = await this.connectionPool.getVectorClient(address);
    const response = await vectorInsert(client, request, this.config.timeout);

    const version = fromProtoVersion(response.version);
    if (!version) {
      throw new Error('Server returned empty version');
    }
    return version;
  }

  /**
   * Delete a vector from an index.
   *
   * @param namespace - The namespace/index name
   * @param id - ID of the vector to delete
   * @param options - Delete options
   * @returns true if deleted, false if didn't exist
   *
   * @example
   * ```ts
   * const deleted = await client.vectorDelete('embeddings', 'doc-123');
   * ```
   */
  async vectorDelete(
    namespace: string,
    id: string,
    options: VectorDeleteOptions = {}
  ): Promise<boolean> {
    this.ensureInitialized();

    if (!namespace) {
      throw new InvalidArgumentError('namespace cannot be empty');
    }
    if (!id) {
      throw new InvalidArgumentError('id cannot be empty');
    }

    const request: ProtoVectorDeleteRequest = {
      namespace,
      id,
      idempotencyKey: options.idempotencyKey || '',
    };

    return await withRetry(
      async () => this.vectorDeleteInternal(request),
      this.config.retry,
      { isIdempotent: !!options.idempotencyKey }
    );
  }

  private async vectorDeleteInternal(request: ProtoVectorDeleteRequest): Promise<boolean> {
    const address = await this.getAnyAddress();
    const client = await this.connectionPool.getVectorClient(address);
    const response = await vectorDelete(client, request, this.config.timeout);
    return response.deleted;
  }

  /**
   * Search for nearest neighbors.
   *
   * @param namespace - The namespace/index name
   * @param query - Query vector
   * @param k - Number of nearest neighbors to return
   * @param options - Search options
   * @returns Search results with matches and timing
   *
   * @example
   * ```ts
   * const result = await client.vectorSearch(
   *   'embeddings',
   *   [0.1, 0.2, 0.3, ...],
   *   10,
   *   { includeVectors: true }
   * );
   * for (const match of result.matches) {
   *   console.log(`${match.id}: distance=${match.distance}`);
   * }
   * ```
   */
  async vectorSearch(
    namespace: string,
    query: number[],
    k: number,
    options: VectorSearchOptions = {}
  ): Promise<VectorSearchResult> {
    this.ensureInitialized();

    if (!namespace) {
      throw new InvalidArgumentError('namespace cannot be empty');
    }
    if (!query || query.length === 0) {
      throw new InvalidArgumentError('query vector cannot be empty');
    }
    if (k <= 0) {
      throw new InvalidArgumentError('k must be greater than 0');
    }

    const request: ProtoVectorSearchRequest = {
      namespace,
      query,
      k,
      includeVectors: options.includeVectors || false,
    };

    return await withRetry(
      async () => this.vectorSearchInternal(request),
      this.config.retry,
      { isIdempotent: true }
    );
  }

  private async vectorSearchInternal(request: ProtoVectorSearchRequest): Promise<VectorSearchResult> {
    const address = await this.getAnyAddress();
    const client = await this.connectionPool.getVectorClient(address);
    const response = await vectorSearch(client, request, this.config.timeout);

    const matches: VectorMatch[] = (response.matches || []).map((m) => ({
      id: m.id,
      distance: m.distance,
      vector: m.vector,
    }));

    return {
      matches,
      searchTimeUs: BigInt(response.searchTimeUs || 0),
    };
  }

  /**
   * Get a vector by ID.
   *
   * @param namespace - The namespace/index name
   * @param id - ID of the vector
   * @returns The vector data, or null if not found
   *
   * @example
   * ```ts
   * const vector = await client.vectorGet('embeddings', 'doc-123');
   * if (vector) {
   *   console.log(`Vector has ${vector.length} dimensions`);
   * }
   * ```
   */
  async vectorGet(
    namespace: string,
    id: string
  ): Promise<number[] | null> {
    this.ensureInitialized();

    if (!namespace) {
      throw new InvalidArgumentError('namespace cannot be empty');
    }
    if (!id) {
      throw new InvalidArgumentError('id cannot be empty');
    }

    const request: ProtoVectorGetRequest = {
      namespace,
      id,
    };

    return await withRetry(
      async () => this.vectorGetInternal(request, id),
      this.config.retry,
      { isIdempotent: true }
    );
  }

  private async vectorGetInternal(request: ProtoVectorGetRequest, id: string): Promise<number[] | null> {
    const address = await this.getAnyAddress();
    const client = await this.connectionPool.getVectorClient(address);
    const response = await vectorGet(client, request, this.config.timeout);

    if (!response.found) {
      return null;
    }
    return response.vector || null;
  }

  /**
   * Get any available node address (for operations that don't need routing).
   */
  private async getAnyAddress(): Promise<string> {
    const view = this.topology.getView();
    if (view && view.nodes.length > 0) {
      return view.nodes[0].addr;
    }
    if (this.config.nodes.length > 0) {
      return this.config.nodes[0];
    }
    throw new NoNodesAvailableError();
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
  private async startClusterWatch(): Promise<void> {
    try {
      // Pick a random node to watch from
      const nodes = this.config.nodes;
      const randomNode = nodes[Math.floor(Math.random() * nodes.length)];

      const client = await this.connectionPool.getMetaClient(randomNode);
      const currentView = this.topology.getView();

      this.clusterWatchStream = metaWatchCluster(
        client,
        currentView ? toProtoClusterView(currentView) : undefined
      );

      this.clusterWatchStream.on('data', (view) => {
        const clusterView = fromProtoClusterView(view);
        this.topology.updateView(clusterView);
      });

      this.clusterWatchStream.on('error', (err) => {
        console.error('Cluster watch error:', err);
        if (!this.closed) {
          // Try to reconnect after a delay
          setTimeout(() => {
            this.startClusterWatch().catch((e) => {
              console.error('Failed to restart cluster watch:', e);
            });
          }, 5000);
        }
      });

      this.clusterWatchStream.on('end', () => {
        if (!this.closed) {
          // Stream ended, try to reconnect
          setTimeout(() => {
            this.startClusterWatch().catch((e) => {
              console.error('Failed to restart cluster watch:', e);
            });
          }, 1000);
        }
      });
    } catch (err) {
      console.error('Failed to start cluster watch:', err);
      if (!this.closed) {
        setTimeout(() => {
          this.startClusterWatch().catch((e) => {
            console.error('Failed to restart cluster watch:', e);
          });
        }, 5000);
      }
    }
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
