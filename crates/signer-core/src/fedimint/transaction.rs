//! The part of a fedimint `Transaction` the signature commits to.

use bitcoin::hashes::{sha256, Hash};

use super::primitives::DynItem;
use crate::encoding::{Decode, DecodeError, DecodeResult, Encode, Reader, Writer};

/// Most items accepted on either side of a transaction. Real transactions use a few dozen; the
/// device caps work per request well below this through its policy.
pub const MAX_ITEMS: u64 = 256;

/// `inputs`, `outputs` and `nonce` of fedimint's `Transaction`, without `signatures`.
///
/// Its encoding is exactly what `Transaction::tx_hash_from_parts` feeds into SHA-256, so the
/// transaction id is the hash of the bytes the phone sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsignedTransaction {
    pub inputs: Vec<DynItem>,
    pub outputs: Vec<DynItem>,
    pub nonce: [u8; 8],
}

impl UnsignedTransaction {
    /// Decodes and requires the input to be the canonical encoding of what was decoded, so the
    /// device hashes and signs exactly what it reviewed.
    pub fn decode_canonical(bytes: &[u8]) -> DecodeResult<Self> {
        let tx = Self::decode_exact(bytes, "transaction")?;
        if tx.to_bytes() != bytes {
            return Err(DecodeError::Invalid {
                what: "transaction",
                reason: "not in canonical encoding".to_owned(),
            });
        }
        Ok(tx)
    }

    /// fedimint `TransactionId`: SHA-256 of the encoding.
    pub fn txid(&self) -> [u8; 32] {
        sha256::Hash::hash(&self.to_bytes()).to_byte_array()
    }
}

fn read_items(reader: &mut Reader<'_>, what: &'static str) -> DecodeResult<Vec<DynItem>> {
    // An item is at least two bytes: the instance id and an empty payload's length.
    let count = reader.read_len(what, 2, MAX_ITEMS)?;
    (0..count).map(|_| DynItem::decode(reader)).collect()
}

fn write_items(writer: &mut Writer, items: &[DynItem]) {
    writer.write_bigsize(items.len() as u64);
    for item in items {
        item.encode(writer);
    }
}

impl Encode for UnsignedTransaction {
    fn encode(&self, writer: &mut Writer) {
        write_items(writer, &self.inputs);
        write_items(writer, &self.outputs);
        writer.write_raw(&self.nonce);
    }
}

impl Decode for UnsignedTransaction {
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            inputs: read_items(reader, "transaction inputs")?,
            outputs: read_items(reader, "transaction outputs")?,
            nonce: reader.read_array("transaction nonce")?,
        })
    }
}
