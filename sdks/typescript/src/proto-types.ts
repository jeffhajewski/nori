/**
 * Type conversions between SDK types and generated protobuf types.
 *
 * This module bridges the SDK's public API types with the generated gRPC types.
 */

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
} from '@norikv/client/proto/norikv';

import type { Version, ClusterView } from '@norikv/client/types';

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
};

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
