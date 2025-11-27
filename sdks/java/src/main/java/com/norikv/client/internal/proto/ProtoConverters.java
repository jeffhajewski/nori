package com.norikv.client.internal.proto;

import com.google.protobuf.ByteString;
import com.norikv.client.types.*;
import norikv.v1.Norikv;

import java.util.ArrayList;
import java.util.List;
import java.util.stream.Collectors;

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

    // =========================================================================
    // Vector Operations
    // =========================================================================

    /**
     * Converts DistanceFunction to proto enum.
     *
     * @param distance the client distance function
     * @return the proto distance function
     */
    public static Norikv.DistanceFunction toProto(DistanceFunction distance) {
        if (distance == null) {
            return Norikv.DistanceFunction.DISTANCE_FUNCTION_UNSPECIFIED;
        }
        switch (distance) {
            case EUCLIDEAN:
                return Norikv.DistanceFunction.DISTANCE_FUNCTION_EUCLIDEAN;
            case COSINE:
                return Norikv.DistanceFunction.DISTANCE_FUNCTION_COSINE;
            case INNER_PRODUCT:
                return Norikv.DistanceFunction.DISTANCE_FUNCTION_INNER_PRODUCT;
            default:
                return Norikv.DistanceFunction.DISTANCE_FUNCTION_UNSPECIFIED;
        }
    }

    /**
     * Converts VectorIndexType to proto enum.
     *
     * @param indexType the client index type
     * @return the proto index type
     */
    public static Norikv.VectorIndexType toProto(VectorIndexType indexType) {
        if (indexType == null) {
            return Norikv.VectorIndexType.VECTOR_INDEX_TYPE_UNSPECIFIED;
        }
        switch (indexType) {
            case BRUTE_FORCE:
                return Norikv.VectorIndexType.VECTOR_INDEX_TYPE_BRUTE_FORCE;
            case HNSW:
                return Norikv.VectorIndexType.VECTOR_INDEX_TYPE_HNSW;
            default:
                return Norikv.VectorIndexType.VECTOR_INDEX_TYPE_UNSPECIFIED;
        }
    }

    /**
     * Builds a CreateVectorIndexRequest from client parameters.
     *
     * @param namespace the index namespace
     * @param dimensions the vector dimensions
     * @param distance the distance function
     * @param indexType the index type
     * @param options the options (may be null)
     * @return the proto request
     */
    public static Norikv.CreateVectorIndexRequest buildCreateVectorIndexRequest(
            String namespace,
            int dimensions,
            DistanceFunction distance,
            VectorIndexType indexType,
            CreateVectorIndexOptions options) {
        Norikv.CreateVectorIndexRequest.Builder builder = Norikv.CreateVectorIndexRequest.newBuilder()
                .setNamespace(namespace)
                .setDimensions(dimensions)
                .setDistance(toProto(distance))
                .setIndexType(toProto(indexType));

        if (options != null && options.getIdempotencyKey() != null) {
            builder.setIdempotencyKey(options.getIdempotencyKey());
        }

        return builder.build();
    }

    /**
     * Builds a DropVectorIndexRequest from client parameters.
     *
     * @param namespace the index namespace
     * @param options the options (may be null)
     * @return the proto request
     */
    public static Norikv.DropVectorIndexRequest buildDropVectorIndexRequest(
            String namespace,
            DropVectorIndexOptions options) {
        Norikv.DropVectorIndexRequest.Builder builder = Norikv.DropVectorIndexRequest.newBuilder()
                .setNamespace(namespace);

        if (options != null && options.getIdempotencyKey() != null) {
            builder.setIdempotencyKey(options.getIdempotencyKey());
        }

        return builder.build();
    }

    /**
     * Builds a VectorInsertRequest from client parameters.
     *
     * @param namespace the index namespace
     * @param id the vector ID
     * @param vector the vector data
     * @param options the options (may be null)
     * @return the proto request
     */
    public static Norikv.VectorInsertRequest buildVectorInsertRequest(
            String namespace,
            String id,
            List<Float> vector,
            VectorInsertOptions options) {
        Norikv.VectorInsertRequest.Builder builder = Norikv.VectorInsertRequest.newBuilder()
                .setNamespace(namespace)
                .setId(id)
                .addAllVector(vector);

        if (options != null && options.getIdempotencyKey() != null) {
            builder.setIdempotencyKey(options.getIdempotencyKey());
        }

        return builder.build();
    }

    /**
     * Builds a VectorDeleteRequest from client parameters.
     *
     * @param namespace the index namespace
     * @param id the vector ID
     * @param options the options (may be null)
     * @return the proto request
     */
    public static Norikv.VectorDeleteRequest buildVectorDeleteRequest(
            String namespace,
            String id,
            VectorDeleteOptions options) {
        Norikv.VectorDeleteRequest.Builder builder = Norikv.VectorDeleteRequest.newBuilder()
                .setNamespace(namespace)
                .setId(id);

        if (options != null && options.getIdempotencyKey() != null) {
            builder.setIdempotencyKey(options.getIdempotencyKey());
        }

        return builder.build();
    }

    /**
     * Builds a VectorSearchRequest from client parameters.
     *
     * @param namespace the index namespace
     * @param query the query vector
     * @param k number of nearest neighbors
     * @param options the options (may be null)
     * @return the proto request
     */
    public static Norikv.VectorSearchRequest buildVectorSearchRequest(
            String namespace,
            List<Float> query,
            int k,
            VectorSearchOptions options) {
        Norikv.VectorSearchRequest.Builder builder = Norikv.VectorSearchRequest.newBuilder()
                .setNamespace(namespace)
                .addAllQuery(query)
                .setK(k);

        if (options != null) {
            builder.setIncludeVectors(options.isIncludeVectors());
        }

        return builder.build();
    }

    /**
     * Builds a VectorGetRequest from client parameters.
     *
     * @param namespace the index namespace
     * @param id the vector ID
     * @return the proto request
     */
    public static Norikv.VectorGetRequest buildVectorGetRequest(String namespace, String id) {
        return Norikv.VectorGetRequest.newBuilder()
                .setNamespace(namespace)
                .setId(id)
                .build();
    }

    /**
     * Converts a VectorSearchResponse to client VectorSearchResult.
     *
     * @param response the proto response
     * @return the client result
     */
    public static VectorSearchResult fromProto(Norikv.VectorSearchResponse response) {
        List<VectorMatch> matches = response.getMatchesList().stream()
                .map(ProtoConverters::fromProto)
                .collect(Collectors.toList());

        return new VectorSearchResult(matches, response.getSearchTimeUs());
    }

    /**
     * Converts a proto VectorMatch to client VectorMatch.
     *
     * @param match the proto match
     * @return the client match
     */
    public static VectorMatch fromProto(Norikv.VectorMatch match) {
        List<Float> vector = match.getVectorCount() > 0
                ? new ArrayList<>(match.getVectorList())
                : null;

        return new VectorMatch(
                match.getId(),
                match.getDistance(),
                vector
        );
    }
}
