/**
 * gRPC service clients for NoriKV.
 *
 * This module provides typed wrappers around the generated gRPC clients
 * with proper error handling and Promise-based interfaces.
 */

import * as grpc from '@grpc/grpc-js';
import {
  KvClient,
  MetaClient,
  VectorClient,
  type PutRequest,
  type PutResponse,
  type GetRequest,
  type GetResponse,
  type DeleteRequest,
  type DeleteResponse,
  type ClusterView,
  type CreateVectorIndexRequest,
  type CreateVectorIndexResponse,
  type DropVectorIndexRequest,
  type DropVectorIndexResponse,
  type VectorInsertRequest,
  type VectorInsertResponse,
  type VectorDeleteRequest,
  type VectorDeleteResponse,
  type VectorSearchRequest,
  type VectorSearchResponse,
  type VectorGetRequest,
  type VectorGetResponse,
} from '@norikv/client/proto/norikv';
import { fromGrpcError } from '@norikv/client/errors';

/**
 * gRPC channel options optimized for NoriKV.
 */
const CHANNEL_OPTIONS: grpc.ChannelOptions = {
  'grpc.keepalive_time_ms': 10000,
  'grpc.keepalive_timeout_ms': 5000,
  'grpc.keepalive_permit_without_calls': 1,
  'grpc.http2.max_pings_without_data': 0,
  'grpc.http2.min_time_between_pings_ms': 10000,
  'grpc.http2.max_ping_strikes': 2,
};

/**
 * Create a KV service client for a given address.
 */
export function createKvClient(address: string, credentials?: grpc.ChannelCredentials): KvClient {
  const creds = credentials || grpc.credentials.createInsecure();
  return new KvClient(address, creds, CHANNEL_OPTIONS);
}

/**
 * Create a Meta service client for a given address.
 */
export function createMetaClient(address: string, credentials?: grpc.ChannelCredentials): MetaClient {
  const creds = credentials || grpc.credentials.createInsecure();
  return new MetaClient(address, creds, CHANNEL_OPTIONS);
}

/**
 * Make a unary KV Put call with proper error handling.
 */
export function kvPut(
  client: KvClient,
  request: PutRequest,
  timeout?: number
): Promise<PutResponse> {
  return new Promise((resolve, reject) => {
    const deadline = timeout ? Date.now() + timeout : undefined;
    const metadata = new grpc.Metadata();
    const options: Partial<grpc.CallOptions> = {};
    if (deadline) {
      options.deadline = deadline;
    }

    client.put(request, metadata, options, (error, response) => {
      if (error) {
        reject(fromGrpcError(error));
      } else if (!response) {
        reject(new Error('Empty response from server'));
      } else {
        resolve(response);
      }
    });
  });
}

/**
 * Make a unary KV Get call with proper error handling.
 */
export function kvGet(
  client: KvClient,
  request: GetRequest,
  timeout?: number
): Promise<GetResponse> {
  return new Promise((resolve, reject) => {
    const deadline = timeout ? Date.now() + timeout : undefined;
    const metadata = new grpc.Metadata();
    const options: Partial<grpc.CallOptions> = {};
    if (deadline) {
      options.deadline = deadline;
    }

    client.get(request, metadata, options, (error, response) => {
      if (error) {
        reject(fromGrpcError(error));
      } else if (!response) {
        reject(new Error('Empty response from server'));
      } else {
        resolve(response);
      }
    });
  });
}

/**
 * Make a unary KV Delete call with proper error handling.
 */
export function kvDelete(
  client: KvClient,
  request: DeleteRequest,
  timeout?: number
): Promise<DeleteResponse> {
  return new Promise((resolve, reject) => {
    const deadline = timeout ? Date.now() + timeout : undefined;
    const metadata = new grpc.Metadata();
    const options: Partial<grpc.CallOptions> = {};
    if (deadline) {
      options.deadline = deadline;
    }

    client.delete(request, metadata, options, (error, response) => {
      if (error) {
        reject(fromGrpcError(error));
      } else if (!response) {
        reject(new Error('Empty response from server'));
      } else {
        resolve(response);
      }
    });
  });
}

/**
 * Start watching cluster changes via Meta.WatchCluster.
 *
 * @returns A readable stream of ClusterView updates
 */
export function metaWatchCluster(
  client: MetaClient,
  initialView?: ClusterView
): grpc.ClientReadableStream<ClusterView> {
  const metadata = new grpc.Metadata();
  const request = initialView || {
    epoch: 0,
    nodes: [],
    shards: [],
  };

  return client.watchCluster(request, metadata);
}

/**
 * Close a gRPC client and clean up resources.
 */
export function closeClient(client: KvClient | MetaClient | VectorClient): void {
  client.close();
}

// ============================================================================
// Vector Service Functions
// ============================================================================

/**
 * Create a Vector service client for a given address.
 */
export function createVectorClient(address: string, credentials?: grpc.ChannelCredentials): VectorClient {
  const creds = credentials || grpc.credentials.createInsecure();
  return new VectorClient(address, creds, CHANNEL_OPTIONS);
}

/**
 * Create a vector index.
 */
export function vectorCreateIndex(
  client: VectorClient,
  request: CreateVectorIndexRequest,
  timeout?: number
): Promise<CreateVectorIndexResponse> {
  return new Promise((resolve, reject) => {
    const deadline = timeout ? Date.now() + timeout : undefined;
    const metadata = new grpc.Metadata();
    const options: Partial<grpc.CallOptions> = {};
    if (deadline) {
      options.deadline = deadline;
    }

    client.createIndex(request, metadata, options, (error, response) => {
      if (error) {
        reject(fromGrpcError(error));
      } else if (!response) {
        reject(new Error('Empty response from server'));
      } else {
        resolve(response);
      }
    });
  });
}

/**
 * Drop a vector index.
 */
export function vectorDropIndex(
  client: VectorClient,
  request: DropVectorIndexRequest,
  timeout?: number
): Promise<DropVectorIndexResponse> {
  return new Promise((resolve, reject) => {
    const deadline = timeout ? Date.now() + timeout : undefined;
    const metadata = new grpc.Metadata();
    const options: Partial<grpc.CallOptions> = {};
    if (deadline) {
      options.deadline = deadline;
    }

    client.dropIndex(request, metadata, options, (error, response) => {
      if (error) {
        reject(fromGrpcError(error));
      } else if (!response) {
        reject(new Error('Empty response from server'));
      } else {
        resolve(response);
      }
    });
  });
}

/**
 * Insert a vector.
 */
export function vectorInsert(
  client: VectorClient,
  request: VectorInsertRequest,
  timeout?: number
): Promise<VectorInsertResponse> {
  return new Promise((resolve, reject) => {
    const deadline = timeout ? Date.now() + timeout : undefined;
    const metadata = new grpc.Metadata();
    const options: Partial<grpc.CallOptions> = {};
    if (deadline) {
      options.deadline = deadline;
    }

    client.insert(request, metadata, options, (error, response) => {
      if (error) {
        reject(fromGrpcError(error));
      } else if (!response) {
        reject(new Error('Empty response from server'));
      } else {
        resolve(response);
      }
    });
  });
}

/**
 * Delete a vector.
 */
export function vectorDelete(
  client: VectorClient,
  request: VectorDeleteRequest,
  timeout?: number
): Promise<VectorDeleteResponse> {
  return new Promise((resolve, reject) => {
    const deadline = timeout ? Date.now() + timeout : undefined;
    const metadata = new grpc.Metadata();
    const options: Partial<grpc.CallOptions> = {};
    if (deadline) {
      options.deadline = deadline;
    }

    client.delete(request, metadata, options, (error, response) => {
      if (error) {
        reject(fromGrpcError(error));
      } else if (!response) {
        reject(new Error('Empty response from server'));
      } else {
        resolve(response);
      }
    });
  });
}

/**
 * Search for similar vectors.
 */
export function vectorSearch(
  client: VectorClient,
  request: VectorSearchRequest,
  timeout?: number
): Promise<VectorSearchResponse> {
  return new Promise((resolve, reject) => {
    const deadline = timeout ? Date.now() + timeout : undefined;
    const metadata = new grpc.Metadata();
    const options: Partial<grpc.CallOptions> = {};
    if (deadline) {
      options.deadline = deadline;
    }

    client.search(request, metadata, options, (error, response) => {
      if (error) {
        reject(fromGrpcError(error));
      } else if (!response) {
        reject(new Error('Empty response from server'));
      } else {
        resolve(response);
      }
    });
  });
}

/**
 * Get a vector by ID.
 */
export function vectorGet(
  client: VectorClient,
  request: VectorGetRequest,
  timeout?: number
): Promise<VectorGetResponse> {
  return new Promise((resolve, reject) => {
    const deadline = timeout ? Date.now() + timeout : undefined;
    const metadata = new grpc.Metadata();
    const options: Partial<grpc.CallOptions> = {};
    if (deadline) {
      options.deadline = deadline;
    }

    client.get(request, metadata, options, (error, response) => {
      if (error) {
        reject(fromGrpcError(error));
      } else if (!response) {
        reject(new Error('Empty response from server'));
      } else {
        resolve(response);
      }
    });
  });
}

// Re-export types for convenience
export type {
  CreateVectorIndexRequest,
  CreateVectorIndexResponse,
  DropVectorIndexRequest,
  DropVectorIndexResponse,
  VectorInsertRequest,
  VectorInsertResponse,
  VectorDeleteRequest,
  VectorDeleteResponse,
  VectorSearchRequest,
  VectorSearchResponse,
  VectorGetRequest,
  VectorGetResponse,
  VectorClient,
};
