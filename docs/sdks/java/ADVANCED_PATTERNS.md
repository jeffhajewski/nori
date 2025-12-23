# Advanced Patterns for NoriKV Java Client

Complex use cases and design patterns for production systems.

## Table of Contents

- [Distributed Counter](#distributed-counter)
- [Session Management](#session-management)
- [Inventory Management](#inventory-management)
- [Caching Layer](#caching-layer)
- [Rate Limiting](#rate-limiting)
- [Leader Election](#leader-election)
- [Event Sourcing](#event-sourcing)
- [Multi-Tenancy](#multi-tenancy)
- [Semantic Search](#semantic-search)
- [Recommendation System](#recommendation-system)
- [Document Deduplication](#document-deduplication)

## Distributed Counter

Implement a distributed counter with CAS for atomic increments.

### Basic Counter

```java
public class DistributedCounter {
    private final NoriKVClient client;
    private final byte[] key;
    private final int maxRetries;

    public DistributedCounter(NoriKVClient client, String counterName) {
        this.client = client;
        this.key = counterName.getBytes(StandardCharsets.UTF_8);
        this.maxRetries = 20;

        // Initialize counter if not exists
        try {
            client.get(key, null);
        } catch (KeyNotFoundException e) {
            try {
                client.put(key, "0".getBytes(StandardCharsets.UTF_8), null);
            } catch (NoriKVException ex) {
                // Ignore - someone else may have initialized
            }
        }
    }

    public long increment() throws NoriKVException {
        return incrementBy(1);
    }

    public long incrementBy(long delta) throws NoriKVException {
        for (int attempt = 0; attempt < maxRetries; attempt++) {
            try {
                // Read current value
                GetResult current = client.get(key, null);
                long value = Long.parseLong(
                    new String(current.getValue(), StandardCharsets.UTF_8));

                // Increment
                long newValue = value + delta;

                // CAS write
                PutOptions options = PutOptions.builder()
                    .ifMatchVersion(current.getVersion())
                    .build();

                client.put(key,
                    String.valueOf(newValue).getBytes(StandardCharsets.UTF_8),
                    options);

                return newValue;

            } catch (VersionMismatchException e) {
                if (attempt == maxRetries - 1) {
                    throw new NoriKVException("COUNTER_CONFLICT",
                        "Failed to increment after " + maxRetries + " attempts");
                }

                // Exponential backoff with jitter
                try {
                    long backoff = Math.min(1L << attempt, 1000);
                    Thread.sleep(backoff + ThreadLocalRandom.current().nextInt(100));
                } catch (InterruptedException ie) {
                    Thread.currentThread().interrupt();
                    throw new NoriKVException("INTERRUPTED", "Interrupted during backoff");
                }
            }
        }

        throw new NoriKVException("COUNTER_FAILED", "Should not reach here");
    }

    public long get() throws NoriKVException {
        GetResult result = client.get(key, null);
        return Long.parseLong(new String(result.getValue(), StandardCharsets.UTF_8));
    }
}
```

### Sharded Counter (High Throughput)

For very high write rates, shard the counter:

```java
public class ShardedCounter {
    private final NoriKVClient client;
    private final String baseName;
    private final int numShards;

    public ShardedCounter(NoriKVClient client, String counterName, int numShards) {
        this.client = client;
        this.baseName = counterName;
        this.numShards = numShards;

        // Initialize all shards
        for (int i = 0; i < numShards; i++) {
            byte[] shardKey = getShardKey(i);
            try {
                client.put(shardKey, "0".getBytes(StandardCharsets.UTF_8), null);
            } catch (NoriKVException e) {
                // Ignore
            }
        }
    }

    private byte[] getShardKey(int shardId) {
        return (baseName + ":shard:" + shardId).getBytes(StandardCharsets.UTF_8);
    }

    public long increment() throws NoriKVException {
        // Hash thread ID to shard
        int shardId = (int) (Thread.currentThread().getId() % numShards);
        byte[] shardKey = getShardKey(shardId);

        // Increment specific shard
        DistributedCounter shard = new DistributedCounter(client, new String(shardKey));
        return shard.increment();
    }

    public long getTotal() throws NoriKVException {
        long total = 0;
        for (int i = 0; i < numShards; i++) {
            byte[] shardKey = getShardKey(i);
            try {
                GetResult result = client.get(shardKey, null);
                long value = Long.parseLong(
                    new String(result.getValue(), StandardCharsets.UTF_8));
                total += value;
            } catch (KeyNotFoundException e) {
                // Shard not initialized
            }
        }
        return total;
    }
}
```

## Session Management

Manage user sessions with TTL:

```java
public class SessionManager {
    private final NoriKVClient client;
    private final long sessionTTL;

    public SessionManager(NoriKVClient client, long sessionTTLMs) {
        this.client = client;
        this.sessionTTL = sessionTTLMs;
    }

    public String createSession(String userId, Map<String, Object> sessionData)
            throws NoriKVException {
        String sessionId = UUID.randomUUID().toString();
        byte[] key = sessionKey(sessionId);

        // Serialize session data
        String json = toJson(sessionData);
        byte[] value = json.getBytes(StandardCharsets.UTF_8);

        // Store with TTL
        PutOptions options = PutOptions.builder()
            .ttlMs(sessionTTL)
            .idempotencyKey("create-session-" + sessionId)
            .build();

        client.put(key, value, options);

        return sessionId;
    }

    public Optional<Map<String, Object>> getSession(String sessionId)
            throws NoriKVException {
        byte[] key = sessionKey(sessionId);

        try {
            GetResult result = client.get(key, null);
            String json = new String(result.getValue(), StandardCharsets.UTF_8);
            return Optional.of(fromJson(json));
        } catch (KeyNotFoundException e) {
            return Optional.empty(); // Session expired or doesn't exist
        }
    }

    public void refreshSession(String sessionId) throws NoriKVException {
        byte[] key = sessionKey(sessionId);

        try {
            // Read current data
            GetResult current = client.get(key, null);

            // Rewrite with new TTL
            PutOptions options = PutOptions.builder()
                .ttlMs(sessionTTL)
                .build();

            client.put(key, current.getValue(), options);

        } catch (KeyNotFoundException e) {
            throw new NoriKVException("SESSION_NOT_FOUND",
                "Session " + sessionId + " not found");
        }
    }

    public void deleteSession(String sessionId) throws NoriKVException {
        byte[] key = sessionKey(sessionId);
        client.delete(key, null);
    }

    private byte[] sessionKey(String sessionId) {
        return ("session:" + sessionId).getBytes(StandardCharsets.UTF_8);
    }

    // Simplified JSON helpers (use Jackson/Gson in production)
    private String toJson(Map<String, Object> data) {
        // Implementation
        return "{}";
    }

    private Map<String, Object> fromJson(String json) {
        // Implementation
        return new HashMap<>();
    }
}
```

## Inventory Management

Implement inventory tracking with CAS to prevent overselling:

```java
public class InventoryManager {
    private final NoriKVClient client;

    public InventoryManager(NoriKVClient client) {
        this.client = client;
    }

    public void initializeProduct(String productId, int initialQuantity)
            throws NoriKVException {
        byte[] key = productKey(productId);

        InventoryRecord record = new InventoryRecord(initialQuantity, 0);
        byte[] value = serialize(record);

        client.put(key, value, null);
    }

    public boolean reserveItems(String productId, int quantity)
            throws NoriKVException {
        byte[] key = productKey(productId);
        int maxRetries = 10;

        for (int attempt = 0; attempt < maxRetries; attempt++) {
            try {
                // Read current inventory
                GetResult current = client.get(key, null);
                InventoryRecord record = deserialize(current.getValue());

                // Check availability
                if (record.available < quantity) {
                    return false; // Not enough inventory
                }

                // Reserve items
                record.available -= quantity;
                record.reserved += quantity;

                // CAS write
                PutOptions options = PutOptions.builder()
                    .ifMatchVersion(current.getVersion())
                    .build();

                client.put(key, serialize(record), options);

                return true; // Reservation succeeded

            } catch (VersionMismatchException e) {
                if (attempt == maxRetries - 1) {
                    throw new NoriKVException("RESERVATION_CONFLICT",
                        "Failed to reserve items after " + maxRetries + " attempts");
                }

                // Backoff
                try {
                    Thread.sleep(10 + ThreadLocalRandom.current().nextInt(20));
                } catch (InterruptedException ie) {
                    Thread.currentThread().interrupt();
                    return false;
                }
            }
        }

        return false;
    }

    public void confirmReservation(String productId, int quantity)
            throws NoriKVException {
        byte[] key = productKey(productId);

        for (int attempt = 0; attempt < 10; attempt++) {
            try {
                GetResult current = client.get(key, null);
                InventoryRecord record = deserialize(current.getValue());

                // Move from reserved to sold
                record.reserved -= quantity;

                PutOptions options = PutOptions.builder()
                    .ifMatchVersion(current.getVersion())
                    .build();

                client.put(key, serialize(record), options);
                return;

            } catch (VersionMismatchException e) {
                // Retry
            }
        }

        throw new NoriKVException("CONFIRM_FAILED", "Failed to confirm reservation");
    }

    public void cancelReservation(String productId, int quantity)
            throws NoriKVException {
        byte[] key = productKey(productId);

        for (int attempt = 0; attempt < 10; attempt++) {
            try {
                GetResult current = client.get(key, null);
                InventoryRecord record = deserialize(current.getValue());

                // Return to available
                record.reserved -= quantity;
                record.available += quantity;

                PutOptions options = PutOptions.builder()
                    .ifMatchVersion(current.getVersion())
                    .build();

                client.put(key, serialize(record), options);
                return;

            } catch (VersionMismatchException e) {
                // Retry
            }
        }

        throw new NoriKVException("CANCEL_FAILED", "Failed to cancel reservation");
    }

    private byte[] productKey(String productId) {
        return ("inventory:" + productId).getBytes(StandardCharsets.UTF_8);
    }

    private static class InventoryRecord {
        int available;
        int reserved;

        InventoryRecord(int available, int reserved) {
            this.available = available;
            this.reserved = reserved;
        }
    }

    private byte[] serialize(InventoryRecord record) {
        String json = String.format("{\"available\":%d,\"reserved\":%d}",
            record.available, record.reserved);
        return json.getBytes(StandardCharsets.UTF_8);
    }

    private InventoryRecord deserialize(byte[] data) {
        // Simple parsing (use JSON library in production)
        String json = new String(data, StandardCharsets.UTF_8);
        // Parse JSON and return InventoryRecord
        return new InventoryRecord(0, 0);
    }
}
```

## Caching Layer

Use NoriKV as a distributed cache:

```java
public class CacheLayer<T> {
    private final NoriKVClient client;
    private final long defaultTTL;
    private final Function<byte[], T> deserializer;
    private final Function<T, byte[]> serializer;

    public CacheLayer(NoriKVClient client, long defaultTTLMs,
                     Function<T, byte[]> serializer,
                     Function<byte[], T> deserializer) {
        this.client = client;
        this.defaultTTL = defaultTTLMs;
        this.serializer = serializer;
        this.deserializer = deserializer;
    }

    public Optional<T> get(String key) {
        try {
            // Use STALE_OK for fastest reads
            GetOptions options = GetOptions.builder()
                .consistency(ConsistencyLevel.STALE_OK)
                .build();

            GetResult result = client.get(
                key.getBytes(StandardCharsets.UTF_8), options);

            return Optional.of(deserializer.apply(result.getValue()));

        } catch (KeyNotFoundException e) {
            return Optional.empty();
        } catch (NoriKVException e) {
            // Log error but return empty (graceful degradation)
            System.err.println("Cache get failed: " + e.getMessage());
            return Optional.empty();
        }
    }

    public void put(String key, T value, long ttlMs) {
        try {
            byte[] keyBytes = key.getBytes(StandardCharsets.UTF_8);
            byte[] valueBytes = serializer.apply(value);

            PutOptions options = PutOptions.builder()
                .ttlMs(ttlMs)
                .build();

            client.put(keyBytes, valueBytes, options);

        } catch (NoriKVException e) {
            // Log error but don't throw (cache failure shouldn't break app)
            System.err.println("Cache put failed: " + e.getMessage());
        }
    }

    public void put(String key, T value) {
        put(key, value, defaultTTL);
    }

    public void invalidate(String key) {
        try {
            client.delete(key.getBytes(StandardCharsets.UTF_8), null);
        } catch (NoriKVException e) {
            System.err.println("Cache invalidate failed: " + e.getMessage());
        }
    }

    public T getOrCompute(String key, Supplier<T> computer) {
        // Try cache first
        Optional<T> cached = get(key);
        if (cached.isPresent()) {
            return cached.get();
        }

        // Compute value
        T value = computer.get();

        // Store in cache
        put(key, value);

        return value;
    }
}

// Usage
CacheLayer<User> userCache = new CacheLayer<>(
    client,
    3600000L, // 1 hour TTL
    user -> serialize(user),
    bytes -> deserialize(bytes)
);

User user = userCache.getOrCompute("user:123", () -> {
    return database.loadUser(123);
});
```

## Rate Limiting

Implement distributed rate limiting:

```java
public class RateLimiter {
    private final NoriKVClient client;
    private final int maxRequests;
    private final long windowMs;

    public RateLimiter(NoriKVClient client, int maxRequestsPerWindow, long windowMs) {
        this.client = client;
        this.maxRequests = maxRequestsPerWindow;
        this.windowMs = windowMs;
    }

    public boolean allow(String identifier) throws NoriKVException {
        byte[] key = rateLimitKey(identifier);
        long now = System.currentTimeMillis();

        for (int attempt = 0; attempt < 10; attempt++) {
            try {
                // Try to get current state
                RateLimitState state;
                Version version;

                try {
                    GetResult current = client.get(key, null);
                    state = deserialize(current.getValue());
                    version = current.getVersion();

                    // Check if window expired
                    if (now - state.windowStart >= windowMs) {
                        // Start new window
                        state = new RateLimitState(now, 0);
                        version = null; // Treat as new key
                    }

                } catch (KeyNotFoundException e) {
                    // First request in window
                    state = new RateLimitState(now, 0);
                    version = null;
                }

                // Check limit
                if (state.count >= maxRequests) {
                    return false; // Rate limit exceeded
                }

                // Increment count
                state.count++;

                // Write with CAS or TTL
                PutOptions options = PutOptions.builder()
                    .ttlMs(windowMs)
                    .ifMatchVersion(version) // null for new keys
                    .build();

                client.put(key, serialize(state), options);

                return true; // Request allowed

            } catch (VersionMismatchException e) {
                // Retry
            }
        }

        // Failed to acquire after retries - be conservative
        return false;
    }

    private byte[] rateLimitKey(String identifier) {
        return ("ratelimit:" + identifier).getBytes(StandardCharsets.UTF_8);
    }

    private static class RateLimitState {
        long windowStart;
        int count;

        RateLimitState(long windowStart, int count) {
            this.windowStart = windowStart;
            this.count = count;
        }
    }

    private byte[] serialize(RateLimitState state) {
        String json = String.format("{\"windowStart\":%d,\"count\":%d}",
            state.windowStart, state.count);
        return json.getBytes(StandardCharsets.UTF_8);
    }

    private RateLimitState deserialize(byte[] data) {
        // Parse JSON
        return new RateLimitState(0, 0);
    }
}
```

## Leader Election

Simple leader election using CAS:

```java
public class LeaderElection {
    private final NoriKVClient client;
    private final String leadershipKey;
    private final String nodeId;
    private final long leaseDurationMs;
    private volatile boolean isLeader;

    public LeaderElection(NoriKVClient client, String group, String nodeId) {
        this.client = client;
        this.leadershipKey = "leader:" + group;
        this.nodeId = nodeId;
        this.leaseDurationMs = 10000; // 10 second lease
        this.isLeader = false;
    }

    public boolean tryAcquireLeadership() throws NoriKVException {
        byte[] key = leadershipKey.getBytes(StandardCharsets.UTF_8);
        byte[] value = nodeId.getBytes(StandardCharsets.UTF_8);

        try {
            // Try to get current leader
            GetResult current = client.get(key, null);
            String currentLeader = new String(current.getValue(), StandardCharsets.UTF_8);

            if (currentLeader.equals(nodeId)) {
                // We're already leader, refresh lease
                PutOptions options = PutOptions.builder()
                    .ttlMs(leaseDurationMs)
                    .build();

                client.put(key, value, options);
                isLeader = true;
                return true;
            }

            // Someone else is leader
            isLeader = false;
            return false;

        } catch (KeyNotFoundException e) {
            // No current leader, try to acquire
            PutOptions options = PutOptions.builder()
                .ttlMs(leaseDurationMs)
                .build();

            try {
                client.put(key, value, options);
                isLeader = true;
                return true;
            } catch (NoriKVException ex) {
                // Race - someone else acquired
                isLeader = false;
                return false;
            }
        }
    }

    public void releaseLeadership() throws NoriKVException {
        if (!isLeader) {
            return;
        }

        byte[] key = leadershipKey.getBytes(StandardCharsets.UTF_8);
        client.delete(key, null);
        isLeader = false;
    }

    public boolean isLeader() {
        return isLeader;
    }

    // Background thread to maintain leadership
    public void startLeadershipMaintenance() {
        ScheduledExecutorService scheduler = Executors.newScheduledThreadPool(1);
        scheduler.scheduleAtFixedRate(() -> {
            try {
                if (isLeader) {
                    tryAcquireLeadership(); // Refresh lease
                }
            } catch (Exception e) {
                System.err.println("Failed to maintain leadership: " + e.getMessage());
                isLeader = false;
            }
        }, 0, leaseDurationMs / 2, TimeUnit.MILLISECONDS);
    }
}
```

## Event Sourcing

Store events in NoriKV:

```java
public class EventStore {
    private final NoriKVClient client;
    private final DistributedCounter sequenceCounter;

    public EventStore(NoriKVClient client) {
        this.client = client;
        this.sequenceCounter = new DistributedCounter(client, "event:sequence");
    }

    public long appendEvent(String aggregateId, String eventType, byte[] eventData)
            throws NoriKVException {
        // Get next sequence number
        long sequence = sequenceCounter.increment();

        // Store event
        byte[] key = eventKey(aggregateId, sequence);

        Event event = new Event(sequence, aggregateId, eventType,
            System.currentTimeMillis(), eventData);

        byte[] value = serialize(event);

        PutOptions options = PutOptions.builder()
            .idempotencyKey("event-" + aggregateId + "-" + sequence)
            .build();

        client.put(key, value, options);

        return sequence;
    }

    public List<Event> getEvents(String aggregateId, long fromSequence)
            throws NoriKVException {
        List<Event> events = new ArrayList<>();

        // Read events sequentially (could optimize with parallel reads)
        for (long seq = fromSequence; seq < fromSequence + 1000; seq++) {
            byte[] key = eventKey(aggregateId, seq);

            try {
                GetResult result = client.get(key, null);
                Event event = deserialize(result.getValue());
                events.add(event);
            } catch (KeyNotFoundException e) {
                // No more events
                break;
            }
        }

        return events;
    }

    private byte[] eventKey(String aggregateId, long sequence) {
        return String.format("event:%s:%010d", aggregateId, sequence)
            .getBytes(StandardCharsets.UTF_8);
    }

    private static class Event {
        long sequence;
        String aggregateId;
        String eventType;
        long timestamp;
        byte[] data;

        Event(long sequence, String aggregateId, String eventType,
              long timestamp, byte[] data) {
            this.sequence = sequence;
            this.aggregateId = aggregateId;
            this.eventType = eventType;
            this.timestamp = timestamp;
            this.data = data;
        }
    }

    private byte[] serialize(Event event) {
        // Serialize to bytes
        return new byte[0];
    }

    private Event deserialize(byte[] data) {
        // Deserialize from bytes
        return null;
    }
}
```

## Multi-Tenancy

Isolate data by tenant:

```java
public class MultiTenantClient {
    private final NoriKVClient client;
    private final String tenantId;

    public MultiTenantClient(NoriKVClient client, String tenantId) {
        this.client = client;
        this.tenantId = tenantId;
    }

    public Version put(String key, byte[] value, PutOptions options)
            throws NoriKVException {
        byte[] tenantKey = tenantKey(key);
        return client.put(tenantKey, value, options);
    }

    public GetResult get(String key, GetOptions options)
            throws NoriKVException {
        byte[] tenantKey = tenantKey(key);
        return client.get(tenantKey, options);
    }

    public boolean delete(String key, DeleteOptions options)
            throws NoriKVException {
        byte[] tenantKey = tenantKey(key);
        return client.delete(tenantKey, options);
    }

    private byte[] tenantKey(String key) {
        // Prefix with tenant ID for isolation
        return (tenantId + ":" + key).getBytes(StandardCharsets.UTF_8);
    }

    // Factory method
    public static MultiTenantClient forTenant(NoriKVClient client, String tenantId) {
        return new MultiTenantClient(client, tenantId);
    }
}

// Usage
NoriKVClient sharedClient = new NoriKVClient(config);

MultiTenantClient tenant1 = MultiTenantClient.forTenant(sharedClient, "tenant-1");
MultiTenantClient tenant2 = MultiTenantClient.forTenant(sharedClient, "tenant-2");

// Data is automatically isolated
tenant1.put("user:123", data, null); // Stored as "tenant-1:user:123"
tenant2.put("user:123", data, null); // Stored as "tenant-2:user:123"
```

## Semantic Search

Build a semantic search engine using vector embeddings:

```java
public class SemanticSearchEngine {
    private final NoriKVClient client;
    private final EmbeddingModel embeddingModel;
    private final String namespace;

    public SemanticSearchEngine(NoriKVClient client, EmbeddingModel embeddingModel) {
        this.client = client;
        this.embeddingModel = embeddingModel;
        this.namespace = "documents";
    }

    public void initialize() throws NoriKVException {
        // Create vector index for document embeddings
        client.vectorCreateIndex(
            namespace,
            embeddingModel.getDimensions(),  // e.g., 1536 for OpenAI
            DistanceFunction.COSINE,
            VectorIndexType.HNSW,
            null
        );
    }

    public void indexDocument(String docId, String title, String content)
            throws NoriKVException {
        // Generate embedding from content
        float[] embedding = embeddingModel.embed(title + " " + content);

        // Store the embedding
        client.vectorInsert(namespace, docId, embedding, null);

        // Also store document metadata
        byte[] key = ("doc:meta:" + docId).getBytes(StandardCharsets.UTF_8);
        String json = String.format(
            "{\"title\":\"%s\",\"content\":\"%s\"}",
            escapeJson(title), escapeJson(content)
        );
        client.put(key, json.getBytes(StandardCharsets.UTF_8), null);
    }

    public List<SearchResult> search(String query, int topK) throws NoriKVException {
        // Generate query embedding
        float[] queryEmbedding = embeddingModel.embed(query);

        // Search for similar documents
        VectorSearchResult result = client.vectorSearch(
            namespace,
            queryEmbedding,
            topK,
            null
        );

        // Fetch document metadata for results
        List<SearchResult> results = new ArrayList<>();
        for (VectorMatch match : result.getMatches()) {
            byte[] metaKey = ("doc:meta:" + match.getId()).getBytes(StandardCharsets.UTF_8);
            try {
                GetResult meta = client.get(metaKey, null);
                String json = new String(meta.getValue(), StandardCharsets.UTF_8);
                DocumentMeta doc = parseDocumentMeta(json);

                results.add(new SearchResult(
                    match.getId(),
                    doc.title,
                    doc.content,
                    1.0f - match.getDistance()  // Convert distance to similarity
                ));
            } catch (KeyNotFoundException e) {
                // Document metadata not found, skip
            }
        }

        return results;
    }

    public void deleteDocument(String docId) throws NoriKVException {
        // Delete vector
        client.vectorDelete(namespace, docId, null);

        // Delete metadata
        byte[] key = ("doc:meta:" + docId).getBytes(StandardCharsets.UTF_8);
        client.delete(key, null);
    }

    private String escapeJson(String s) {
        return s.replace("\"", "\\\"").replace("\n", "\\n");
    }

    private DocumentMeta parseDocumentMeta(String json) {
        // Parse JSON to DocumentMeta
        return new DocumentMeta("", "");
    }

    public static class SearchResult {
        public final String id;
        public final String title;
        public final String content;
        public final float score;

        public SearchResult(String id, String title, String content, float score) {
            this.id = id;
            this.title = title;
            this.content = content;
            this.score = score;
        }
    }

    private static class DocumentMeta {
        String title;
        String content;

        DocumentMeta(String title, String content) {
            this.title = title;
            this.content = content;
        }
    }
}

// Usage
SemanticSearchEngine engine = new SemanticSearchEngine(client, openAIEmbedding);
engine.initialize();

// Index documents
engine.indexDocument("doc1", "Introduction to Machine Learning",
    "Machine learning is a subset of artificial intelligence...");
engine.indexDocument("doc2", "Deep Learning Fundamentals",
    "Deep learning uses neural networks with many layers...");

// Search
List<SearchResult> results = engine.search("AI neural networks", 5);
for (SearchResult result : results) {
    System.out.printf("%.2f: %s%n", result.score, result.title);
}
```

## Recommendation System

Build a product recommendation engine:

```java
public class RecommendationEngine {
    private final NoriKVClient client;
    private final EmbeddingModel embeddingModel;
    private static final String PRODUCT_NS = "products";
    private static final String USER_NS = "user_prefs";

    public RecommendationEngine(NoriKVClient client, EmbeddingModel embeddingModel) {
        this.client = client;
        this.embeddingModel = embeddingModel;
    }

    public void initialize() throws NoriKVException {
        // Create index for product embeddings
        client.vectorCreateIndex(
            PRODUCT_NS,
            embeddingModel.getDimensions(),
            DistanceFunction.COSINE,
            VectorIndexType.HNSW,
            null
        );

        // Create index for user preference embeddings
        client.vectorCreateIndex(
            USER_NS,
            embeddingModel.getDimensions(),
            DistanceFunction.COSINE,
            VectorIndexType.HNSW,
            null
        );
    }

    public void indexProduct(String productId, String name, String description,
                            String category) throws NoriKVException {
        // Generate embedding from product info
        String text = name + " " + description + " " + category;
        float[] embedding = embeddingModel.embed(text);

        // Store product embedding
        client.vectorInsert(PRODUCT_NS, productId, embedding, null);

        // Store product metadata
        byte[] key = ("product:meta:" + productId).getBytes(StandardCharsets.UTF_8);
        String json = String.format(
            "{\"name\":\"%s\",\"description\":\"%s\",\"category\":\"%s\"}",
            name, description, category
        );
        client.put(key, json.getBytes(StandardCharsets.UTF_8), null);
    }

    public void recordUserInteraction(String userId, String productId)
            throws NoriKVException {
        // Get product embedding
        float[] productEmbedding = client.vectorGet(PRODUCT_NS, productId);
        if (productEmbedding == null) {
            return; // Product not found
        }

        // Update user preference embedding (simple average for demo)
        float[] currentPref;
        try {
            currentPref = client.vectorGet(USER_NS, userId);
            if (currentPref != null) {
                // Average with new product embedding
                for (int i = 0; i < currentPref.length; i++) {
                    currentPref[i] = (currentPref[i] + productEmbedding[i]) / 2;
                }
            } else {
                currentPref = productEmbedding;
            }
        } catch (Exception e) {
            currentPref = productEmbedding;
        }

        // Store updated user preference
        client.vectorInsert(USER_NS, userId, currentPref, null);
    }

    public List<ProductRecommendation> getRecommendations(String userId, int topK)
            throws NoriKVException {
        // Get user preference embedding
        float[] userPref = client.vectorGet(USER_NS, userId);
        if (userPref == null) {
            return Collections.emptyList(); // No preferences yet
        }

        // Find similar products
        VectorSearchResult result = client.vectorSearch(
            PRODUCT_NS,
            userPref,
            topK,
            null
        );

        // Build recommendations
        List<ProductRecommendation> recommendations = new ArrayList<>();
        for (VectorMatch match : result.getMatches()) {
            byte[] metaKey = ("product:meta:" + match.getId())
                .getBytes(StandardCharsets.UTF_8);
            try {
                GetResult meta = client.get(metaKey, null);
                ProductMeta product = parseProductMeta(
                    new String(meta.getValue(), StandardCharsets.UTF_8));

                recommendations.add(new ProductRecommendation(
                    match.getId(),
                    product.name,
                    product.category,
                    1.0f - match.getDistance()
                ));
            } catch (KeyNotFoundException e) {
                // Skip if metadata not found
            }
        }

        return recommendations;
    }

    public List<ProductRecommendation> getSimilarProducts(String productId, int topK)
            throws NoriKVException {
        // Get product embedding
        float[] embedding = client.vectorGet(PRODUCT_NS, productId);
        if (embedding == null) {
            return Collections.emptyList();
        }

        // Find similar products (excluding the original)
        VectorSearchResult result = client.vectorSearch(
            PRODUCT_NS,
            embedding,
            topK + 1,  // +1 because the product itself will match
            null
        );

        List<ProductRecommendation> similar = new ArrayList<>();
        for (VectorMatch match : result.getMatches()) {
            if (match.getId().equals(productId)) {
                continue; // Skip the product itself
            }

            byte[] metaKey = ("product:meta:" + match.getId())
                .getBytes(StandardCharsets.UTF_8);
            try {
                GetResult meta = client.get(metaKey, null);
                ProductMeta product = parseProductMeta(
                    new String(meta.getValue(), StandardCharsets.UTF_8));

                similar.add(new ProductRecommendation(
                    match.getId(),
                    product.name,
                    product.category,
                    1.0f - match.getDistance()
                ));
            } catch (KeyNotFoundException e) {
                // Skip
            }

            if (similar.size() >= topK) break;
        }

        return similar;
    }

    private ProductMeta parseProductMeta(String json) {
        return new ProductMeta("", "", "");
    }

    public static class ProductRecommendation {
        public final String productId;
        public final String name;
        public final String category;
        public final float score;

        public ProductRecommendation(String productId, String name,
                                    String category, float score) {
            this.productId = productId;
            this.name = name;
            this.category = category;
            this.score = score;
        }
    }

    private static class ProductMeta {
        String name;
        String description;
        String category;

        ProductMeta(String name, String description, String category) {
            this.name = name;
            this.description = description;
            this.category = category;
        }
    }
}

// Usage
RecommendationEngine engine = new RecommendationEngine(client, embeddingModel);
engine.initialize();

// Index products
engine.indexProduct("prod1", "Running Shoes", "Lightweight running shoes", "Footwear");
engine.indexProduct("prod2", "Trail Shoes", "Durable trail running shoes", "Footwear");
engine.indexProduct("prod3", "Tennis Racket", "Professional tennis racket", "Sports");

// Record user interactions
engine.recordUserInteraction("user123", "prod1");
engine.recordUserInteraction("user123", "prod2");

// Get recommendations
List<ProductRecommendation> recs = engine.getRecommendations("user123", 5);
for (ProductRecommendation rec : recs) {
    System.out.printf("%.2f: %s (%s)%n", rec.score, rec.name, rec.category);
}

// Get similar products
List<ProductRecommendation> similar = engine.getSimilarProducts("prod1", 3);
```

## Document Deduplication

Detect near-duplicate documents using vector similarity:

```java
public class DocumentDeduplicator {
    private final NoriKVClient client;
    private final EmbeddingModel embeddingModel;
    private final float similarityThreshold;
    private static final String NAMESPACE = "doc_fingerprints";

    public DocumentDeduplicator(NoriKVClient client, EmbeddingModel embeddingModel,
                                float similarityThreshold) {
        this.client = client;
        this.embeddingModel = embeddingModel;
        this.similarityThreshold = similarityThreshold; // e.g., 0.95
    }

    public void initialize() throws NoriKVException {
        client.vectorCreateIndex(
            NAMESPACE,
            embeddingModel.getDimensions(),
            DistanceFunction.COSINE,
            VectorIndexType.HNSW,
            null
        );
    }

    public DeduplicationResult checkAndStore(String docId, String content)
            throws NoriKVException {
        // Generate embedding
        float[] embedding = embeddingModel.embed(content);

        // Search for similar documents
        VectorSearchResult searchResult = client.vectorSearch(
            NAMESPACE,
            embedding,
            5,  // Check top 5 candidates
            null
        );

        // Check for duplicates
        for (VectorMatch match : searchResult.getMatches()) {
            float similarity = 1.0f - match.getDistance();
            if (similarity >= similarityThreshold) {
                return new DeduplicationResult(
                    true,
                    match.getId(),
                    similarity
                );
            }
        }

        // No duplicate found, store the new document
        client.vectorInsert(NAMESPACE, docId, embedding, null);

        return new DeduplicationResult(false, null, 0f);
    }

    public List<DuplicateCluster> findDuplicateClusters() throws NoriKVException {
        // This would scan all documents and cluster them by similarity
        // Simplified implementation - in production, use more efficient clustering
        Map<String, List<String>> clusters = new HashMap<>();
        Set<String> processed = new HashSet<>();

        // Note: In production, you'd need to iterate through all documents
        // This is a conceptual example
        return new ArrayList<>();
    }

    public void removeDocument(String docId) throws NoriKVException {
        client.vectorDelete(NAMESPACE, docId, null);
    }

    public static class DeduplicationResult {
        public final boolean isDuplicate;
        public final String duplicateOf;
        public final float similarity;

        public DeduplicationResult(boolean isDuplicate, String duplicateOf,
                                   float similarity) {
            this.isDuplicate = isDuplicate;
            this.duplicateOf = duplicateOf;
            this.similarity = similarity;
        }
    }

    public static class DuplicateCluster {
        public final String canonicalId;
        public final List<String> duplicateIds;

        public DuplicateCluster(String canonicalId, List<String> duplicateIds) {
            this.canonicalId = canonicalId;
            this.duplicateIds = duplicateIds;
        }
    }
}

// Usage
DocumentDeduplicator dedup = new DocumentDeduplicator(client, embeddingModel, 0.95f);
dedup.initialize();

// Check documents for duplicates before storing
String doc1 = "The quick brown fox jumps over the lazy dog.";
String doc2 = "A quick brown fox jumped over a lazy dog.";  // Near-duplicate
String doc3 = "Machine learning is transforming industries.";  // Different

DeduplicationResult result1 = dedup.checkAndStore("doc1", doc1);
System.out.println("doc1 is duplicate: " + result1.isDuplicate);  // false

DeduplicationResult result2 = dedup.checkAndStore("doc2", doc2);
System.out.println("doc2 is duplicate: " + result2.isDuplicate);  // true
System.out.println("doc2 duplicate of: " + result2.duplicateOf);  // doc1
System.out.println("Similarity: " + result2.similarity);          // ~0.96

DeduplicationResult result3 = dedup.checkAndStore("doc3", doc3);
System.out.println("doc3 is duplicate: " + result3.isDuplicate);  // false
```

## See Also

- [API Guide](API_GUIDE.md) - Complete API reference
- [Architecture Guide](ARCHITECTURE.md) - Internal design
- [Troubleshooting Guide](TROUBLESHOOTING.md) - Common issues
- [Examples](../src/main/java/com/norikv/client/examples/) - Working code samples
