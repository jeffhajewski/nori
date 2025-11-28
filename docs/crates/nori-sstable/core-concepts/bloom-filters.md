# Bloom Filters

How probabilistic data structures prevent unnecessary disk reads in SSTables.

---

## The Problem

```
User: get("user:99999")  // Key doesn't exist

Without bloom filter:
1. Check index → might be in Block 42
2. Read Block 42 from disk (100µs)
3. Decompress block (~1µs)
4. Binary search → not found
Total: ~101µs wasted on disk I/O
```

**With 1000 non-existent keys:** 1000 * 100µs = 100ms wasted!

---

## The Solution: Bloom Filters

**A bloom filter is a probabilistic data structure** that answers: "Is key X **definitely not** in the set?"

```
User: get("user:99999")

With bloom filter:
1. Check bloom filter (67ns): NOT PRESENT → return None
Total: 67ns (1500x faster!)
```

**Key properties:**
- **No false negatives**: If bloom says "not present", key is definitely absent
- **Possible false positives**: If bloom says "maybe present", key might not be there
- **Space-efficient**: 10 bits per key for ~0.9% false positive rate

---

## How Bloom Filters Work

### Basic Concept

A bloom filter is a **bit array** with **hash functions**:

```
Bit array (128 bits):
[0][0][0][0]...[0][0][0]

Add key "alice":
  hash1("alice") = 5  → set bit 5
  hash2("alice") = 89 → set bit 89
  hash3("alice") = 120 → set bit 120

Bit array after adding "alice":
[0][0][0][0][0][1]...[1]...[1][0][0]
      bit5      bit89    bit120

Query "alice":
  hash1("alice") = 5   → check bit 5: SET Yes
  hash2("alice") = 89  → check bit 89: SET Yes
  hash3("alice") = 120 → check bit 120: SET Yes
  Result: MAYBE PRESENT (all bits set)

Query "bob":
  hash1("bob") = 12 → check bit 12: NOT SET No
  Result: DEFINITELY NOT PRESENT (stop here!)
```

---

## Bloom Filter in SSTables

### File Layout

```
SSTable:
┌─────────────┐
│  Blocks     │
├─────────────┤
│  Index      │
├─────────────┤
│  Bloom      │ ← Bloom filter here
│  Filter     │
├─────────────┤
│  Footer     │
└─────────────┘
```

### Read Path with Bloom

```rust
impl SSTableReader {
    pub async fn get(&self, key: &[u8]) -> Result<Option<Entry>> {
        // 1. Check bloom filter (67ns)
        if !self.bloom.contains(key) {
            return Ok(None);  // Definitely not present
        }

        // 2. Might be present, check index
        let block_id = self.index.find_block(key)?;

        // 3. Read block
        let block = self.read_block(block_id).await?;

        // 4. Search in block
        block.get(key)
    }
}
```

---

## False Positive Rate

### What is a False Positive?

```
Key "charlie" NOT in SSTable

Bloom filter checks:
  hash1("charlie") = 5   → bit 5 SET (from "alice")
  hash2("charlie") = 89  → bit 89 SET (from "alice")
  hash3("charlie") = 120 → bit 120 SET (from "alice")

Result: MAYBE PRESENT (false positive!)

Then:
  1. Check index
  2. Read block from disk (wasted!)
  3. Binary search → not found
```

**False positive = wasted disk read.**

---

### Controlling False Positive Rate

**Formula:**

```
FP rate ≈ (1 - e^(-kn/m))^k

where:
  k = number of hash functions
  n = number of keys
  m = number of bits
```

**Simplified:**

```
m/n = bits per key

For k=3 hash functions:
  10 bits/key → ~0.9% FP rate
  12 bits/key → ~0.3% FP rate
  16 bits/key → ~0.05% FP rate
```

**nori-sstable default:**

```rust
pub const DEFAULT_BLOOM_BITS_PER_KEY: usize = 10;
```

**Result:** 0.9% false positive rate

---

### Trade-Off Analysis

| Bits/Key | FP Rate | Bloom Size (100K keys) | Wasted Reads (per 100K queries) |
|----------|---------|------------------------|----------------------------------|
| 5        | ~10%    | 62KB                   | 10,000                           |
| 10       | ~0.9%   | 125KB                  | 900                              |
| 12       | ~0.3%   | 150KB                  | 300                              |
| 16       | ~0.05%  | 200KB                  | 50                               |

**Optimal:** 10 bits/key balances bloom size vs false positives.

---

## Hash Functions

### Why Multiple Hashes?

Single hash function:

```
Only 1 bit set per key → collision likely
  "alice" → bit 42
  "bob"   → bit 42 (collision!)
```

Multiple hash functions:

```
k bits set per key → lower collision probability
  "alice" → bits {5, 89, 120}
  "bob"   → bits {12, 67, 104}
```

**Optimal k:**

```
k = (m/n) * ln(2) ≈ 0.693 * (m/n)

For m/n = 10:
  k = 0.693 * 10 ≈ 7 hash functions
```

---

### Double Hashing Trick

**Problem:** Computing k independent hash functions is expensive.

**Solution:** Use **double hashing** with only 2 hash functions:

```rust
fn bloom_hash(key: &[u8], i: usize, m: usize) -> usize {
    let h1 = xxhash64(key, seed=0);
    let h2 = xxhash64(key, seed=1);
    ((h1 + i * h2) % m) as usize
}
```

**Generate k hashes from 2:**

```
hash_0 = h1
hash_1 = h1 + h2
hash_2 = h1 + 2*h2
...
hash_k = h1 + k*h2
```

**Benefit:** Compute 2 hashes, derive k positions (fast!).

---

### Why xxHash64?

```rust
use xxhash_rust::xxh64;

let hash = xxh64::xxh64(key, seed);
```

**Properties:**
- **Fast:** ~10 GB/s throughput
- **Good distribution:** Low collision rate
- **Deterministic:** Same input → same output
- **Non-cryptographic:** Don't need security, just speed

**Alternatives considered:**
- CRC32: Faster but worse distribution
- SipHash: Cryptographic (overkill, slower)
- MurmurHash3: Similar speed, less Rust support

---

## Building a Bloom Filter

### During SSTable Creation

```rust
impl SSTableBuilder {
    pub async fn finish(self) -> Result<SSTableMetadata> {
        // 1. Write data blocks
        for block in self.blocks {
            write_block(&block).await?;
        }

        // 2. Write index
        write_index(&self.index).await?;

        // 3. Build bloom filter
        let bloom = BloomFilter::new(
            self.entry_count,
            BLOOM_BITS_PER_KEY
        );
        for key in self.all_keys {
            bloom.insert(&key);
        }

        // 4. Write bloom filter
        write_bloom(&bloom).await?;

        // 5. Write footer
        write_footer(...).await?;
    }
}
```

### Memory Usage

```
100,000 keys * 10 bits/key = 1,000,000 bits = 125 KB

Bloom filter in memory during build: ~125KB
Bloom filter in file: ~125KB
Bloom filter loaded on read: ~125KB
```

**Small overhead for huge performance gain.**

---

## Bloom Filter Performance

### Benchmark Results

```
Bloom filter operations on M1 Pro:

Insert: ~20ns per key
Query:  ~67ns per key

For 100K key SSTable:
  Build time: 100K * 20ns = 2ms
  Query time: 67ns per lookup
```

### Savings Calculation

```
Scenario: 100K queries on SSTable with 100K keys
  50% keys present, 50% absent

Without bloom filter:
  50K absent keys * 100µs disk read = 5 seconds wasted

With bloom filter (0.9% FP rate):
  50K absent * 67ns = 3.35ms (bloom checks)
  450 false positives * 100µs = 45ms (wasted disk)
  Total: ~48ms

Speedup: 5000ms / 48ms ≈ 100x faster!
```

---

## Advanced Topics

### Partitioned Bloom Filters

**Idea:** Split bloom into multiple small filters (one per block).

**Pros:**
- Only load bloom for needed blocks
- Better cache locality

**Cons:**
- Higher false positive rate (smaller filters)
- More complex implementation

**Decision:** nori-sstable uses single whole-file bloom for simplicity.

---

### Blocked Bloom Filters

**Idea:** Organize bits into cache-line-sized blocks (64 bytes).

```
Traditional: bits[0..1000000]
Blocked: blocks[0..15625] where each block = 64 bytes
```

**Benefit:** All k hash probes hit same cache line (faster).

**Implementation complexity:** Medium

**Future work:** Consider for v2.

---

### Counting Bloom Filters

**Idea:** Store counts instead of bits (supports deletion).

**Benefit:** Can remove keys from bloom filter.

**Cost:** 4x memory (32-bit counters vs 1-bit flags).

**Use case:** Compaction could update bloom (remove deleted keys).

**Decision:** Not needed (bloom rebuilt during compaction anyway).

---

## Bloom Filter vs Alternatives

### Cuckoo Filters

**Idea:** Hash table with relocation for collisions.

**Pros:**
- Supports deletion
- Better FP rate for same space

**Cons:**
- More complex
- Slightly slower queries

**Decision:** Bloom simpler, fast enough for SSTables.

---

### XOR Filters

**Idea:** Use XOR operations for compact representation.

**Pros:**
- 10-20% smaller than bloom
- Faster construction

**Cons:**
- Immutable (can't add keys incrementally)
- More complex implementation

**Future:** Consider for v2 if bloom filter size becomes bottleneck.

---

### Perfect Hashing

**Idea:** Build minimal perfect hash function.

**Pros:**
- No false positives
- Compact

**Cons:**
- Expensive to build
- Read-only (can't add keys)

**Decision:** Overkill for SSTables (false positives acceptable).

---

## Practical Guidelines

### 1. Use Default Settings

```rust
// Recommended for most workloads
SSTableConfig {
    bloom_bits_per_key: 10,  // ~0.9% FP rate
    ..Default::default()
}
```

### 2. Increase for Large SSTables

```rust
// For >1GB SSTables, reduce FP rate
SSTableConfig {
    bloom_bits_per_key: 12,  // ~0.3% FP rate
    // Cost: +20% bloom size (acceptable for large files)
    ..Default::default()
}
```

### 3. Decrease for Tiny SSTables

```rust
// For <1MB SSTables, bloom overhead not worth it
SSTableConfig {
    bloom_bits_per_key: 0,  // Disable bloom
    // Rationale: Index lookup is already fast
    ..Default::default()
}
```

### 4. Monitor False Positive Rate

```rust
let fp_rate = false_positives / total_queries;

// If FP rate > 2%:
//   - Increase bloom_bits_per_key
//   - Check for hash collisions
//   - Verify bloom filter wasn't corrupted
```

---

## Bloom Filter in Compaction

### Problem: Stale Bloom Filters

```
Original SSTable: {alice, bob, charlie}
After compaction: {alice, bob}  // charlie deleted

Old bloom filter still contains "charlie" hash
  → False positive on "charlie" queries
```

### Solution: Rebuild Bloom

```rust
async fn compact(inputs: Vec<SSTable>) -> SSTable {
    let mut builder = SSTableBuilder::new(config).await?;

    // Merge all entries
    for entry in merge_iterator(inputs) {
        if !entry.tombstone {  // Skip deleted
            builder.add(&entry).await?;
        }
    }

    // Bloom automatically rebuilt with only live keys
    builder.finish().await?
}
```

**Result:** Compacted SSTable has fresh, accurate bloom filter.

---

## Real-World Impact

### Case Study: 1GB SSTable with 10M Keys

```
Configuration:
  Keys: 10M
  Bloom bits/key: 10
  False positive rate: 0.9%

Bloom filter size: 10M * 10 bits = 12.5 MB (1.25% of SSTable size)

Query workload: 1M queries (50% hits, 50% misses)

Without bloom:
  500K misses * 100µs disk = 50 seconds

With bloom:
  500K misses * 67ns bloom = 33.5ms
  4.5K false positives * 100µs = 450ms
  Total: ~483ms

Speedup: 50s / 0.48s ≈ 100x faster!

Cost: 12.5 MB memory (negligible)
```

---

## Summary

**Bloom filters are essential for SSTables:**

 **Fast negative lookups** - 67ns to confirm absence
 **Massive I/O savings** - Skip 99%+ of unnecessary disk reads
 **Space-efficient** - 10 bits per key for 0.9% FP rate
 **Simple** - Single global bloom, no complex logic
 **Battle-tested** - Used in LevelDB, RocksDB, Cassandra

**Key insight:** Trading ~1% false positives for 100x speedup on absent keys is an excellent trade-off.

---

## Next Steps

**Understand the implementation:**
See [How It Works: Bloom Filter](../how-it-works/bloom-filter-impl.md) for implementation details.

**Learn about indexing:**
Read [How It Works: Index Structure](../how-it-works/index-structure.md) for how bloom works with block index.

**Optimize reads:**
Check [Performance: Tuning](../performance/tuning.md) for bloom filter configuration guidance.

**See the full read path:**
Explore [What is an SSTable?](what-is-sstable.md#read-path) for how bloom fits into lookups.
