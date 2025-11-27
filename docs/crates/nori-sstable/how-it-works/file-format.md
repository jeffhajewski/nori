# File Format

Complete specification of the `.sst` file format.

---

## File Layout

```
┌─────────────────────────────────────┐  Offset 0
│         Data Blocks                 │
│  ┌──────────────────────────┐      │
│  │ Block 0 (4KB compressed) │      │
│  ├──────────────────────────┤      │
│  │ Block 1 (4KB compressed) │      │
│  ├──────────────────────────┤      │
│  │         ...              │      │
│  ├──────────────────────────┤      │
│  │ Block N (4KB compressed) │      │
│  └──────────────────────────┘      │
├─────────────────────────────────────┤  blocks_end_offset
│         Block Index                 │
│  (first_key, offset, size) × N+1   │
├─────────────────────────────────────┤  index_end_offset
│       Bloom Filter                  │
│  (bit array + metadata)             │
├─────────────────────────────────────┤  bloom_end_offset
│         Footer (64 bytes)           │
│  - Magic: 0xABCD1234               │
│  - Version: 1                       │
│  - Index offset/size                │
│  - Bloom offset/size                │
│  - Compression type                 │
│  - Entry count                      │
│  - CRC32C checksum                  │
└─────────────────────────────────────┘  file_end
```

---

## Footer Format (64 bytes)

```
Offset  Size  Field               Description
------  ----  -----               -----------
0       4     magic               0xABCD1234 (file type identifier)
4       2     version             Format version (currently 1)
6       2     compression         0=None, 1=LZ4, 2=Zstd
8       8     index_offset        Byte offset to index start
16      8     index_size          Size of index in bytes
24      8     bloom_offset        Byte offset to bloom filter
32      8     bloom_size          Size of bloom filter
40      8     entry_count         Total number of entries
48      8     first_key_len       Length of first key
56      4     crc32c              CRC32C of bytes 0-55
60      4     padding             Reserved for future use
```

**Reading footer:**
```rust
let mut file = File::open(path).await?;
file.seek(SeekFrom::End(-64)).await?;
let footer_bytes = read_exact(&mut file, 64).await?;
let footer = Footer::decode(footer_bytes)?;
```

---

## Block Index Format

```
Entry format (variable length):
┌──────────────┬──────────────┬──────────────┬──────────────┐
│ key_len (u32)│ key (bytes)  │ offset (u64) │ size (u32)   │
└──────────────┴──────────────┴──────────────┴──────────────┘

Full index:
[Entry 0][Entry 1]...[Entry N][num_entries: u32]
```

**Example:**
```
Block 0: key="aaa", offset=0, size=1420
Block 1: key="bbb", offset=1420, size=1510
Block 2: key="ccc", offset=2930, size=1380
```

---

## Bloom Filter Format

```
┌──────────────────────────────────┐
│ num_bits (u64)                   │  8 bytes
├──────────────────────────────────┤
│ num_hashes (u32)                 │  4 bytes
├──────────────────────────────────┤
│ bits_per_key (u32)               │  4 bytes
├──────────────────────────────────┤
│ bit_array (variable length)      │  (num_bits + 7) / 8 bytes
└──────────────────────────────────┘
```

**Size calculation:**
```
100,000 keys × 10 bits/key = 1,000,000 bits
Byte size: 1,000,000 / 8 = 125,000 bytes = 125 KB
```

---

## Block Format

```
┌──────────────────────────────────┐
│ Entries (prefix-compressed)      │
│  [Entry 0]                       │
│  [Entry 1]                       │
│  ...                             │
│  [Entry K]                       │
├──────────────────────────────────┤
│ Restart Points Array             │
│  [offset_0: u32]                 │
│  [offset_16: u32]                │
│  ...                             │
│  [offset_N: u32]                 │
├──────────────────────────────────┤
│ num_restarts (u32)               │
└──────────────────────────────────┘
```

**Compression wrapper:**
```
If compressed:
  [compressed_size: u32][compressed_data]
Else:
  [uncompressed_data]
```

---

## Summary

**File structure:**
1. Data blocks (compressed 4KB blocks)
2. Block index (sparse, one entry per block)
3. Bloom filter (bit array for membership)
4. Footer (64-byte metadata with offsets)

**Reading algorithm:**
1. Read footer (last 64 bytes)
2. Validate CRC32C
3. Load bloom filter (at bloom_offset)
4. Load index (at index_offset)
5. Ready for queries!
