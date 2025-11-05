package com.norikv.client.testing;

import io.grpc.Server;
import io.grpc.ServerBuilder;
import io.grpc.stub.StreamObserver;
import norikv.v1.KvGrpc;
import norikv.v1.Norikv;

import java.io.IOException;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicLong;

/**
 * In-memory ephemeral KV store for testing.
 *
 * <p>Implements the NoriKV gRPC API with an in-memory storage backend.
 * Useful for integration tests without requiring a real cluster.
 *
 * <p>Features:
 * <ul>
 * <li>Thread-safe in-memory storage</li>
 * <li>Version tracking with monotonic counter</li>
 * <li>TTL support (checked on read)</li>
 * <li>Idempotency key tracking</li>
 * <li>Version matching for CAS operations</li>
 * </ul>
 *
 * <p>Limitations:
 * <ul>
 * <li>No persistence - data lost on shutdown</li>
 * <li>No Raft replication</li>
 * <li>No sharding - single node only</li>
 * <li>No background TTL cleanup</li>
 * <li>No cluster membership operations</li>
 * </ul>
 *
 * <p>Usage:
 * <pre>
 * EphemeralServer server = EphemeralServer.start(9001);
 * try {
 *     // Use server for tests
 * } finally {
 *     server.stop();
 * }
 * </pre>
 */
public class EphemeralServer {
    private final Server grpcServer;
    private final KvService kvService;
    private final int port;

    private EphemeralServer(int port, Server grpcServer, KvService kvService) {
        this.port = port;
        this.grpcServer = grpcServer;
        this.kvService = kvService;
    }

    /**
     * Starts an ephemeral server on the specified port.
     *
     * @param port the port to listen on
     * @return the started server
     * @throws IOException if server fails to start
     */
    public static EphemeralServer start(int port) throws IOException {
        KvService kvService = new KvService();
        Server grpcServer = ServerBuilder.forPort(port)
                .addService(kvService)
                .build()
                .start();

        System.out.println("EphemeralServer started on port " + port);
        return new EphemeralServer(port, grpcServer, kvService);
    }

    /**
     * Gets the port the server is listening on.
     *
     * @return the port number
     */
    public int getPort() {
        return port;
    }

    /**
     * Gets the address of the server (localhost:port).
     *
     * @return the server address
     */
    public String getAddress() {
        return "localhost:" + port;
    }

    /**
     * Stops the server gracefully.
     */
    public void stop() {
        if (grpcServer != null) {
            try {
                grpcServer.shutdown().awaitTermination(5, TimeUnit.SECONDS);
                System.out.println("EphemeralServer stopped on port " + port);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                grpcServer.shutdownNow();
            }
        }
    }

    /**
     * Clears all stored data.
     */
    public void clear() {
        kvService.clear();
    }

    /**
     * Gets the number of keys stored.
     *
     * @return the key count
     */
    public int size() {
        return kvService.size();
    }

    /**
     * In-memory storage entry with version and TTL.
     */
    private static class StorageEntry {
        final byte[] value;
        final long term;
        final long index;
        final Long expiresAt; // null = no expiration

        StorageEntry(byte[] value, long term, long index, Long expiresAt) {
            this.value = value;
            this.term = term;
            this.index = index;
            this.expiresAt = expiresAt;
        }

        boolean isExpired() {
            return expiresAt != null && System.currentTimeMillis() >= expiresAt;
        }

        Norikv.Version toProtoVersion() {
            return Norikv.Version.newBuilder()
                    .setTerm(term)
                    .setIndex(index)
                    .build();
        }
    }

    /**
     * gRPC service implementation for KV operations.
     */
    private static class KvService extends KvGrpc.KvImplBase {
        private final ConcurrentHashMap<String, StorageEntry> store;
        private final ConcurrentHashMap<String, StorageEntry> idempotencyCache;
        private final AtomicLong versionCounter;
        private static final long TERM = 1; // Single term for ephemeral server

        KvService() {
            this.store = new ConcurrentHashMap<>();
            this.idempotencyCache = new ConcurrentHashMap<>();
            this.versionCounter = new AtomicLong(0);
        }

        @Override
        public void put(Norikv.PutRequest request, StreamObserver<Norikv.PutResponse> responseObserver) {
            try {
                String keyStr = request.getKey().toStringUtf8();

                // Check idempotency key
                String idempotencyKey = request.getIdempotencyKey();
                if (idempotencyKey != null && !idempotencyKey.isEmpty()) {
                    StorageEntry cached = idempotencyCache.get(idempotencyKey);
                    if (cached != null) {
                        // Return cached response
                        Norikv.PutResponse response = Norikv.PutResponse.newBuilder()
                                .setVersion(cached.toProtoVersion())
                                .build();
                        responseObserver.onNext(response);
                        responseObserver.onCompleted();
                        return;
                    }
                }

                // Check version match (CAS)
                if (request.hasIfMatch()) {
                    StorageEntry existing = store.get(keyStr);
                    if (existing == null || existing.isExpired()) {
                        responseObserver.onError(io.grpc.Status.FAILED_PRECONDITION
                                .withDescription("Key does not exist")
                                .asRuntimeException());
                        return;
                    }

                    Norikv.Version expectedVersion = request.getIfMatch();
                    if (existing.term != expectedVersion.getTerm() ||
                        existing.index != expectedVersion.getIndex()) {
                        responseObserver.onError(io.grpc.Status.FAILED_PRECONDITION
                                .withDescription("Version mismatch")
                                .asRuntimeException());
                        return;
                    }
                }

                // Compute expiration
                Long expiresAt = null;
                if (request.getTtlMs() > 0) {
                    expiresAt = System.currentTimeMillis() + request.getTtlMs();
                }

                // Store value
                long index = versionCounter.incrementAndGet();
                byte[] value = request.getValue().toByteArray();
                StorageEntry entry = new StorageEntry(value, TERM, index, expiresAt);
                store.put(keyStr, entry);

                // Cache for idempotency
                if (idempotencyKey != null && !idempotencyKey.isEmpty()) {
                    idempotencyCache.put(idempotencyKey, entry);
                }

                // Return response
                Norikv.PutResponse response = Norikv.PutResponse.newBuilder()
                        .setVersion(entry.toProtoVersion())
                        .build();
                responseObserver.onNext(response);
                responseObserver.onCompleted();

            } catch (Exception e) {
                responseObserver.onError(io.grpc.Status.INTERNAL
                        .withDescription(e.getMessage())
                        .asRuntimeException());
            }
        }

        @Override
        public void get(Norikv.GetRequest request, StreamObserver<Norikv.GetResponse> responseObserver) {
            try {
                String keyStr = request.getKey().toStringUtf8();
                StorageEntry entry = store.get(keyStr);

                if (entry == null || entry.isExpired()) {
                    responseObserver.onError(io.grpc.Status.NOT_FOUND
                            .withDescription("Key not found")
                            .asRuntimeException());
                    return;
                }

                // Note: Consistency level ignored in ephemeral server (always consistent)
                Norikv.GetResponse response = Norikv.GetResponse.newBuilder()
                        .setValue(com.google.protobuf.ByteString.copyFrom(entry.value))
                        .setVersion(entry.toProtoVersion())
                        .build();
                responseObserver.onNext(response);
                responseObserver.onCompleted();

            } catch (Exception e) {
                responseObserver.onError(io.grpc.Status.INTERNAL
                        .withDescription(e.getMessage())
                        .asRuntimeException());
            }
        }

        @Override
        public void delete(Norikv.DeleteRequest request, StreamObserver<Norikv.DeleteResponse> responseObserver) {
            try {
                String keyStr = request.getKey().toStringUtf8();

                // Check idempotency key
                String idempotencyKey = request.getIdempotencyKey();
                if (idempotencyKey != null && !idempotencyKey.isEmpty()) {
                    StorageEntry cached = idempotencyCache.get(idempotencyKey);
                    if (cached != null) {
                        // Return cached response (always true for delete)
                        Norikv.DeleteResponse response = Norikv.DeleteResponse.newBuilder()
                                .setTombstoned(true)
                                .build();
                        responseObserver.onNext(response);
                        responseObserver.onCompleted();
                        return;
                    }
                }

                // Check version match (conditional delete)
                StorageEntry existing = store.get(keyStr);
                if (request.hasIfMatch()) {
                    if (existing == null || existing.isExpired()) {
                        responseObserver.onError(io.grpc.Status.FAILED_PRECONDITION
                                .withDescription("Key does not exist")
                                .asRuntimeException());
                        return;
                    }

                    Norikv.Version expectedVersion = request.getIfMatch();
                    if (existing.term != expectedVersion.getTerm() ||
                        existing.index != expectedVersion.getIndex()) {
                        responseObserver.onError(io.grpc.Status.FAILED_PRECONDITION
                                .withDescription("Version mismatch")
                                .asRuntimeException());
                        return;
                    }
                }

                // Delete key
                boolean deleted = (existing != null && !existing.isExpired());
                if (deleted) {
                    store.remove(keyStr);
                }

                // Cache for idempotency (using dummy entry)
                if (idempotencyKey != null && !idempotencyKey.isEmpty()) {
                    long index = versionCounter.incrementAndGet();
                    StorageEntry dummyEntry = new StorageEntry(new byte[0], TERM, index, null);
                    idempotencyCache.put(idempotencyKey, dummyEntry);
                }

                // Return response
                Norikv.DeleteResponse response = Norikv.DeleteResponse.newBuilder()
                        .setTombstoned(deleted)
                        .build();
                responseObserver.onNext(response);
                responseObserver.onCompleted();

            } catch (Exception e) {
                responseObserver.onError(io.grpc.Status.INTERNAL
                        .withDescription(e.getMessage())
                        .asRuntimeException());
            }
        }

        void clear() {
            store.clear();
            idempotencyCache.clear();
            versionCounter.set(0);
        }

        int size() {
            // Count non-expired keys
            int count = 0;
            for (StorageEntry entry : store.values()) {
                if (!entry.isExpired()) {
                    count++;
                }
            }
            return count;
        }
    }
}
