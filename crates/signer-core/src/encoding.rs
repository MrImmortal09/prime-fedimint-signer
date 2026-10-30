//! The subset of fedimint's consensus encoding the signer reads and writes.
//!
//! Rules, as implemented by `fedimint-core` 0.13 (`encoding/mod.rs`, `encoding/collections.rs`,
//! `fedimint-derive`):
//!
//! - `u16`, `u32`, `u64` are BigSize varints (LDK's encoding), and must be canonical.
//! - `u8` and `bool` are one plain byte.
//! - `[u8; N]` is the raw bytes; `Vec<T>` and strings are a BigSize length then the elements.
//! - Structs are their fields in order, with no framing.
//! - Derived enums are a BigSize variant index followed by the variant's fields as a length-prefixed
//!   byte string, which must be consumed exactly.
//! - Module items (`DynInput`, `DynOutput`) are a BigSize module instance id followed by the module's
//!   encoding as a length-prefixed byte string.
//!
//! Decoding here is stricter than fedimint's in one way: callers decode whole messages and refuse
//! trailing bytes, so a value has exactly one accepted encoding.

use std::fmt;

/// Why bytes did not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    UnexpectedEnd {
        what: &'static str,
        needed: usize,
        available: usize,
    },
    NonCanonicalBigSize,
    OutOfRange {
        what: &'static str,
        value: u64,
    },
    TrailingBytes {
        what: &'static str,
        remaining: usize,
    },
    UnknownVariant {
        what: &'static str,
        variant: u64,
    },
    TooMany {
        what: &'static str,
        count: u64,
        limit: u64,
    },
    Invalid {
        what: &'static str,
        reason: String,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEnd {
                what,
                needed,
                available,
            } => write!(
                f,
                "{what}: needs {needed} more bytes, only {available} left"
            ),
            Self::NonCanonicalBigSize => f.write_str("integer is not canonically encoded"),
            Self::OutOfRange { what, value } => write!(f, "{what}: value {value} out of range"),
            Self::TrailingBytes { what, remaining } => {
                write!(f, "{what}: {remaining} unexpected trailing bytes")
            }
            Self::UnknownVariant { what, variant } => {
                write!(f, "{what}: variant {variant} is not supported")
            }
            Self::TooMany { what, count, limit } => {
                write!(f, "{what}: {count} items, the limit is {limit}")
            }
            Self::Invalid { what, reason } => write!(f, "{what}: {reason}"),
        }
    }
}

impl std::error::Error for DecodeError {}

pub type DecodeResult<T> = Result<T, DecodeError>;

/// A cursor over borrowed bytes.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// Fails unless every byte was consumed.
    pub fn finish(&self, what: &'static str) -> DecodeResult<()> {
        match self.remaining() {
            0 => Ok(()),
            remaining => Err(DecodeError::TrailingBytes { what, remaining }),
        }
    }

    pub fn read_slice(&mut self, len: usize, what: &'static str) -> DecodeResult<&'a [u8]> {
        if self.remaining() < len {
            return Err(DecodeError::UnexpectedEnd {
                what,
                needed: len,
                available: self.remaining(),
            });
        }
        let slice = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }

    pub fn read_array<const N: usize>(&mut self, what: &'static str) -> DecodeResult<[u8; N]> {
        let slice = self.read_slice(N, what)?;
        Ok(slice.try_into().expect("slice has length N"))
    }

    pub fn read_u8(&mut self, what: &'static str) -> DecodeResult<u8> {
        Ok(self.read_array::<1>(what)?[0])
    }

    pub fn read_bool(&mut self, what: &'static str) -> DecodeResult<bool> {
        match self.read_u8(what)? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(DecodeError::OutOfRange {
                what,
                value: u64::from(other),
            }),
        }
    }

    /// A canonical BigSize varint.
    pub fn read_bigsize(&mut self, what: &'static str) -> DecodeResult<u64> {
        let first = self.read_u8(what)?;
        let (value, minimum) = match first {
            0..=0xfc => return Ok(u64::from(first)),
            0xfd => (u64::from(u16::from_be_bytes(self.read_array(what)?)), 0xfd),
            0xfe => (
                u64::from(u32::from_be_bytes(self.read_array(what)?)),
                0x1_0000,
            ),
            0xff => (u64::from_be_bytes(self.read_array(what)?), 0x1_0000_0000),
        };
        if value < minimum {
            return Err(DecodeError::NonCanonicalBigSize);
        }
        Ok(value)
    }

    pub fn read_u16(&mut self, what: &'static str) -> DecodeResult<u16> {
        let value = self.read_bigsize(what)?;
        u16::try_from(value).map_err(|_| DecodeError::OutOfRange { what, value })
    }

    pub fn read_u32(&mut self, what: &'static str) -> DecodeResult<u32> {
        let value = self.read_bigsize(what)?;
        u32::try_from(value).map_err(|_| DecodeError::OutOfRange { what, value })
    }

    pub fn read_u64(&mut self, what: &'static str) -> DecodeResult<u64> {
        self.read_bigsize(what)
    }

    /// A collection length, bounded so a hostile length cannot force a large allocation: every
    /// element takes at least `min_element_size` bytes of the remaining input.
    pub fn read_len(
        &mut self,
        what: &'static str,
        min_element_size: usize,
        limit: u64,
    ) -> DecodeResult<usize> {
        let count = self.read_bigsize(what)?;
        if count > limit {
            return Err(DecodeError::TooMany { what, count, limit });
        }
        // `usize` is 32 bits on the device: never truncate a count.
        let count_usize =
            usize::try_from(count).map_err(|_| DecodeError::OutOfRange { what, value: count })?;
        let needed = count_usize.saturating_mul(min_element_size.max(1));
        if needed > self.remaining() {
            return Err(DecodeError::UnexpectedEnd {
                what,
                needed,
                available: self.remaining(),
            });
        }
        Ok(count_usize)
    }

    /// `Vec<u8>`: a BigSize length then the bytes.
    pub fn read_byte_vec(&mut self, what: &'static str) -> DecodeResult<&'a [u8]> {
        let len = self.read_bigsize(what)?;
        let len = usize::try_from(len).map_err(|_| DecodeError::OutOfRange { what, value: len })?;
        self.read_slice(len, what)
    }

    /// `String`: a `Vec<u8>` holding UTF-8.
    pub fn read_string(&mut self, what: &'static str) -> DecodeResult<&'a str> {
        let bytes = self.read_byte_vec(what)?;
        std::str::from_utf8(bytes).map_err(|error| DecodeError::Invalid {
            what,
            reason: error.to_string(),
        })
    }

    /// A derived enum: the variant index and a reader over exactly that variant's fields. Callers
    /// must [`Reader::finish`] the returned reader.
    pub fn read_variant(&mut self, what: &'static str) -> DecodeResult<(u64, Reader<'a>)> {
        let variant = self.read_bigsize(what)?;
        let fields = self.read_byte_vec(what)?;
        Ok((variant, Reader::new(fields)))
    }
}

/// An append-only output buffer.
#[derive(Debug, Default, Clone)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn write_raw(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    pub fn write_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn write_bool(&mut self, value: bool) {
        self.bytes.push(u8::from(value));
    }

    pub fn write_bigsize(&mut self, value: u64) {
        match value {
            0..=0xfc => self.bytes.push(value as u8),
            0xfd..=0xffff => {
                self.bytes.push(0xfd);
                self.write_raw(&(value as u16).to_be_bytes());
            }
            0x1_0000..=0xffff_ffff => {
                self.bytes.push(0xfe);
                self.write_raw(&(value as u32).to_be_bytes());
            }
            _ => {
                self.bytes.push(0xff);
                self.write_raw(&value.to_be_bytes());
            }
        }
    }

    pub fn write_byte_vec(&mut self, bytes: &[u8]) {
        self.write_bigsize(bytes.len() as u64);
        self.write_raw(bytes);
    }

    pub fn write_string(&mut self, value: &str) {
        self.write_byte_vec(value.as_bytes());
    }

    /// A derived enum variant: the index, then `fields` written into a length-prefixed blob.
    pub fn write_variant(&mut self, variant: u64, fields: impl FnOnce(&mut Writer)) {
        let mut inner = Writer::new();
        fields(&mut inner);
        self.write_bigsize(variant);
        self.write_byte_vec(&inner.bytes);
    }
}

/// A type with a fedimint consensus encoding.
pub trait Encode {
    fn encode(&self, writer: &mut Writer);

    fn to_bytes(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        self.encode(&mut writer);
        writer.into_bytes()
    }
}

/// A type that decodes from its fedimint consensus encoding.
pub trait Decode: Sized {
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self>;

    /// Decodes and refuses trailing bytes.
    fn decode_exact(bytes: &[u8], what: &'static str) -> DecodeResult<Self> {
        let mut reader = Reader::new(bytes);
        let value = Self::decode(&mut reader)?;
        reader.finish(what)?;
        Ok(value)
    }
}
