//! SSTable file format constants and layout specification.
//!
//! File layout:
//! ```text
//! [Data Blocks] [Index Blocks] [Bloom Filter] [Footer]
//! ```
//!
//! Footer (last 64 bytes):
//! - index_offset: u64
//! - index_size: u64
//! - bloom_offset: u64
//! - bloom_size: u64
//! - compression: u8
//! - block_size: u32
//! - entry_count: u64
//! - reserved: 7 bytes
//! - magic: u64 (0x4E4F52495353544C "NORISSTL")
//! - crc32c: u32

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
    /// Offset of bloom filter in file.
    pub bloom_offset: u64,
    /// Size of bloom filter in bytes.
    pub bloom_size: u64,
    /// Compression type used for data blocks.
    pub compression: Compression,
    /// Block size in bytes.
    pub block_size: u32,
    /// Total number of entries in SSTable.
    pub entry_count: u64,
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
        // 45..52: reserved (7 bytes)
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
            buf[52], buf[53], buf[54], buf[55],
            buf[56], buf[57], buf[58], buf[59],
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

        Ok(Footer {
            index_offset,
            index_size,
            bloom_offset,
            bloom_size,
            compression,
            block_size,
            entry_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SSTableError;

    #[test]
    fn test_footer_roundtrip() {
        let footer = Footer {
            index_offset: 1024,
            index_size: 512,
            bloom_offset: 1536,
            bloom_size: 256,
            compression: Compression::Lz4,
            block_size: 4096,
            entry_count: 1000,
        };

        let encoded = footer.encode();
        let decoded = Footer::decode(&encoded).unwrap();

        assert_eq!(footer, decoded);
    }

    #[test]
    fn test_footer_crc_validation() {
        let footer = Footer {
            index_offset: 1024,
            index_size: 512,
            bloom_offset: 1536,
            bloom_size: 256,
            compression: Compression::Zstd,
            block_size: 4096,
            entry_count: 1000,
        };

        let mut encoded = footer.encode();

        // Corrupt a byte
        encoded[0] ^= 0xFF;

        let result = Footer::decode(&encoded);
        assert!(matches!(result, Err(SSTableError::CrcMismatch { .. })));
    }

    #[test]
    fn test_footer_magic_validation() {
        let footer = Footer {
            index_offset: 1024,
            index_size: 512,
            bloom_offset: 1536,
            bloom_size: 256,
            compression: Compression::None,
            block_size: 4096,
            entry_count: 1000,
        };

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
}
