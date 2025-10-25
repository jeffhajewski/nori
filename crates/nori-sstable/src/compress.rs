//! Block compression and decompression.
//!
//! This module provides compression support for SSTable blocks using LZ4 and Zstd.
//!
//! # Design
//!
//! - **LZ4**: Fast compression/decompression (3,900 MB/s decompress, 750 MB/s compress)
//!   - Best for hot workloads where decompression speed is critical
//!   - Default choice for most use cases
//!   - Typical compression ratio: 2-3x
//!
//! - **Zstd**: Higher compression ratio but slower (1,200 MB/s decompress)
//!   - Best for cold storage where compression ratio matters more than speed
//!   - Typical compression ratio: 3-5x
//!
//! # Performance
//!
//! With block caching, decompression happens once per block miss, then the
//! uncompressed block is cached. This makes decompression speed less critical
//! for hot workloads.

use crate::error::{Result, SSTableError};
use crate::format::Compression;

/// Compresses data using the specified algorithm.
///
/// # Arguments
///
/// * `data` - Raw uncompressed data
/// * `algo` - Compression algorithm to use
///
/// # Returns
///
/// Compressed data, or original data if `Compression::None` is specified.
pub fn compress(data: &[u8], algo: Compression) -> Result<Vec<u8>> {
    match algo {
        Compression::None => Ok(data.to_vec()),
        Compression::Lz4 => compress_lz4(data),
        Compression::Zstd => compress_zstd(data),
    }
}

/// Decompresses data using the specified algorithm.
///
/// # Arguments
///
/// * `data` - Compressed data
/// * `algo` - Compression algorithm that was used
///
/// # Returns
///
/// Decompressed data, or original data if `Compression::None` is specified.
pub fn decompress(data: &[u8], algo: Compression) -> Result<Vec<u8>> {
    match algo {
        Compression::None => Ok(data.to_vec()),
        Compression::Lz4 => decompress_lz4(data),
        Compression::Zstd => decompress_zstd(data),
    }
}

/// Compresses data using LZ4 block compression.
///
/// Uses default compression settings (acceleration = 1) for good balance
/// between speed and compression ratio.
fn compress_lz4(data: &[u8]) -> Result<Vec<u8>> {
    lz4::block::compress(data, None, false)
        .map_err(|e| SSTableError::CompressionFailed(e.to_string()))
}

/// Decompresses LZ4-compressed data.
///
/// For SSTable blocks, we know the max size is bounded by block_size (typically 4KB).
/// However, since this is a general-purpose function, we use a very conservative
/// limit to prevent decompression bombs while allowing high compression ratios.
fn decompress_lz4(data: &[u8]) -> Result<Vec<u8>> {
    // Very conservative limit: allow up to 256KB decompressed from any input
    // This is safe for SSTable blocks (max 4KB) and prevents decompression bombs
    // For highly compressible data (e.g., all same byte), ratio can be >100x
    let max_size = 256 * 1024;
    lz4::block::decompress(data, Some(max_size))
        .map_err(|e| SSTableError::DecompressionFailed(e.to_string()))
}

/// Compresses data using Zstd compression.
///
/// Uses default compression level (3) for good balance between speed,
/// compression ratio, and memory usage.
fn compress_zstd(data: &[u8]) -> Result<Vec<u8>> {
    zstd::encode_all(data, 3)
        .map_err(|e| SSTableError::CompressionFailed(e.to_string()))
}

/// Decompresses Zstd-compressed data.
fn decompress_zstd(data: &[u8]) -> Result<Vec<u8>> {
    zstd::decode_all(data)
        .map_err(|e| SSTableError::DecompressionFailed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_none() {
        let data = b"hello world";
        let compressed = compress(data, Compression::None).unwrap();
        assert_eq!(compressed, data);

        let decompressed = decompress(&compressed, Compression::None).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_lz4_roundtrip() {
        let data = b"hello world hello world hello world hello world";
        let compressed = compress(data, Compression::Lz4).unwrap();

        // Should be compressed (smaller than original)
        assert!(compressed.len() < data.len(),
                "Compressed size {} should be less than original {}",
                compressed.len(), data.len());

        let decompressed = decompress(&compressed, Compression::Lz4).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_zstd_roundtrip() {
        let data = b"hello world hello world hello world hello world";
        let compressed = compress(data, Compression::Zstd).unwrap();

        // Should be compressed
        assert!(compressed.len() < data.len(),
                "Compressed size {} should be less than original {}",
                compressed.len(), data.len());

        let decompressed = decompress(&compressed, Compression::Zstd).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_lz4_large_data() {
        // Test with 4KB block (typical SSTable block size)
        let data = vec![b'x'; 4096];
        let compressed = compress(&data, Compression::Lz4).unwrap();

        // Highly compressible data should compress well
        assert!(compressed.len() < 100,
                "Highly compressible data should compress to <100 bytes, got {}",
                compressed.len());

        let decompressed = decompress(&compressed, Compression::Lz4).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_zstd_large_data() {
        let data = vec![b'x'; 4096];
        let compressed = compress(&data, Compression::Zstd).unwrap();

        // Zstd should compress even better than LZ4
        assert!(compressed.len() < 100);

        let decompressed = decompress(&compressed, Compression::Zstd).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_incompressible_data() {
        // Random-like data (shouldn't compress well)
        let data: Vec<u8> = (0..256).map(|i| (i * 37) as u8).collect();

        let lz4_compressed = compress(&data, Compression::Lz4).unwrap();
        // LZ4 may expand data if incompressible
        assert!(lz4_compressed.len() >= data.len() - 10);

        let decompressed = decompress(&lz4_compressed, Compression::Lz4).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_empty_data() {
        let data = b"";

        let compressed = compress(data, Compression::Lz4).unwrap();
        let decompressed = decompress(&compressed, Compression::Lz4).unwrap();
        assert_eq!(decompressed, data);

        let compressed = compress(data, Compression::Zstd).unwrap();
        let decompressed = decompress(&compressed, Compression::Zstd).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_single_byte() {
        let data = b"x";

        let compressed = compress(data, Compression::Lz4).unwrap();
        let decompressed = decompress(&compressed, Compression::Lz4).unwrap();
        assert_eq!(decompressed, data);
    }
}
