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
export { NoriKVClient, createClient } from './client.js';

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
} from './types.js';

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
} from './errors.js';

// Utilities
export {
  xxhash64,
  jumpConsistentHash,
  getShardForKey,
  keyToBytes,
  valueToBytes,
  bytesToString,
  initializeHasher,
} from './hash.js';

// Retry policy builder (advanced usage)
export { retryPolicy, type RetryPolicy } from './retry.js';

// Version constant
export const VERSION = '0.1.0';
