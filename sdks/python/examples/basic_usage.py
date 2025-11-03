"""Basic usage example for NoriKV Python SDK."""

import asyncio

from norikv import ClientConfig, NoriKVClient, PutOptions


async def main() -> None:
    """Demonstrate basic NoriKV operations."""
    # Create client configuration
    config = ClientConfig(
        nodes=["localhost:50051"],  # List of node addresses
        total_shards=1024,  # Default: 1024 virtual shards
        timeout=5000,  # Request timeout in milliseconds
    )

    # Use async context manager for automatic connection management
    async with NoriKVClient(config) as client:
        print("Connected to NoriKV cluster")

        # Put a value
        print("\n1. Putting key-value pair...")
        version = await client.put("user:123", "Alice")
        print(f"   Stored with version: term={version.term}, index={version.index}")

        # Get a value
        print("\n2. Getting value...")
        result = await client.get("user:123")
        if result.value:
            print(f"   Retrieved: {result.value.decode('utf-8')}")
            print(f"   Version: term={result.version.term}, index={result.version.index}")  # type: ignore
        else:
            print("   Key not found")

        # Put with TTL
        print("\n3. Putting with TTL (5 seconds)...")
        version = await client.put(
            "session:456", "temporary-data", PutOptions(ttl_ms=5000)
        )
        print(f"   Stored with TTL: term={version.term}, index={version.index}")

        # Conditional put (only if not exists)
        print("\n4. Conditional put (if_not_exists)...")
        try:
            await client.put(
                "user:123", "Bob", PutOptions(if_not_exists=True)
            )
            print("   Value updated")
        except Exception as e:
            print(f"   Expected error: {e}")

        # Delete a value
        print("\n5. Deleting key...")
        deleted = await client.delete("user:123")
        print(f"   Deleted: {deleted}")

        # Verify deletion
        print("\n6. Verifying deletion...")
        result = await client.get("user:123")
        if result.value is None:
            print("   Key successfully deleted")
        else:
            print("   Key still exists")

        # Calculate shard for a key
        print("\n7. Calculate shard...")
        shard_id = client.get_shard_for_key("user:123")
        print(f"   Key 'user:123' belongs to shard {shard_id}")


if __name__ == "__main__":
    asyncio.run(main())
