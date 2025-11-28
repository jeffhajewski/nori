# NoriKV Go Client Advanced Patterns

Complex real-world usage patterns and design examples.

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

Implement a high-throughput distributed counter with sharding to reduce contention.

### Implementation

```go
package main

import (
    "context"
    "crypto/rand"
    "encoding/binary"
    "fmt"
    "strconv"
    "sync"

    norikv "github.com/norikv/norikv-go"
)

type DistributedCounter struct {
    client     *norikv.Client
    name       string
    numShards  int
}

func NewDistributedCounter(client *norikv.Client, name string, numShards int) *DistributedCounter {
    return &DistributedCounter{
        client:    client,
        name:      name,
        numShards: numShards,
    }
}

// Increment atomically increments the counter by 1
func (c *DistributedCounter) Increment(ctx context.Context) error {
    // Choose random shard to reduce contention
    shardID := c.randomShard()
    key := []byte(fmt.Sprintf("%s:shard:%d", c.name, shardID))

    const maxRetries = 10
    for attempt := 0; attempt < maxRetries; attempt++ {
        // Read current value
        result, err := c.client.Get(ctx, key, nil)
        if err != nil && !errors.Is(err, norikv.ErrKeyNotFound) {
            return err
        }

        var currentValue int
        if err == nil {
            currentValue, _ = strconv.Atoi(string(result.Value))
        }

        // Increment
        newValue := []byte(strconv.Itoa(currentValue + 1))

        // CAS write
        options := &norikv.PutOptions{}
        if err == nil {
            options.IfMatchVersion = result.Version
        } else {
            options.IfNotExists = true
        }

        _, err = c.client.Put(ctx, key, newValue, options)
        if err == nil {
            return nil
        }

        if !errors.Is(err, norikv.ErrVersionMismatch) && !errors.Is(err, norikv.ErrAlreadyExists) {
            return err
        }

        // Exponential backoff with jitter
        backoff := time.Duration(1<<attempt) * 10 * time.Millisecond
        jitter := time.Duration(rand.Intn(10)) * time.Millisecond
        time.Sleep(backoff + jitter)
    }

    return fmt.Errorf("increment failed after %d retries", maxRetries)
}

// Get returns the total count across all shards
func (c *DistributedCounter) Get(ctx context.Context) (int, error) {
    var wg sync.WaitGroup
    results := make(chan int, c.numShards)
    errors := make(chan error, c.numShards)

    for i := 0; i < c.numShards; i++ {
        wg.Add(1)
        go func(shardID int) {
            defer wg.Done()

            key := []byte(fmt.Sprintf("%s:shard:%d", c.name, shardID))
            result, err := c.client.Get(ctx, key, nil)
            if err != nil {
                if errors.Is(err, norikv.ErrKeyNotFound) {
                    results <- 0
                    return
                }
                errors <- err
                return
            }

            value, _ := strconv.Atoi(string(result.Value))
            results <- value
        }(i)
    }

    wg.Wait()
    close(results)
    close(errors)

    // Check for errors
    select {
    case err := <-errors:
        return 0, err
    default:
    }

    // Sum all shards
    total := 0
    for value := range results {
        total += value
    }

    return total, nil
}

func (c *DistributedCounter) randomShard() int {
    var b [8]byte
    rand.Read(b[:])
    return int(binary.LittleEndian.Uint64(b[:]) % uint64(c.numShards))
}
```

### Usage

```go
counter := NewDistributedCounter(client, "page-views", 10)

// Increment from multiple goroutines
var wg sync.WaitGroup
for i := 0; i < 1000; i++ {
    wg.Add(1)
    go func() {
        defer wg.Done()
        counter.Increment(ctx)
    }()
}
wg.Wait()

// Get total
total, _ := counter.Get(ctx)
fmt.Printf("Total: %d\n", total)
```

## Session Management

Implement session storage with automatic expiration using TTL.

### Implementation

```go
package main

import (
    "context"
    "encoding/json"
    "fmt"
    "time"

    "github.com/google/uuid"
    norikv "github.com/norikv/norikv-go"
)

type Session struct {
    ID        string
    UserID    int
    Data      map[string]interface{}
    CreatedAt time.Time
    ExpiresAt time.Time
}

type SessionStore struct {
    client *norikv.Client
    ttl    time.Duration
}

func NewSessionStore(client *norikv.Client, ttl time.Duration) *SessionStore {
    return &SessionStore{
        client: client,
        ttl:    ttl,
    }
}

// Create creates a new session
func (s *SessionStore) Create(ctx context.Context, userID int, data map[string]interface{}) (*Session, error) {
    session := &Session{
        ID:        uuid.New().String(),
        UserID:    userID,
        Data:      data,
        CreatedAt: time.Now(),
        ExpiresAt: time.Now().Add(s.ttl),
    }

    sessionData, err := json.Marshal(session)
    if err != nil {
        return nil, err
    }

    key := []byte(fmt.Sprintf("session:%s", session.ID))
    ttlMs := uint64(s.ttl.Milliseconds())

    options := &norikv.PutOptions{
        TTLMs:          &ttlMs,
        IdempotencyKey: session.ID,
    }

    _, err = s.client.Put(ctx, key, sessionData, options)
    if err != nil {
        return nil, err
    }

    return session, nil
}

// Get retrieves a session by ID
func (s *SessionStore) Get(ctx context.Context, sessionID string) (*Session, error) {
    key := []byte(fmt.Sprintf("session:%s", sessionID))

    result, err := s.client.Get(ctx, key, nil)
    if err != nil {
        return nil, err
    }

    var session Session
    if err := json.Unmarshal(result.Value, &session); err != nil {
        return nil, err
    }

    return &session, nil
}

// Update updates session data
func (s *SessionStore) Update(ctx context.Context, sessionID string, data map[string]interface{}) error {
    session, err := s.Get(ctx, sessionID)
    if err != nil {
        return err
    }

    session.Data = data

    sessionData, err := json.Marshal(session)
    if err != nil {
        return err
    }

    key := []byte(fmt.Sprintf("session:%s", sessionID))
    ttlMs := uint64(time.Until(session.ExpiresAt).Milliseconds())

    options := &norikv.PutOptions{
        TTLMs: &ttlMs,
    }

    _, err = s.client.Put(ctx, key, sessionData, options)
    return err
}

// Delete explicitly deletes a session (logout)
func (s *SessionStore) Delete(ctx context.Context, sessionID string) error {
    key := []byte(fmt.Sprintf("session:%s", sessionID))
    return s.client.Delete(ctx, key, nil)
}
```

### Usage

```go
store := NewSessionStore(client, 30*time.Minute)

// Create session
session, _ := store.Create(ctx, 123, map[string]interface{}{
    "role":       "admin",
    "last_login": time.Now(),
})

// Get session
retrieved, _ := store.Get(ctx, session.ID)

// Update session
store.Update(ctx, session.ID, map[string]interface{}{
    "role":       "admin",
    "last_login": time.Now(),
    "page_views": 5,
})

// Delete session (logout)
store.Delete(ctx, session.ID)
```

## Inventory Management

Prevent overselling with atomic CAS operations.

### Implementation

```go
package main

import (
    "context"
    "encoding/json"
    "fmt"
    "time"

    norikv "github.com/norikv/norikv-go"
)

type InventoryItem struct {
    SKU      string
    Quantity int
    Reserved int
}

type InventoryManager struct {
    client *norikv.Client
}

func NewInventoryManager(client *norikv.Client) *InventoryManager {
    return &InventoryManager{client: client}
}

// Reserve atomically reserves inventory
func (im *InventoryManager) Reserve(ctx context.Context, sku string, quantity int) error {
    key := []byte(fmt.Sprintf("inventory:%s", sku))

    const maxRetries = 10
    for attempt := 0; attempt < maxRetries; attempt++ {
        // Read current inventory
        result, err := im.client.Get(ctx, key, nil)
        if err != nil {
            return err
        }

        var item InventoryItem
        if err := json.Unmarshal(result.Value, &item); err != nil {
            return err
        }

        // Check availability
        available := item.Quantity - item.Reserved
        if available < quantity {
            return fmt.Errorf("insufficient inventory: need %d, have %d", quantity, available)
        }

        // Reserve
        item.Reserved += quantity

        itemData, err := json.Marshal(item)
        if err != nil {
            return err
        }

        // CAS update
        options := &norikv.PutOptions{
            IfMatchVersion: result.Version,
        }

        _, err = im.client.Put(ctx, key, itemData, options)
        if err == nil {
            return nil
        }

        if !errors.Is(err, norikv.ErrVersionMismatch) {
            return err
        }

        // Exponential backoff
        time.Sleep(time.Duration(1<<attempt) * 10 * time.Millisecond)
    }

    return fmt.Errorf("reserve failed after %d retries", maxRetries)
}

// Commit converts reservation to sale
func (im *InventoryManager) Commit(ctx context.Context, sku string, quantity int) error {
    key := []byte(fmt.Sprintf("inventory:%s", sku))

    const maxRetries = 10
    for attempt := 0; attempt < maxRetries; attempt++ {
        result, err := im.client.Get(ctx, key, nil)
        if err != nil {
            return err
        }

        var item InventoryItem
        if err := json.Unmarshal(result.Value, &item); err != nil {
            return err
        }

        // Commit: reduce quantity and reserved
        item.Quantity -= quantity
        item.Reserved -= quantity

        itemData, err := json.Marshal(item)
        if err != nil {
            return err
        }

        options := &norikv.PutOptions{
            IfMatchVersion: result.Version,
        }

        _, err = im.client.Put(ctx, key, itemData, options)
        if err == nil {
            return nil
        }

        if !errors.Is(err, norikv.ErrVersionMismatch) {
            return err
        }

        time.Sleep(time.Duration(1<<attempt) * 10 * time.Millisecond)
    }

    return fmt.Errorf("commit failed after %d retries", maxRetries)
}

// Release cancels a reservation
func (im *InventoryManager) Release(ctx context.Context, sku string, quantity int) error {
    key := []byte(fmt.Sprintf("inventory:%s", sku))

    const maxRetries = 10
    for attempt := 0; attempt < maxRetries; attempt++ {
        result, err := im.client.Get(ctx, key, nil)
        if err != nil {
            return err
        }

        var item InventoryItem
        if err := json.Unmarshal(result.Value, &item); err != nil {
            return err
        }

        // Release reservation
        item.Reserved -= quantity

        itemData, err := json.Marshal(item)
        if err != nil {
            return err
        }

        options := &norikv.PutOptions{
            IfMatchVersion: result.Version,
        }

        _, err = im.client.Put(ctx, key, itemData, options)
        if err == nil {
            return nil
        }

        if !errors.Is(err, norikv.ErrVersionMismatch) {
            return err
        }

        time.Sleep(time.Duration(1<<attempt) * 10 * time.Millisecond)
    }

    return fmt.Errorf("release failed after %d retries", maxRetries)
}
```

### Usage

```go
im := NewInventoryManager(client)

// Reserve inventory
err := im.Reserve(ctx, "SKU-12345", 2)
if err != nil {
    log.Printf("Reservation failed: %v", err)
    return
}

// Process payment...

// Commit or release
if paymentSucceeded {
    im.Commit(ctx, "SKU-12345", 2)
} else {
    im.Release(ctx, "SKU-12345", 2)
}
```

## Caching Layer

Implement a write-through cache with NoriKV.

### Implementation

```go
package main

import (
    "context"
    "encoding/json"
    "fmt"
    "sync"
    "time"

    norikv "github.com/norikv/norikv-go"
)

type CacheEntry struct {
    Value     []byte
    ExpiresAt time.Time
}

type CacheLayer struct {
    client     *norikv.Client
    localCache sync.Map
    ttl        time.Duration
}

func NewCacheLayer(client *norikv.Client, ttl time.Duration) *CacheLayer {
    cache := &CacheLayer{
        client: client,
        ttl:    ttl,
    }

    // Background cleanup of expired local cache entries
    go cache.cleanupLoop()

    return cache
}

// Get retrieves a value with local cache fallback
func (c *CacheLayer) Get(ctx context.Context, key []byte) ([]byte, error) {
    keyStr := string(key)

    // Check local cache first
    if entry, ok := c.localCache.Load(keyStr); ok {
        cached := entry.(*CacheEntry)
        if time.Now().Before(cached.ExpiresAt) {
            return cached.Value, nil
        }
        c.localCache.Delete(keyStr)
    }

    // Fetch from NoriKV
    result, err := c.client.Get(ctx, key, &norikv.GetOptions{
        Consistency: norikv.ConsistencyStaleOK, // Allow stale for cache
    })
    if err != nil {
        return nil, err
    }

    // Update local cache
    c.localCache.Store(keyStr, &CacheEntry{
        Value:     result.Value,
        ExpiresAt: time.Now().Add(c.ttl),
    })

    return result.Value, nil
}

// Put writes through to NoriKV and updates local cache
func (c *CacheLayer) Put(ctx context.Context, key, value []byte) error {
    // Write to NoriKV
    ttlMs := uint64(c.ttl.Milliseconds())
    _, err := c.client.Put(ctx, key, value, &norikv.PutOptions{
        TTLMs: &ttlMs,
    })
    if err != nil {
        return err
    }

    // Update local cache
    c.localCache.Store(string(key), &CacheEntry{
        Value:     value,
        ExpiresAt: time.Now().Add(c.ttl),
    })

    return nil
}

// Delete removes from both caches
func (c *CacheLayer) Delete(ctx context.Context, key []byte) error {
    // Delete from local cache
    c.localCache.Delete(string(key))

    // Delete from NoriKV
    return c.client.Delete(ctx, key, nil)
}

func (c *CacheLayer) cleanupLoop() {
    ticker := time.NewTicker(c.ttl / 2)
    defer ticker.Stop()

    for range ticker.C {
        now := time.Now()
        c.localCache.Range(func(key, value interface{}) bool {
            entry := value.(*CacheEntry)
            if now.After(entry.ExpiresAt) {
                c.localCache.Delete(key)
            }
            return true
        })
    }
}
```

### Usage

```go
cache := NewCacheLayer(client, 5*time.Minute)

// Write (goes to both caches)
cache.Put(ctx, []byte("user:123"), userData)

// Read (local cache first, then NoriKV)
data, _ := cache.Get(ctx, []byte("user:123"))

// Delete (removes from both)
cache.Delete(ctx, []byte("user:123"))
```

## Rate Limiting

Implement a distributed rate limiter using sliding window.

### Implementation

```go
package main

import (
    "context"
    "fmt"
    "strconv"
    "strings"
    "time"

    norikv "github.com/norikv/norikv-go"
)

type RateLimiter struct {
    client      *norikv.Client
    maxRequests int
    window      time.Duration
}

func NewRateLimiter(client *norikv.Client, maxRequests int, window time.Duration) *RateLimiter {
    return &RateLimiter{
        client:      client,
        maxRequests: maxRequests,
        window:      window,
    }
}

// Allow checks if a request is allowed
func (rl *RateLimiter) Allow(ctx context.Context, identifier string) (bool, error) {
    now := time.Now()
    windowStart := now.Add(-rl.window).Unix()

    // Use minute bucket for rate limiting
    bucket := now.Unix() / 60
    key := []byte(fmt.Sprintf("ratelimit:%s:%d", identifier, bucket))

    const maxRetries = 5
    for attempt := 0; attempt < maxRetries; attempt++ {
        // Get current count
        result, err := rl.client.Get(ctx, key, nil)

        var count int
        var version *norikv.Version

        if err == nil {
            count, _ = strconv.Atoi(string(result.Value))
            version = result.Version
        } else if !errors.Is(err, norikv.ErrKeyNotFound) {
            return false, err
        }

        // Check if under limit
        if count >= rl.maxRequests {
            return false, nil
        }

        // Increment
        count++
        ttlMs := uint64(rl.window.Milliseconds())

        options := &norikv.PutOptions{
            TTLMs: &ttlMs,
        }
        if version != nil {
            options.IfMatchVersion = version
        } else {
            options.IfNotExists = true
        }

        _, err = rl.client.Put(ctx, key, []byte(strconv.Itoa(count)), options)
        if err == nil {
            return true, nil
        }

        if !errors.Is(err, norikv.ErrVersionMismatch) && !errors.Is(err, norikv.ErrAlreadyExists) {
            return false, err
        }

        time.Sleep(10 * time.Millisecond)
    }

    return false, fmt.Errorf("rate limit check failed after retries")
}
```

### Usage

```go
limiter := NewRateLimiter(client, 100, 1*time.Minute)

// Check if request is allowed
allowed, _ := limiter.Allow(ctx, "user:123")
if !allowed {
    http.Error(w, "Rate limit exceeded", http.StatusTooManyRequests)
    return
}

// Process request...
```

## Semantic Search

Build a semantic search engine using vector embeddings:

```go
package main

import (
    "context"
    "encoding/json"
    "fmt"

    norikv "github.com/norikv/norikv-go"
)

type SemanticSearchEngine struct {
    client         *norikv.Client
    embeddingModel EmbeddingModel
    namespace      string
}

type SearchResult struct {
    ID      string
    Title   string
    Content string
    Score   float32
}

type DocumentMeta struct {
    Title   string `json:"title"`
    Content string `json:"content"`
}

func NewSemanticSearchEngine(client *norikv.Client, model EmbeddingModel) *SemanticSearchEngine {
    return &SemanticSearchEngine{
        client:         client,
        embeddingModel: model,
        namespace:      "documents",
    }
}

func (s *SemanticSearchEngine) Initialize(ctx context.Context) error {
    _, err := s.client.VectorCreateIndex(
        ctx,
        s.namespace,
        s.embeddingModel.Dimensions(),
        norikv.DistanceCosine,
        norikv.VectorIndexHNSW,
        nil,
    )
    return err
}

func (s *SemanticSearchEngine) IndexDocument(ctx context.Context, docID, title, content string) error {
    // Generate embedding
    embedding := s.embeddingModel.Embed(title + " " + content)

    // Store embedding
    _, err := s.client.VectorInsert(ctx, s.namespace, docID, embedding, nil)
    if err != nil {
        return err
    }

    // Store metadata
    meta := DocumentMeta{Title: title, Content: content}
    metaBytes, _ := json.Marshal(meta)
    _, err = s.client.Put(ctx, []byte("doc:meta:"+docID), metaBytes, nil)
    return err
}

func (s *SemanticSearchEngine) Search(ctx context.Context, query string, topK int) ([]SearchResult, error) {
    // Generate query embedding
    queryEmbedding := s.embeddingModel.Embed(query)

    // Search
    result, err := s.client.VectorSearch(ctx, s.namespace, queryEmbedding, topK, nil)
    if err != nil {
        return nil, err
    }

    // Fetch metadata
    var results []SearchResult
    for _, match := range result.Matches {
        metaResult, err := s.client.Get(ctx, []byte("doc:meta:"+match.ID), nil)
        if err != nil {
            continue
        }

        var meta DocumentMeta
        json.Unmarshal(metaResult.Value, &meta)

        results = append(results, SearchResult{
            ID:      match.ID,
            Title:   meta.Title,
            Content: meta.Content,
            Score:   1.0 - match.Distance,
        })
    }

    return results, nil
}

// Usage
func main() {
    ctx := context.Background()
    engine := NewSemanticSearchEngine(client, openAIEmbedding)
    engine.Initialize(ctx)

    // Index documents
    engine.IndexDocument(ctx, "doc1", "Machine Learning",
        "Machine learning is a subset of AI...")
    engine.IndexDocument(ctx, "doc2", "Deep Learning",
        "Deep learning uses neural networks...")

    // Search
    results, _ := engine.Search(ctx, "AI neural networks", 5)
    for _, r := range results {
        fmt.Printf("%.2f: %s\n", r.Score, r.Title)
    }
}
```

## Recommendation System

Build a product recommendation engine:

```go
type RecommendationEngine struct {
    client         *norikv.Client
    embeddingModel EmbeddingModel
}

type ProductRecommendation struct {
    ProductID string
    Name      string
    Category  string
    Score     float32
}

func NewRecommendationEngine(client *norikv.Client, model EmbeddingModel) *RecommendationEngine {
    return &RecommendationEngine{
        client:         client,
        embeddingModel: model,
    }
}

func (r *RecommendationEngine) Initialize(ctx context.Context) error {
    // Create product index
    _, err := r.client.VectorCreateIndex(
        ctx, "products",
        r.embeddingModel.Dimensions(),
        norikv.DistanceCosine,
        norikv.VectorIndexHNSW,
        nil,
    )
    if err != nil {
        return err
    }

    // Create user preference index
    _, err = r.client.VectorCreateIndex(
        ctx, "user_prefs",
        r.embeddingModel.Dimensions(),
        norikv.DistanceCosine,
        norikv.VectorIndexHNSW,
        nil,
    )
    return err
}

func (r *RecommendationEngine) IndexProduct(ctx context.Context, productID, name, desc, category string) error {
    embedding := r.embeddingModel.Embed(name + " " + desc + " " + category)
    _, err := r.client.VectorInsert(ctx, "products", productID, embedding, nil)
    return err
}

func (r *RecommendationEngine) RecordInteraction(ctx context.Context, userID, productID string) error {
    // Get product embedding
    productEmbed, err := r.client.VectorGet(ctx, "products", productID)
    if err != nil {
        return err
    }

    // Update user preferences (simple moving average)
    currentPref, _ := r.client.VectorGet(ctx, "user_prefs", userID)
    if currentPref != nil {
        for i := range currentPref {
            currentPref[i] = (currentPref[i] + productEmbed[i]) / 2
        }
    } else {
        currentPref = productEmbed
    }

    _, err = r.client.VectorInsert(ctx, "user_prefs", userID, currentPref, nil)
    return err
}

func (r *RecommendationEngine) GetRecommendations(ctx context.Context, userID string, topK int) ([]ProductRecommendation, error) {
    userPref, err := r.client.VectorGet(ctx, "user_prefs", userID)
    if err != nil || userPref == nil {
        return nil, err
    }

    result, err := r.client.VectorSearch(ctx, "products", userPref, topK, nil)
    if err != nil {
        return nil, err
    }

    var recs []ProductRecommendation
    for _, match := range result.Matches {
        recs = append(recs, ProductRecommendation{
            ProductID: match.ID,
            Score:     1.0 - match.Distance,
        })
    }
    return recs, nil
}

// Usage
func main() {
    ctx := context.Background()
    engine := NewRecommendationEngine(client, embeddingModel)
    engine.Initialize(ctx)

    // Index products
    engine.IndexProduct(ctx, "prod1", "Running Shoes", "Lightweight", "Footwear")
    engine.IndexProduct(ctx, "prod2", "Trail Shoes", "Durable", "Footwear")

    // Record user interactions
    engine.RecordInteraction(ctx, "user123", "prod1")

    // Get recommendations
    recs, _ := engine.GetRecommendations(ctx, "user123", 5)
    for _, r := range recs {
        fmt.Printf("%.2f: %s\n", r.Score, r.ProductID)
    }
}
```

## Document Deduplication

Detect near-duplicate documents using vector similarity:

```go
type DocumentDeduplicator struct {
    client              *norikv.Client
    embeddingModel      EmbeddingModel
    similarityThreshold float32
}

type DeduplicationResult struct {
    IsDuplicate bool
    DuplicateOf string
    Similarity  float32
}

func NewDocumentDeduplicator(client *norikv.Client, model EmbeddingModel, threshold float32) *DocumentDeduplicator {
    return &DocumentDeduplicator{
        client:              client,
        embeddingModel:      model,
        similarityThreshold: threshold,
    }
}

func (d *DocumentDeduplicator) Initialize(ctx context.Context) error {
    _, err := d.client.VectorCreateIndex(
        ctx, "doc_fingerprints",
        d.embeddingModel.Dimensions(),
        norikv.DistanceCosine,
        norikv.VectorIndexHNSW,
        nil,
    )
    return err
}

func (d *DocumentDeduplicator) CheckAndStore(ctx context.Context, docID, content string) (*DeduplicationResult, error) {
    embedding := d.embeddingModel.Embed(content)

    // Search for similar documents
    result, err := d.client.VectorSearch(ctx, "doc_fingerprints", embedding, 5, nil)
    if err != nil {
        return nil, err
    }

    // Check for duplicates
    for _, match := range result.Matches {
        similarity := 1.0 - match.Distance
        if similarity >= d.similarityThreshold {
            return &DeduplicationResult{
                IsDuplicate: true,
                DuplicateOf: match.ID,
                Similarity:  similarity,
            }, nil
        }
    }

    // No duplicate found, store new document
    _, err = d.client.VectorInsert(ctx, "doc_fingerprints", docID, embedding, nil)
    if err != nil {
        return nil, err
    }

    return &DeduplicationResult{IsDuplicate: false}, nil
}

// Usage
func main() {
    ctx := context.Background()
    dedup := NewDocumentDeduplicator(client, embeddingModel, 0.95)
    dedup.Initialize(ctx)

    doc1 := "The quick brown fox jumps over the lazy dog."
    doc2 := "A quick brown fox jumped over a lazy dog."

    result1, _ := dedup.CheckAndStore(ctx, "doc1", doc1)
    fmt.Printf("doc1 is duplicate: %v\n", result1.IsDuplicate) // false

    result2, _ := dedup.CheckAndStore(ctx, "doc2", doc2)
    fmt.Printf("doc2 is duplicate: %v\n", result2.IsDuplicate) // true
    fmt.Printf("doc2 duplicate of: %s\n", result2.DuplicateOf) // doc1
}
```

## Next Steps

- [API Guide](API_GUIDE.md) - Complete API reference
- [Architecture Guide](ARCHITECTURE.md) - Internal design
- [Troubleshooting Guide](TROUBLESHOOTING.md) - Common issues
- [Examples](../examples/index.md) - Working code samples
