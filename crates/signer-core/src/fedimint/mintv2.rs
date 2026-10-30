//! The `mintv2` module (`fedimint-mintv2-common` 0.13).

use bitcoin::secp256k1::PublicKey;
use bls12_381::G1Affine;

use super::primitives::{read_g1, read_public_key};
use crate::encoding::{Decode, DecodeError, DecodeResult, Encode, Reader, Writer};

pub const KIND: &str = "mintv2";

/// Largest denomination exponent a federation issues: `consensus_denominations()` is `0..42`.
pub const MAX_DENOMINATION: u8 = 41;

/// `Denomination::amount`: `2^denomination` msat.
pub fn denomination_amount_msat(denomination: u8) -> Option<u64> {
    (denomination <= MAX_DENOMINATION).then(|| 1u64 << denomination)
}

/// `Note { denomination, nonce, signature }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Note {
    pub denomination: u8,
    /// The note's spend public key.
    pub nonce: PublicKey,
    /// The unblinded mint signature on `nonce_message(nonce)`.
    pub signature: G1Affine,
}

impl Encode for Note {
    fn encode(&self, writer: &mut Writer) {
        writer.write_u8(self.denomination);
        writer.write_raw(&self.nonce.serialize());
        writer.write_raw(&self.signature.to_compressed());
    }
}

impl Decode for Note {
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            denomination: reader.read_u8("note denomination")?,
            nonce: read_public_key(reader, "note nonce")?,
            signature: read_g1(reader, "note signature")?,
        })
    }
}

/// `MintInput::V0(MintInputV0 { note })`, the module payload of a mint v2 input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MintInput {
    pub note: Note,
}

impl Encode for MintInput {
    fn encode(&self, writer: &mut Writer) {
        writer.write_variant(0, |fields| self.note.encode(fields));
    }
}

impl Decode for MintInput {
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        let (variant, mut fields) = reader.read_variant("mint input")?;
        if variant != 0 {
            return Err(DecodeError::UnknownVariant {
                what: "mint input",
                variant,
            });
        }
        let note = Note::decode(&mut fields)?;
        fields.finish("mint input")?;
        Ok(Self { note })
    }
}

/// `MintOutput::V0(MintOutputV0 { denomination, nonce, tweak })`, the module payload of a mint v2
/// output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MintOutput {
    pub denomination: u8,
    /// The blinded nonce message the mint signs.
    pub nonce: G1Affine,
    /// Public per-note randomness the client derives the note's keys from.
    pub tweak: [u8; 16],
}

impl Encode for MintOutput {
    fn encode(&self, writer: &mut Writer) {
        writer.write_variant(0, |fields| {
            fields.write_u8(self.denomination);
            fields.write_raw(&self.nonce.to_compressed());
            fields.write_raw(&self.tweak);
        });
    }
}

impl Decode for MintOutput {
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        let (variant, mut fields) = reader.read_variant("mint output")?;
        if variant != 0 {
            return Err(DecodeError::UnknownVariant {
                what: "mint output",
                variant,
            });
        }
        let output = Self {
            denomination: fields.read_u8("output denomination")?,
            nonce: read_g1(&mut fields, "output nonce")?,
            tweak: fields.read_array("output tweak")?,
        };
        fields.finish("mint output")?;
        Ok(output)
    }
}
