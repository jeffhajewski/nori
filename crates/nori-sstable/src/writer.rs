//! Low-level SSTable file writer with async I/O.
//!
//! The writer handles the physical file I/O for creating SSTable files.
//! It writes components in order: data blocks → index → bloom filter → footer.

use crate::bloom::BloomFilter;
use crate::error::{Result, SSTableError};
use crate::format::Footer;
use crate::index::Index;
use std::path::PathBuf;
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;

/// Low-level writer for SSTable files.
pub struct SSTableWriter {
    file: File,
    path: PathBuf,
    bytes_written: u64,
}

impl SSTableWriter {
    /// Creates a new SSTable file for writing.
    ///
    /// The file is created with truncate mode, so any existing file at this path
    /// will be overwritten.
    pub async fn create(path: PathBuf) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .await
            .map_err(SSTableError::Io)?;

        Ok(Self {
            file,
            path,
            bytes_written: 0,
        })
    }

    /// Writes a data block to the file.
    ///
    /// Returns the byte offset where the block was written.
    pub async fn write_block(&mut self, block_data: &[u8]) -> Result<u64> {
        let offset = self.bytes_written;

        self.file
            .write_all(block_data)
            .await
            .map_err(SSTableError::Io)?;

        self.bytes_written += block_data.len() as u64;

        Ok(offset)
    }

    /// Writes the index to the file.
    ///
    /// Returns a tuple of (offset, size) where the index was written.
    pub async fn write_index(&mut self, index: &Index) -> Result<(u64, u64)> {
        let offset = self.bytes_written;
        let index_data = index.encode();

        self.file
            .write_all(&index_data)
            .await
            .map_err(SSTableError::Io)?;

        let size = index_data.len() as u64;
        self.bytes_written += size;

        Ok((offset, size))
    }

    /// Writes the bloom filter to the file.
    ///
    /// Returns a tuple of (offset, size) where the bloom filter was written.
    pub async fn write_bloom(&mut self, bloom: &BloomFilter) -> Result<(u64, u64)> {
        let offset = self.bytes_written;
        let bloom_data = bloom.encode();

        self.file
            .write_all(&bloom_data)
            .await
            .map_err(SSTableError::Io)?;

        let size = bloom_data.len() as u64;
        self.bytes_written += size;

        Ok((offset, size))
    }

    /// Writes the footer to the file.
    ///
    /// The footer is always the last 64 bytes of the file.
    pub async fn write_footer(&mut self, footer: &Footer) -> Result<()> {
        let footer_data = footer.encode();

        self.file
            .write_all(&footer_data)
            .await
            .map_err(SSTableError::Io)?;

        self.bytes_written += footer_data.len() as u64;

        Ok(())
    }

    /// Syncs all written data to disk.
    ///
    /// This ensures durability by calling fsync on the file.
    pub async fn sync(&mut self) -> Result<()> {
        self.file.sync_all().await.map_err(SSTableError::Io)?;

        Ok(())
    }

    /// Closes the file handle.
    ///
    /// This consumes the writer and ensures the file is properly closed.
    pub async fn finish(self) -> Result<()> {
        // File handle is automatically closed when dropped
        Ok(())
    }

    /// Returns the number of bytes written so far.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Returns the path of the file being written.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::Compression;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_writer_create() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        let writer = SSTableWriter::create(path.clone()).await.unwrap();

        assert_eq!(writer.bytes_written(), 0);
        assert_eq!(writer.path(), &path);
    }

    #[tokio::test]
    async fn test_writer_write_block() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        let mut writer = SSTableWriter::create(path.clone()).await.unwrap();

        let block_data = b"test_block_data";
        let offset = writer.write_block(block_data).await.unwrap();

        assert_eq!(offset, 0);
        assert_eq!(writer.bytes_written(), block_data.len() as u64);

        writer.sync().await.unwrap();
        writer.finish().await.unwrap();

        // Verify file exists and has correct size
        let metadata = tokio::fs::metadata(&path).await.unwrap();
        assert_eq!(metadata.len(), block_data.len() as u64);
    }

    #[tokio::test]
    async fn test_writer_multiple_blocks() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        let mut writer = SSTableWriter::create(path.clone()).await.unwrap();

        // Write first block
        let block1 = b"first_block";
        let offset1 = writer.write_block(block1).await.unwrap();
        assert_eq!(offset1, 0);

        // Write second block
        let block2 = b"second_block";
        let offset2 = writer.write_block(block2).await.unwrap();
        assert_eq!(offset2, block1.len() as u64);

        // Write third block
        let block3 = b"third_block";
        let offset3 = writer.write_block(block3).await.unwrap();
        assert_eq!(offset3, (block1.len() + block2.len()) as u64);

        writer.sync().await.unwrap();
        writer.finish().await.unwrap();

        // Verify total file size
        let metadata = tokio::fs::metadata(&path).await.unwrap();
        let expected_size = (block1.len() + block2.len() + block3.len()) as u64;
        assert_eq!(metadata.len(), expected_size);
    }

    #[tokio::test]
    async fn test_writer_full_layout() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        let mut writer = SSTableWriter::create(path.clone()).await.unwrap();

        // Write data blocks
        let block1 = b"data_block_1";
        let block2 = b"data_block_2";
        writer.write_block(block1).await.unwrap();
        writer.write_block(block2).await.unwrap();

        // Write index
        let mut index = Index::new();
        index.add_block(bytes::Bytes::from("key1"), 0, block1.len() as u32);
        index.add_block(
            bytes::Bytes::from("key2"),
            block1.len() as u64,
            block2.len() as u32,
        );
        let (index_offset, index_size) = writer.write_index(&index).await.unwrap();

        // Write bloom filter
        let mut bloom = BloomFilter::new(100, 10);
        bloom.add(b"key1");
        bloom.add(b"key2");
        let (bloom_offset, bloom_size) = writer.write_bloom(&bloom).await.unwrap();

        // Write footer
        let footer = Footer {
            index_offset,
            index_size,
            bloom_offset,
            bloom_size,
            compression: Compression::None,
            block_size: 4096,
            entry_count: 2,
        };
        writer.write_footer(&footer).await.unwrap();

        writer.sync().await.unwrap();
        let total_bytes = writer.bytes_written();
        writer.finish().await.unwrap();

        // Verify file size
        let metadata = tokio::fs::metadata(&path).await.unwrap();
        assert_eq!(metadata.len(), total_bytes);

        // Verify footer is at end
        let file_data = tokio::fs::read(&path).await.unwrap();
        assert_eq!(file_data.len(), total_bytes as usize);

        // Decode and verify footer
        let footer_start = file_data.len() - 64;
        let footer_bytes: [u8; 64] = file_data[footer_start..].try_into().unwrap();
        let decoded_footer = Footer::decode(&footer_bytes).unwrap();

        assert_eq!(decoded_footer.entry_count, 2);
        assert_eq!(decoded_footer.index_offset, index_offset);
        assert_eq!(decoded_footer.bloom_offset, bloom_offset);
    }

    #[tokio::test]
    async fn test_writer_truncate_existing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        // Write some data
        {
            let mut writer = SSTableWriter::create(path.clone()).await.unwrap();
            writer.write_block(b"initial_data").await.unwrap();
            writer.finish().await.unwrap();
        }

        // Truncate and write new data
        {
            let mut writer = SSTableWriter::create(path.clone()).await.unwrap();
            writer.write_block(b"new").await.unwrap();
            writer.finish().await.unwrap();
        }

        // Verify file only contains new data
        let file_data = tokio::fs::read(&path).await.unwrap();
        assert_eq!(file_data, b"new");
    }
}
