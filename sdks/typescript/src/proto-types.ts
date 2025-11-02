/**
 * Manual TypeScript definitions for gRPC protobuf messages.
 *
 * These will be replaced by generated code once we set up the buf tooling.
 * For now, this allows us to implement the client without blocking on proto generation.
 */

/**
 * Version message from protobuf.
 */
export interface ProtoVersion {
  term: string; // uint64 as string
  index: string; // uint64 as string
}

/**
 * PutRequest message.
 */
export interface ProtoPutRequest {
  key: Uint8Array;
  value: Uint8Array;
  ttl_ms?: string; // uint64 as string
  idempotency_key?: string;
  if_match?: ProtoVersion;
}

/**
 * PutResponse message.
 */
export interface ProtoPutResponse {
  version?: ProtoVersion;
  meta?: { [key: string]: string };
}

/**
 * GetRequest message.
 */
export interface ProtoGetRequest {
  key: Uint8Array;
  consistency?: string;
}

/**
 * GetResponse message.
 */
export interface ProtoGetResponse {
  value?: Uint8Array;
  version?: ProtoVersion;
  meta?: { [key: string]: string };
}

/**
 * DeleteRequest message.
 */
export interface ProtoDeleteRequest {
  key: Uint8Array;
  idempotency_key?: string;
  if_match?: ProtoVersion;
}

/**
 * DeleteResponse message.
 */
export interface ProtoDeleteResponse {
  tombstoned: boolean;
}

/**
 * ClusterNode message.
 */
export interface ProtoClusterNode {
  id: string;
  addr: string;
  role: string;
}

/**
 * ShardReplica message.
 */
export interface ProtoShardReplica {
  node_id: string;
  leader: boolean;
}

/**
 * ShardInfo message.
 */
export interface ProtoShardInfo {
  id: number;
  replicas: ProtoShardReplica[];
}

/**
 * ClusterView message.
 */
export interface ProtoClusterView {
  epoch: string; // uint64 as string
  nodes: ProtoClusterNode[];
  shards: ProtoShardInfo[];
}

/**
 * Convert our Version type to protobuf Version.
 */
export function toProtoVersion(v: { term: bigint; index: bigint } | undefined): ProtoVersion | undefined {
  if (!v) return undefined;
  return {
    term: v.term.toString(),
    index: v.index.toString(),
  };
}

/**
 * Convert protobuf Version to our Version type.
 */
export function fromProtoVersion(v: ProtoVersion | undefined): { term: bigint; index: bigint } | null {
  if (!v) return null;
  return {
    term: BigInt(v.term),
    index: BigInt(v.index),
  };
}

/**
 * Convert protobuf ClusterView to our ClusterView type.
 */
export function fromProtoClusterView(proto: ProtoClusterView): import('./types.js').ClusterView {
  return {
    epoch: BigInt(proto.epoch),
    nodes: proto.nodes.map((n) => ({
      id: n.id,
      addr: n.addr,
      role: n.role,
    })),
    shards: proto.shards.map((s) => ({
      id: s.id,
      replicas: s.replicas.map((r) => ({
        nodeId: r.node_id,
        leader: r.leader,
      })),
    })),
  };
}
