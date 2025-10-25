---
layout: default
title: Recipes
nav_order: 6
has_children: true
---

# Recipes

Practical examples and common patterns for using nori-wal.
{: .fs-6 .fw-300 }

This section provides complete, working examples for common use cases. Each recipe is a fully functional implementation you can adapt for your needs.

## Available Recipes

### [Building a Key-Value Store](key-value-store)
Complete implementation of an in-memory key-value store with WAL durability.

**What you'll learn:**
- Using WAL for durability
- Rebuilding state from WAL on recovery
- Handling PUT and DELETE operations
- Implementing snapshots

### [Event Sourcing](event-sourcing)
Event-sourced system with command handling and event replay.

**What you'll learn:**
- Appending events to WAL
- Event replay and state reconstruction
- Handling event versioning
- Snapshotting for performance

### [Message Queue](message-queue)
Simple message queue with consumer position tracking.

**What you'll learn:**
- Publishing messages to WAL
- Consumer offset management
- Multiple consumers
- Retention and cleanup

### [Replication](replication)
Replicating WAL to followers for high availability.

**What you'll learn:**
- Streaming WAL to replicas
- Handling network failures
- Catchup after disconnection
- Consistency guarantees

### [Custom Serialization](custom-serialization)
Using different serialization formats with WAL.

**What you'll learn:**
- Protobuf with WAL
- JSON with compression
- MessagePack for efficiency
- Schema evolution

### [Performance Tuning](performance-tuning)
Optimizing WAL for your workload.

**What you'll learn:**
- Choosing segment size
- Batching strategies
- Compression trade-offs
- Monitoring and profiling

## Using These Recipes

Each recipe follows this structure:

1. **Problem** - What we're trying to solve
2. **Solution** - Complete working code
3. **How it works** - Step-by-step explanation
4. **Testing** - How to verify it works
5. **Production considerations** - What to watch out for

You can copy and adapt the code directly into your projects.

## Code Examples

All examples use:
- Rust 2021 edition
- `tokio` for async runtime
- `anyhow` for error handling (you can use any error handling)
- Latest stable Rust

### Running the Examples

```bash
# Clone the examples
git clone https://github.com/j-haj/nori
cd nori/examples

# Run a recipe
cargo run --example key-value-store
cargo run --example event-sourcing
cargo run --example message-queue
```

Or copy the code from the recipe pages and integrate into your project.

## Contributing Recipes

Have a useful pattern? Contribute it!

1. Fork the repository
2. Add your recipe to `docs/recipes/`
3. Follow the recipe template
4. Submit a pull request

See [Contributing Guide](https://github.com/j-haj/nori/blob/main/CONTRIBUTING.md) for details.
