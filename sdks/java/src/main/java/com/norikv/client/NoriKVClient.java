package com.norikv.client;

import com.norikv.client.hash.HashFunctions;
import com.norikv.client.internal.conn.ConnectionPool;
import com.norikv.client.internal.proto.ProtoConverters;
import com.norikv.client.internal.retry.RetryPolicy;
import com.norikv.client.internal.router.Router;
import com.norikv.client.internal.topology.TopologyManager;
import com.norikv.client.types.*;
import io.grpc.ManagedChannel;
import io.grpc.Status;
import io.grpc.StatusRuntimeException;
import norikv.v1.KvGrpc;
import norikv.v1.Norikv;
import norikv.v1.VectorGrpc;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * Main client for interacting with a NoriKV cluster.
 *
 * <p>This client provides:
 * <ul>
 * <li>Smart routing to shard leaders</li>
 * <li>Automatic retries with exponential backoff</li>
 * <li>Connection pooling and management</li>
 * <li>Cluster topology tracking</li>
 * <li>Thread-safe concurrent operations</li>
 * </ul>
 *
 * <p>Example usage:
 * <pre>{@code
 * ClientConfig config = ClientConfig.builder()
 *     .nodes("localhost:9001", "localhost:9002", "localhost:9003")
 *     .totalShards(1024)
 *     .timeoutMs(5000)
 *     .build();
 *
 * try (NoriKVClient client = new NoriKVClient(config)) {
 *     // Put a value
 *     byte[] key = "user:123".getBytes();
 *     byte[] value = "{\"name\":\"Alice\"}".getBytes();
 *     Version version = client.put(key, value, null);
 *
 *     // Get the value
 *     GetResult result = client.get(key, null);
 *     System.out.println(new String(result.getValue()));
 *
 *     // Conditional update (CAS)
 *     byte[] newValue = "{\"name\":\"Alice\",\"age\":30}".getBytes();
 *     PutOptions options = PutOptions.builder()
 *         .ifMatchVersion(version)
 *         .build();
 *     client.put(key, newValue, options);
 *
 *     // Delete
 *     client.delete(key, null);
 * }
 * }</pre>
 *
 * <p>Thread-safe for concurrent use. A single client instance can be shared
 * across multiple threads.
 */
public final class NoriKVClient implements AutoCloseable {
    private final ClientConfig config;
    private final ConnectionPool connectionPool;
    private final TopologyManager topologyManager;
    private final Router router;
    private final RetryPolicy retryPolicy;
    private final AtomicBoolean closed;

    /**
     * Creates a new NoriKVClient with the given configuration.
     *
     * @param config the client configuration
     */
    public NoriKVClient(ClientConfig config) {
        if (config == null) {
            throw new IllegalArgumentException("config cannot be null");
        }

        this.config = config;
        this.connectionPool = new ConnectionPool(config.getMaxConnectionsPerNode());
        this.topologyManager = new TopologyManager();
        this.router = new Router(
                config.getNodes(),
                config.getTotalShards(),
                connectionPool
        );
        this.retryPolicy = new RetryPolicy(config.getRetry());
        this.closed = new AtomicBoolean(false);

        // Initialize topology with seed nodes
        initializeSeedTopology();
    }

    /**
     * Initializes topology with seed nodes as a starting point.
     */
    private void initializeSeedTopology() {
        // Create initial cluster view with seed nodes
        // This will be updated when we fetch the actual cluster view
        // For now, this ensures the router has nodes to work with
    }

    /**
     * Puts a key-value pair into the store.
     *
     * <p>This operation will:
     * <ul>
     * <li>Route to the correct shard leader</li>
     * <li>Retry on transient failures</li>
     * <li>Update leader cache on NOT_LEADER errors</li>
     * </ul>
     *
     * @param key the key to store
     * @param value the value to store
     * @param options optional put options (null for defaults)
     * @return the version of the stored value
     * @throws NoriKVException if the operation fails
     */
    public Version put(byte[] key, byte[] value, PutOptions options) throws NoriKVException {
        ensureNotClosed();
        validateKey(key);
        validateValue(value);

        final PutOptions opts = options != null ? options : PutOptions.builder().build();

        return retryPolicy.execute(() -> {
            Router.RouteInfo route = router.getChannelForKey(key);
            ManagedChannel channel = route.getChannel();

            try {
                // TODO: Call gRPC KV.Put when proto stubs are generated
                // For now, return a placeholder version
                Version version = performPut(channel, key, value, opts);

                return version;
            } catch (NotLeaderException e) {
                // Update leader cache and retry
                router.handleNotLeader(e);
                throw e;
            } catch (StatusRuntimeException e) {
                // Convert gRPC status to NoriKVException
                throw convertGrpcException(e);
            }
        });
    }

    /**
     * Gets a value by key.
     *
     * @param key the key to retrieve
     * @param options optional get options (null for defaults)
     * @return the result containing the value and version
     * @throws KeyNotFoundException if the key does not exist
     * @throws NoriKVException if the operation fails
     */
    public GetResult get(byte[] key, GetOptions options) throws NoriKVException {
        ensureNotClosed();
        validateKey(key);

        final GetOptions opts = options != null ? options : GetOptions.builder().build();

        return retryPolicy.execute(() -> {
            Router.RouteInfo route = router.getChannelForKey(key);
            ManagedChannel channel = route.getChannel();

            try {
                // TODO: Call gRPC KV.Get when proto stubs are generated
                GetResult result = performGet(channel, key, opts);

                return result;
            } catch (NotLeaderException e) {
                // Update leader cache and retry
                router.handleNotLeader(e);
                throw e;
            } catch (StatusRuntimeException e) {
                throw convertGrpcException(e);
            }
        });
    }

    /**
     * Deletes a key.
     *
     * @param key the key to delete
     * @param options optional delete options (null for defaults)
     * @return true if the key was deleted, false if it didn't exist
     * @throws NoriKVException if the operation fails
     */
    public boolean delete(byte[] key, DeleteOptions options) throws NoriKVException {
        ensureNotClosed();
        validateKey(key);

        final DeleteOptions opts = options != null ? options : DeleteOptions.builder().build();

        return retryPolicy.execute(() -> {
            Router.RouteInfo route = router.getChannelForKey(key);
            ManagedChannel channel = route.getChannel();

            try {
                // TODO: Call gRPC KV.Delete when proto stubs are generated
                boolean deleted = performDelete(channel, key, opts);

                return deleted;
            } catch (NotLeaderException e) {
                // Update leader cache and retry
                router.handleNotLeader(e);
                throw e;
            } catch (StatusRuntimeException e) {
                throw convertGrpcException(e);
            }
        });
    }

    // =========================================================================
    // Vector Operations
    // =========================================================================

    /**
     * Creates a vector index.
     *
     * @param namespace the index namespace/name
     * @param dimensions the number of dimensions for vectors
     * @param distance the distance function
     * @param indexType the index type
     * @param options optional create options (null for defaults)
     * @return true if created, false if already existed
     * @throws NoriKVException if the operation fails
     */
    public boolean vectorCreateIndex(
            String namespace,
            int dimensions,
            DistanceFunction distance,
            VectorIndexType indexType,
            CreateVectorIndexOptions options) throws NoriKVException {
        ensureNotClosed();
        validateNamespace(namespace);
        if (dimensions <= 0) {
            throw new NoriKVException("INVALID_ARGUMENT", "dimensions must be greater than 0");
        }

        return retryPolicy.execute(() -> {
            ManagedChannel channel = getAnyChannel();

            try {
                return performVectorCreateIndex(channel, namespace, dimensions, distance, indexType, options);
            } catch (StatusRuntimeException e) {
                throw convertGrpcException(e);
            }
        });
    }

    /**
     * Drops a vector index.
     *
     * @param namespace the index namespace/name
     * @param options optional drop options (null for defaults)
     * @return true if dropped, false if didn't exist
     * @throws NoriKVException if the operation fails
     */
    public boolean vectorDropIndex(String namespace, DropVectorIndexOptions options) throws NoriKVException {
        ensureNotClosed();
        validateNamespace(namespace);

        return retryPolicy.execute(() -> {
            ManagedChannel channel = getAnyChannel();

            try {
                return performVectorDropIndex(channel, namespace, options);
            } catch (StatusRuntimeException e) {
                throw convertGrpcException(e);
            }
        });
    }

    /**
     * Inserts a vector into an index.
     *
     * @param namespace the index namespace/name
     * @param id the unique vector ID
     * @param vector the vector data
     * @param options optional insert options (null for defaults)
     * @return the version of the inserted vector
     * @throws NoriKVException if the operation fails
     */
    public Version vectorInsert(
            String namespace,
            String id,
            List<Float> vector,
            VectorInsertOptions options) throws NoriKVException {
        ensureNotClosed();
        validateNamespace(namespace);
        validateVectorId(id);
        validateVector(vector);

        return retryPolicy.execute(() -> {
            ManagedChannel channel = getAnyChannel();

            try {
                return performVectorInsert(channel, namespace, id, vector, options);
            } catch (StatusRuntimeException e) {
                throw convertGrpcException(e);
            }
        });
    }

    /**
     * Deletes a vector from an index.
     *
     * @param namespace the index namespace/name
     * @param id the vector ID to delete
     * @param options optional delete options (null for defaults)
     * @return true if deleted, false if didn't exist
     * @throws NoriKVException if the operation fails
     */
    public boolean vectorDelete(
            String namespace,
            String id,
            VectorDeleteOptions options) throws NoriKVException {
        ensureNotClosed();
        validateNamespace(namespace);
        validateVectorId(id);

        return retryPolicy.execute(() -> {
            ManagedChannel channel = getAnyChannel();

            try {
                return performVectorDelete(channel, namespace, id, options);
            } catch (StatusRuntimeException e) {
                throw convertGrpcException(e);
            }
        });
    }

    /**
     * Searches for nearest neighbors.
     *
     * @param namespace the index namespace/name
     * @param query the query vector
     * @param k the number of nearest neighbors to return
     * @param options optional search options (null for defaults)
     * @return search results with matches and timing
     * @throws NoriKVException if the operation fails
     */
    public VectorSearchResult vectorSearch(
            String namespace,
            List<Float> query,
            int k,
            VectorSearchOptions options) throws NoriKVException {
        ensureNotClosed();
        validateNamespace(namespace);
        validateVector(query);
        if (k <= 0) {
            throw new NoriKVException("INVALID_ARGUMENT", "k must be greater than 0");
        }

        return retryPolicy.execute(() -> {
            ManagedChannel channel = getAnyChannel();

            try {
                return performVectorSearch(channel, namespace, query, k, options);
            } catch (StatusRuntimeException e) {
                throw convertGrpcException(e);
            }
        });
    }

    /**
     * Gets a vector by ID.
     *
     * @param namespace the index namespace/name
     * @param id the vector ID
     * @return the vector data, or null if not found
     * @throws NoriKVException if the operation fails
     */
    public List<Float> vectorGet(String namespace, String id) throws NoriKVException {
        ensureNotClosed();
        validateNamespace(namespace);
        validateVectorId(id);

        return retryPolicy.execute(() -> {
            ManagedChannel channel = getAnyChannel();

            try {
                return performVectorGet(channel, namespace, id);
            } catch (StatusRuntimeException e) {
                throw convertGrpcException(e);
            }
        });
    }

    /**
     * Gets the current cluster view.
     *
     * @return the cluster view, or null if not available
     */
    public ClusterView getClusterView() {
        return topologyManager.getView();
    }

    /**
     * Registers a listener for topology changes.
     *
     * @param listener callback to invoke on topology changes
     * @return runnable to unregister the listener
     */
    public Runnable onTopologyChange(java.util.function.Consumer<TopologyManager.TopologyChangeEvent> listener) {
        return topologyManager.onTopologyChange(listener);
    }

    /**
     * Gets statistics about the client.
     *
     * @return client statistics
     */
    public ClientStats getStats() {
        return new ClientStats(
                router.getStats(),
                connectionPool.getStats(),
                topologyManager.getStats(),
                closed.get()
        );
    }

    /**
     * Checks if the client is closed.
     *
     * @return true if closed
     */
    public boolean isClosed() {
        return closed.get();
    }

    /**
     * Closes the client and releases all resources.
     *
     * <p>This will:
     * <ul>
     * <li>Stop cluster watching</li>
     * <li>Close all connections</li>
     * <li>Clean up internal state</li>
     * </ul>
     *
     * <p>This method is idempotent - calling it multiple times has no additional effect.
     */
    @Override
    public void close() {
        if (closed.compareAndSet(false, true)) {
            // Stop cluster watching
            // TODO: Cancel cluster watch stream when implemented

            // Close connection pool
            connectionPool.close();

            // Clear topology
            topologyManager.clear();
        }
    }

    /**
     * Ensures the client is not closed.
     *
     * @throws ConnectionException if the client is closed
     */
    private void ensureNotClosed() throws ConnectionException {
        if (closed.get()) {
            throw new ConnectionException("Client is closed", null);
        }
    }

    /**
     * Validates a key.
     *
     * @param key the key to validate
     * @throws NoriKVException if the key is invalid
     */
    private void validateKey(byte[] key) throws NoriKVException {
        if (key == null || key.length == 0) {
            throw new NoriKVException("INVALID_ARGUMENT", "Key cannot be null or empty");
        }
        // TODO: Add max key size validation
    }

    /**
     * Validates a value.
     *
     * @param value the value to validate
     * @throws NoriKVException if the value is invalid
     */
    private void validateValue(byte[] value) throws NoriKVException {
        if (value == null) {
            throw new NoriKVException("INVALID_ARGUMENT", "Value cannot be null");
        }
        // TODO: Add max value size validation
    }

    /**
     * Performs the actual Put RPC call.
     *
     * @param channel the gRPC channel
     * @param key the key
     * @param value the value
     * @param options the put options
     * @return the version
     * @throws NoriKVException on error
     */
    private Version performPut(ManagedChannel channel, byte[] key, byte[] value,
                               PutOptions options) throws NoriKVException {
        KvGrpc.KvBlockingStub stub = KvGrpc.newBlockingStub(channel)
                .withDeadlineAfter(config.getTimeoutMs(), TimeUnit.MILLISECONDS);

        Norikv.PutRequest request = ProtoConverters.buildPutRequest(key, value, options);

        Norikv.PutResponse response = stub.put(request);

        return ProtoConverters.fromProto(response.getVersion());
    }

    /**
     * Performs the actual Get RPC call.
     *
     * @param channel the gRPC channel
     * @param key the key
     * @param options the get options
     * @return the result
     * @throws NoriKVException on error
     */
    private GetResult performGet(ManagedChannel channel, byte[] key,
                                 GetOptions options) throws NoriKVException {
        KvGrpc.KvBlockingStub stub = KvGrpc.newBlockingStub(channel)
                .withDeadlineAfter(config.getTimeoutMs(), TimeUnit.MILLISECONDS);

        Norikv.GetRequest request = ProtoConverters.buildGetRequest(key, options);

        Norikv.GetResponse response = stub.get(request);

        return ProtoConverters.fromProto(response);
    }

    /**
     * Performs the actual Delete RPC call.
     *
     * @param channel the gRPC channel
     * @param key the key
     * @param options the delete options
     * @return true if deleted
     * @throws NoriKVException on error
     */
    private boolean performDelete(ManagedChannel channel, byte[] key,
                                  DeleteOptions options) throws NoriKVException {
        KvGrpc.KvBlockingStub stub = KvGrpc.newBlockingStub(channel)
                .withDeadlineAfter(config.getTimeoutMs(), TimeUnit.MILLISECONDS);

        Norikv.DeleteRequest request = ProtoConverters.buildDeleteRequest(key, options);

        Norikv.DeleteResponse response = stub.delete(request);

        return response.getTombstoned();
    }

    /**
     * Converts a gRPC StatusRuntimeException to NoriKVException.
     *
     * @param e the gRPC exception
     * @return the converted exception
     */
    private NoriKVException convertGrpcException(StatusRuntimeException e) {
        String code = e.getStatus().getCode().name();
        String message = e.getStatus().getDescription();

        // Map gRPC status codes to NoriKV exceptions
        switch (e.getStatus().getCode()) {
            case NOT_FOUND:
                return new KeyNotFoundException("Key not found", null);
            case ALREADY_EXISTS:
                return new AlreadyExistsException("Key already exists", null);
            case FAILED_PRECONDITION:
                // Check if it's a version mismatch
                if (message != null && message.contains("ersion")) {
                    return new VersionMismatchException(message, null, null, null, e);
                }
                return new NoriKVException(code, message, e);
            case UNAVAILABLE:
            case DEADLINE_EXCEEDED:
                return new ConnectionException(message, null, e);
            default:
                return new NoriKVException(code, message, e);
        }
    }

    /**
     * Validates a namespace.
     *
     * @param namespace the namespace to validate
     * @throws NoriKVException if the namespace is invalid
     */
    private void validateNamespace(String namespace) throws NoriKVException {
        if (namespace == null || namespace.isEmpty()) {
            throw new NoriKVException("INVALID_ARGUMENT", "Namespace cannot be null or empty");
        }
    }

    /**
     * Validates a vector ID.
     *
     * @param id the ID to validate
     * @throws NoriKVException if the ID is invalid
     */
    private void validateVectorId(String id) throws NoriKVException {
        if (id == null || id.isEmpty()) {
            throw new NoriKVException("INVALID_ARGUMENT", "Vector ID cannot be null or empty");
        }
    }

    /**
     * Validates a vector.
     *
     * @param vector the vector to validate
     * @throws NoriKVException if the vector is invalid
     */
    private void validateVector(List<Float> vector) throws NoriKVException {
        if (vector == null || vector.isEmpty()) {
            throw new NoriKVException("INVALID_ARGUMENT", "Vector cannot be null or empty");
        }
    }

    /**
     * Gets any available channel for non-key-routed operations.
     *
     * @return a gRPC channel
     * @throws NoriKVException if no channels are available
     */
    private ManagedChannel getAnyChannel() throws NoriKVException {
        List<String> nodes = config.getNodes();
        if (nodes.isEmpty()) {
            throw new ConnectionException("No nodes available", null);
        }

        // Try to get a channel from any node
        for (String node : nodes) {
            try {
                return connectionPool.getChannel(node);
            } catch (Exception e) {
                // Try next node
            }
        }

        throw new ConnectionException("Unable to connect to any node", null);
    }

    // =========================================================================
    // Vector RPC Implementations
    // =========================================================================

    private boolean performVectorCreateIndex(
            ManagedChannel channel,
            String namespace,
            int dimensions,
            DistanceFunction distance,
            VectorIndexType indexType,
            CreateVectorIndexOptions options) throws NoriKVException {
        VectorGrpc.VectorBlockingStub stub = VectorGrpc.newBlockingStub(channel)
                .withDeadlineAfter(config.getTimeoutMs(), TimeUnit.MILLISECONDS);

        Norikv.CreateVectorIndexRequest request = ProtoConverters.buildCreateVectorIndexRequest(
                namespace, dimensions, distance, indexType, options);

        Norikv.CreateVectorIndexResponse response = stub.createIndex(request);

        return response.getCreated();
    }

    private boolean performVectorDropIndex(
            ManagedChannel channel,
            String namespace,
            DropVectorIndexOptions options) throws NoriKVException {
        VectorGrpc.VectorBlockingStub stub = VectorGrpc.newBlockingStub(channel)
                .withDeadlineAfter(config.getTimeoutMs(), TimeUnit.MILLISECONDS);

        Norikv.DropVectorIndexRequest request = ProtoConverters.buildDropVectorIndexRequest(
                namespace, options);

        Norikv.DropVectorIndexResponse response = stub.dropIndex(request);

        return response.getDropped();
    }

    private Version performVectorInsert(
            ManagedChannel channel,
            String namespace,
            String id,
            List<Float> vector,
            VectorInsertOptions options) throws NoriKVException {
        VectorGrpc.VectorBlockingStub stub = VectorGrpc.newBlockingStub(channel)
                .withDeadlineAfter(config.getTimeoutMs(), TimeUnit.MILLISECONDS);

        Norikv.VectorInsertRequest request = ProtoConverters.buildVectorInsertRequest(
                namespace, id, vector, options);

        Norikv.VectorInsertResponse response = stub.insert(request);

        return ProtoConverters.fromProto(response.getVersion());
    }

    private boolean performVectorDelete(
            ManagedChannel channel,
            String namespace,
            String id,
            VectorDeleteOptions options) throws NoriKVException {
        VectorGrpc.VectorBlockingStub stub = VectorGrpc.newBlockingStub(channel)
                .withDeadlineAfter(config.getTimeoutMs(), TimeUnit.MILLISECONDS);

        Norikv.VectorDeleteRequest request = ProtoConverters.buildVectorDeleteRequest(
                namespace, id, options);

        Norikv.VectorDeleteResponse response = stub.delete(request);

        return response.getDeleted();
    }

    private VectorSearchResult performVectorSearch(
            ManagedChannel channel,
            String namespace,
            List<Float> query,
            int k,
            VectorSearchOptions options) throws NoriKVException {
        VectorGrpc.VectorBlockingStub stub = VectorGrpc.newBlockingStub(channel)
                .withDeadlineAfter(config.getTimeoutMs(), TimeUnit.MILLISECONDS);

        Norikv.VectorSearchRequest request = ProtoConverters.buildVectorSearchRequest(
                namespace, query, k, options);

        Norikv.VectorSearchResponse response = stub.search(request);

        return ProtoConverters.fromProto(response);
    }

    private List<Float> performVectorGet(
            ManagedChannel channel,
            String namespace,
            String id) throws NoriKVException {
        VectorGrpc.VectorBlockingStub stub = VectorGrpc.newBlockingStub(channel)
                .withDeadlineAfter(config.getTimeoutMs(), TimeUnit.MILLISECONDS);

        Norikv.VectorGetRequest request = ProtoConverters.buildVectorGetRequest(namespace, id);

        Norikv.VectorGetResponse response = stub.get(request);

        if (!response.getFound()) {
            return null;
        }

        return new ArrayList<>(response.getVectorList());
    }

    /**
     * Statistics about the client.
     */
    public static final class ClientStats {
        private final Router.RouterStats routerStats;
        private final ConnectionPool.PoolStats poolStats;
        private final TopologyManager.TopologyStats topologyStats;
        private final boolean closed;

        ClientStats(Router.RouterStats routerStats,
                   ConnectionPool.PoolStats poolStats,
                   TopologyManager.TopologyStats topologyStats,
                   boolean closed) {
            this.routerStats = routerStats;
            this.poolStats = poolStats;
            this.topologyStats = topologyStats;
            this.closed = closed;
        }

        public Router.RouterStats getRouterStats() {
            return routerStats;
        }

        public ConnectionPool.PoolStats getPoolStats() {
            return poolStats;
        }

        public TopologyManager.TopologyStats getTopologyStats() {
            return topologyStats;
        }

        public boolean isClosed() {
            return closed;
        }

        @Override
        public String toString() {
            return "ClientStats{" +
                    "router=" + routerStats +
                    ", pool=" + poolStats +
                    ", topology=" + topologyStats +
                    ", closed=" + closed +
                    '}';
        }
    }
}
