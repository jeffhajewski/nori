/**
 * Type conversions between SDK types and generated protobuf types.
 *
 * This module bridges the SDK's public API types with the generated gRPC types.
 */

import {
  DistanceFunction as ProtoDistanceFunction,
  VectorIndexType as ProtoVectorIndexType,
} from '@norikv/client/proto/norikv';
import type {
  Version as ProtoVersion,
  PutRequest as ProtoPutRequest,
  PutResponse as ProtoPutResponse,
  GetRequest as ProtoGetRequest,
  GetResponse as ProtoGetResponse,
  DeleteRequest as ProtoDeleteRequest,
  DeleteResponse as ProtoDeleteResponse,
  ClusterView as ProtoClusterView,
  ClusterNode as ProtoClusterNode,
  ShardInfo as ProtoShardInfo,
  ShardReplica as ProtoShardReplica,
  CreateVectorIndexRequest as ProtoCreateVectorIndexRequest,
  CreateVectorIndexResponse as ProtoCreateVectorIndexResponse,
  DropVectorIndexRequest as ProtoDropVectorIndexRequest,
  DropVectorIndexResponse as ProtoDropVectorIndexResponse,
  VectorInsertRequest as ProtoVectorInsertRequest,
  VectorInsertResponse as ProtoVectorInsertResponse,
  VectorDeleteRequest as ProtoVectorDeleteRequest,
  VectorDeleteResponse as ProtoVectorDeleteResponse,
  VectorSearchRequest as ProtoVectorSearchRequest,
  VectorSearchResponse as ProtoVectorSearchResponse,
  VectorGetRequest as ProtoVectorGetRequest,
  VectorGetResponse as ProtoVectorGetResponse,
} from '@norikv/client/proto/norikv';

import type {
  Version,
  ClusterView,
  DistanceFunction,
  VectorIndexType,
} from '@norikv/client/types';

// Re-export proto types for convenience
export type {
  ProtoVersion,
  ProtoPutRequest,
  ProtoPutResponse,
  ProtoGetRequest,
  ProtoGetResponse,
  ProtoDeleteRequest,
  ProtoDeleteResponse,
  ProtoClusterView,
  ProtoCreateVectorIndexRequest,
  ProtoCreateVectorIndexResponse,
  ProtoDropVectorIndexRequest,
  ProtoDropVectorIndexResponse,
  ProtoVectorInsertRequest,
  ProtoVectorInsertResponse,
  ProtoVectorDeleteRequest,
  ProtoVectorDeleteResponse,
  ProtoVectorSearchRequest,
  ProtoVectorSearchResponse,
  ProtoVectorGetRequest,
  ProtoVectorGetResponse,
};

// Re-export proto enums
export { ProtoDistanceFunction, ProtoVectorIndexType };

/**
 * Convert SDK Version to proto Version.
 */
export function toProtoVersion(version?: Version): ProtoVersion | undefined {
  if (!version) return undefined;
  return {
    term: version.term,
    index: version.index,
  };
}

/**
 * Convert proto Version to SDK Version.
 */
export function fromProtoVersion(version?: ProtoVersion): Version | null {
  if (!version) return null;
  return {
    term: version.term,
    index: version.index,
  };
}

/**
 * Convert proto ClusterView to SDK ClusterView.
 */
export function fromProtoClusterView(view: ProtoClusterView): ClusterView {
  return {
    epoch: BigInt(view.epoch || 0),
    nodes: view.nodes.map(fromProtoClusterNode),
    shards: view.shards.map(fromProtoShardInfo),
  };
}

/**
 * Convert SDK ClusterView to proto ClusterView.
 */
export function toProtoClusterView(view: ClusterView): ProtoClusterView {
  return {
    epoch: Number(view.epoch),
    nodes: view.nodes.map((node) => ({
      id: node.id,
      addr: node.addr,
      role: node.role,
    })),
    shards: view.shards.map((shard) => ({
      id: shard.id,
      replicas: shard.replicas.map((replica) => ({
        nodeId: replica.nodeId,
        leader: replica.leader,
      })),
    })),
  };
}

/**
 * Convert proto ClusterNode to SDK ClusterNode.
 */
function fromProtoClusterNode(node: ProtoClusterNode) {
  return {
    id: node.id,
    addr: node.addr,
    role: node.role as 'leader' | 'follower' | 'candidate' | 'unknown',
  };
}

/**
 * Convert proto ShardInfo to SDK ShardInfo.
 */
function fromProtoShardInfo(shard: ProtoShardInfo) {
  return {
    id: shard.id,
    replicas: shard.replicas.map(fromProtoShardReplica),
  };
}

/**
 * Convert proto ShardReplica to SDK ShardReplica.
 */
function fromProtoShardReplica(replica: ProtoShardReplica) {
  return {
    nodeId: replica.nodeId,
    leader: replica.leader,
  };
}

/**
 * Convert SDK DistanceFunction to proto DistanceFunction.
 */
export function toProtoDistanceFunction(distance: DistanceFunction): ProtoDistanceFunction {
  switch (distance) {
    case 'euclidean':
      return ProtoDistanceFunction.DISTANCE_FUNCTION_EUCLIDEAN;
    case 'cosine':
      return ProtoDistanceFunction.DISTANCE_FUNCTION_COSINE;
    case 'inner_product':
      return ProtoDistanceFunction.DISTANCE_FUNCTION_INNER_PRODUCT;
    default:
      return ProtoDistanceFunction.DISTANCE_FUNCTION_UNSPECIFIED;
  }
}

/**
 * Convert SDK VectorIndexType to proto VectorIndexType.
 */
export function toProtoVectorIndexType(indexType: VectorIndexType): ProtoVectorIndexType {
  switch (indexType) {
    case 'brute_force':
      return ProtoVectorIndexType.VECTOR_INDEX_TYPE_BRUTE_FORCE;
    case 'hnsw':
      return ProtoVectorIndexType.VECTOR_INDEX_TYPE_HNSW;
    default:
      return ProtoVectorIndexType.VECTOR_INDEX_TYPE_UNSPECIFIED;
  }
}
