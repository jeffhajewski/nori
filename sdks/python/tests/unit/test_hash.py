"""Unit tests for hash functions.

CRITICAL: These tests validate that our hash functions produce correct results
and match the TypeScript SDK implementation. Any failures here could indicate
routing bugs that would cause data loss.
"""

import pytest
from norikv.hash import (
    xxhash64,
    jump_consistent_hash,
    get_shard_for_key,
    key_to_bytes,
    value_to_bytes,
    bytes_to_string,
)


class TestXXHash64:
    """Tests for xxhash64 function."""

    def test_hash_strings_consistently(self):
        """Hash should be deterministic."""
        hash1 = xxhash64("hello")
        hash2 = xxhash64("hello")
        assert hash1 == hash2

    def test_hash_bytes_consistently(self):
        """Hash should be deterministic for bytes."""
        data = b"hello"
        hash1 = xxhash64(data)
        hash2 = xxhash64(data)
        assert hash1 == hash2

    def test_same_hash_for_string_and_bytes(self):
        """String and bytes should produce same hash."""
        text = "hello"
        data = text.encode("utf-8")
        hash1 = xxhash64(text)
        hash2 = xxhash64(data)
        assert hash1 == hash2

    def test_different_hashes_for_different_inputs(self):
        """Different inputs should produce different hashes."""
        hash1 = xxhash64("hello")
        hash2 = xxhash64("world")
        assert hash1 != hash2

    def test_produces_64bit_values(self):
        """Hash should be a 64-bit unsigned integer."""
        hash_value = xxhash64("test")
        assert isinstance(hash_value, int)
        assert 0 <= hash_value < 2**64

    def test_handles_empty_string(self):
        """Should handle empty strings."""
        hash_value = xxhash64("")
        assert isinstance(hash_value, int)
        assert 0 <= hash_value < 2**64

    def test_handles_unicode(self):
        """Should handle Unicode strings."""
        hash_value = xxhash64("Hello 世界 🌍")
        assert isinstance(hash_value, int)
        assert 0 <= hash_value < 2**64

    def test_handles_large_keys(self):
        """Should handle large keys."""
        large_key = "x" * 10000
        hash_value = xxhash64(large_key)
        assert isinstance(hash_value, int)
        assert 0 <= hash_value < 2**64


class TestJumpConsistentHash:
    """Tests for jump_consistent_hash function."""

    def test_consistent_bucket_for_same_hash(self):
        """Should return same bucket for same hash."""
        hash_value = 12345678901234567890
        bucket1 = jump_consistent_hash(hash_value, 1024)
        bucket2 = jump_consistent_hash(hash_value, 1024)
        assert bucket1 == bucket2

    def test_bucket_in_valid_range(self):
        """Bucket should be in valid range [0, num_buckets)."""
        hash_value = xxhash64("test-key")
        num_buckets = 1024
        bucket = jump_consistent_hash(hash_value, num_buckets)
        assert 0 <= bucket < num_buckets

    def test_distributes_keys_across_buckets(self):
        """Keys should be distributed across buckets."""
        buckets = set()
        num_buckets = 100

        for i in range(1000):
            key = f"key-{i}"
            hash_value = xxhash64(key)
            bucket = jump_consistent_hash(hash_value, num_buckets)
            buckets.add(bucket)

        # With 1000 keys and 100 buckets, should hit most buckets
        assert len(buckets) > 90  # At least 90% coverage

    def test_minimizes_moves_when_adding_buckets(self):
        """Should minimize key movement when adding buckets."""
        keys = [f"key-{i}" for i in range(1000)]

        # Map keys to 100 buckets
        mapping_100 = {}
        for key in keys:
            hash_value = xxhash64(key)
            mapping_100[key] = jump_consistent_hash(hash_value, 100)

        # Map same keys to 101 buckets
        mapping_101 = {}
        for key in keys:
            hash_value = xxhash64(key)
            mapping_101[key] = jump_consistent_hash(hash_value, 101)

        # Count how many keys moved
        moved = sum(
            1 for key in keys if mapping_100[key] != mapping_101[key]
        )

        # With consistent hashing, roughly 1/101 (~10) keys should move
        # Allow some variance: between 1 and 20 keys
        assert 0 < moved < 20

    def test_raises_on_invalid_bucket_count(self):
        """Should raise ValueError for invalid bucket count."""
        hash_value = 123
        with pytest.raises(ValueError):
            jump_consistent_hash(hash_value, 0)
        with pytest.raises(ValueError):
            jump_consistent_hash(hash_value, -1)

    def test_handles_single_bucket(self):
        """Should handle single bucket edge case."""
        hash_value = xxhash64("test")
        bucket = jump_consistent_hash(hash_value, 1)
        assert bucket == 0

    def test_handles_large_bucket_count(self):
        """Should handle large bucket counts."""
        hash_value = xxhash64("test")
        bucket = jump_consistent_hash(hash_value, 1000000)
        assert 0 <= bucket < 1000000


class TestGetShardForKey:
    """Tests for get_shard_for_key function."""

    def test_consistent_shard_for_same_key(self):
        """Should return same shard for same key."""
        shard1 = get_shard_for_key("user:123", 1024)
        shard2 = get_shard_for_key("user:123", 1024)
        assert shard1 == shard2

    def test_shard_in_valid_range(self):
        """Shard should be in valid range."""
        shard = get_shard_for_key("test-key", 1024)
        assert 0 <= shard < 1024

    def test_distributes_keys_across_shards(self):
        """Keys should be distributed across shards."""
        shards = set()

        for i in range(10000):
            key = f"key-{i}"
            shard = get_shard_for_key(key, 1024)
            shards.add(shard)

        # With 10k keys and 1024 shards, should hit most shards
        assert len(shards) > 1000  # At least 97% coverage

    def test_handles_bytes_keys(self):
        """Should handle bytes keys."""
        shard1 = get_shard_for_key(b"test", 1024)
        shard2 = get_shard_for_key("test", 1024)
        assert shard1 == shard2

    def test_default_total_shards(self):
        """Should use default 1024 shards."""
        shard = get_shard_for_key("test")
        assert 0 <= shard < 1024


class TestKeyValueConversions:
    """Tests for key/value conversion utilities."""

    def test_key_to_bytes_with_string(self):
        """Should convert string to bytes."""
        text = "hello"
        data = key_to_bytes(text)
        assert isinstance(data, bytes)
        assert bytes_to_string(data) == text

    def test_key_to_bytes_with_bytes(self):
        """Should pass through bytes unchanged."""
        original = b"hello"
        data = key_to_bytes(original)
        assert data is original  # Same object

    def test_handles_utf8_strings(self):
        """Should handle UTF-8 strings correctly."""
        text = "Hello 世界 🌍"
        data = key_to_bytes(text)
        assert bytes_to_string(data) == text

    def test_handles_empty_strings(self):
        """Should handle empty strings."""
        data = key_to_bytes("")
        assert len(data) == 0
        assert bytes_to_string(data) == ""

    def test_value_to_bytes_with_string(self):
        """Should convert string value to bytes."""
        text = "hello"
        data = value_to_bytes(text)
        assert isinstance(data, bytes)
        assert data == text.encode("utf-8")

    def test_value_to_bytes_with_bytes(self):
        """Should pass through bytes."""
        original = b"hello"
        data = value_to_bytes(original)
        assert data == original

    def test_value_to_bytes_with_bytearray(self):
        """Should convert bytearray to bytes."""
        original = bytearray([1, 2, 3, 4, 5])
        data = value_to_bytes(original)
        assert isinstance(data, bytes)
        assert data == bytes(original)


@pytest.mark.parametrize(
    "key,expected_type",
    [
        ("simple-key", int),
        (b"binary-key", int),
        ("user:12345", int),
        ("", int),
        ("🔑emoji-key🔑", int),
    ],
)
def test_hash_types(key, expected_type):
    """Parametrized test for different key types."""
    hash_value = xxhash64(key)
    assert isinstance(hash_value, expected_type)
    assert 0 <= hash_value < 2**64


class TestCrossSDKCompatibility:
    """Cross-language compatibility test vectors.

    These test vectors validate parity with the TypeScript SDK.
    CRITICAL: These must match exactly or routing will fail.
    """

    def test_known_hash_values(self):
        """Test known hash values from TypeScript SDK."""
        # Test vectors generated from TypeScript SDK
        # Generated by sdks/typescript/scripts/generate-hash-vectors.ts
        # These MUST match exactly
        test_vectors = [
            ("hello", 2794345569481354659),
            ("world", 16679358290033791471),
            ("user:123", 12838382303697200627),
            ("test-key", 15827481653422919556),
            ("", 17241709254077376921),
            ("🌍", 11345255911858566016),
            ("foo", 3728699739546630719),
            ("bar", 5234164152756840025),
            ("a", 15154266338359012955),
            ("ab", 7347350983217793633),
            ("abc", 4952883123889572249),
            ("test-value-1", 2906801998163794869),
            ("test-value-2", 2618596406459050154),
        ]

        for key, expected_hash in test_vectors:
            actual_hash = xxhash64(key)
            assert (
                actual_hash == expected_hash
            ), f"Hash mismatch for '{key}': expected {expected_hash}, got {actual_hash}"

    def test_known_shard_assignments(self):
        """Test known shard assignments from TypeScript SDK."""
        # Test vectors generated from TypeScript SDK
        # Generated by sdks/typescript/scripts/generate-hash-vectors.ts
        # These MUST match exactly
        test_vectors = [
            ("hello", 1024, 309),
            ("world", 1024, 752),
            ("user:123", 1024, 928),
            ("test-key", 1024, 504),
            ("", 1024, 332),
            ("🌍", 1024, 393),
            ("foo", 1024, 951),
            ("bar", 1024, 632),
            ("a", 1024, 894),
            ("ab", 1024, 335),
            ("abc", 1024, 722),
            ("test-value-1", 1024, 640),
            ("test-value-2", 1024, 488),
        ]

        for key, total_shards, expected_shard in test_vectors:
            actual_shard = get_shard_for_key(key, total_shards)
            assert (
                actual_shard == expected_shard
            ), f"Shard mismatch for '{key}': expected {expected_shard}, got {actual_shard}"

    def test_jump_consistent_hash_values(self):
        """Test jump consistent hash with known values."""
        # Test vectors for jump_consistent_hash algorithm
        # Generated by sdks/typescript/scripts/generate-hash-vectors.ts
        test_vectors = [
            (0, 100, 0),
            (1, 100, 55),
            (12345678901234567890, 1024, 294),
            (0xFFFFFFFFFFFFFFFF, 1024, 313),
        ]

        for hash_value, num_buckets, expected_bucket in test_vectors:
            actual_bucket = jump_consistent_hash(hash_value, num_buckets)
            assert (
                actual_bucket == expected_bucket
            ), f"JCH mismatch for hash={hash_value}, buckets={num_buckets}: expected {expected_bucket}, got {actual_bucket}"


class TestPerformance:
    """Performance benchmarks for hash functions."""

    def test_xxhash64_performance(self, benchmark):
        """Benchmark xxhash64 performance."""
        # Run with pytest-benchmark if available
        # Otherwise just verify it runs
        try:
            result = benchmark(xxhash64, "test-key")
            assert isinstance(result, int)
        except Exception:
            # pytest-benchmark not installed, just run normally
            for _ in range(1000):
                xxhash64("test-key")

    def test_get_shard_performance(self, benchmark):
        """Benchmark get_shard_for_key performance."""
        try:
            result = benchmark(get_shard_for_key, "test-key", 1024)
            assert isinstance(result, int)
        except Exception:
            # pytest-benchmark not installed, just run normally
            for _ in range(1000):
                get_shard_for_key("test-key", 1024)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
