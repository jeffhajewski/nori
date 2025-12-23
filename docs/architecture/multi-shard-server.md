# Multi-Shard Server Architecture

How NoriKV servers manage 1024 virtual shards with lazy initialization and consistent hashing.

---

---

## Overview

NoriKV uses **virtual sharding** to achieve horizontal scalability. Each server can host multiple shards, each shard is a separate Raft group with its own LSM engine.

**Key Concepts:**
- **1024 virtual shards** - Fixed shard count, independent of node count
- **Lazy initialization** - Shards created on first access
- **Jump Consistent Hash** - Deterministic key-to-shard mapping
- **Multi-shard routing** - Requests routed to correct shard
- **Per-shard isolation** - Independent Raft leaders, LSM compaction

---

## Architecture

### Component Layering

```
┌──────────────────────────────────────────────────┐
│  gRPC Server (Tonic)                             │
│  - KvServer::put/get/delete                      │
└────────────────┬─────────────────────────────────┘
                 │
┌────────────────▼─────────────────────────────────┐
│  MultiShardBackend                               │
│  - Routes key → shard_id                         │
│  - Lazy shard creation                           │
└────────────────┬─────────────────────────────────┘
                 │
┌────────────────▼─────────────────────────────────┐
│  ShardManager                                    │
│  - HashMap<ShardId, Arc<ReplicatedLSM>>          │
│  - RwLock for concurrent access                  │
└────────────────┬─────────────────────────────────┘
                 │
        ┌────────┴───────┐
        ▼                ▼
┌───────────────┐  ┌───────────────┐
│  Shard 0      │  │  Shard N      │
│  ReplicatedLSM│  │  ReplicatedLSM│
│  ┌──────────┐ │  │  ┌──────────┐ │
│  │ Raft     │ │  │  │ Raft     │ │
│  │ - Term 5 │ │  │  │ - Term 3 │ │
│  │ - Leader │ │  │  │ - Leader │ │
│  └──────────┘ │  │  └──────────┘ │
│  ┌──────────┐ │  │  ┌──────────┐ │
│  │ LSM      │ │  │  │ LSM      │ │
│  │ - Memtab │ │  │  │ - Memtab │ │
│  │ - L0-L6  │ │  │  │ - L0-L6  │ │
│  └──────────┘ │  │  └──────────┘ │
└───────────────┘  └───────────────┘
```

### Request Flow

**PUT Request:**

```
Client: PUT("user:42", "alice")
  ↓ gRPC
KvService::put(key, value)
  ↓
MultiShardBackend::put(key, value)
  ↓ hash key
Router::shard_for_key("user:42")
  → xxhash64("user:42") = 0x1a2b3c4d5e6f7890
  → jump_consistent_hash(0x1a2b3c4d5e6f7890, 1024) = 42
  ↓
ShardManager::get_or_create_shard(42)
  ↓ if not exists
Create Raft + LSM for shard 42
  ↓
Shard42::put("user:42", "alice")
  ↓ Raft replication
WAL append → replicate to followers → commit
  ↓
LSM apply → memtable
  ↓
Return version to client
```

---

## ShardManager

### Responsibilities

The **ShardManager** is the central component that:
1. **Manages shard lifecycle** - Create, start, shutdown
2. **Lazy initialization** - Shards created on first access
3. **Concurrent access** - Thread-safe via RwLock
4. **Shared config** - Raft and LSM config for all shards

### Code Structure

**Location:** `apps/norikv-server/src/shard_manager.rs`

```rust
pub struct ShardManager {
    /// Server configuration (node_id, total_shards, etc.)
    config: ServerConfig,

    /// Raft configuration (election timeout, heartbeat, etc.)
    raft_config: RaftConfig,

    /// LSM configuration (compaction, bloom, cache)
    lsm_config: ATLLConfig,

    /// Raft transport (in-memory or gRPC)
    transport: Arc<dyn RaftTransport>,

    /// Initial cluster configuration (single-node or multi-node)
    initial_config: ConfigEntry,

    /// Active shards: ShardId → ReplicatedLSM
    shards: Arc<RwLock<HashMap<ShardId, Arc<ReplicatedLSM>>>>,
}
```

### Lazy Shard Creation

**get_or_create_shard() - The core method:**

```rust
pub async fn get_or_create_shard(
    &self,
    shard_id: ShardId,
) -> Result<Arc<ReplicatedLSM>, ShardManagerError> {
    // Fast path: shard already exists
    {
        let shards = self.shards.read();
        if let Some(shard) = shards.get(&shard_id) {
            return Ok(shard.clone());
        }
    }

    // Slow path: create new shard
    let mut shards = self.shards.write();

    // Double-check (another thread may have created it)
    if let Some(shard) = shards.get(&shard_id) {
        return Ok(shard.clone());
    }

    tracing::info!("Creating shard {}", shard_id);

    // Create directories
    let raft_dir = self.config.raft_dir().join(format!("shard-{}", shard_id));
    let lsm_dir = self.config.lsm_dir().join(format!("shard-{}", shard_id));
    std::fs::create_dir_all(&raft_dir)?;
    std::fs::create_dir_all(&lsm_dir)?;

    // Update LSM config with shard-specific directory
    let mut lsm_config = self.lsm_config.clone();
    lsm_config.data_dir = lsm_dir;

    // Create ReplicatedLSM (Raft + LSM)
    let node_id = NodeId::new(&self.config.node_id);
    let shard = Arc::new(
        ReplicatedLSM::new(
            node_id.clone(),
            raft_dir,
            self.raft_config.clone(),
            lsm_config,
            self.transport.clone(),
            self.initial_config.clone(),
        ).await?
    );

    // Store in map
    shards.insert(shard_id, shard.clone());

    tracing::info!("Shard {} created and started", shard_id);

    Ok(shard)
}
```

**Key Design Decisions:**

1. **Double-checked locking pattern**
   - Fast path: Read lock only (99% of requests)
   - Slow path: Write lock for creation (1% of requests)
   - Prevents duplicate creation by concurrent requests

2. **Separate directories per shard**
   - Raft WAL: `/data/raft/shard-0/`, `/data/raft/shard-1/`, ...
   - LSM data: `/data/lsm/shard-0/`, `/data/lsm/shard-1/`, ...
   - Enables shard migration (copy directory)

3. **Shared config, separate state**
   - All shards use same `RaftConfig` (election timeout, etc.)
   - All shards use same `ATLLConfig` (compaction strategy, etc.)
   - But each shard has independent Raft term, LSM levels

---

## MultiShardBackend

### Responsibilities

**MultiShardBackend** implements the **KvBackend** trait and routes requests to the correct shard:

```rust
#[async_trait]
pub trait KvBackend: Send + Sync {
    async fn put(&self, key: &[u8], value: &[u8], options: PutOptions)
        -> Result<Version, KvError>;
    async fn get(&self, key: &[u8], options: GetOptions)
        -> Result<Option<GetResult>, KvError>;
    async fn delete(&self, key: &[u8], options: DeleteOptions)
        -> Result<(), KvError>;
}
```

### Implementation

**Location:** `apps/norikv-server/src/multi_shard_backend.rs`

```rust
pub struct MultiShardBackend {
    shard_manager: Arc<ShardManager>,
    router: Router,
}

impl MultiShardBackend {
    pub fn new(shard_manager: Arc<ShardManager>, total_shards: u32) -> Self {
        Self {
            shard_manager,
            router: Router::new(total_shards),
        }
    }

    async fn get_shard_for_key(&self, key: &[u8]) -> Result<Arc<ReplicatedLSM>, KvError> {
        // Hash key to shard
        let shard_id = self.router.shard_for_key(key);

        // Get or create shard
        self.shard_manager
            .get_or_create_shard(shard_id)
            .await
            .map_err(|e| KvError::Internal(e.to_string()))
    }
}

#[async_trait]
impl KvBackend for MultiShardBackend {
    async fn put(&self, key: &[u8], value: &[u8], options: PutOptions)
        -> Result<Version, KvError>
    {
        let shard = self.get_shard_for_key(key).await?;
        shard.put(key, value).await
            .map_err(|e| KvError::Internal(e.to_string()))
    }

    async fn get(&self, key: &[u8], options: GetOptions)
        -> Result<Option<GetResult>, KvError>
    {
        let shard = self.get_shard_for_key(key).await?;
        match shard.get(key).await {
            Ok(Some(value)) => Ok(Some(GetResult {
                value: value.into(),
                version: 1, // TODO: track versions
            })),
            Ok(None) => Ok(None),
            Err(e) => Err(KvError::Internal(e.to_string())),
        }
    }

    async fn delete(&self, key: &[u8], options: DeleteOptions) -> Result<(), KvError> {
        let shard = self.get_shard_for_key(key).await?;
        shard.delete(key).await
            .map_err(|e| KvError::Internal(e.to_string()))
    }
}
```

**Benefits:**
- **Transparent routing** - KvService doesn't know about shards
- **Lazy creation** - Shards created only when needed
- **Consistent hashing** - Same key always routes to same shard

---

## Routing Algorithm

### Jump Consistent Hash

**norikv-placement/src/router.rs:**

```rust
pub struct Router {
    total_shards: u32,
}

impl Router {
    pub fn shard_for_key(&self, key: &[u8]) -> ShardId {
        // Step 1: Hash key with xxhash64
        let hash = xxhash64(key, seed: 0);

        // Step 2: Map hash to shard with Jump Consistent Hash
        jump_consistent_hash(hash, self.total_shards)
    }
}

/// Jump Consistent Hash - deterministic, minimal-movement hash function.
fn jump_consistent_hash(mut key: u64, num_buckets: u32) -> u32 {
    let mut b: i64 = -1;
    let mut j: i64 = 0;

    while j < num_buckets as i64 {
        b = j;
        key = key.wrapping_mul(2862933555777941757).wrapping_add(1);
        j = ((b + 1) as f64 * (((1i64 << 31) as f64) / (((key >> 33) + 1) as f64))) as i64;
    }

    b as u32
}
```

### Why Jump Consistent Hash?

**vs. Traditional Consistent Hash (ring with virtual nodes):**

| Feature | Jump Hash | Ring Hash |
|---------|-----------|-----------|
| **Deterministic** |  Yes |  Yes |
| **Minimal movement** |  K/N keys move |  K/N keys move |
| **Routing table** |  None (computed) |  Required (O(N)) |
| **Latency** | ~20ns | ~100ns (binary search) |
| **Code complexity** | ~10 lines | ~100 lines |
| **Memory** | 0 bytes | 100KB (1024 vnodes) |

**Trade-offs:**

 **Pros:**
- Zero memory overhead (no routing table)
- Lock-free (pure function)
- Fast (<20ns per lookup)
- Simple implementation

 **Cons:**
- Can't skip buckets (all N buckets must exist)
- Adding/removing buckets affects distribution
- Not suitable for heterogeneous node capacities

**NoriKV's choice:**
- Fixed 1024 virtual shards → perfect for Jump Hash
- Clients compute routing locally → no network RTT
- All SDKs use same algorithm → consistent routing

---

## Shard Lifecycle

### 1. Server Startup

```rust
// Node::new()
let shard_manager = ShardManager::new(
    config,
    raft_config,
    lsm_config,
    transport,
    initial_config,
);

// Node::start()
// Create shard 0 (primary shard for bootstrapping)
let shard_0 = shard_manager.get_or_create_shard(0).await?;
```

**Why create shard 0 on startup?**
- Ensures at least one shard is ready
- Health checks use shard 0
- Faster first request (no creation latency)

---

### 2. First Request to Shard N

```
Client: PUT("product:123", "laptop")
  ↓
Router: shard_for_key("product:123") = 42
  ↓
ShardManager: get_or_create_shard(42)
  ↓ shard 42 doesn't exist
Create directories: /data/raft/shard-42, /data/lsm/shard-42
  ↓
Create ReplicatedLSM(raft_config, lsm_config, transport)
  ↓ Raft initialization
Load WAL, elect leader (single-node: immediate)
  ↓ LSM initialization
Create memtable, load SSTables
  ↓
Insert into shards map
  ↓
Return Arc<ReplicatedLSM>
  ↓
PUT("product:123", "laptop") → Raft → LSM
```

**Latency impact:**
- Shard creation: ~50-100ms (one-time cost)
- Subsequent requests: <1ms (shard already exists)

---

### 3. Steady State

```
Client: GET("product:123")
  ↓
Router: shard_for_key("product:123") = 42
  ↓
ShardManager: get_or_create_shard(42)
  ↓ Fast path (shard exists)
Read lock → HashMap lookup → return Arc<ReplicatedLSM>
  ↓
GET("product:123") → Raft read-index → LSM read
  ↓
Return value
```

**Performance:**
- Routing: ~20ns (hash computation)
- Shard lookup: ~50ns (read lock + HashMap)
- Raft+LSM read: ~5-10ms

---

### 4. Server Shutdown

```rust
// Node::shutdown()
shard_manager.shutdown().await?;

// ShardManager::shutdown()
let shards = self.shards.write();
for (shard_id, shard) in shards.iter() {
    tracing::info!("Shutting down shard {}", shard_id);
    shard.shutdown().await?;
}
```

**Graceful shutdown:**
1. Stop accepting new requests
2. Flush all memtables to disk
3. Close WAL files
4. Stop Raft heartbeats
5. Wait for in-flight requests
6. Exit

---

## Design Decisions

### 1. Fixed 1024 Virtual Shards

**Decision:** Use 1024 shards, regardless of cluster size.

**Rationale:**
- **Rebalancing:** Easy to move shards between nodes (1024 / 3 nodes = ~341 shards/node)
- **Granularity:** Fine-grained load balancing (vs. 8 shards = coarse)
- **Jump Hash:** Works best with fixed bucket count
- **Memory:** 1024 shards × 10MB memtable = 10GB worst case (acceptable)

**Alternative:** Dynamic shard count
-  Complicates Jump Hash (need consistent rehashing)
-  Client routing complexity (need to know current shard count)
-  Rebalancing: All keys rehash

**Trade-off:**
- More shards = more Raft overhead (more leaders, more heartbeats)
- Fewer shards = less granular rebalancing

**1024 is a sweet spot:**
- Enough for 100-node clusters (~10 shards/node)
- Small enough for low overhead (1024 heartbeats/sec ≈ negligible)

---

### 2. Lazy Shard Initialization

**Decision:** Create shards on first access, not at startup.

**Rationale:**
- **Fast startup:** Don't create 1024 shards upfront (1024 × 100ms = 102 seconds!)
- **Memory efficiency:** Only create shards that receive requests
- **Small workloads:** A single-node cluster may only use 10-20 shards

**Example:**
- Single-node server starts: Creates shard 0 only (~100ms)
- First PUT to "user:42": Creates shard 42 (~100ms)
- Subsequent PUTs to "user:*": Reuse existing shards (~0ms creation)

**Alternative:** Create all shards at startup
-  102-second startup time
-  10GB memory (1024 × 10MB memtables)
-  Unnecessary for small workloads

**Trade-off:**
- First request to each shard is slower (~100ms)
- Acceptable for production (amortized over millions of requests)

---

### 3. Shared Config, Separate State

**Decision:** All shards use same RaftConfig and LSMConfig, but separate directories.

**Rationale:**
- **Consistency:** All shards behave identically
- **Simplicity:** One config for all shards
- **Tuning:** Change one config, affects all shards

**Separate directories:**
- Enables shard migration (copy `/data/lsm/shard-42/` to another node)
- Isolation (corruption in shard 0 doesn't affect shard 1)
- Debugging (inspect specific shard's WAL/SSTables)

**Alternative:** Per-shard config
-  Complex: 1024 different configs
-  Hard to tune: Which shard is slow?
-  No clear benefit

---

### 4. RwLock for Shard Map

**Decision:** Use `RwLock<HashMap<ShardId, Arc<ReplicatedLSM>>>` for shard map.

**Rationale:**
- **Read-heavy workload:** 99% of requests are reads (shard already exists)
- **RwLock performance:** Multiple readers, no contention
- **Write-rare:** Shard creation happens once per shard (1024 times max)

**Performance:**
- Read lock: ~10ns (uncontended)
- Write lock: ~50ns (rare)

**Alternative:** DashMap (lock-free concurrent HashMap)
-  Better write performance
-  Larger dependency
-  Overkill for 1024-entry map with rare writes

**Alternative:** Mutex
-  Serializes all reads (slower)

---

### 5. ReplicatedLSM per Shard

**Decision:** Each shard is a separate `ReplicatedLSM` (Raft + LSM).

**Rationale:**
- **Independent leadership:** Shard 0 leader ≠ shard 1 leader (load balancing)
- **Parallel compaction:** Each shard compacts independently
- **Fault isolation:** Failure in shard 0 doesn't affect shard 1

**Alternative:** Shared LSM, separate Raft
-  LSM global lock (serializes all writes)
-  Compaction blocks all shards
-  No clear benefit

**Alternative:** Shared Raft, separate LSM
-  Single Raft leader (bottleneck)
-  Large Raft log (all shards mixed)
-  No clear benefit

---

## Performance Characteristics

### Memory Usage

**Per shard:**
- Memtable: ~4-16MB (configurable)
- Raft log: ~10MB (truncated after snapshots)
- Block cache: ~64MB (shared across shards)

**Total:**
- 1024 shards × 16MB memtable = 16GB
- 1024 shards × 10MB Raft log = 10GB
- Block cache: 64MB (shared)
- **Total: ~26GB** worst case (all shards active)

**Typical production:**
- Active shards: 100-200 (depends on key distribution)
- Memory: 3-5GB (100 shards × 30MB)

---

### CPU Usage

**Raft heartbeats:**
- 1024 shards × 10 heartbeats/sec = 10,240 heartbeats/sec
- Each heartbeat: ~10µs (serialize + send)
- Total CPU: ~100ms/sec = 10% of one core

**LSM compaction:**
- Independent per shard (parallelized)
- Typically 1-2 shards compacting at once
- CPU: ~50% of one core during compaction

**Request handling:**
- Routing: ~20ns (hash)
- Raft replication: ~5ms (network + fsync)
- LSM write: ~100µs (memtable)

---

### Disk I/O

**WAL writes:**
- 1024 shards × 1000 writes/sec = 1M writes/sec
- Batched by Raft (100 entries/batch)
- Actual fsyncs: ~10,000/sec
- Throughput: 10,000 × 4KB = 40MB/sec

**SSTable compaction:**
- Runs in background (per shard)
- Read: ~100MB/sec per shard
- Write: ~100MB/sec per shard
- Parallelized across shards (saturates disk)

---

## Failure Scenarios

### Shard Leader Failure

**Scenario:** Node hosting leader for shard 42 crashes.

**Timeline:**

```
t=0:     Leader crashes
t=500ms: Followers detect (missed heartbeat)
t=1s:    Election starts
t=1.5s:  New leader elected
t=2s:    Client retries, succeeds
```

**Impact:**
- Writes to shard 42: Blocked 1-2 seconds (election time)
- Reads from shard 42: Blocked 1-2 seconds (need new leader)
- Other shards: Unaffected (independent Raft groups)

**Recovery:**
- Automatic (Raft election)
- No data loss (majority quorum)

---

### Disk Full (One Shard)

**Scenario:** Shard 42's LSM fills disk.

**Timeline:**

```
t=0:     Shard 42 compaction fails (disk full)
t=1s:    Shard 42 stops accepting writes
t=5s:    Health check detects shard 42 unhealthy
t=10s:   Operator adds disk space or migrates shard
```

**Impact:**
- Writes to shard 42: Fail with "disk full" error
- Other shards: Unaffected (separate directories)

**Mitigation:**
- Monitor disk space per shard
- Alert before full (>80% usage)
- Migrate shard to another node with space

---

### Hot Shard

**Scenario:** All requests go to shard 42 (poor key distribution).

**Timeline:**

```
t=0:     Shard 42 receives 100K QPS
t=10s:   Memtable flushes frequently (high write rate)
t=30s:   L0 storm (10+ SSTables in L0)
t=60s:   Compaction throttling (slow writes)
t=120s:  Latency degrades (p95 = 100ms)
```

**Impact:**
- Writes to shard 42: Slow (throttled by compaction)
- Reads from shard 42: Slow (many L0 files to check)
- Other shards: Unaffected

**Mitigation:**
- Use better key distribution (add randomness)
- Increase shard 42's memtable size (reduce flushes)
- Add more replicas for shard 42 (spread read load)

---

## Monitoring

### Key Metrics

```promql
# Active shards
count(lsm_memtable_size_bytes)

# Shards per node
count by (node_id) (raft_commit_index)

# Hot shards (high write rate)
topk(10, rate(raft_commit_index[5m]))

# Shards without leader
count(raft_term) - count(raft_leader == 1)
```

### Shard Health Dashboard

**Grafana panel:**

```json
{
  "targets": [
    {
      "expr": "count(lsm_sstable_count{level=\"L0\"} > 10)",
      "legendFormat": "L0 storms"
    },
    {
      "expr": "count(lsm_memtable_size_bytes > 67108864)",
      "legendFormat": "Large memtables (>64MB)"
    },
    {
      "expr": "count(raft_leader == 0)",
      "legendFormat": "Shards without leader"
    }
  ]
}
```

---

## Best Practices

### Key Distribution

**Good:**
```
user:{uuid}     → Uniform distribution
order:{timestamp}:{uuid} → Uniform distribution
```

**Bad:**
```
user:admin      → All requests to one shard
counter         → All requests to one shard (hot key)
```

**Solution:** Add randomness or use composite keys.

---

### Capacity Planning

**Rule of thumb:**
- 1 shard per 1GB of data
- 10 shards per CPU core
- 100 shards per node (max)

**Example: 100GB dataset, 3-node cluster:**
- 100 shards total (1 per GB)
- 33 shards per node
- Replication factor 3 → 100 shards × 3 replicas = 300GB total

---

### Rebalancing

**When to rebalance:**
- Adding nodes (move shards from heavy → light nodes)
- Removing nodes (move shards from departing node)
- Hot shards (move replicas to spread read load)

**How to rebalance:**
1. Identify target shard to move
2. Add new replica on destination node
3. Wait for Raft catch-up (log replication)
4. Promote new replica to voting member (joint consensus)
5. Remove old replica

**Tools:**
```bash
# List shards per node
norikv-admin list-shards --group-by node

# Move shard 42 from node0 to node1
norikv-admin move-shard --shard-id 42 --from node0 --to node1
```

---

## Next Steps

- **[SWIM Topology Tracking](swim-topology.md)** - How cluster membership is tracked
- **[Operations Guide](../operations/index.md)** - Production deployment
