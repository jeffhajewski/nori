/**
 * NoriKV TypeScript Client
 *
 * A smart client for NoriKV - a sharded, Raft-replicated key-value store.
 *
 * @example
 * ```ts
 * import { NoriKVClient, bytesToString } from '@norikv/client';
 *
 * const client = new NoriKVClient({
 *   nodes: ['localhost:50051', 'localhost:50052', 'localhost:50053'],
 * });
 *
 * await client.connect();
 *
 * await client.put('user:123', 'Alice');
 * const result = await client.get('user:123');
 * console.log(bytesToString(result.value));
 *
 * await client.close();
 * ```
 *
 * @packageDocumentation
 */

// Main client
export { NoriKVClient, createClient } from '@norikv/client/client';

// Ephemeral (in-memory) server for testing
export {
  EphemeralNoriKV,
  createEphemeral,
  EphemeralServerError,
  type EphemeralOptions,
} from '@norikv/client/ephemeral';

// Types
export type {
  Key,
  Value,
  Version,
  PutOptions,
  GetOptions,
  DeleteOptions,
  GetResult,
  ClusterView,
  ClusterNode,
  ShardInfo,
  ShardReplica,
  ClientConfig,
  RetryConfig,
  NodeConnection,
  RouteInfo,
  ConsistencyLevel,
} from '@norikv/client/types';

// Errors
export {
  NoriKVError,
  NotLeaderError,
  AlreadyExistsError,
  VersionMismatchError,
  UnavailableError,
  DeadlineExceededError,
  InvalidArgumentError,
  ConnectionError,
  NoNodesAvailableError,
  RetryExhaustedError,
} from '@norikv/client/errors';

// Utilities
export {
  xxhash64,
  jumpConsistentHash,
  getShardForKey,
  keyToBytes,
  valueToBytes,
  bytesToString,
  initializeHasher,
} from '@norikv/client/hash';

// Retry policy builder (advanced usage)
export { retryPolicy, type RetryPolicy } from '@norikv/client/retry';

// Version constant
export const VERSION = '0.1.0';
