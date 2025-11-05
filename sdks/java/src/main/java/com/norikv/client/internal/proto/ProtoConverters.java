package com.norikv.client.internal.proto;

import com.google.protobuf.ByteString;
import com.norikv.client.types.*;
import norikv.v1.Norikv;

import java.util.ArrayList;
import java.util.List;

/**
 * Utility class for converting between client types and protobuf types.
 */
public final class ProtoConverters {
    private ProtoConverters() {
        // Utility class
    }

    /**
     * Converts a client Version to proto Version.
     *
     * @param version the client version (may be null)
     * @return the proto version
     */
    public static Norikv.Version toProto(com.norikv.client.types.Version version) {
        if (version == null) {
            return Norikv.Version.getDefaultInstance();
        }
        return Norikv.Version.newBuilder()
                .setTerm(version.getTerm())
                .setIndex(version.getIndex())
                .build();
    }

    /**
     * Converts a proto Version to client Version.
     *
     * @param version the proto version
     * @return the client version
     */
    public static com.norikv.client.types.Version fromProto(Norikv.Version version) {
        return new com.norikv.client.types.Version(
                version.getTerm(),
                version.getIndex()
        );
    }

    /**
     * Converts ConsistencyLevel to proto string.
     *
     * @param level the client consistency level
     * @return the proto consistency string
     */
    public static String toProtoString(ConsistencyLevel level) {
        if (level == null) {
            return "lease";
        }
        // Use the enum's built-in getValue() method
        return level.getValue();
    }

    /**
     * Builds a PutRequest from client parameters.
     *
     * @param key the key
     * @param value the value
     * @param options the put options
     * @return the proto PutRequest
     */
    public static Norikv.PutRequest buildPutRequest(byte[] key, byte[] value, PutOptions options) {
        Norikv.PutRequest.Builder builder = Norikv.PutRequest.newBuilder()
                .setKey(ByteString.copyFrom(key))
                .setValue(ByteString.copyFrom(value));

        if (options.getTtlMs() != null && options.getTtlMs() > 0) {
            builder.setTtlMs(options.getTtlMs());
        }

        // Note: ifNotExists is not in the proto - it would need to be handled
        // by checking if_match with a zero version or using metadata

        if (options.getIfMatchVersion() != null) {
            builder.setIfMatch(toProto(options.getIfMatchVersion()));
        }

        if (options.getIdempotencyKey() != null && !options.getIdempotencyKey().isEmpty()) {
            builder.setIdempotencyKey(options.getIdempotencyKey());
        }

        return builder.build();
    }

    /**
     * Builds a GetRequest from client parameters.
     *
     * @param key the key
     * @param options the get options
     * @return the proto GetRequest
     */
    public static Norikv.GetRequest buildGetRequest(byte[] key, GetOptions options) {
        Norikv.GetRequest.Builder builder = Norikv.GetRequest.newBuilder()
                .setKey(ByteString.copyFrom(key));

        if (options.getConsistency() != null) {
            builder.setConsistency(toProtoString(options.getConsistency()));
        }

        return builder.build();
    }

    /**
     * Builds a DeleteRequest from client parameters.
     *
     * @param key the key
     * @param options the delete options
     * @return the proto DeleteRequest
     */
    public static Norikv.DeleteRequest buildDeleteRequest(byte[] key, DeleteOptions options) {
        Norikv.DeleteRequest.Builder builder = Norikv.DeleteRequest.newBuilder()
                .setKey(ByteString.copyFrom(key));

        if (options.getIfMatchVersion() != null) {
            builder.setIfMatch(toProto(options.getIfMatchVersion()));
        }

        if (options.getIdempotencyKey() != null && !options.getIdempotencyKey().isEmpty()) {
            builder.setIdempotencyKey(options.getIdempotencyKey());
        }

        return builder.build();
    }

    /**
     * Converts a GetResponse to client GetResult.
     *
     * @param response the proto GetResponse
     * @return the client GetResult
     */
    public static GetResult fromProto(Norikv.GetResponse response) {
        return new GetResult(
                response.getValue().toByteArray(),
                fromProto(response.getVersion())
        );
    }

    /**
     * Converts a proto ClusterView to client ClusterView.
     *
     * @param view the proto ClusterView
     * @return the client ClusterView
     */
    public static ClusterView fromProto(Norikv.ClusterView view) {
        List<ClusterNode> nodes = new ArrayList<>();
        for (Norikv.ClusterNode protoNode : view.getNodesList()) {
            nodes.add(new ClusterNode(protoNode.getId(), protoNode.getAddr()));
        }

        List<ShardInfo> shards = new ArrayList<>();
        for (Norikv.ShardInfo protoShard : view.getShardsList()) {
            List<ShardReplica> replicas = new ArrayList<>();
            for (Norikv.ShardReplica protoReplica : protoShard.getReplicasList()) {
                replicas.add(new ShardReplica(
                        protoReplica.getNodeId(),
                        protoReplica.getLeader()
                ));
            }
            shards.add(new ShardInfo(protoShard.getId(), replicas));
        }

        return new ClusterView(view.getEpoch(), nodes, shards);
    }
}
