---
layout: default
title: Bloom Filter Strategy
parent: Design Decisions
grand_parent: nori-sstable
nav_order: 3
---

# Bloom Filter Strategy
{: .no_toc }

Design decisions for bloom filters in nori-sstable.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Key Decisions

1. **10 bits per key** (0.9% false positive rate)
2. **xxHash64** with double hashing
3. **Whole-file bloom** (not per-block)
4. **Loaded on open** (always in RAM)

---

## Why 10 Bits Per Key?

**False positive rate formula:**
```
FP ≈ 0.6185^(m/n)

m/n = 10 bits/key → FP ≈ 0.9%
m/n = 12 bits/key → FP ≈ 0.3%
```

**Trade-off analysis:**

```
100K key SSTable:

10 bits/key:
  Size: 125KB
  FP rate: 0.9%
  Wasted reads: 900 per 100K misses

12 bits/key:
  Size: 150KB (+25KB = +20%)
  FP rate: 0.3%
  Wasted reads: 300 per 100K misses

Savings: 600 wasted reads
Cost: 25KB more memory
Value: 600 * 100µs = 60ms saved

Verdict: Not worth 20% more memory for 60ms
```

**Decision:** 10 bits/key is the sweet spot.

---

## Why xxHash64?

**Requirements:**
- Fast (billions of keys to hash)
- Good distribution (low collision)
- Non-cryptographic OK (not for security)

**Comparison:**

| Hash | Speed | Quality | Choice |
|------|-------|---------|--------|
| CRC32 | 10 GB/s | Medium | Too simple |
| **xxHash64** | **10 GB/s** | **Excellent** | ✅ **Chosen** |
| SipHash | 3 GB/s | Excellent | Too slow |
| SHA-256 | 0.5 GB/s | Excellent | Overkill |

**xxHash64 wins:** Speed of CRC32, quality of cryptographic hashes.

---

## Double Hashing Trick

**Problem:** Need k=7 hashes per key.

**Naive:** Compute 7 independent hashes ❌ (slow)

**Double hashing:** Generate k hashes from 2:

```rust
h1 = xxhash64(key, seed=0)
h2 = xxhash64(key, seed=1)

hash_i = (h1 + i * h2) % m
```

**Speed:**
- Compute 2 hashes: 2 * 100ns = 200ns
- Generate 7 positions: 7 * 5ns = 35ns
- Total: 235ns (vs 700ns for 7 independent hashes)

**Trade-off:** Slightly higher collision probability (negligible in practice).

---

## Whole-File vs Per-Block Bloom

**Alternatives:**

### 1. Whole-File Bloom (Chosen)
```
One bloom filter for entire SSTable
Size: 10 bits * total_keys
Load: All on open
```

**Pros:**
- Simple implementation
- Lower FP rate (larger filter)
- Always in memory (no lookup needed)

**Cons:**
- Must load entire bloom on open
- Memory proportional to SSTable size

### 2. Per-Block Bloom
```
One bloom filter per 4KB block
Size: 10 bits * keys_per_block * num_blocks
Load: On-demand per block
```

**Pros:**
- Only load needed blooms
- Better locality

**Cons:**
- Higher FP rate (smaller filters)
- More complex implementation
- Need to load bloom before checking

**Decision:** Whole-file for simplicity. Bloom size (125KB per 100K keys) is acceptable memory overhead.

---

## Load on Open vs On-Demand

**Always load on open:**

```rust
impl SSTableReader {
    pub async fn open(path: PathBuf) -> Result<Self> {
        // Load bloom filter into memory
        let bloom = read_bloom_filter(&file).await?;

        Ok(Self { bloom, ... })
    }
}
```

**Benefits:**
- ~67ns check (in-memory)
- No lazy loading complexity
- Predictable memory usage

**Cost:** Initial load time (~1ms per MB of bloom)

**Alternative:** Lazy load bloom on first get() ❌
- More complex
- Unpredictable first-query latency
- Bloom small enough to always load

---

## Summary

**Design choices:**

✅ 10 bits/key (0.9% FP, 125KB per 100K keys)
✅ xxHash64 (10 GB/s, excellent distribution)
✅ Double hashing (2 hashes → 7 positions)
✅ Whole-file bloom (simple, low FP rate)
✅ Loaded on open (always in RAM, ~67ns checks)

**Result:** ~67ns checks prevent 100µs disk reads with <1% false positives.
