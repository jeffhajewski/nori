# norikv-vizd

Visualization daemon for NoriKV. Aggregates `VizEvent` streams from cluster nodes and serves them to the dashboard via WebSocket.

## Features

- **Event Aggregation** - Coalesces events into 50ms batches (20 FPS)
- **Ring Buffer** - Stores 60 minutes of event history for playback
- **WebSocket Streaming** - Real-time event delivery to dashboard clients
- **Subscription Filters** - Filter by node, event type, or shard
- **JSON/MessagePack** - Supports both encoding formats

## Quick Start

```bash
cd apps/norikv-vizd
cargo run
```

The server starts on `http://localhost:9090` with these endpoints:

| Endpoint | Description |
|----------|-------------|
| `GET /ws/events?json=true` | WebSocket event stream |
| `GET /api/history?from=<ms>&to=<ms>` | Query historical events |
| `GET /api/stats` | Server statistics |
| `GET /health` | Health check |

## WebSocket Protocol

### Server Messages

```typescript
// Connection established
{ "type": "connected", "version": "0.1.0", "oldest_timestamp_ms": 123, "newest_timestamp_ms": 456 }

// Event batch (sent every 50ms)
{ "type": "batch", "data": { "window_start": 123, "window_end": 173, "events": [...], "summary": {...} } }

// Keepalive response
{ "type": "pong", "timestamp_ms": 123 }

// Error
{ "type": "error", "message": "..." }
```

### Client Messages

```typescript
// Filter subscription
{ "type": "filter", "nodes": [1, 2], "types": ["Raft", "Swim"], "shards": [1, 2, 3] }

// Pause/resume stream
{ "type": "pause" }
{ "type": "resume" }

// Request historical data
{ "type": "history", "from_ms": 123, "to_ms": 456 }

// Keepalive ping
{ "type": "ping" }
```

## Event Types

Events are sourced from `nori-observe::VizEvent`:

| Type | Description |
|------|-------------|
| `Wal` | WAL segment operations |
| `Compaction` | Level compaction progress |
| `Lsm` | LSM state changes |
| `Raft` | Consensus events |
| `Swim` | Membership changes |
| `Shard` | Shard lifecycle |
| `Cache` | Cache statistics |
| `SlotHeat` | LSM heat tracking |

## Configuration

Environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `VIZD_PORT` | `9090` | HTTP/WebSocket port |
| `VIZD_BUFFER_MINUTES` | `60` | Ring buffer retention |
| `VIZD_COALESCE_MS` | `50` | Batch window size |

## Architecture

```
src/
├── main.rs          # Axum server setup, routes
├── event.rs         # WireEvent types (serde)
├── aggregator.rs    # Event coalescing (50ms windows)
├── ringbuffer.rs    # 60-minute event storage
├── subscription.rs  # Client filters, message types
└── ws.rs            # WebSocket handler
```

## Integration

vizd receives events via gRPC from NoriKV nodes:

```
┌──────────────┐     gRPC      ┌──────────┐   WebSocket   ┌───────────┐
│  NoriKV Node │ ──────────▶  │   vizd   │ ◀──────────▶  │ Dashboard │
└──────────────┘              └──────────┘                └───────────┘
```

## Development

```bash
# Run with debug logging
RUST_LOG=norikv_vizd=debug cargo run

# Run tests
cargo test -p norikv-vizd
```
