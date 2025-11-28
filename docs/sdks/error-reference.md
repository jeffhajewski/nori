# Unified Error Reference

Complete error code reference for all NoriKV client SDKs.

## Overview

All NoriKV SDKs use consistent error types mapped from gRPC status codes. This ensures predictable error handling across languages.

## Error Hierarchy

```
NoriKVError (base)
├── KeyNotFoundError
├── VersionMismatchError
├── AlreadyExistsError
├── ConnectionError
├── TimeoutError
├── VectorIndexNotFoundError
├── VectorDimensionMismatchError
├── VectorIndexAlreadyExistsError
├── VectorNotFoundError
└── (other errors)
```

## Error Types

### KeyNotFoundError

**Description**: The requested key does not exist.

**gRPC Status**: `NOT_FOUND`

**When It Occurs**:
- GET operation on non-existent key
- DELETE operation on non-existent key (may succeed without error)
- CAS operation on non-existent key

**Retry?**: No - application must handle missing key

#### Java
```java
try {
    GetResult result = client.get(key, null);
} catch (KeyNotFoundException e) {
    System.out.println("Key not found: " + e.getMessage());
}
```

#### Go
```go
result, err := client.Get(ctx, key, nil)
if errors.Is(err, norikv.ErrKeyNotFound) {
    fmt.Println("Key not found")
}
```

#### TypeScript
```typescript
try {
    const result = await client.get(key);
} catch (err) {
    if (err instanceof KeyNotFoundError) {
        console.log('Key not found');
    }
}
```

#### Python
```python
try:
    result = await client.get(key)
except KeyNotFoundError:
    print("Key not found")
```

---

### VersionMismatchError

**Description**: Version did not match during CAS operation.

**gRPC Status**: `FAILED_PRECONDITION` with message containing "version"

**When It Occurs**:
- PUT with `ifMatchVersion` and version has changed
- DELETE with `ifMatchVersion` and version has changed
- Concurrent modifications to same key

**Retry?**: Application must retry with updated version

#### Java
```java
try {
    PutOptions options = PutOptions.builder()
        .ifMatchVersion(expectedVersion)
        .build();
    client.put(key, newValue, options);
} catch (VersionMismatchException e) {
    // Retry: read latest version and try again
    GetResult current = client.get(key, null);
    // ... retry logic
}
```

#### Go
```go
options := &norikv.PutOptions{
    IfMatchVersion: expectedVersion,
}
_, err := client.Put(ctx, key, newValue, options)
if errors.Is(err, norikv.ErrVersionMismatch) {
    // Retry: read latest version and try again
}
```

#### TypeScript
```typescript
try {
    await client.put(key, newValue, {
        ifMatchVersion: expectedVersion,
    });
} catch (err) {
    if (err instanceof VersionMismatchError) {
        // Retry: read latest version and try again
    }
}
```

#### Python
```python
try:
    await client.put(key, new_value, PutOptions(
        if_match_version=expected_version,
    ))
except VersionMismatchError:
    # Retry: read latest version and try again
    pass
```

---

### AlreadyExistsError

**Description**: Key already exists during conditional creation.

**gRPC Status**: `ALREADY_EXISTS`

**When It Occurs**:
- PUT with `ifNotExists=true` on existing key

**Retry?**: No - key already exists

#### Java
```java
try {
    PutOptions options = PutOptions.builder()
        .ifNotExists(true)
        .build();
    client.put(key, value, options);
} catch (AlreadyExistsException e) {
    System.out.println("Key already exists");
}
```

#### Go
```go
options := &norikv.PutOptions{
    IfNotExists: true,
}
_, err := client.Put(ctx, key, value, options)
if errors.Is(err, norikv.ErrAlreadyExists) {
    fmt.Println("Key already exists")
}
```

#### TypeScript
```typescript
try {
    await client.put(key, value, {
        ifNotExists: true,
    });
} catch (err) {
    if (err instanceof AlreadyExistsError) {
        console.log('Key already exists');
    }
}
```

#### Python
```python
try:
    await client.put(key, value, PutOptions(
        if_not_exists=True,
    ))
except AlreadyExistsError:
    print("Key already exists")
```

---

### ConnectionError

**Description**: Network or connection failure.

**gRPC Status**: `UNAVAILABLE`, `DEADLINE_EXCEEDED`, `CANCELLED`

**When It Occurs**:
- Server unreachable
- Network partition
- Connection closed
- Request timeout

**Retry?**: Yes - with exponential backoff

#### Java
```java
try {
    GetResult result = client.get(key, null);
} catch (ConnectionException e) {
    // Automatic retry via RetryPolicy
    // Or manual retry:
    Thread.sleep(1000);
    result = client.get(key, null);
}
```

#### Go
```go
result, err := client.Get(ctx, key, nil)
if errors.Is(err, norikv.ErrConnection) {
    // Automatic retry via RetryPolicy
    // Or manual retry:
    time.Sleep(1 * time.Second)
    result, err = client.Get(ctx, key, nil)
}
```

#### TypeScript
```typescript
try {
    const result = await client.get(key);
} catch (err) {
    if (err instanceof ConnectionError) {
        // Automatic retry via RetryPolicy
        // Or manual retry:
        await new Promise(r => setTimeout(r, 1000));
        const result = await client.get(key);
    }
}
```

#### Python
```python
try:
    result = await client.get(key)
except ConnectionError:
    # Automatic retry via RetryPolicy
    # Or manual retry:
    await asyncio.sleep(1)
    result = await client.get(key)
```

---

### InvalidArgumentError

**Description**: Invalid request parameters.

**gRPC Status**: `INVALID_ARGUMENT`

**When It Occurs**:
- Null or empty key
- Null or empty value
- Invalid consistency level
- Invalid options

**Retry?**: No - fix client code

#### Java
```java
try {
    client.put(null, value, null);  // Invalid!
} catch (NoriKVException e) {
    if (e.getCode().equals("INVALID_ARGUMENT")) {
        System.out.println("Invalid argument: " + e.getMessage());
    }
}
```

#### Go
```go
_, err := client.Put(ctx, nil, value, nil)  // Invalid!
if err != nil {
    fmt.Printf("Error: %v\n", err)
}
```

#### TypeScript
```typescript
try {
    await client.put(null, value);  // Invalid!
} catch (err) {
    if (err instanceof NoriKVError && err.code === 'INVALID_ARGUMENT') {
        console.log('Invalid argument');
    }
}
```

#### Python
```python
try:
    await client.put(None, value)  # Invalid!
except NoriKVError as err:
    if err.code == "INVALID_ARGUMENT":
        print("Invalid argument")
```

---

### PermissionDeniedError

**Description**: Authentication or authorization failure.

**gRPC Status**: `PERMISSION_DENIED`, `UNAUTHENTICATED`

**When It Occurs**:
- Missing authentication credentials
- Invalid credentials
- Insufficient permissions

**Retry?**: No - fix credentials

#### Java
```java
try {
    GetResult result = client.get(key, null);
} catch (NoriKVException e) {
    if (e.getCode().equals("PERMISSION_DENIED")) {
        System.out.println("Permission denied");
    }
}
```

#### Go
```go
result, err := client.Get(ctx, key, nil)
if err != nil && strings.Contains(err.Error(), "permission denied") {
    fmt.Println("Permission denied")
}
```

#### TypeScript
```typescript
try {
    const result = await client.get(key);
} catch (err) {
    if (err instanceof NoriKVError && err.code === 'PERMISSION_DENIED') {
        console.log('Permission denied');
    }
}
```

#### Python
```python
try:
    result = await client.get(key)
except NoriKVError as err:
    if err.code == "PERMISSION_DENIED":
        print("Permission denied")
```

---

### ResourceExhaustedError

**Description**: Rate limit or quota exceeded.

**gRPC Status**: `RESOURCE_EXHAUSTED`

**When It Occurs**:
- Too many requests
- Quota exceeded
- Memory limit reached

**Retry?**: Yes - with longer backoff

#### Java
```java
try {
    client.put(key, value, null);
} catch (NoriKVException e) {
    if (e.getCode().equals("RESOURCE_EXHAUSTED")) {
        Thread.sleep(5000);  // Wait longer
        // Retry
    }
}
```

#### Go
```go
_, err := client.Put(ctx, key, value, nil)
if err != nil && strings.Contains(err.Error(), "resource exhausted") {
    time.Sleep(5 * time.Second)  // Wait longer
    // Retry
}
```

#### TypeScript
```typescript
try {
    await client.put(key, value);
} catch (err) {
    if (err instanceof NoriKVError && err.code === 'RESOURCE_EXHAUSTED') {
        await new Promise(r => setTimeout(r, 5000));  // Wait longer
        // Retry
    }
}
```

#### Python
```python
try:
    await client.put(key, value)
except NoriKVError as err:
    if err.code == "RESOURCE_EXHAUSTED":
        await asyncio.sleep(5)  # Wait longer
        # Retry
```

---

## Vector Error Types

The following errors are specific to vector search operations.

### VectorIndexNotFoundError

**Description**: The specified vector index (namespace) does not exist.

**gRPC Status**: `NOT_FOUND` with message containing "vector index"

**When It Occurs**:
- Vector search on non-existent index
- Vector insert on non-existent index
- Vector get/delete on non-existent index

**Retry?**: No - create the index first

#### Java
```java
try {
    VectorSearchResult result = client.vectorSearch(
        "nonexistent", queryVector, 10, null);
} catch (VectorIndexNotFoundException e) {
    // Create the index first
    client.vectorCreateIndex(
        "nonexistent", 1536,
        DistanceFunction.COSINE,
        VectorIndexType.HNSW, null);
}
```

#### Go
```go
result, err := client.VectorSearch(ctx, "nonexistent", queryVector, 10, nil)
if errors.Is(err, norikv.ErrVectorIndexNotFound) {
    // Create the index first
    client.VectorCreateIndex(ctx, "nonexistent", 1536,
        norikv.DistanceCosine, norikv.VectorIndexHNSW, nil)
}
```

#### TypeScript
```typescript
try {
    const result = await client.vectorSearch('nonexistent', queryVector, 10);
} catch (err) {
    if (err instanceof VectorIndexNotFoundError) {
        // Create the index first
        await client.vectorCreateIndex('nonexistent', 1536, 'cosine', 'hnsw');
    }
}
```

#### Python
```python
try:
    result = await client.vector_search("nonexistent", query_vector, 10)
except VectorIndexNotFoundError:
    # Create the index first
    await client.vector_create_index(
        "nonexistent", 1536,
        DistanceFunction.COSINE,
        VectorIndexType.HNSW,
    )
```

---

### VectorDimensionMismatchError

**Description**: Vector dimensions don't match the index configuration.

**gRPC Status**: `INVALID_ARGUMENT` with message containing "dimension"

**When It Occurs**:
- Vector insert with wrong number of dimensions
- Vector search with wrong query vector dimensions

**Retry?**: No - fix vector dimensions

#### Java
```java
try {
    // Index expects 1536 dimensions, but we provide 768
    float[] wrongVector = new float[768];
    client.vectorInsert("embeddings", "doc-1", wrongVector, null);
} catch (VectorDimensionMismatchException e) {
    System.out.printf("Expected %d dimensions, got %d%n",
        e.getExpectedDimensions(), e.getActualDimensions());
}
```

#### Go
```go
// Index expects 1536 dimensions, but we provide 768
wrongVector := make([]float32, 768)
_, err := client.VectorInsert(ctx, "embeddings", "doc-1", wrongVector, nil)
if errors.Is(err, norikv.ErrVectorDimensionMismatch) {
    fmt.Println("Vector dimensions don't match index configuration")
}
```

#### TypeScript
```typescript
try {
    // Index expects 1536 dimensions, but we provide 768
    const wrongVector = new Float32Array(768);
    await client.vectorInsert('embeddings', 'doc-1', wrongVector);
} catch (err) {
    if (err instanceof VectorDimensionMismatchError) {
        console.log(`Expected ${err.expectedDimensions}, got ${err.actualDimensions}`);
    }
}
```

#### Python
```python
try:
    # Index expects 1536 dimensions, but we provide 768
    wrong_vector = [0.0] * 768
    await client.vector_insert("embeddings", "doc-1", wrong_vector)
except VectorDimensionMismatchError as err:
    print(f"Expected {err.expected_dimensions}, got {err.actual_dimensions}")
```

---

### VectorIndexAlreadyExistsError

**Description**: Vector index already exists.

**gRPC Status**: `ALREADY_EXISTS` with message containing "vector index"

**When It Occurs**:
- Creating a vector index that already exists

**Retry?**: No - index already exists

#### Java
```java
try {
    client.vectorCreateIndex("embeddings", 1536,
        DistanceFunction.COSINE, VectorIndexType.HNSW, null);
} catch (VectorIndexAlreadyExistsException e) {
    System.out.println("Index already exists, using existing index");
}
```

#### Go
```go
created, err := client.VectorCreateIndex(ctx, "embeddings", 1536,
    norikv.DistanceCosine, norikv.VectorIndexHNSW, nil)
if errors.Is(err, norikv.ErrVectorIndexAlreadyExists) {
    fmt.Println("Index already exists, using existing index")
}
```

#### TypeScript
```typescript
try {
    await client.vectorCreateIndex('embeddings', 1536, 'cosine', 'hnsw');
} catch (err) {
    if (err instanceof VectorIndexAlreadyExistsError) {
        console.log('Index already exists, using existing index');
    }
}
```

#### Python
```python
try:
    await client.vector_create_index(
        "embeddings", 1536,
        DistanceFunction.COSINE,
        VectorIndexType.HNSW,
    )
except VectorIndexAlreadyExistsError:
    print("Index already exists, using existing index")
```

---

### VectorNotFoundError

**Description**: The specified vector ID does not exist in the index.

**gRPC Status**: `NOT_FOUND` with message containing "vector"

**When It Occurs**:
- Vector get on non-existent ID
- Vector delete on non-existent ID (may succeed without error)

**Retry?**: No - vector doesn't exist

#### Java
```java
try {
    float[] vector = client.vectorGet("embeddings", "nonexistent-doc");
} catch (VectorNotFoundException e) {
    System.out.println("Vector not found: " + e.getVectorId());
}
```

#### Go
```go
vector, err := client.VectorGet(ctx, "embeddings", "nonexistent-doc")
if errors.Is(err, norikv.ErrVectorNotFound) {
    fmt.Println("Vector not found")
}
```

#### TypeScript
```typescript
try {
    const vector = await client.vectorGet('embeddings', 'nonexistent-doc');
} catch (err) {
    if (err instanceof VectorNotFoundError) {
        console.log('Vector not found');
    }
}
```

#### Python
```python
try:
    vector = await client.vector_get("embeddings", "nonexistent-doc")
except VectorNotFoundError:
    print("Vector not found")
```

---

## Error Code Mapping

### Key-Value Operations

| gRPC Status | Error Type | Retry? | SDK Error Name |
|-------------|------------|--------|----------------|
| NOT_FOUND | KeyNotFoundError | No | KeyNotFoundException (Java), ErrKeyNotFound (Go), KeyNotFoundError (TS/Py) |
| FAILED_PRECONDITION (version) | VersionMismatchError | App-level | VersionMismatchException (Java), ErrVersionMismatch (Go), VersionMismatchError (TS/Py) |
| ALREADY_EXISTS | AlreadyExistsError | No | AlreadyExistsException (Java), ErrAlreadyExists (Go), AlreadyExistsError (TS/Py) |
| UNAVAILABLE | ConnectionError | Yes | ConnectionException (Java), ErrConnection (Go), ConnectionError (TS/Py) |
| DEADLINE_EXCEEDED | TimeoutError | Yes | TimeoutException (Java), ErrTimeout (Go), TimeoutError (TS/Py) |
| ABORTED | NoriKVError | Yes | NoriKVException (Java), ErrAborted (Go), NoriKVError (TS/Py) |
| RESOURCE_EXHAUSTED | NoriKVError | Yes | NoriKVException (Java), ErrResourceExhausted (Go), NoriKVError (TS/Py) |
| INVALID_ARGUMENT | NoriKVError | No | NoriKVException (Java), ErrInvalidArgument (Go), NoriKVError (TS/Py) |
| PERMISSION_DENIED | NoriKVError | No | NoriKVException (Java), ErrPermissionDenied (Go), NoriKVError (TS/Py) |

### Vector Operations

| gRPC Status | Error Type | Retry? | SDK Error Name |
|-------------|------------|--------|----------------|
| NOT_FOUND (vector index) | VectorIndexNotFoundError | No | VectorIndexNotFoundException (Java), ErrVectorIndexNotFound (Go), VectorIndexNotFoundError (TS/Py) |
| NOT_FOUND (vector) | VectorNotFoundError | No | VectorNotFoundException (Java), ErrVectorNotFound (Go), VectorNotFoundError (TS/Py) |
| ALREADY_EXISTS (vector index) | VectorIndexAlreadyExistsError | No | VectorIndexAlreadyExistsException (Java), ErrVectorIndexAlreadyExists (Go), VectorIndexAlreadyExistsError (TS/Py) |
| INVALID_ARGUMENT (dimension) | VectorDimensionMismatchError | No | VectorDimensionMismatchException (Java), ErrVectorDimensionMismatch (Go), VectorDimensionMismatchError (TS/Py) |

## Retry Strategy

### Retryable Errors

These errors should be retried with exponential backoff:
- ConnectionError
- TimeoutError
- Aborted
- ResourceExhausted

### Non-Retryable Errors

These errors should NOT be automatically retried:
- KeyNotFoundError - application must handle
- VersionMismatchError - application must re-read and retry with new version
- AlreadyExistsError - key exists, no retry needed
- InvalidArgumentError - client bug, fix code
- PermissionDeniedError - credentials issue, fix config
- VectorIndexNotFoundError - create the index first
- VectorDimensionMismatchError - fix vector dimensions
- VectorIndexAlreadyExistsError - index exists, use existing
- VectorNotFoundError - vector doesn't exist

### CAS Retry Pattern

VersionMismatchError requires special handling:

#### Java
```java
public void updateWithRetry(byte[] key, Function<byte[], byte[]> transform) {
    int maxRetries = 10;
    for (int attempt = 0; attempt < maxRetries; attempt++) {
        try {
            GetResult current = client.get(key, null);
            byte[] newValue = transform.apply(current.getValue());

            PutOptions options = PutOptions.builder()
                .ifMatchVersion(current.getVersion())
                .build();

            client.put(key, newValue, options);
            return;  // Success

        } catch (VersionMismatchException e) {
            if (attempt == maxRetries - 1) throw e;
            // Exponential backoff
            Thread.sleep((long) Math.pow(2, attempt) * 10);
        }
    }
}
```

#### Go
```go
func updateWithRetry(ctx context.Context, client *norikv.Client, key []byte, transform func([]byte) []byte) error {
    maxRetries := 10
    for attempt := 0; attempt < maxRetries; attempt++ {
        current, err := client.Get(ctx, key, nil)
        if err != nil {
            return err
        }

        newValue := transform(current.Value)

        options := &norikv.PutOptions{
            IfMatchVersion: current.Version,
        }

        _, err = client.Put(ctx, key, newValue, options)
        if err == nil {
            return nil  // Success
        }

        if !errors.Is(err, norikv.ErrVersionMismatch) {
            return err
        }

        if attempt == maxRetries-1 {
            return err
        }

        // Exponential backoff
        time.Sleep(time.Duration(1<<attempt) * 10 * time.Millisecond)
    }
    return nil
}
```

#### TypeScript
```typescript
async function updateWithRetry(
    key: string,
    transform: (value: Uint8Array) => Uint8Array,
    maxRetries: number = 10
): Promise<void> {
    for (let attempt = 0; attempt < maxRetries; attempt++) {
        try {
            const current = await client.get(key);
            const newValue = transform(current.value);

            await client.put(key, newValue, {
                ifMatchVersion: current.version,
            });
            return;  // Success

        } catch (err) {
            if (!(err instanceof VersionMismatchError)) {
                throw err;
            }

            if (attempt === maxRetries - 1) {
                throw err;
            }

            // Exponential backoff
            await new Promise(r => setTimeout(r, Math.pow(2, attempt) * 10));
        }
    }
}
```

#### Python
```python
async def update_with_retry(
    key: str,
    transform: Callable[[bytes], bytes],
    max_retries: int = 10,
) -> None:
    for attempt in range(max_retries):
        try:
            current = await client.get(key)
            new_value = transform(current.value)

            await client.put(key, new_value, PutOptions(
                if_match_version=current.version,
            ))
            return  # Success

        except VersionMismatchError:
            if attempt == max_retries - 1:
                raise

            # Exponential backoff
            await asyncio.sleep((2 ** attempt) * 0.01)
```

## Debugging Errors

### Enable Debug Logging

#### Java
```java
System.setProperty("org.slf4j.simpleLogger.log.com.norikv", "DEBUG");
```

#### Go
```go
import "log"
log.SetFlags(log.LstdFlags | log.Lshortfile)
```

#### TypeScript
```typescript
process.env.DEBUG = 'norikv:*';
```

#### Python
```python
import logging
logging.basicConfig(level=logging.DEBUG)
```

### Error Properties

All SDK errors include:

| Property | Description |
|----------|-------------|
| message | Human-readable error message |
| code | Error code (e.g., "KEY_NOT_FOUND") |
| cause | Original gRPC error (if applicable) |

#### Java
```java
catch (NoriKVException e) {
    System.out.println("Message: " + e.getMessage());
    System.out.println("Code: " + e.getCode());
    System.out.println("Cause: " + e.getCause());
}
```

#### Go
```go
if err != nil {
    fmt.Printf("Error: %v\n", err)
    // Unwrap to get cause
    cause := errors.Unwrap(err)
}
```

#### TypeScript
```typescript
catch (err) {
    if (err instanceof NoriKVError) {
        console.log('Message:', err.message);
        console.log('Code:', err.code);
        console.log('Cause:', err.cause);
    }
}
```

#### Python
```python
except NoriKVError as err:
    print(f"Message: {err}")
    print(f"Code: {err.code}")
    print(f"Cause: {err.cause}")
```

## Best Practices

### 1. Handle Specific Errors

```python
#  Good: Handle specific errors
try:
    result = await client.get(key)
except KeyNotFoundError:
    return default_value
except VersionMismatchError:
    # Retry with updated version
    pass

#  Bad: Catch all exceptions
try:
    result = await client.get(key)
except Exception:
    pass  # What went wrong?
```

### 2. Use Retry Policies

Configure retry policies in the client:

```typescript
const client = new NoriKVClient({
    nodes: ['localhost:9001'],
    totalShards: 1024,
    retry: {
        maxAttempts: 5,
        initialDelayMs: 100,
        maxDelayMs: 2000,
    },
});
```

### 3. Log Errors with Context

```java
catch (NoriKVException e) {
    logger.error("Failed to get key: key={}, error={}",
        new String(key), e.getMessage(), e);
}
```

### 4. Monitor Error Rates

Track error rates by type in production:

```python
from prometheus_client import Counter

error_counter = Counter('norikv_errors_total', 'Total errors', ['error_type'])

try:
    result = await client.get(key)
except NoriKVError as err:
    error_counter.labels(error_type=err.code).inc()
    raise
```

## Next Steps

- [Getting Started](./getting-started.md) - Quick start for all SDKs
- [Hash Compatibility](./hash-compatibility.md) - Cross-SDK hash validation
- SDK-specific guides:
  - [Java API Guide](./java/API_GUIDE.md)
  - [Go API Guide](./go/API_GUIDE.md)
  - [TypeScript API Guide](./typescript/API_GUIDE.md)
  - [Python API Guide](./python/API_GUIDE.md)
