package com.norikv.client.hash;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.DisplayName;
import static org.junit.jupiter.api.Assertions.*;

import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.Map;

/**
 * Test suite for hash functions.
 *
 * <p>CRITICAL: These tests validate cross-SDK compatibility.
 * All test vectors must match exactly with Go, Python, and TypeScript SDKs.
 */
public class HashFunctionsTest {

    @Test
    @DisplayName("XXHash64: consistent hashes for same input")
    public void testXXHash64Consistency() {
        byte[] key = "hello".getBytes(StandardCharsets.UTF_8);
        long hash1 = HashFunctions.xxhash64(key);
        long hash2 = HashFunctions.xxhash64(key);
        assertEquals(hash1, hash2, "Hash should be consistent");
    }

    @Test
    @DisplayName("XXHash64: different inputs produce different hashes")
    public void testXXHash64Different() {
        long hash1 = HashFunctions.xxhash64("hello".getBytes(StandardCharsets.UTF_8));
        long hash2 = HashFunctions.xxhash64("world".getBytes(StandardCharsets.UTF_8));
        assertNotEquals(hash1, hash2, "Different inputs should produce different hashes");
    }

    @Test
    @DisplayName("XXHash64: handles empty string")
    public void testXXHash64Empty() {
        long hash = HashFunctions.xxhash64("".getBytes(StandardCharsets.UTF_8));
        // Just verify it doesn't throw and returns a value
        assertTrue(hash != 0 || hash == 0); // Tautology to verify it completes
    }

    @Test
    @DisplayName("XXHash64: handles unicode")
    public void testXXHash64Unicode() {
        long hash = HashFunctions.xxhash64("Hello 世界 🌍".getBytes(StandardCharsets.UTF_8));
        // Just verify it doesn't throw
        assertTrue(hash != 0 || hash == 0);
    }

    @Test
    @DisplayName("XXHash64: handles large keys")
    public void testXXHash64Large() {
        byte[] largeKey = new byte[10000];
        for (int i = 0; i < largeKey.length; i++) {
            largeKey[i] = 'x';
        }
        long hash = HashFunctions.xxhash64(largeKey);
        // Just verify it doesn't throw
        assertTrue(hash != 0 || hash == 0);
    }

    @Test
    @DisplayName("JumpConsistentHash: consistent bucket for same hash")
    public void testJumpConsistentHashConsistency() {
        long hash = 12345678901234567890L;
        int bucket1 = HashFunctions.jumpConsistentHash(hash, 1024);
        int bucket2 = HashFunctions.jumpConsistentHash(hash, 1024);
        assertEquals(bucket1, bucket2, "Bucket should be consistent");
    }

    @Test
    @DisplayName("JumpConsistentHash: bucket in valid range")
    public void testJumpConsistentHashRange() {
        long hash = HashFunctions.xxhash64("test-key".getBytes(StandardCharsets.UTF_8));
        int numBuckets = 1024;
        int bucket = HashFunctions.jumpConsistentHash(hash, numBuckets);
        assertTrue(bucket >= 0 && bucket < numBuckets,
                "Bucket " + bucket + " out of range [0, " + numBuckets + ")");
    }

    @Test
    @DisplayName("JumpConsistentHash: distributes keys across buckets")
    public void testJumpConsistentHashDistribution() {
        Map<Integer, Boolean> buckets = new HashMap<>();
        int numBuckets = 100;

        for (int i = 0; i < 1000; i++) {
            String key = "key-" + i;
            long hash = HashFunctions.xxhash64(key.getBytes(StandardCharsets.UTF_8));
            int bucket = HashFunctions.jumpConsistentHash(hash, numBuckets);
            buckets.put(bucket, true);
        }

        // With 1000 keys and 100 buckets, should hit most buckets
        assertTrue(buckets.size() > 90,
                "Poor distribution: only " + buckets.size() + "/" + numBuckets + " buckets used");
    }

    @Test
    @DisplayName("JumpConsistentHash: minimizes moves when adding buckets")
    public void testJumpConsistentHashMinimizeMoves() {
        int numKeys = 1000;
        String[] keys = new String[numKeys];
        for (int i = 0; i < numKeys; i++) {
            keys[i] = "key-" + i;
        }

        // Map keys to 100 buckets
        Map<String, Integer> mapping100 = new HashMap<>();
        for (String key : keys) {
            long hash = HashFunctions.xxhash64(key.getBytes(StandardCharsets.UTF_8));
            mapping100.put(key, HashFunctions.jumpConsistentHash(hash, 100));
        }

        // Map same keys to 101 buckets
        Map<String, Integer> mapping101 = new HashMap<>();
        for (String key : keys) {
            long hash = HashFunctions.xxhash64(key.getBytes(StandardCharsets.UTF_8));
            mapping101.put(key, HashFunctions.jumpConsistentHash(hash, 101));
        }

        // Count how many keys moved
        int moved = 0;
        for (String key : keys) {
            if (!mapping100.get(key).equals(mapping101.get(key))) {
                moved++;
            }
        }

        // With consistent hashing, roughly 1/101 (~10) keys should move
        // Allow some variance: between 1 and 20 keys
        assertTrue(moved > 0 && moved < 20,
                "Too many keys moved: " + moved + " (expected 1-20)");
    }

    @Test
    @DisplayName("JumpConsistentHash: throws on invalid bucket count")
    public void testJumpConsistentHashInvalidBuckets() {
        assertThrows(IllegalArgumentException.class, () -> {
            HashFunctions.jumpConsistentHash(123L, 0);
        });

        assertThrows(IllegalArgumentException.class, () -> {
            HashFunctions.jumpConsistentHash(123L, -1);
        });
    }

    @Test
    @DisplayName("JumpConsistentHash: handles single bucket")
    public void testJumpConsistentHashSingleBucket() {
        long hash = HashFunctions.xxhash64("test".getBytes(StandardCharsets.UTF_8));
        int bucket = HashFunctions.jumpConsistentHash(hash, 1);
        assertEquals(0, bucket, "Single bucket should be 0");
    }

    @Test
    @DisplayName("JumpConsistentHash: handles large bucket count")
    public void testJumpConsistentHashLargeBuckets() {
        long hash = HashFunctions.xxhash64("test".getBytes(StandardCharsets.UTF_8));
        int numBuckets = 1000000;
        int bucket = HashFunctions.jumpConsistentHash(hash, numBuckets);
        assertTrue(bucket >= 0 && bucket < numBuckets,
                "Bucket " + bucket + " out of range [0, " + numBuckets + ")");
    }

    @Test
    @DisplayName("GetShardForKey: consistent shard for same key")
    public void testGetShardForKeyConsistency() {
        int shard1 = HashFunctions.getShardForKey("user:123".getBytes(StandardCharsets.UTF_8), 1024);
        int shard2 = HashFunctions.getShardForKey("user:123".getBytes(StandardCharsets.UTF_8), 1024);
        assertEquals(shard1, shard2, "Shard should be consistent");
    }

    @Test
    @DisplayName("GetShardForKey: shard in valid range")
    public void testGetShardForKeyRange() {
        int shard = HashFunctions.getShardForKey("test-key".getBytes(StandardCharsets.UTF_8), 1024);
        assertTrue(shard >= 0 && shard < 1024,
                "Shard " + shard + " out of range [0, 1024)");
    }

    @Test
    @DisplayName("GetShardForKey: distributes keys across shards")
    public void testGetShardForKeyDistribution() {
        Map<Integer, Boolean> shards = new HashMap<>();

        for (int i = 0; i < 10000; i++) {
            String key = "key-" + i;
            int shard = HashFunctions.getShardForKey(key.getBytes(StandardCharsets.UTF_8), 1024);
            shards.put(shard, true);
        }

        // With 10k keys and 1024 shards, should hit most shards
        assertTrue(shards.size() > 1000,
                "Poor distribution: only " + shards.size() + "/1024 shards used");
    }

    /**
     * CRITICAL: Cross-SDK compatibility test for known hash values.
     * These test vectors are generated from the TypeScript SDK and MUST match exactly.
     */
    @Test
    @DisplayName("Cross-SDK: known hash values")
    public void testCrossSDKHashValues() {
        Object[][] testVectors = {
            {"hello", 2794345569481354659L},
            {"world", -1767385783675760145L}, // 16679358290033791471 as signed long
            {"user:123", -5608361770012350989L}, // 12838382303697200627 as signed long
            {"test-key", -2619262420286632060L}, // 15827481653422919556 as signed long
            {"", -1205034819632174695L}, // 17241709254077376921 as signed long
            {"🌍", -7101488161850985600L}, // 11345255911858566016 as signed long
            {"foo", 3728699739546630719L},
            {"bar", 5234164152756840025L},
            {"a", -3292477735350538661L}, // 15154266338359012955 as signed long
            {"ab", 7347350983217793633L},
            {"abc", 4952883123889572249L},
            {"test-value-1", 2906801998163794869L},
            {"test-value-2", 2618596406459050154L},
        };

        for (Object[] tv : testVectors) {
            String key = (String) tv[0];
            long expectedHash = (Long) tv[1];
            long actualHash = HashFunctions.xxhash64(key.getBytes(StandardCharsets.UTF_8));
            assertEquals(expectedHash, actualHash,
                    "Hash mismatch for '" + key + "': expected " + expectedHash + ", got " + actualHash);
        }
    }

    /**
     * CRITICAL: Cross-SDK compatibility test for known shard assignments.
     * These test vectors are generated from the TypeScript SDK and MUST match exactly.
     */
    @Test
    @DisplayName("Cross-SDK: known shard assignments")
    public void testCrossSDKShardAssignments() {
        Object[][] testVectors = {
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
        };

        for (Object[] tv : testVectors) {
            String key = (String) tv[0];
            int totalShards = (Integer) tv[1];
            int expectedShard = (Integer) tv[2];
            int actualShard = HashFunctions.getShardForKey(key.getBytes(StandardCharsets.UTF_8), totalShards);
            assertEquals(expectedShard, actualShard,
                    "Shard mismatch for '" + key + "': expected " + expectedShard + ", got " + actualShard);
        }
    }

    /**
     * CRITICAL: Cross-SDK compatibility test for jump consistent hash values.
     * These test vectors validate the algorithm implementation.
     */
    @Test
    @DisplayName("Cross-SDK: jump consistent hash values")
    public void testCrossSDKJumpConsistentHashValues() {
        Object[][] testVectors = {
            {0L, 100, 0},
            {1L, 100, 55},
            {12345678901234567890L, 1024, 294},
            {-1L, 1024, 313}, // 0xFFFFFFFFFFFFFFFF as signed long
        };

        for (Object[] tv : testVectors) {
            long hashValue = (Long) tv[0];
            int numBuckets = (Integer) tv[1];
            int expectedBucket = (Integer) tv[2];
            int actualBucket = HashFunctions.jumpConsistentHash(hashValue, numBuckets);
            assertEquals(expectedBucket, actualBucket,
                    "JCH mismatch for hash=" + hashValue + ", buckets=" + numBuckets +
                    ": expected " + expectedBucket + ", got " + actualBucket);
        }
    }

    @Test
    @DisplayName("KeyToBytes: converts string to bytes")
    public void testKeyToBytes() {
        String key = "hello";
        byte[] bytes = HashFunctions.keyToBytes(key);
        assertEquals(key, new String(bytes, StandardCharsets.UTF_8));
    }

    @Test
    @DisplayName("ValueToBytes: converts string to bytes")
    public void testValueToBytes() {
        String value = "world";
        byte[] bytes = HashFunctions.valueToBytes(value);
        assertEquals(value, new String(bytes, StandardCharsets.UTF_8));
    }

    @Test
    @DisplayName("BytesToString: converts bytes to string")
    public void testBytesToString() {
        byte[] bytes = "hello".getBytes(StandardCharsets.UTF_8);
        String str = HashFunctions.bytesToString(bytes);
        assertEquals("hello", str);
    }

    @Test
    @DisplayName("UTF-8 round trip: preserves unicode")
    public void testUTF8RoundTrip() {
        String original = "Hello 世界 🌍";
        byte[] bytes = HashFunctions.keyToBytes(original);
        String result = HashFunctions.bytesToString(bytes);
        assertEquals(original, result);
    }

    @Test
    @DisplayName("Empty string: round trip")
    public void testEmptyStringRoundTrip() {
        byte[] bytes = HashFunctions.keyToBytes("");
        assertEquals(0, bytes.length);
        String str = HashFunctions.bytesToString(bytes);
        assertEquals("", str);
    }
}
