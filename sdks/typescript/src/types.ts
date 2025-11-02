/**
 * TypeScript type definitions for NoriKV client.
 */

/**
 * Key type - can be string or raw bytes.
 */
export type Key = Uint8Array | string;

/**
 * Value type - can be string or raw bytes.
 */
export type Value = Uint8Array | string;

/**
 * Version identifies a specific version of a key-value pair.
 * Corresponds to the Raft log index where it was committed.
 */
export interface Version {
  /** Raft term when this version was written */
  term: bigint;
  /** Raft log index */
  index: bigint;
}

/**
 * Options for Put operations.
 */
export interface PutOptions {
  /** Time-to-live in milliseconds. 0 means no expiration. */
  ttlMs?: number;

  /** Only write if key doesn't exist. Returns error if key exists. */
  ifNotExists?: boolean;

  /** Only write if current version matches. For optimistic concurrency control. */
  ifMatchVersion?: Version;

  /** Idempotency key for retries. Same key + idempotency_key = same result. */
  idempotencyKey?: string;
}

/**
 * Consistency level for Get operations.
 */
export type ConsistencyLevel =
  | 'lease'         // Read from leader with lease (linearizable, fast)
  | 'linearizable'  // Full linearizable read (read-index + quorum)
  | 'stale_ok';     // Read from any replica (eventual consistency, fastest)

/**
 * Options for Get operations.
 */
export interface GetOptions {
  /** Consistency level for this read */
  consistency?: ConsistencyLevel;
}

/**
 * Options for Delete operations.
 */
export interface DeleteOptions {
  /** Only delete if current version matches */
  ifMatchVersion?: Version;

  /** Idempotency key for retries */
  idempotencyKey?: string;
}

/**
 * Result of a Get operation.
 */
export interface GetResult {
  /** The value, or null if key doesn't exist */
  value: Uint8Array | null;

  /** Version of the value, or null if key doesn't exist */
  version: Version | null;

  /** Metadata from the server */
  metadata?: Record<string, string>;
}

/**
 * Cluster node information.
 */
export interface ClusterNode {
  /** Unique node ID */
  id: string;

  /** gRPC address (host:port) */
  addr: string;

  /** Node role: "leader" | "follower" | "candidate" */
  role: string;
}

/**
 * Shard replica information.
 */
export interface ShardReplica {
  /** Node ID hosting this replica */
  nodeId: string;

  /** Whether this replica is the leader for this shard */
  leader: boolean;
}

/**
 * Shard information.
 */
export interface ShardInfo {
  /** Shard ID (0 to totalShards-1) */
  id: number;

  /** Replicas for this shard */
  replicas: ShardReplica[];
}

/**
 * Complete cluster view.
 */
export interface ClusterView {
  /** Monotonically increasing epoch number */
  epoch: bigint;

  /** All nodes in the cluster */
  nodes: ClusterNode[];

  /** All shards and their replica assignments */
  shards: ShardInfo[];
}

/**
 * Connection configuration for a single node.
 */
export interface NodeConnection {
  /** Node address (host:port) */
  address: string;

  /** gRPC channel (managed internally) */
  channel?: any; // grpc.Channel

  /** Last known status */
  status: 'connected' | 'connecting' | 'disconnected' | 'failed';

  /** Last successful health check timestamp */
  lastHealthCheck?: Date;
}

/**
 * Configuration for NoriKVClient.
 */
export interface ClientConfig {
  /** List of node addresses to connect to (at least one required) */
  nodes: string[];

  /** Total number of virtual shards (default: 1024) */
  totalShards?: number;

  /** Default request timeout in milliseconds (default: 5000) */
  timeout?: number;

  /** Retry policy configuration */
  retry?: RetryConfig;

  /** Enable cluster view watching (default: true) */
  watchCluster?: boolean;

  /** Max connections per node (default: 10) */
  maxConnectionsPerNode?: number;
}

/**
 * Retry policy configuration.
 */
export interface RetryConfig {
  /** Maximum number of retry attempts (default: 3) */
  maxAttempts?: number;

  /** Initial backoff delay in milliseconds (default: 10) */
  initialDelayMs?: number;

  /** Maximum backoff delay in milliseconds (default: 1000) */
  maxDelayMs?: number;

  /** Backoff multiplier (default: 2) */
  backoffMultiplier?: number;

  /** Maximum jitter in milliseconds (default: 100) */
  jitterMs?: number;

  /** Whether to retry on NOT_LEADER errors (default: true) */
  retryOnNotLeader?: boolean;
}

/**
 * Routing information for a key.
 */
export interface RouteInfo {
  /** Shard ID this key belongs to */
  shardId: number;

  /** Leader node address for this shard (if known) */
  leaderAddr: string | null;

  /** All replica addresses for this shard */
  replicaAddrs: string[];
}
