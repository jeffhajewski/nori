///! Error types for nori-sstable operations.

use std::io;
use thiserror::Error;

/// Errors that can occur during SSTable operations.
#[derive(Debug, Error)]
pub enum SSTableError {
    /// I/O error from filesystem operations.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// CRC checksum mismatch indicating data corruption.
    #[error("CRC mismatch: expected {expected:#x}, got {actual:#x}")]
    CrcMismatch { expected: u32, actual: u32 },

    /// Invalid compression type in entry header.
    #[error("Invalid compression type: {0}")]
    InvalidCompression(u8),

    /// Compression operation failed.
    #[error("Compression failed: {0}")]
    CompressionFailed(String),

    /// Decompression operation failed.
    #[error("Decompression failed: {0}")]
    DecompressionFailed(String),

    /// Keys are not in sorted order during build.
    #[error("Keys must be inserted in sorted order: {0:?} <= {1:?}")]
    KeysNotSorted(Vec<u8>, Vec<u8>),

    /// Attempted to read past end of file or block.
    #[error("Unexpected end of file")]
    UnexpectedEof,

    /// Invalid SSTable file format or magic number.
    #[error("Invalid SSTable format: {0}")]
    InvalidFormat(String),

    /// SSTable file not found.
    #[error("SSTable file not found: {0}")]
    NotFound(String),

    /// Invalid configuration parameter.
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// Key not found in SSTable.
    #[error("Key not found")]
    KeyNotFound,

    /// Incomplete data structure (e.g., incomplete block).
    #[error("Incomplete data")]
    Incomplete,
}

/// Result type alias for SSTable operations.
pub type Result<T> = std::result::Result<T, SSTableError>;
