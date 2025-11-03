# NoriKV Docker Setup

This directory contains Docker configuration for running a local 3-node NoriKV cluster for development and testing.

## Quick Start

From the repository root:

```bash
# Build the Docker image
docker compose build

# Start the 3-node cluster
docker compose up -d

# View logs
docker compose logs -f

# Stop the cluster
docker compose down

# Stop and remove all data
docker compose down -v
```

## Cluster Configuration

The Docker Compose setup creates a 3-node cluster with the following configuration:

| Node | Internal Port | External gRPC Port | External Metrics Port |
|------|--------------|-------------------|---------------------|
| node1 | 7447 | localhost:7447 | localhost:9090 |
| node2 | 7447 | localhost:7448 | localhost:9091 |
| node3 | 7447 | localhost:7449 | localhost:9092 |

**Cluster Settings:**
- Total shards: 1024
- Replication factor: 3
- Each node seeds from the other two nodes via SWIM gossip

## Connecting from SDKs

### Python

```python
from norikv import NoriKVClient, ClientConfig

config = ClientConfig(
    nodes=["localhost:7447", "localhost:7448", "localhost:7449"],
    total_shards=1024,
)

async with NoriKVClient(config) as client:
    await client.put("key", "value")
    result = await client.get("key")
    print(result.value.decode())
```

### TypeScript

```typescript
import { NoriKVClient } from '@norikv/client';

const client = new NoriKVClient({
  nodes: ['localhost:7447', 'localhost:7448', 'localhost:7449'],
  totalShards: 1024,
});

await client.put('key', 'value');
const result = await client.get('key');
console.log(result.value.toString());
```

## Viewing Metrics

Each node exposes Prometheus metrics on its metrics port:

```bash
# Node 1 metrics
curl http://localhost:9090/metrics

# Node 2 metrics
curl http://localhost:9091/metrics

# Node 3 metrics
curl http://localhost:9092/metrics
```

## Data Persistence

Each node stores data in a named Docker volume:
- `node1-data` → `/var/lib/norikv` in node1
- `node2-data` → `/var/lib/norikv` in node2
- `node3-data` → `/var/lib/norikv` in node3

Data persists across container restarts. To completely reset the cluster:

```bash
docker compose down -v
```

## Configuration Files

Each node has a YAML configuration file in `docker/configs/`:
- `node1.yaml` - Configuration for node1
- `node2.yaml` - Configuration for node2
- `node3.yaml` - Configuration for node3

These files are mounted read-only into the containers at `/home/norikv/norikv.yaml`.

## Troubleshooting

### Check cluster health

```bash
# View all container statuses
docker compose ps

# Check logs for a specific node
docker compose logs node1
docker compose logs node2
docker compose logs node3

# Follow logs for all nodes
docker compose logs -f
```

### Restart a single node

```bash
# Restart node2 (simulates failure)
docker compose restart node2

# Stop node3, wait, then start it again
docker compose stop node3
sleep 10
docker compose start node3
```

### Connect to a node's shell

```bash
docker compose exec node1 /bin/sh
```

### Clean rebuild

```bash
# Remove all containers, volumes, and rebuild
docker compose down -v
docker compose build --no-cache
docker compose up -d
```

## Architecture

```
┌─────────────────────────────────────────────────────┐
│  Host Machine                                       │
│                                                     │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────┐ │
│  │ node1        │  │ node2        │  │ node3    │ │
│  │ :7447 (gRPC) │  │ :7448 (gRPC) │  │ :7449    │ │
│  │ :9090 (prom) │  │ :9091 (prom) │  │ :9092    │ │
│  └──────────────┘  └──────────────┘  └──────────┘ │
│         │                  │                │      │
│         └──────────────────┴────────────────┘      │
│                norikv-cluster network               │
│                                                     │
│  ┌──────────────────────────────────────────────┐  │
│  │ Persistent Volumes                           │  │
│  │ - node1-data, node2-data, node3-data         │  │
│  └──────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────┘
```

## Development Workflow

1. **Start cluster for development:**
   ```bash
   docker compose up -d
   ```

2. **Run SDK tests against the cluster:**
   ```bash
   # Python
   cd sdks/python
   .venv/bin/python -m pytest tests/integration

   # TypeScript
   cd sdks/typescript
   npm test
   ```

3. **Make changes to NoriKV server code**

4. **Rebuild and restart:**
   ```bash
   docker compose down
   docker compose build
   docker compose up -d
   ```

5. **View metrics and logs:**
   ```bash
   docker compose logs -f
   curl http://localhost:9090/metrics | grep norikv
   ```

## Advanced: Custom Configuration

To modify cluster parameters:

1. Edit `docker-compose.yml` environment variables:
   - `NORIKV_TOTAL_SHARDS` - Number of virtual shards (default: 1024)
   - `NORIKV_REPLICATION_FACTOR` - Replica count per shard (default: 3)
   - `RUST_LOG` - Logging level (info, debug, trace)

2. Or edit the YAML config files in `docker/configs/`:
   - Modify shard count, replication factor, ports, etc.
   - Restart containers: `docker compose restart`

## Production Considerations

This Docker setup is intended for **local development only**. For production deployments:

- Use separate physical/virtual machines for each node
- Configure proper networking and firewall rules
- Set up persistent storage (not Docker volumes)
- Enable OTLP telemetry for distributed tracing
- Configure authentication and TLS
- Use a proper orchestration platform (Kubernetes, Nomad, etc.)

See the main [NoriKV documentation](../README.md) for production deployment guidance.
