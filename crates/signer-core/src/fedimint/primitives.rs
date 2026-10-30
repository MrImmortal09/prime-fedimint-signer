//! Shared fedimint primitives: keys, curve points, outpoints and module items.

use bitcoin::secp256k1::{PublicKey, XOnlyPublicKey};
use bls12_381::{G1Affine, G2Affine};

use crate::encoding::{Decode, DecodeError, DecodeResult, Encode, Reader, Writer};

/// `secp256k1::PublicKey`: 33 bytes, compressed.
pub fn read_public_key(reader: &mut Reader<'_>, what: &'static str) -> DecodeResult<PublicKey> {
    let bytes = reader.read_array::<33>(what)?;
    PublicKey::from_slice(&bytes).map_err(|error| DecodeError::Invalid {
        what,
        reason: error.to_string(),
    })
}

/// `secp256k1::XOnlyPublicKey`: 32 bytes.
pub fn read_xonly_key(reader: &mut Reader<'_>, what: &'static str) -> DecodeResult<XOnlyPublicKey> {
    let bytes = reader.read_array::<32>(what)?;
    XOnlyPublicKey::from_slice(&bytes).map_err(|error| DecodeError::Invalid {
        what,
        reason: error.to_string(),
    })
}

/// A BLS12-381 G1 point (tbs `Signature`, `BlindedMessage`): 48 bytes, compressed. Rejects points
/// off the curve or outside the prime-order subgroup, as fedimint does.
pub fn read_g1(reader: &mut Reader<'_>, what: &'static str) -> DecodeResult<G1Affine> {
    let bytes = reader.read_array::<48>(what)?;
    g1_from_bytes(&bytes).ok_or(DecodeError::Invalid {
        what,
        reason: "not a valid compressed G1 point".to_owned(),
    })
}

pub fn g1_from_bytes(bytes: &[u8; 48]) -> Option<G1Affine> {
    Option::from(G1Affine::from_compressed(bytes))
}

/// A BLS12-381 G2 point (tbs `AggregatePublicKey`): 96 bytes, compressed.
pub fn g2_from_bytes(bytes: &[u8; 96]) -> Option<G2Affine> {
    Option::from(G2Affine::from_compressed(bytes))
}

/// fedimint `OutPoint { txid: TransactionId, out_idx: u64 }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutPoint {
    pub txid: [u8; 32],
    pub out_idx: u64,
}

impl Encode for OutPoint {
    fn encode(&self, writer: &mut Writer) {
        writer.write_raw(&self.txid);
        writer.write_bigsize(self.out_idx);
    }
}

impl Decode for OutPoint {
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            txid: reader.read_array("outpoint txid")?,
            out_idx: reader.read_u64("outpoint index")?,
        })
    }
}

/// A module item as fedimint frames it in a transaction (`DynInput`, `DynOutput`): the module
/// instance id, then the module's own encoding as a length-prefixed byte string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynItem {
    pub instance: u16,
    pub payload: Vec<u8>,
}

impl Encode for DynItem {
    fn encode(&self, writer: &mut Writer) {
        writer.write_bigsize(u64::from(self.instance));
        writer.write_byte_vec(&self.payload);
    }
}

impl Decode for DynItem {
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            instance: reader.read_u16("module instance id")?,
            payload: reader.read_byte_vec("module item")?.to_vec(),
        })
    }
}
