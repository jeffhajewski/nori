//! Vector block encoding/decoding for SSTable vector support.
//!
//! Provides storage for vectors within SSTables with support for:
//! - Raw vector storage (for small datasets or memtable flushes)
//! - HNSW-indexed storage (for L1-L2 warm tiers)
//! - Vamana/PQ-compressed storage (for L3+ cold tiers)
//!
//! Vector Block Layout:
//! ```text
//! [Header: 16 bytes]
//!   - magic: u32 = "NVEC"
//!   - version: u8 = 1
//!   - block_type: u8 (0=Raw, 1=HNSW, 2=Vamana)
//!   - dimensions: u16
//!   - vector_count: u32
//!   - distance_fn: u8 (0=Euclidean, 1=Cosine, 2=InnerProduct)
//!   - reserved: 3 bytes
//!
//! [ID Table]
//!   - For each vector:
//!     - id_len: u16
//!     - id_bytes: [u8; id_len]
//!
//! [Vector Data]
//!   - For Raw: f32 × dimensions × vector_count
//!   - For HNSW: [HNSW graph serialization]
//!   - For Vamana: [PQ codebook + codes + graph]
//!
//! [Footer: 8 bytes]
//!   - id_table_offset: u32
//!   - vector_data_offset: u32
//! ```

use bytes::{Buf, BufMut, Bytes, BytesMut};
use crate::error::{Result, SSTableError};

/// Magic number for vector blocks: "NVEC"
pub const VECTOR_BLOCK_MAGIC: u32 = 0x4345564E; // "NVEC" in little-endian

/// Current vector block format version
pub const VECTOR_BLOCK_VERSION: u8 = 1;

/// Header size in bytes
pub const VECTOR_HEADER_SIZE: usize = 16;

/// Footer size in bytes
pub const VECTOR_FOOTER_SIZE: usize = 8;

/// Type of vector block storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VectorBlockType {
    /// Raw vectors (no indexing)
    Raw = 0,
    /// HNSW indexed (for warm tiers)
    Hnsw = 1,
    /// Vamana/DiskANN indexed with PQ (for cold tiers)
    Vamana = 2,
}

impl VectorBlockType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Raw),
            1 => Some(Self::Hnsw),
            2 => Some(Self::Vamana),
            _ => None,
        }
    }
}

/// Distance function type (mirrors nori_vector::DistanceFunction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DistanceFn {
    Euclidean = 0,
    Cosine = 1,
    InnerProduct = 2,
}

impl DistanceFn {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Euclidean),
            1 => Some(Self::Cosine),
            2 => Some(Self::InnerProduct),
            _ => None,
        }
    }
}

/// Header for a vector block.
#[derive(Debug, Clone)]
pub struct VectorBlockHeader {
    /// Block type (Raw, HNSW, Vamana)
    pub block_type: VectorBlockType,
    /// Vector dimensions
    pub dimensions: u16,
    /// Number of vectors
    pub vector_count: u32,
    /// Distance function
    pub distance_fn: DistanceFn,
}

impl VectorBlockHeader {
    /// Encode header to bytes.
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_u32_le(VECTOR_BLOCK_MAGIC);
        buf.put_u8(VECTOR_BLOCK_VERSION);
        buf.put_u8(self.block_type as u8);
        buf.put_u16_le(self.dimensions);
        buf.put_u32_le(self.vector_count);
        buf.put_u8(self.distance_fn as u8);
        buf.put_bytes(0, 3); // reserved
    }

    /// Decode header from bytes.
    pub fn decode(buf: &mut &[u8]) -> Result<Self> {
        if buf.len() < VECTOR_HEADER_SIZE {
            return Err(SSTableError::Incomplete);
        }

        let magic = buf.get_u32_le();
        if magic != VECTOR_BLOCK_MAGIC {
            return Err(SSTableError::InvalidFormat(format!(
                "Invalid vector block magic: {:#x}",
                magic
            )));
        }

        let version = buf.get_u8();
        if version != VECTOR_BLOCK_VERSION {
            return Err(SSTableError::InvalidFormat(format!(
                "Unsupported vector block version: {}",
                version
            )));
        }

        let block_type = VectorBlockType::from_u8(buf.get_u8())
            .ok_or_else(|| SSTableError::InvalidFormat("Invalid block type".to_string()))?;
        let dimensions = buf.get_u16_le();
        let vector_count = buf.get_u32_le();
        let distance_fn = DistanceFn::from_u8(buf.get_u8())
            .ok_or_else(|| SSTableError::InvalidFormat("Invalid distance function".to_string()))?;
        buf.advance(3); // reserved

        Ok(Self {
            block_type,
            dimensions,
            vector_count,
            distance_fn,
        })
    }
}

/// A vector entry in the block.
#[derive(Debug, Clone)]
pub struct VectorEntry {
    /// Vector ID
    pub id: String,
    /// Vector data
    pub vector: Vec<f32>,
}

/// Builder for raw vector blocks.
pub struct RawVectorBlockBuilder {
    dimensions: u16,
    distance_fn: DistanceFn,
    entries: Vec<VectorEntry>,
}

impl RawVectorBlockBuilder {
    /// Create a new raw vector block builder.
    pub fn new(dimensions: u16, distance_fn: DistanceFn) -> Self {
        Self {
            dimensions,
            distance_fn,
            entries: Vec::new(),
        }
    }

    /// Add a vector to the block.
    pub fn add(&mut self, id: String, vector: Vec<f32>) -> Result<()> {
        if vector.len() != self.dimensions as usize {
            return Err(SSTableError::InvalidFormat(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.dimensions,
                vector.len()
            )));
        }
        self.entries.push(VectorEntry { id, vector });
        Ok(())
    }

    /// Get number of vectors added.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Build the vector block.
    pub fn finish(self) -> Bytes {
        let mut buf = BytesMut::new();

        // Header
        let header = VectorBlockHeader {
            block_type: VectorBlockType::Raw,
            dimensions: self.dimensions,
            vector_count: self.entries.len() as u32,
            distance_fn: self.distance_fn,
        };
        header.encode(&mut buf);

        let id_table_offset = buf.len() as u32;

        // ID table
        for entry in &self.entries {
            let id_bytes = entry.id.as_bytes();
            buf.put_u16_le(id_bytes.len() as u16);
            buf.put_slice(id_bytes);
        }

        let vector_data_offset = buf.len() as u32;

        // Vector data (raw f32 values)
        for entry in &self.entries {
            for &val in &entry.vector {
                buf.put_f32_le(val);
            }
        }

        // Footer
        buf.put_u32_le(id_table_offset);
        buf.put_u32_le(vector_data_offset);

        buf.freeze()
    }
}

/// Reader for raw vector blocks.
pub struct RawVectorBlockReader {
    data: Bytes,
    header: VectorBlockHeader,
    id_table_offset: u32,
    vector_data_offset: u32,
}

impl RawVectorBlockReader {
    /// Open a raw vector block from bytes.
    pub fn open(data: Bytes) -> Result<Self> {
        if data.len() < VECTOR_HEADER_SIZE + VECTOR_FOOTER_SIZE {
            return Err(SSTableError::Incomplete);
        }

        // Read footer first
        let footer_start = data.len() - VECTOR_FOOTER_SIZE;
        let id_table_offset = u32::from_le_bytes([
            data[footer_start],
            data[footer_start + 1],
            data[footer_start + 2],
            data[footer_start + 3],
        ]);
        let vector_data_offset = u32::from_le_bytes([
            data[footer_start + 4],
            data[footer_start + 5],
            data[footer_start + 6],
            data[footer_start + 7],
        ]);

        // Read header
        let mut buf = &data[..VECTOR_HEADER_SIZE];
        let header = VectorBlockHeader::decode(&mut buf)?;

        if header.block_type != VectorBlockType::Raw {
            return Err(SSTableError::InvalidFormat(format!(
                "Expected Raw block type, got {:?}",
                header.block_type
            )));
        }

        Ok(Self {
            data,
            header,
            id_table_offset,
            vector_data_offset,
        })
    }

    /// Get number of vectors.
    pub fn len(&self) -> usize {
        self.header.vector_count as usize
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.header.vector_count == 0
    }

    /// Get dimensions.
    pub fn dimensions(&self) -> usize {
        self.header.dimensions as usize
    }

    /// Iterate over all vectors.
    pub fn iter(&self) -> RawVectorBlockIterator {
        RawVectorBlockIterator {
            data: self.data.clone(),
            dimensions: self.header.dimensions as usize,
            vector_count: self.header.vector_count as usize,
            id_offset: self.id_table_offset as usize,
            vector_offset: self.vector_data_offset as usize,
            current_id_offset: self.id_table_offset as usize,
            current_idx: 0,
        }
    }

    /// Get vector by index.
    pub fn get_by_index(&self, idx: usize) -> Result<Option<VectorEntry>> {
        if idx >= self.header.vector_count as usize {
            return Ok(None);
        }

        // Navigate to the ID
        let mut id_offset = self.id_table_offset as usize;
        for _ in 0..idx {
            let id_len = u16::from_le_bytes([self.data[id_offset], self.data[id_offset + 1]]) as usize;
            id_offset += 2 + id_len;
        }

        let id_len = u16::from_le_bytes([self.data[id_offset], self.data[id_offset + 1]]) as usize;
        let id = String::from_utf8_lossy(&self.data[id_offset + 2..id_offset + 2 + id_len]).to_string();

        // Get vector data
        let dims = self.header.dimensions as usize;
        let vector_start = self.vector_data_offset as usize + idx * dims * 4;
        let mut vector = Vec::with_capacity(dims);
        for i in 0..dims {
            let offset = vector_start + i * 4;
            let val = f32::from_le_bytes([
                self.data[offset],
                self.data[offset + 1],
                self.data[offset + 2],
                self.data[offset + 3],
            ]);
            vector.push(val);
        }

        Ok(Some(VectorEntry { id, vector }))
    }

    /// Search for a vector by ID.
    pub fn get_by_id(&self, target_id: &str) -> Result<Option<VectorEntry>> {
        // Linear scan through IDs (could be optimized with sorted IDs + binary search)
        let mut id_offset = self.id_table_offset as usize;
        for idx in 0..self.header.vector_count as usize {
            let id_len = u16::from_le_bytes([self.data[id_offset], self.data[id_offset + 1]]) as usize;
            let id = std::str::from_utf8(&self.data[id_offset + 2..id_offset + 2 + id_len])
                .map_err(|e| SSTableError::InvalidFormat(format!("Invalid UTF-8 in ID: {}", e)))?;

            if id == target_id {
                return self.get_by_index(idx);
            }

            id_offset += 2 + id_len;
        }

        Ok(None)
    }
}

/// Iterator over vectors in a raw vector block.
pub struct RawVectorBlockIterator {
    data: Bytes,
    dimensions: usize,
    vector_count: usize,
    #[allow(dead_code)]
    id_offset: usize,
    vector_offset: usize,
    current_id_offset: usize,
    current_idx: usize,
}

impl Iterator for RawVectorBlockIterator {
    type Item = Result<VectorEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_idx >= self.vector_count {
            return None;
        }

        // Read ID
        let id_len = u16::from_le_bytes([
            self.data[self.current_id_offset],
            self.data[self.current_id_offset + 1],
        ]) as usize;
        let id = match std::str::from_utf8(&self.data[self.current_id_offset + 2..self.current_id_offset + 2 + id_len]) {
            Ok(s) => s.to_string(),
            Err(e) => return Some(Err(SSTableError::InvalidFormat(format!("Invalid UTF-8 in ID: {}", e)))),
        };
        self.current_id_offset += 2 + id_len;

        // Read vector
        let vector_start = self.vector_offset + self.current_idx * self.dimensions * 4;
        let mut vector = Vec::with_capacity(self.dimensions);
        for i in 0..self.dimensions {
            let offset = vector_start + i * 4;
            let val = f32::from_le_bytes([
                self.data[offset],
                self.data[offset + 1],
                self.data[offset + 2],
                self.data[offset + 3],
            ]);
            vector.push(val);
        }

        self.current_idx += 1;

        Some(Ok(VectorEntry { id, vector }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_vector_block_roundtrip() {
        let mut builder = RawVectorBlockBuilder::new(4, DistanceFn::Euclidean);

        builder.add("vec1".to_string(), vec![1.0, 2.0, 3.0, 4.0]).unwrap();
        builder.add("vec2".to_string(), vec![5.0, 6.0, 7.0, 8.0]).unwrap();
        builder.add("vec3".to_string(), vec![9.0, 10.0, 11.0, 12.0]).unwrap();

        let data = builder.finish();

        let reader = RawVectorBlockReader::open(data).unwrap();
        assert_eq!(reader.len(), 3);
        assert_eq!(reader.dimensions(), 4);

        // Test get by index
        let entry = reader.get_by_index(0).unwrap().unwrap();
        assert_eq!(entry.id, "vec1");
        assert_eq!(entry.vector, vec![1.0, 2.0, 3.0, 4.0]);

        let entry = reader.get_by_index(2).unwrap().unwrap();
        assert_eq!(entry.id, "vec3");
        assert_eq!(entry.vector, vec![9.0, 10.0, 11.0, 12.0]);

        // Test get by ID
        let entry = reader.get_by_id("vec2").unwrap().unwrap();
        assert_eq!(entry.id, "vec2");
        assert_eq!(entry.vector, vec![5.0, 6.0, 7.0, 8.0]);

        // Test non-existent
        assert!(reader.get_by_id("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_raw_vector_block_iterator() {
        let mut builder = RawVectorBlockBuilder::new(2, DistanceFn::Cosine);

        for i in 0..10 {
            builder
                .add(format!("vec{}", i), vec![i as f32, (i * 2) as f32])
                .unwrap();
        }

        let data = builder.finish();
        let reader = RawVectorBlockReader::open(data).unwrap();

        let entries: Vec<_> = reader.iter().collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(entries.len(), 10);

        for (i, entry) in entries.iter().enumerate() {
            assert_eq!(entry.id, format!("vec{}", i));
            assert_eq!(entry.vector, vec![i as f32, (i * 2) as f32]);
        }
    }

    #[test]
    fn test_dimension_mismatch() {
        let mut builder = RawVectorBlockBuilder::new(4, DistanceFn::Euclidean);

        let result = builder.add("vec1".to_string(), vec![1.0, 2.0]); // Wrong dimensions
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_block() {
        let builder = RawVectorBlockBuilder::new(4, DistanceFn::Euclidean);
        let data = builder.finish();

        let reader = RawVectorBlockReader::open(data).unwrap();
        assert!(reader.is_empty());
        assert_eq!(reader.len(), 0);
    }

    #[test]
    fn test_header_encoding() {
        let header = VectorBlockHeader {
            block_type: VectorBlockType::Hnsw,
            dimensions: 128,
            vector_count: 1000,
            distance_fn: DistanceFn::InnerProduct,
        };

        let mut buf = BytesMut::new();
        header.encode(&mut buf);

        assert_eq!(buf.len(), VECTOR_HEADER_SIZE);

        let mut slice = &buf[..];
        let decoded = VectorBlockHeader::decode(&mut slice).unwrap();

        assert_eq!(decoded.block_type, VectorBlockType::Hnsw);
        assert_eq!(decoded.dimensions, 128);
        assert_eq!(decoded.vector_count, 1000);
        assert_eq!(decoded.distance_fn, DistanceFn::InnerProduct);
    }
}
