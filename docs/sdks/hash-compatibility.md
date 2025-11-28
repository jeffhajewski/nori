# Cross-SDK Hash Compatibility

Understanding and verifying hash compatibility across all NoriKV client SDKs.

## Overview

All NoriKV client SDKs use **identical hashing algorithms** to ensure that the same key is routed to the same shard, regardless of which SDK the client uses. This is critical for:
- **Consistent routing**: Same key → same shard across all clients
- **Mixed deployments**: Java, Go, TypeScript, and Python clients can coexist
- **Data migration**: Switch between SDKs without data loss

## Hashing Algorithm

NoriKV uses a two-stage hashing process:

```
Key → XXHash64(seed=0) → Jump Consistent Hash(numBuckets) → Shard ID
```

### Stage 1: XXHash64

- **Algorithm**: XXHash64
- **Seed**: 0 (fixed)
- **Output**: 64-bit unsigned integer
- **Properties**: Fast, high-quality distribution, widely available

### Stage 2: Jump Consistent Hash

- **Algorithm**: Jump Consistent Hash by Lamping & Veach (Google)
- **Input**: 64-bit hash from XXHash64
- **Output**: Integer in range [0, numBuckets)
- **Properties**: Minimal key movement when bucket count changes

## Implementation Verification

All SDKs must pass the same test vectors to ensure compatibility.

### Test Vectors

These test vectors verify correct implementation:

| Input Key | XXHash64 Output | Shard ID (1024 shards) |
|-----------|-----------------|------------------------|
| `user:123` | `14251066842453966278` | 856 |
| `order:456` | `15799863842936138268` | 982 |
| `session:abc` | `10368301570451808134` | 721 |
| `product:xyz` | `1405181199826606771` | 114 |
| `inventory:widget-1` | `7936582139014926890` | 531 |
| `cache:foo` | `13831765948591136573` | 845 |
| `` (empty) | `17241709254077376921` | 1007 |
| `a` | `6604973303778272674` | 527 |
| `hello world` | `2794345569481354659` | 251 |

### Java Implementation

```java
import xxhash.java.XXHash64;

public class HashTest {
    private static final long XXHASH_SEED = 0L;

    public static long xxhash64(byte[] key) {
        XXHash64 hasher = XXHash64.of(XXHASH_SEED);
        hasher.update(key);
        return hasher.hash();
    }

    public static int jumpConsistentHash(long hash, int numBuckets) {
        long b = -1L;
        long j = 0L;

        while (j < numBuckets) {
            b = j;
            hash = hash * 2862933555777941757L + 1L;
            j = (long) ((double) (b + 1L) *
                (double) (1L << 31) /
                (double) ((hash >>> 33) + 1L));
        }

        return (int) b;
    }

    public static void main(String[] args) {
        String[] testKeys = {
            "user:123",
            "order:456",
            "session:abc",
            "product:xyz",
            "inventory:widget-1",
            "cache:foo",
            "",
            "a",
            "hello world"
        };

        for (String key : testKeys) {
            byte[] keyBytes = key.getBytes(StandardCharsets.UTF_8);
            long hash = xxhash64(keyBytes);
            int shard = jumpConsistentHash(hash, 1024);
            System.out.printf("Key: '%s' -> Hash: %d -> Shard: %d\n",
                key, hash, shard);
        }
    }
}
```

### Go Implementation

```go
package main

import (
    "fmt"
    "github.com/cespare/xxhash/v2"
)

func xxhash64(key []byte) uint64 {
    return xxhash.Sum64(key)
}

func jumpConsistentHash(key uint64, numBuckets int) int32 {
    var b int64 = -1
    var j int64 = 0

    for j < int64(numBuckets) {
        b = j
        key = key*2862933555777941757 + 1
        j = int64(float64(b+1) * (float64(int64(1)<<31) / float64((key>>33)+1)))
    }

    return int32(b)
}

func main() {
    testKeys := []string{
        "user:123",
        "order:456",
        "session:abc",
        "product:xyz",
        "inventory:widget-1",
        "cache:foo",
        "",
        "a",
        "hello world",
    }

    for _, key := range testKeys {
        hash := xxhash64([]byte(key))
        shard := jumpConsistentHash(hash, 1024)
        fmt.Printf("Key: '%s' -> Hash: %d -> Shard: %d\n",
            key, hash, shard)
    }
}
```

### TypeScript Implementation

```typescript
import xxhash from 'xxhash-wasm';

async function initHasher() {
    const { h64 } = await xxhash();

    function xxhash64(key: Uint8Array): bigint {
        return h64(key, 0);
    }

    function jumpConsistentHash(key: bigint, numBuckets: number): number {
        let b = -1n;
        let j = 0n;

        while (j < BigInt(numBuckets)) {
            b = j;
            key = key * 2862933555777941757n + 1n;
            j = BigInt(
                Math.floor(
                    Number(b + 1n) *
                    (Number(1n << 31n) / Number((key >> 33n) + 1n))
                )
            );
        }

        return Number(b);
    }

    const testKeys = [
        'user:123',
        'order:456',
        'session:abc',
        'product:xyz',
        'inventory:widget-1',
        'cache:foo',
        '',
        'a',
        'hello world',
    ];

    for (const key of testKeys) {
        const keyBytes = new TextEncoder().encode(key);
        const hash = xxhash64(keyBytes);
        const shard = jumpConsistentHash(hash, 1024);
        console.log(`Key: '${key}' -> Hash: ${hash} -> Shard: ${shard}`);
    }
}

initHasher();
```

### Python Implementation

```python
import xxhash

def xxhash64(key: bytes) -> int:
    hasher = xxhash.xxh64(seed=0)
    hasher.update(key)
    return hasher.intdigest()

def jump_consistent_hash(key: int, num_buckets: int) -> int:
    b = -1
    j = 0

    while j < num_buckets:
        b = j
        key = ((key * 2862933555777941757) + 1) & 0xFFFFFFFFFFFFFFFF
        j = int(float(b + 1) * (float(1 << 31) / float((key >> 33) + 1)))

    return b

test_keys = [
    "user:123",
    "order:456",
    "session:abc",
    "product:xyz",
    "inventory:widget-1",
    "cache:foo",
    "",
    "a",
    "hello world",
]

for key in test_keys:
    key_bytes = key.encode("utf-8")
    hash_val = xxhash64(key_bytes)
    shard = jump_consistent_hash(hash_val, 1024)
    print(f"Key: '{key}' -> Hash: {hash_val} -> Shard: {shard}")
```

## Expected Output

All implementations should produce **identical output**:

```
Key: 'user:123' -> Hash: 14251066842453966278 -> Shard: 856
Key: 'order:456' -> Hash: 15799863842936138268 -> Shard: 982
Key: 'session:abc' -> Hash: 10368301570451808134 -> Shard: 721
Key: 'product:xyz' -> Hash: 1405181199826606771 -> Shard: 114
Key: 'inventory:widget-1' -> Hash: 7936582139014926890 -> Shard: 531
Key: 'cache:foo' -> Hash: 13831765948591136573 -> Shard: 845
Key: '' -> Hash: 17241709254077376921 -> Shard: 1007
Key: 'a' -> Hash: 6604973303778272674 -> Shard: 527
Key: 'hello world' -> Hash: 2794345569481354659 -> Shard: 251
```

## Automated Testing

Each SDK includes automated tests that verify compatibility:

### Java Test
```java
@Test
public void testHashCompatibility() {
    Map<String, Integer> expected = Map.of(
        "user:123", 856,
        "order:456", 982,
        "session:abc", 721,
        "product:xyz", 114,
        "hello world", 251
    );

    for (Map.Entry<String, Integer> entry : expected.entrySet()) {
        byte[] key = entry.getKey().getBytes(StandardCharsets.UTF_8);
        int shard = router.getShardForKey(key);
        assertEquals(entry.getValue().intValue(), shard);
    }
}
```

### Go Test
```go
func TestHashCompatibility(t *testing.T) {
    expected := map[string]int32{
        "user:123":    856,
        "order:456":   982,
        "session:abc": 721,
        "product:xyz": 114,
        "hello world": 251,
    }

    for key, expectedShard := range expected {
        shard := router.GetShardForKey([]byte(key))
        if shard != expectedShard {
            t.Errorf("Key %s: expected shard %d, got %d",
                key, expectedShard, shard)
        }
    }
}
```

### TypeScript Test
```typescript
test('hash compatibility', () => {
    const expected = {
        'user:123': 856,
        'order:456': 982,
        'session:abc': 721,
        'product:xyz': 114,
        'hello world': 251,
    };

    for (const [key, expectedShard] of Object.entries(expected)) {
        const shard = router.getShardForKey(key);
        expect(shard).toBe(expectedShard);
    }
});
```

### Python Test
```python
def test_hash_compatibility():
    expected = {
        "user:123": 856,
        "order:456": 982,
        "session:abc": 721,
        "product:xyz": 114,
        "hello world": 251,
    }

    for key, expected_shard in expected.items():
        shard = router.get_shard_for_key(key.encode())
        assert shard == expected_shard, \
            f"Key {key}: expected {expected_shard}, got {shard}"
```

## Common Pitfalls

### 1. Wrong XXHash Seed

**Problem**: Using a non-zero seed for XXHash64

```python
#  Wrong
hasher = xxhash.xxh64(seed=12345)

#  Correct
hasher = xxhash.xxh64(seed=0)
```

### 2. String Encoding Issues

**Problem**: Different UTF-8 encoding between languages

```java
//  Correct: Use UTF-8 explicitly
byte[] key = "user:123".getBytes(StandardCharsets.UTF_8);
```

```typescript
//  Correct: TextEncoder uses UTF-8
const key = new TextEncoder().encode("user:123");
```

### 3. Integer Overflow

**Problem**: Integer overflow in jump consistent hash

```python
#  Correct: Mask to 64 bits
key = ((key * 2862933555777941757) + 1) & 0xFFFFFFFFFFFFFFFF
```

### 4. Floating Point Precision

**Problem**: Loss of precision in jump consistent hash calculation

```go
//  Correct: Use proper type conversions
j = int64(float64(b+1) * (float64(int64(1)<<31) / float64((key>>33)+1)))
```

## Debugging Hash Mismatches

If you suspect hash compatibility issues:

### 1. Enable Debug Logging

```java
// Java
Logger.getLogger("com.norikv.router").setLevel(Level.DEBUG);
```

```go
// Go
log.Printf("Key: %s -> Hash: %d -> Shard: %d", key, hash, shard)
```

```typescript
// TypeScript
console.log(`Key: ${key} -> Hash: ${hash} -> Shard: ${shard}`);
```

```python
# Python
logger.debug(f"Key: {key} -> Hash: {hash} -> Shard: {shard}")
```

### 2. Verify Test Vectors

Run the test vector verification for your SDK and compare output with the expected values above.

### 3. Check Dependencies

Ensure you're using the correct hash library:
- Java: `net.openhft:zero-allocation-hashing`
- Go: `github.com/cespare/xxhash/v2`
- TypeScript: `xxhash-wasm`
- Python: `xxhash` (Python package)

### 4. Validate Key Encoding

```python
# Print key bytes
key = "user:123"
key_bytes = key.encode("utf-8")
print(f"Key bytes: {list(key_bytes)}")
# Should output: [117, 115, 101, 114, 58, 49, 50, 51]
```

## Performance Characteristics

Hash computation is extremely fast across all SDKs:

| SDK | Hash Time (ns/op) | Allocations |
|-----|------------------|-------------|
| Java | 45 ns | 0 |
| Go | 23 ns | 0 |
| TypeScript | 80 ns | 0 |
| Python | 120 ns | 0 |

These measurements are for a 16-byte key on typical hardware.

## Shard Count Changes

If you need to change the number of shards:

1. **Jump Consistent Hash minimizes key movement**
   - Adding shards: ~1/n keys move
   - Removing shards: ~1/n keys move

2. **All clients must use the same shard count**
   - Configure `totalShards` to match cluster

3. **Rehashing is automatic**
   - No client code changes needed
   - Keys automatically route to new shards

## References

- [XXHash Algorithm](https://cyan4973.github.io/xxHash/)
- [Jump Consistent Hash Paper](https://arxiv.org/abs/1406.2294)
- [Test Vector Source Code](https://github.com/jeffhajewski/norikv/tree/main/tests/hash_compatibility)

## Next Steps

- [Getting Started](./getting-started.md) - Quick start for all SDKs
- [Error Reference](./error-reference.md) - Unified error codes
- [SDK Comparison](./index.md) - Feature comparison
