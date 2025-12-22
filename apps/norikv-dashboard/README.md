# NoriKV Dashboard

Real-time observability dashboard for NoriKV clusters. Built with Next.js 14, React 18, and Tailwind CSS.

## Features

- **Live Event Streaming** - Real-time visualization of cluster events via WebSocket
- **Storage View** - LSM tree heatmap, compaction activity, write pressure monitoring
- **Consensus View** - Raft shard leadership, election timeline, term tracking
- **Network View** - SWIM membership topology, node health status
- **Shards View** - Shard placement, replica distribution
- **Events View** - Filterable event stream with pause/resume
- **Dark/Light Themes** - Toggle between dark and light modes

## Quick Start

### Development (Mock Mode)

Run the dashboard with simulated events (no backend required):

```bash
cd apps/norikv-dashboard
npm install
npm run dev
```

Open [http://localhost:3001](http://localhost:3001) in your browser.

Mock mode generates realistic events including:
- SWIM node membership (Alive/Suspect)
- Raft leadership elections
- LSM slot heat updates
- Compaction progress
- Write pressure changes

### Production (with vizd)

Connect to a running vizd instance:

```bash
# Start vizd first
cd apps/norikv-vizd
cargo run

# In another terminal, start dashboard
cd apps/norikv-dashboard
NEXT_PUBLIC_USE_MOCK=false npm run dev
```

## Configuration

Environment variables (`.env.local`):

| Variable | Default | Description |
|----------|---------|-------------|
| `NEXT_PUBLIC_VIZD_WS_URL` | `ws://localhost:9090/ws/events` | WebSocket URL for vizd |
| `NEXT_PUBLIC_USE_MOCK` | `true` | Use mock events (set `false` for real vizd) |

## Pages

| Route | Description |
|-------|-------------|
| `/` | Cluster overview with health stats, node grid, shard leaders |
| `/storage` | LSM heatmap, compaction conveyor, write pressure gauge |
| `/consensus` | Shard leadership grid, election activity timeline |
| `/network` | Cluster topology ring, SWIM membership events |
| `/shards` | Shard placement view, replica distribution |
| `/events` | Real-time event stream with type filtering |

## Architecture

```
src/
├── app/                    # Next.js App Router pages
│   ├── page.tsx           # Overview
│   ├── storage/           # Storage view
│   ├── consensus/         # Consensus view
│   ├── network/           # Network view
│   ├── shards/            # Shards view
│   └── events/            # Events view
├── components/
│   ├── shell/             # Layout (Sidebar, Header)
│   ├── storage/           # LsmHeatmap, WritePressureGauge, CompactionPanel
│   ├── consensus/         # ShardLeadershipGrid, ElectionTimeline
│   └── network/           # ClusterTopology, NodeDetailCard, SwimTimeline
├── stores/
│   └── eventStore.ts      # Zustand state management
├── lib/
│   ├── wsClient.ts        # WebSocket client with reconnection
│   ├── mockEvents.ts      # Mock event generator
│   └── utils.ts           # Formatting utilities
└── types/
    └── events.ts          # TypeScript event types (matches vizd)
```

## Event Types

The dashboard visualizes these event types from `nori-observe`:

| Type | Description |
|------|-------------|
| `Wal` | Write-ahead log events (segment roll, fsync, GC) |
| `Compaction` | Level compaction (scheduled, progress, finish) |
| `Lsm` | LSM events (flush, L0 stall, write pressure) |
| `Raft` | Consensus events (leader elected, vote, step down) |
| `Swim` | Membership events (alive, suspect, confirm, leave) |
| `Shard` | Shard lifecycle (plan, snapshot, cutover) |
| `Cache` | Cache hit ratio updates |
| `SlotHeat` | LSM slot temperature tracking |

## Development

```bash
# Type check
npm run type-check

# Lint
npm run lint

# Build for production
npm run build

# Start production server
npm run start
```

## Tech Stack

- **Framework**: Next.js 14 (App Router)
- **UI**: React 18, Tailwind CSS, Radix UI primitives
- **State**: Zustand with Immer middleware
- **Animation**: Framer Motion
- **Charts**: Recharts, D3
- **Protocol**: WebSocket with MessagePack/JSON encoding
