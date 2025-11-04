package hash

import (
	"testing"
)

func TestXXHash64Consistency(t *testing.T) {
	t.Run("hash strings consistently", func(t *testing.T) {
		key := []byte("hello")
		hash1 := XXHash64(key)
		hash2 := XXHash64(key)
		if hash1 != hash2 {
			t.Errorf("Inconsistent hashes: %d != %d", hash1, hash2)
		}
	})

	t.Run("different inputs produce different hashes", func(t *testing.T) {
		hash1 := XXHash64([]byte("hello"))
		hash2 := XXHash64([]byte("world"))
		if hash1 == hash2 {
			t.Errorf("Different inputs produced same hash: %d", hash1)
		}
	})

	t.Run("handles empty string", func(t *testing.T) {
		hash := XXHash64([]byte(""))
		// Just verify it doesn't panic and returns a value
		if hash < 0 {
			t.Errorf("Hash should be non-negative, got: %d", hash)
		}
	})

	t.Run("handles unicode", func(t *testing.T) {
		hash := XXHash64([]byte("Hello 世界 🌍"))
		if hash < 0 {
			t.Errorf("Hash should be non-negative, got: %d", hash)
		}
	})

	t.Run("handles large keys", func(t *testing.T) {
		largeKey := make([]byte, 10000)
		for i := range largeKey {
			largeKey[i] = 'x'
		}
		hash := XXHash64(largeKey)
		if hash < 0 {
			t.Errorf("Hash should be non-negative, got: %d", hash)
		}
	})
}

func TestJumpConsistentHash(t *testing.T) {
	t.Run("consistent bucket for same hash", func(t *testing.T) {
		hash := uint64(12345678901234567890)
		bucket1 := JumpConsistentHash(hash, 1024)
		bucket2 := JumpConsistentHash(hash, 1024)
		if bucket1 != bucket2 {
			t.Errorf("Inconsistent buckets: %d != %d", bucket1, bucket2)
		}
	})

	t.Run("bucket in valid range", func(t *testing.T) {
		hash := XXHash64([]byte("test-key"))
		numBuckets := 1024
		bucket := JumpConsistentHash(hash, numBuckets)
		if bucket < 0 || bucket >= numBuckets {
			t.Errorf("Bucket %d out of range [0, %d)", bucket, numBuckets)
		}
	})

	t.Run("distributes keys across buckets", func(t *testing.T) {
		buckets := make(map[int]bool)
		numBuckets := 100

		for i := 0; i < 1000; i++ {
			key := []byte("key-" + string(rune(i)))
			hash := XXHash64(key)
			bucket := JumpConsistentHash(hash, numBuckets)
			buckets[bucket] = true
		}

		// With 1000 keys and 100 buckets, should hit most buckets
		if len(buckets) <= 90 {
			t.Errorf("Poor distribution: only %d/%d buckets used", len(buckets), numBuckets)
		}
	})

	t.Run("minimizes moves when adding buckets", func(t *testing.T) {
		numKeys := 1000
		keys := make([][]byte, numKeys)
		for i := 0; i < numKeys; i++ {
			keys[i] = []byte("key-" + string(rune(i)))
		}

		// Map keys to 100 buckets
		mapping100 := make(map[string]int)
		for _, key := range keys {
			hash := XXHash64(key)
			mapping100[string(key)] = JumpConsistentHash(hash, 100)
		}

		// Map same keys to 101 buckets
		mapping101 := make(map[string]int)
		for _, key := range keys {
			hash := XXHash64(key)
			mapping101[string(key)] = JumpConsistentHash(hash, 101)
		}

		// Count how many keys moved
		moved := 0
		for _, key := range keys {
			if mapping100[string(key)] != mapping101[string(key)] {
				moved++
			}
		}

		// With consistent hashing, roughly 1/101 (~10) keys should move
		// Allow some variance: between 1 and 20 keys
		if moved <= 0 || moved >= 20 {
			t.Errorf("Too many keys moved: %d (expected 1-20)", moved)
		}
	})

	t.Run("panics on invalid bucket count", func(t *testing.T) {
		defer func() {
			if r := recover(); r == nil {
				t.Errorf("Expected panic for invalid bucket count")
			}
		}()
		JumpConsistentHash(123, 0)
	})

	t.Run("handles single bucket", func(t *testing.T) {
		hash := XXHash64([]byte("test"))
		bucket := JumpConsistentHash(hash, 1)
		if bucket != 0 {
			t.Errorf("Single bucket should be 0, got: %d", bucket)
		}
	})

	t.Run("handles large bucket count", func(t *testing.T) {
		hash := XXHash64([]byte("test"))
		numBuckets := 1000000
		bucket := JumpConsistentHash(hash, numBuckets)
		if bucket < 0 || bucket >= numBuckets {
			t.Errorf("Bucket %d out of range [0, %d)", bucket, numBuckets)
		}
	})
}

func TestGetShardForKey(t *testing.T) {
	t.Run("consistent shard for same key", func(t *testing.T) {
		shard1 := GetShardForKey([]byte("user:123"), 1024)
		shard2 := GetShardForKey([]byte("user:123"), 1024)
		if shard1 != shard2 {
			t.Errorf("Inconsistent shards: %d != %d", shard1, shard2)
		}
	})

	t.Run("shard in valid range", func(t *testing.T) {
		shard := GetShardForKey([]byte("test-key"), 1024)
		if shard < 0 || shard >= 1024 {
			t.Errorf("Shard %d out of range [0, 1024)", shard)
		}
	})

	t.Run("distributes keys across shards", func(t *testing.T) {
		shards := make(map[int]bool)

		for i := 0; i < 10000; i++ {
			key := []byte("key-" + string(rune(i)))
			shard := GetShardForKey(key, 1024)
			shards[shard] = true
		}

		// With 10k keys and 1024 shards, should hit most shards
		if len(shards) <= 1000 {
			t.Errorf("Poor distribution: only %d/1024 shards used", len(shards))
		}
	})
}

// TestCrossSDKCompatibility validates parity with Python and TypeScript SDKs.
// CRITICAL: These test vectors must match exactly or routing will fail.
func TestCrossSDKCompatibility(t *testing.T) {
	t.Run("known hash values", func(t *testing.T) {
		// Test vectors generated from TypeScript SDK
		// Generated by sdks/typescript/scripts/generate-hash-vectors.ts
		// These MUST match exactly
		testVectors := []struct {
			key          string
			expectedHash uint64
		}{
			{"hello", 2794345569481354659},
			{"world", 16679358290033791471},
			{"user:123", 12838382303697200627},
			{"test-key", 15827481653422919556},
			{"", 17241709254077376921},
			{"🌍", 11345255911858566016},
			{"foo", 3728699739546630719},
			{"bar", 5234164152756840025},
			{"a", 15154266338359012955},
			{"ab", 7347350983217793633},
			{"abc", 4952883123889572249},
			{"test-value-1", 2906801998163794869},
			{"test-value-2", 2618596406459050154},
		}

		for _, tv := range testVectors {
			actualHash := XXHash64([]byte(tv.key))
			if actualHash != tv.expectedHash {
				t.Errorf("Hash mismatch for '%s': expected %d, got %d",
					tv.key, tv.expectedHash, actualHash)
			}
		}
	})

	t.Run("known shard assignments", func(t *testing.T) {
		// Test vectors generated from TypeScript SDK
		// Generated by sdks/typescript/scripts/generate-hash-vectors.ts
		// These MUST match exactly
		testVectors := []struct {
			key           string
			totalShards   int
			expectedShard int
		}{
			{"hello", 1024, 309},
			{"world", 1024, 752},
			{"user:123", 1024, 928},
			{"test-key", 1024, 504},
			{"", 1024, 332},
			{"🌍", 1024, 393},
			{"foo", 1024, 951},
			{"bar", 1024, 632},
			{"a", 1024, 894},
			{"ab", 1024, 335},
			{"abc", 1024, 722},
			{"test-value-1", 1024, 640},
			{"test-value-2", 1024, 488},
		}

		for _, tv := range testVectors {
			actualShard := GetShardForKey([]byte(tv.key), tv.totalShards)
			if actualShard != tv.expectedShard {
				t.Errorf("Shard mismatch for '%s': expected %d, got %d",
					tv.key, tv.expectedShard, actualShard)
			}
		}
	})

	t.Run("jump consistent hash values", func(t *testing.T) {
		// Test vectors for jump_consistent_hash algorithm
		// Generated by sdks/typescript/scripts/generate-hash-vectors.ts
		testVectors := []struct {
			hashValue      uint64
			numBuckets     int
			expectedBucket int
		}{
			{0, 100, 0},
			{1, 100, 55},
			{12345678901234567890, 1024, 294},
			{0xFFFFFFFFFFFFFFFF, 1024, 313},
		}

		for _, tv := range testVectors {
			actualBucket := JumpConsistentHash(tv.hashValue, tv.numBuckets)
			if actualBucket != tv.expectedBucket {
				t.Errorf("JCH mismatch for hash=%d, buckets=%d: expected %d, got %d",
					tv.hashValue, tv.numBuckets, tv.expectedBucket, actualBucket)
			}
		}
	})
}

// TestKeyValueConversions tests convenience conversion functions.
func TestKeyValueConversions(t *testing.T) {
	t.Run("KeyToBytes", func(t *testing.T) {
		key := "hello"
		bytes := KeyToBytes(key)
		if string(bytes) != key {
			t.Errorf("KeyToBytes mismatch: expected %s, got %s", key, string(bytes))
		}
	})

	t.Run("ValueToBytes", func(t *testing.T) {
		value := "world"
		bytes := ValueToBytes(value)
		if string(bytes) != value {
			t.Errorf("ValueToBytes mismatch: expected %s, got %s", value, string(bytes))
		}
	})

	t.Run("BytesToString", func(t *testing.T) {
		bytes := []byte("hello")
		str := BytesToString(bytes)
		if str != "hello" {
			t.Errorf("BytesToString mismatch: expected hello, got %s", str)
		}
	})

	t.Run("UTF-8 round trip", func(t *testing.T) {
		original := "Hello 世界 🌍"
		bytes := KeyToBytes(original)
		result := BytesToString(bytes)
		if result != original {
			t.Errorf("UTF-8 round trip failed: expected %s, got %s", original, result)
		}
	})

	t.Run("empty string", func(t *testing.T) {
		bytes := KeyToBytes("")
		if len(bytes) != 0 {
			t.Errorf("Empty string should produce empty bytes, got length %d", len(bytes))
		}
		str := BytesToString(bytes)
		if str != "" {
			t.Errorf("Empty bytes should produce empty string, got %s", str)
		}
	})
}

// BenchmarkXXHash64 benchmarks the hash function performance.
func BenchmarkXXHash64(b *testing.B) {
	key := []byte("test-key")
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		XXHash64(key)
	}
}

// BenchmarkGetShardForKey benchmarks the shard assignment performance.
func BenchmarkGetShardForKey(b *testing.B) {
	key := []byte("test-key")
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		GetShardForKey(key, 1024)
	}
}

// BenchmarkJumpConsistentHash benchmarks the consistent hash algorithm.
func BenchmarkJumpConsistentHash(b *testing.B) {
	hash := uint64(12345678901234567890)
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		JumpConsistentHash(hash, 1024)
	}
}
