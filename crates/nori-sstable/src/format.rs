//! SSTable file format constants and layout specification.
//!
//! # File Layout
//!
//! ## Version 1 (legacy): Per-file Bloom Filter
//! ```text
//! [Data Blocks] [Index] [Bloom Filter] [Footer]
//! ```
//!
//! ## Version 2: Per-block Quotient Filters
//! ```text
//! [Data Blocks with inline QF] [Index] [Footer]
//! ```
//! Each data block includes its Quotient Filter at the end.
//!
//! # Footer (last 64 bytes)
//!
//! | Offset | Size | Field |
//! |--------|------|-------|
//! | 0-7    | 8    | index_offset: u64 |
//! | 8-15   | 8    | index_size: u64 |
//! | 16-23  | 8    | bloom_offset: u64 (v1) or reserved (v2) |
//! | 24-31  | 8    | bloom_size: u64 (v1) or reserved (v2) |
//! | 32     | 1    | compression: u8 |
//! | 33-36  | 4    | block_size: u32 |
//! | 37-44  | 8    | entry_count: u64 |
//! | 45     | 1    | format_version: u8 (0/1=bloom, 2=per-block QF) |
//! | 46     | 1    | qf_remainder_bits: u8 (for v2) |
//! | 47-51  | 5    | reserved |
//! | 52-59  | 8    | magic: u64 "NORISSTL" |
//! | 60-63  | 4    | crc32c: u32 |

/// Default block size (4 KB).
pub const DEFAULT_BLOCK_SIZE: u32 = 4096;

/// Restart interval for prefix compression within blocks.
pub const DEFAULT_RESTART_INTERVAL: usize = 16;

/// SSTable magic number "NORISSTL" in little-endian.
pub const SSTABLE_MAGIC: u64 = 0x4C54535349524F4E;

/// Size of the footer in bytes.
pub const FOOTER_SIZE: usize = 64;

/// Bloom filter bits per key.
pub const BLOOM_BITS_PER_KEY: usize = 10;

/// Target false positive rate for bloom filter (~0.9%).
pub const BLOOM_FP_RATE: f64 = 0.009;

/// SSTable format version for per-file Bloom filter.
pub const FORMAT_VERSION_BLOOM: u8 = 1;

/// SSTable format version for per-block Quotient Filters.
pub const FORMAT_VERSION_QUOTIENT: u8 = 2;

/// Default Quotient Filter remainder bits for ~0.78% FPR.
pub const DEFAULT_QF_REMAINDER_BITS: u8 = 7;

/// Compression type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Compression {
    None = 0,
    Lz4 = 1,
    Zstd = 2,
}

impl Compression {
    /// Converts a u8 to a Compression type.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Compression::None),
            1 => Some(Compression::Lz4),
            2 => Some(Compression::Zstd),
            _ => None,
        }
    }

    /// Converts Compression to u8.
    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

/// SSTable footer containing metadata and offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footer {
    /// Offset of index blocks in file.
    pub index_offset: u64,
    /// Size of index blocks in bytes.
    pub index_size: u64,
    /// Offset of bloom filter in file (v1 only, 0 for v2).
    pub bloom_offset: u64,
    /// Size of bloom filter in bytes (v1 only, 0 for v2).
    pub bloom_size: u64,
    /// Compression type used for data blocks.
    pub compression: Compression,
    /// Block size in bytes.
    pub block_size: u32,
    /// Total number of entries in SSTable.
    pub entry_count: u64,
    /// Format version: 0/1 = per-file Bloom, 2 = per-block Quotient Filter.
    pub format_version: u8,
    /// Quotient Filter remainder bits (for v2 format).
    pub qf_remainder_bits: u8,
}

impl Footer {
    /// Creates a new Footer with default Bloom filter format (v1).
    pub fn new_bloom(
        index_offset: u64,
        index_size: u64,
        bloom_offset: u64,
        bloom_size: u64,
        compression: Compression,
        block_size: u32,
        entry_count: u64,
    ) -> Self {
        Self {
            index_offset,
            index_size,
            bloom_offset,
            bloom_size,
            compression,
            block_size,
            entry_count,
            format_version: FORMAT_VERSION_BLOOM,
            qf_remainder_bits: 0,
        }
    }

    /// Creates a new Footer with per-block Quotient Filter format (v2).
    pub fn new_quotient(
        index_offset: u64,
        index_size: u64,
        compression: Compression,
        block_size: u32,
        entry_count: u64,
        qf_remainder_bits: u8,
    ) -> Self {
        Self {
            index_offset,
            index_size,
            bloom_offset: 0,
            bloom_size: 0,
            compression,
            block_size,
            entry_count,
            format_version: FORMAT_VERSION_QUOTIENT,
            qf_remainder_bits,
        }
    }

    /// Returns true if this SSTable uses per-file Bloom filter (v1 format).
    pub fn uses_bloom_filter(&self) -> bool {
        self.format_version <= FORMAT_VERSION_BLOOM
    }

    /// Returns true if this SSTable uses per-block Quotient Filters (v2 format).
    pub fn uses_quotient_filter(&self) -> bool {
        self.format_version == FORMAT_VERSION_QUOTIENT
    }
}

impl Footer {
    /// Encodes footer to 64-byte array.
    pub fn encode(&self) -> [u8; FOOTER_SIZE] {
        let mut buf = [0u8; FOOTER_SIZE];

        buf[0..8].copy_from_slice(&self.index_offset.to_le_bytes());
        buf[8..16].copy_from_slice(&self.index_size.to_le_bytes());
        buf[16..24].copy_from_slice(&self.bloom_offset.to_le_bytes());
        buf[24..32].copy_from_slice(&self.bloom_size.to_le_bytes());
        buf[32] = self.compression.to_u8();
        buf[33..37].copy_from_slice(&self.block_size.to_le_bytes());
        buf[37..45].copy_from_slice(&self.entry_count.to_le_bytes());
        buf[45] = self.format_version;
        buf[46] = self.qf_remainder_bits;
        // 47..52: reserved (5 bytes)
        buf[52..60].copy_from_slice(&SSTABLE_MAGIC.to_le_bytes());

        // CRC32C of first 60 bytes
        let crc = crc32c::crc32c(&buf[0..60]);
        buf[60..64].copy_from_slice(&crc.to_le_bytes());

        buf
    }

    /// Decodes footer from 64-byte array.
    pub fn decode(buf: &[u8; FOOTER_SIZE]) -> crate::error::Result<Self> {
        use crate::error::SSTableError;

        // Verify CRC
        let expected_crc = u32::from_le_bytes([buf[60], buf[61], buf[62], buf[63]]);
        let actual_crc = crc32c::crc32c(&buf[0..60]);
        if expected_crc != actual_crc {
            return Err(SSTableError::CrcMismatch {
                expected: expected_crc,
                actual: actual_crc,
            });
        }

        // Verify magic
        let magic = u64::from_le_bytes([
            buf[52], buf[53], buf[54], buf[55], buf[56], buf[57], buf[58], buf[59],
        ]);
        if magic != SSTABLE_MAGIC {
            return Err(SSTableError::InvalidFormat(format!(
                "invalid magic number: {:#x}",
                magic
            )));
        }

        let index_offset = u64::from_le_bytes([
            buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
        ]);
        let index_size = u64::from_le_bytes([
            buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
        ]);
        let bloom_offset = u64::from_le_bytes([
            buf[16], buf[17], buf[18], buf[19], buf[20], buf[21], buf[22], buf[23],
        ]);
        let bloom_size = u64::from_le_bytes([
            buf[24], buf[25], buf[26], buf[27], buf[28], buf[29], buf[30], buf[31],
        ]);
        let compression = Compression::from_u8(buf[32])
            .ok_or_else(|| SSTableError::InvalidCompression(buf[32]))?;
        let block_size = u32::from_le_bytes([buf[33], buf[34], buf[35], buf[36]]);
        let entry_count = u64::from_le_bytes([
            buf[37], buf[38], buf[39], buf[40], buf[41], buf[42], buf[43], buf[44],
        ]);
        let format_version = buf[45];
        let qf_remainder_bits = buf[46];

        Ok(Footer {
            index_offset,
            index_size,
            bloom_offset,
            bloom_size,
            compression,
            block_size,
            entry_count,
            format_version,
            qf_remainder_bits,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SSTableError;

    #[test]
    fn test_footer_roundtrip_bloom() {
        let footer = Footer::new_bloom(1024, 512, 1536, 256, Compression::Lz4, 4096, 1000);

        let encoded = footer.encode();
        let decoded = Footer::decode(&encoded).unwrap();

        assert_eq!(footer, decoded);
        assert!(decoded.uses_bloom_filter());
        assert!(!decoded.uses_quotient_filter());
    }

    #[test]
    fn test_footer_roundtrip_quotient() {
        let footer = Footer::new_quotient(1024, 512, Compression::Lz4, 4096, 1000, 7);

        let encoded = footer.encode();
        let decoded = Footer::decode(&encoded).unwrap();

        assert_eq!(footer, decoded);
        assert!(!decoded.uses_bloom_filter());
        assert!(decoded.uses_quotient_filter());
        assert_eq!(decoded.qf_remainder_bits, 7);
    }

    #[test]
    fn test_footer_crc_validation() {
        let footer = Footer::new_bloom(1024, 512, 1536, 256, Compression::Zstd, 4096, 1000);

        let mut encoded = footer.encode();

        // Corrupt a byte
        encoded[0] ^= 0xFF;

        let result = Footer::decode(&encoded);
        assert!(matches!(result, Err(SSTableError::CrcMismatch { .. })));
    }

    #[test]
    fn test_footer_magic_validation() {
        let footer = Footer::new_bloom(1024, 512, 1536, 256, Compression::None, 4096, 1000);

        let mut encoded = footer.encode();

        // Corrupt magic
        encoded[52] = 0xFF;

        // Fix CRC to pass CRC check
        let crc = crc32c::crc32c(&encoded[0..60]);
        encoded[60..64].copy_from_slice(&crc.to_le_bytes());

        let result = Footer::decode(&encoded);
        assert!(matches!(result, Err(SSTableError::InvalidFormat(_))));
    }

    #[test]
    fn test_compression_conversion() {
        assert_eq!(Compression::from_u8(0), Some(Compression::None));
        assert_eq!(Compression::from_u8(1), Some(Compression::Lz4));
        assert_eq!(Compression::from_u8(2), Some(Compression::Zstd));
        assert_eq!(Compression::from_u8(3), None);

        assert_eq!(Compression::None.to_u8(), 0);
        assert_eq!(Compression::Lz4.to_u8(), 1);
        assert_eq!(Compression::Zstd.to_u8(), 2);
    }

    #[test]
    fn test_legacy_footer_compatibility() {
        // A footer with format_version=0 (legacy) should be treated as Bloom
        let footer = Footer {
            index_offset: 1024,
            index_size: 512,
            bloom_offset: 1536,
            bloom_size: 256,
            compression: Compression::None,
            block_size: 4096,
            entry_count: 1000,
            format_version: 0, // Legacy
            qf_remainder_bits: 0,
        };

        assert!(footer.uses_bloom_filter());
        assert!(!footer.uses_quotient_filter());
    }
}
