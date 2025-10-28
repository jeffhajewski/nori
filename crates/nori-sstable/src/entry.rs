//! Key-value entry representation for SSTable.
//!
//! Entry format (with sequence number for MVCC):
//! - klen: varint
//! - vlen: varint
//! - flags: u8 (bits: 0=tombstone, 1-7=reserved)
//! - seqno: varint (u64, monotonically increasing sequence number)
//! - key: bytes[klen]
//! - value: bytes[vlen]
//! - crc32c: u32 (little-endian)

use crate::error::{Result, SSTableError};
use bytes::{Buf, BufMut, Bytes, BytesMut};

bitflags::bitflags! {
    struct Flags: u8 {
        const TOMBSTONE = 0b0000_0001;
    }
}

/// A key-value entry in an SSTable.
///
/// # Sequence Numbers
/// The `seqno` field enables MVCC (Multi-Version Concurrency Control):
/// - Higher sequence numbers represent newer versions
/// - During compaction, entries with higher seqno shadow older versions
/// - Enables snapshot isolation for reads at specific seqno
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: Bytes,
    pub value: Bytes,
    pub tombstone: bool,
    pub seqno: u64,
}

impl Entry {
    /// Creates a new PUT entry with a sequence number.
    pub fn put(key: impl Into<Bytes>, value: impl Into<Bytes>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            tombstone: false,
            seqno: 0, // Default to 0; caller should set appropriate seqno
        }
    }

    /// Creates a new PUT entry with an explicit sequence number.
    pub fn put_with_seqno(key: impl Into<Bytes>, value: impl Into<Bytes>, seqno: u64) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            tombstone: false,
            seqno,
        }
    }

    /// Creates a new DELETE entry (tombstone).
    pub fn delete(key: impl Into<Bytes>) -> Self {
        Self {
            key: key.into(),
            value: Bytes::new(),
            tombstone: true,
            seqno: 0, // Default to 0; caller should set appropriate seqno
        }
    }

    /// Creates a new DELETE entry with an explicit sequence number.
    pub fn delete_with_seqno(key: impl Into<Bytes>, seqno: u64) -> Self {
        Self {
            key: key.into(),
            value: Bytes::new(),
            tombstone: true,
            seqno,
        }
    }

    /// Encodes the entry into bytes with CRC32C checksum.
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::new();

        // Encode klen and vlen as varints
        encode_varint(&mut buf, self.key.len() as u64);
        encode_varint(&mut buf, self.value.len() as u64);

        // Encode flags
        let mut flags = Flags::empty();
        if self.tombstone {
            flags |= Flags::TOMBSTONE;
        }
        buf.put_u8(flags.bits());

        // Encode sequence number (for MVCC)
        encode_varint(&mut buf, self.seqno);

        // Encode key and value
        buf.put_slice(&self.key);
        buf.put_slice(&self.value);

        // Compute and append CRC32C
        let crc = crc32c::crc32c(&buf);
        buf.put_u32_le(crc);

        buf.freeze()
    }

    /// Decodes an entry from bytes.
    pub fn decode(buf: impl Buf) -> Result<Self> {
        // Convert to Bytes for easier handling
        let mut bytes = BytesMut::new();
        let mut buf = buf;
        while buf.has_remaining() {
            bytes.put(buf.chunk());
            let len = buf.chunk().len();
            buf.advance(len);
        }
        let data = bytes.freeze();

        if data.len() < 3 {
            return Err(SSTableError::Incomplete);
        }

        let mut cursor = &data[..];
        let total_len = data.len();

        // Decode klen and vlen
        let klen = decode_varint(&mut cursor)? as usize;
        let vlen = decode_varint(&mut cursor)? as usize;

        // Decode flags
        if cursor.is_empty() {
            return Err(SSTableError::Incomplete);
        }
        let flags = Flags::from_bits_truncate(cursor[0]);
        cursor = &cursor[1..];
        let tombstone = flags.contains(Flags::TOMBSTONE);

        // Decode sequence number
        let seqno = decode_varint(&mut cursor)?;

        // Decode key and value
        if cursor.len() < klen + vlen + 4 {
            return Err(SSTableError::Incomplete);
        }

        let key = Bytes::copy_from_slice(&cursor[..klen]);
        cursor = &cursor[klen..];

        let value = Bytes::copy_from_slice(&cursor[..vlen]);
        cursor = &cursor[vlen..];

        // Verify CRC
        let expected_crc = u32::from_le_bytes([cursor[0], cursor[1], cursor[2], cursor[3]]);
        let data_len = total_len - 4; // All data except CRC
        let actual_crc = crc32c::crc32c(&data[..data_len]);

        if expected_crc != actual_crc {
            return Err(SSTableError::CrcMismatch {
                expected: expected_crc,
                actual: actual_crc,
            });
        }

        Ok(Entry {
            key,
            value,
            tombstone,
            seqno,
        })
    }

    /// Returns the encoded size of this entry in bytes.
    pub fn encoded_size(&self) -> usize {
        varint_size(self.key.len() as u64)
            + varint_size(self.value.len() as u64)
            + 1 // flags
            + varint_size(self.seqno) // sequence number
            + self.key.len()
            + self.value.len()
            + 4 // crc32c
    }
}

/// Encodes a u64 as a varint.
fn encode_varint(buf: &mut BytesMut, mut value: u64) {
    while value >= 0x80 {
        buf.put_u8((value as u8) | 0x80);
        value >>= 7;
    }
    buf.put_u8(value as u8);
}

/// Decodes a varint from a buffer.
fn decode_varint(buf: &mut impl Buf) -> Result<u64> {
    let mut value = 0u64;
    let mut shift = 0;

    loop {
        if shift >= 64 {
            return Err(SSTableError::InvalidFormat("varint overflow".to_string()));
        }
        if buf.remaining() < 1 {
            return Err(SSTableError::Incomplete);
        }

        let byte = buf.get_u8();
        value |= ((byte & 0x7F) as u64) << shift;

        if byte & 0x80 == 0 {
            return Ok(value);
        }

        shift += 7;
    }
}

/// Returns the size of a varint-encoded value in bytes.
fn varint_size(value: u64) -> usize {
    if value == 0 {
        1
    } else {
        ((63 - value.leading_zeros()) / 7 + 1) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entry_roundtrip_put() {
        let entry = Entry::put(&b"test_key"[..], &b"test_value"[..]);
        let encoded = entry.encode();
        let decoded = Entry::decode(encoded.as_ref()).unwrap();

        assert_eq!(entry, decoded);
    }

    #[test]
    fn test_entry_roundtrip_delete() {
        let entry = Entry::delete(&b"test_key"[..]);
        let encoded = entry.encode();
        let decoded = Entry::decode(encoded.as_ref()).unwrap();

        assert_eq!(entry, decoded);
        assert!(decoded.tombstone);
        assert_eq!(decoded.value.len(), 0);
    }

    #[test]
    fn test_entry_crc_validation() {
        let entry = Entry::put(&b"key"[..], &b"value"[..]);
        let encoded = entry.encode();

        // Corrupt a byte in the encoded data
        let mut corrupted = BytesMut::from(encoded.as_ref());
        if corrupted.len() > 10 {
            corrupted[10] ^= 0xFF;
        }

        let result = Entry::decode(corrupted.freeze().as_ref());
        assert!(matches!(result, Err(SSTableError::CrcMismatch { .. })));
    }

    #[test]
    fn test_varint_encoding() {
        let test_values = vec![0, 1, 127, 128, 255, 256, 16383, 16384, u64::MAX];

        for value in test_values {
            let mut buf = BytesMut::new();
            encode_varint(&mut buf, value);

            let mut buf = buf.freeze();
            let decoded = decode_varint(&mut buf).unwrap();

            assert_eq!(value, decoded);
        }
    }

    #[test]
    fn test_varint_size() {
        assert_eq!(varint_size(0), 1);
        assert_eq!(varint_size(127), 1);
        assert_eq!(varint_size(128), 2);
        assert_eq!(varint_size(16383), 2);
        assert_eq!(varint_size(16384), 3);
        assert_eq!(varint_size(u64::MAX), 10);
    }

    #[test]
    fn test_entry_encoded_size() {
        let entry = Entry::put(&b"key"[..], &b"value"[..]);
        let encoded = entry.encode();
        assert_eq!(entry.encoded_size(), encoded.len());
    }
}
