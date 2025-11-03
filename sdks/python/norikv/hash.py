"""Hashing utilities for NoriKV client.

CRITICAL: These hash functions must produce identical results to the server's
Rust implementation and the TypeScript SDK. Any deviation will cause routing failures.

- xxhash64: Fast non-cryptographic hash (seed=0)
- jump_consistent_hash: Minimal-disruption consistent hashing

Reference implementations:
- Rust server: uses xxhash crate + jump_consistent_hash
- TypeScript SDK: xxhash-wasm + jumpConsistentHash
"""

from typing import Union
import xxhash


def xxhash64(key: Union[bytes, str]) -> int:
    """Compute xxhash64 of a key with seed=0.

    Args:
        key: The key to hash (bytes or string)

    Returns:
        64-bit hash as unsigned integer

    Example:
        >>> hash_value = xxhash64('my-key')
        >>> hash_value = xxhash64(b'my-key')
    """
    key_bytes = key.encode("utf-8") if isinstance(key, str) else key

    # Use seed=0 to match server implementation
    # xxhash.xxh64 returns a hash object, we get the integer value
    hasher = xxhash.xxh64(key_bytes, seed=0)
    return hasher.intdigest()


def jump_consistent_hash(hash_value: int, num_buckets: int) -> int:
    """Jump Consistent Hash - minimal-disruption consistent hashing.

    Maps a 64-bit hash to a bucket in [0, num_buckets).
    When num_buckets changes, only ~(1/num_buckets) keys move.

    Reference: https://arxiv.org/abs/1406.2294

    This implementation MUST match the server's exactly:
    ```rust
    pub fn jump_consistent_hash(key: u64, num_buckets: u32) -> u32 {
        let mut b = -1i64;
        let mut j = 0i64;
        while j < num_buckets as i64 {
            b = j;
            let key = key.wrapping_mul(2862933555777941757u64).wrapping_add(1);
            j = ((b + 1) as f64 * (f64::from(1u32 << 31) / ((key >> 33) + 1) as f64)) as i64;
        }
        b as u32
    }
    ```

    Args:
        hash_value: 64-bit hash value
        num_buckets: Number of buckets (must be > 0)

    Returns:
        Bucket index in [0, num_buckets)

    Raises:
        ValueError: If num_buckets <= 0

    Example:
        >>> hash_val = 12345678901234567890
        >>> shard = jump_consistent_hash(hash_val, 1024)
    """
    if num_buckets <= 0:
        raise ValueError(f"num_buckets must be > 0, got {num_buckets}")

    # Ensure we're working with unsigned 64-bit integer
    key = hash_value & 0xFFFFFFFFFFFFFFFF
    b = -1
    j = 0

    while j < num_buckets:
        b = j

        # key = key * 2862933555777941757 + 1 (wrapping)
        key = (key * 2862933555777941757 + 1) & 0xFFFFFFFFFFFFFFFF

        # j = (b + 1) * (2^31 / ((key >> 33) + 1))
        divisor = (key >> 33) + 1
        numerator = (b + 1) * 2147483648  # 2^31
        j = int(numerator / divisor)

    return b


def get_shard_for_key(
    key: Union[bytes, str], total_shards: int = 1024
) -> int:
    """Compute the shard for a given key.

    This combines xxhash64 and jump_consistent_hash to route a key to a shard.

    Args:
        key: The key to route (bytes or string)
        total_shards: Total number of shards (default: 1024)

    Returns:
        Shard ID in [0, total_shards)

    Example:
        >>> shard_id = get_shard_for_key('user:12345', 1024)
        >>> shard_id = get_shard_for_key(b'user:12345', 1024)
    """
    hash_value = xxhash64(key)
    return jump_consistent_hash(hash_value, total_shards)


def key_to_bytes(key: Union[str, bytes]) -> bytes:
    """Convert a key (string or bytes) to bytes.

    Args:
        key: String or bytes

    Returns:
        UTF-8 encoded bytes
    """
    return key.encode("utf-8") if isinstance(key, str) else key


def value_to_bytes(value: Union[str, bytes, bytearray]) -> bytes:
    """Convert a value (string, bytes, or bytearray) to bytes.

    Args:
        value: String, bytes, or bytearray

    Returns:
        UTF-8 encoded bytes if string, otherwise bytes
    """
    if isinstance(value, str):
        return value.encode("utf-8")
    if isinstance(value, bytearray):
        return bytes(value)
    return value


def bytes_to_string(data: bytes) -> str:
    """Convert bytes to string (UTF-8 decoding).

    Args:
        data: Bytes to decode

    Returns:
        UTF-8 decoded string
    """
    return data.decode("utf-8")
