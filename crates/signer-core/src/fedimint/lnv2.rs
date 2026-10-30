//! The `lnv2` module (`fedimint-lnv2-common` 0.13): outgoing contracts and their refund inputs.
//! Incoming contracts are recognised only so they can be refused.

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::PublicKey;

use super::primitives::{read_public_key, OutPoint};
use crate::encoding::{Decode, DecodeError, DecodeResult, Encode, Reader, Writer};

pub const KIND: &str = "lnv2";

/// `PaymentImage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentImage {
    Hash([u8; 32]),
    Point(PublicKey),
}

impl Encode for PaymentImage {
    fn encode(&self, writer: &mut Writer) {
        match self {
            Self::Hash(hash) => writer.write_variant(0, |fields| fields.write_raw(hash)),
            Self::Point(point) => {
                writer.write_variant(1, |fields| fields.write_raw(&point.serialize()))
            }
        }
    }
}

impl Decode for PaymentImage {
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        let (variant, mut fields) = reader.read_variant("payment image")?;
        let image = match variant {
            0 => Self::Hash(fields.read_array("payment hash")?),
            1 => Self::Point(read_public_key(&mut fields, "payment point")?),
            variant => {
                return Err(DecodeError::UnknownVariant {
                    what: "payment image",
                    variant,
                })
            }
        };
        fields.finish("payment image")?;
        Ok(image)
    }
}

/// `OutgoingContract`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutgoingContract {
    pub payment_image: PaymentImage,
    pub amount_msat: u64,
    /// Block height after which the refund key can reclaim the contract.
    pub expiration: u64,
    /// The gateway's key; it claims the contract with the preimage.
    pub claim_pk: PublicKey,
    /// The payer's key; it reclaims the contract after expiry or after the gateway cancels.
    pub refund_pk: PublicKey,
    pub ephemeral_pk: PublicKey,
}

impl OutgoingContract {
    /// `ContractId`: SHA-256 of the encoding.
    pub fn contract_id(&self) -> [u8; 32] {
        sha256::Hash::hash(&self.to_bytes()).to_byte_array()
    }
}

impl Encode for OutgoingContract {
    fn encode(&self, writer: &mut Writer) {
        self.payment_image.encode(writer);
        writer.write_bigsize(self.amount_msat);
        writer.write_bigsize(self.expiration);
        writer.write_raw(&self.claim_pk.serialize());
        writer.write_raw(&self.refund_pk.serialize());
        writer.write_raw(&self.ephemeral_pk.serialize());
    }
}

impl Decode for OutgoingContract {
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(Self {
            payment_image: PaymentImage::decode(reader)?,
            amount_msat: reader.read_u64("contract amount")?,
            expiration: reader.read_u64("contract expiration")?,
            claim_pk: read_public_key(reader, "contract claim key")?,
            refund_pk: read_public_key(reader, "contract refund key")?,
            ephemeral_pk: read_public_key(reader, "contract ephemeral key")?,
        })
    }
}

/// `LightningOutput::V0(LightningOutputV0)`.
// Decoded once per output, so the size of the largest variant does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LightningOutput {
    Outgoing(OutgoingContract),
    /// `Incoming(IncomingContract)`; only gateways create these. Kept as raw variant bytes.
    Incoming(Vec<u8>),
}

impl Encode for LightningOutput {
    fn encode(&self, writer: &mut Writer) {
        match self {
            Self::Outgoing(contract) => writer.write_variant(0, |v0| {
                v0.write_variant(0, |fields| contract.encode(fields));
            }),
            Self::Incoming(raw) => writer.write_variant(0, |v0| {
                v0.write_variant(1, |fields| fields.write_raw(raw));
            }),
        }
    }
}

impl Decode for LightningOutput {
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        let mut v0 = read_v0(reader, "lightning output")?;
        let (variant, mut fields) = v0.read_variant("lightning output v0")?;
        v0.finish("lightning output")?;
        let output = match variant {
            0 => Self::Outgoing(OutgoingContract::decode(&mut fields)?),
            1 => {
                return Ok(Self::Incoming(
                    fields
                        .read_slice(fields.remaining(), "incoming contract")?
                        .to_vec(),
                ))
            }
            variant => {
                return Err(DecodeError::UnknownVariant {
                    what: "lightning output v0",
                    variant,
                })
            }
        };
        fields.finish("outgoing contract")?;
        Ok(output)
    }
}

/// `OutgoingWitness`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutgoingWitness {
    /// The gateway claims with the preimage. Never the device's.
    Claim([u8; 32]),
    /// The payer reclaims after expiry.
    Refund,
    /// The payer reclaims early with the gateway's forfeit signature.
    Cancel([u8; 64]),
}

impl Encode for OutgoingWitness {
    fn encode(&self, writer: &mut Writer) {
        match self {
            Self::Claim(preimage) => writer.write_variant(0, |fields| fields.write_raw(preimage)),
            Self::Refund => writer.write_variant(1, |_| {}),
            Self::Cancel(signature) => {
                writer.write_variant(2, |fields| fields.write_raw(signature))
            }
        }
    }
}

impl Decode for OutgoingWitness {
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        let (variant, mut fields) = reader.read_variant("outgoing witness")?;
        let witness = match variant {
            0 => Self::Claim(fields.read_array("preimage")?),
            1 => Self::Refund,
            2 => Self::Cancel(fields.read_array("forfeit signature")?),
            variant => {
                return Err(DecodeError::UnknownVariant {
                    what: "outgoing witness",
                    variant,
                })
            }
        };
        fields.finish("outgoing witness")?;
        Ok(witness)
    }
}

/// `LightningInput::V0(LightningInputV0)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LightningInput {
    Outgoing {
        outpoint: OutPoint,
        witness: OutgoingWitness,
    },
    /// `Incoming(OutPoint, AggregateDecryptionKey)`; receives are signed by the phone. Kept as raw
    /// variant bytes.
    Incoming(Vec<u8>),
}

impl Encode for LightningInput {
    fn encode(&self, writer: &mut Writer) {
        match self {
            Self::Outgoing { outpoint, witness } => writer.write_variant(0, |v0| {
                v0.write_variant(0, |fields| {
                    outpoint.encode(fields);
                    witness.encode(fields);
                });
            }),
            Self::Incoming(raw) => writer.write_variant(0, |v0| {
                v0.write_variant(1, |fields| fields.write_raw(raw));
            }),
        }
    }
}

impl Decode for LightningInput {
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        let mut v0 = read_v0(reader, "lightning input")?;
        let (variant, mut fields) = v0.read_variant("lightning input v0")?;
        v0.finish("lightning input")?;
        let input = match variant {
            0 => Self::Outgoing {
                outpoint: OutPoint::decode(&mut fields)?,
                witness: OutgoingWitness::decode(&mut fields)?,
            },
            1 => {
                return Ok(Self::Incoming(
                    fields
                        .read_slice(fields.remaining(), "incoming input")?
                        .to_vec(),
                ))
            }
            variant => {
                return Err(DecodeError::UnknownVariant {
                    what: "lightning input v0",
                    variant,
                })
            }
        };
        fields.finish("lightning input")?;
        Ok(input)
    }
}

/// The outer `extensible_associated_module_type!` wrapper: only `V0` is understood.
fn read_v0<'a>(reader: &mut Reader<'a>, what: &'static str) -> DecodeResult<Reader<'a>> {
    let (variant, fields) = reader.read_variant(what)?;
    if variant != 0 {
        return Err(DecodeError::UnknownVariant { what, variant });
    }
    Ok(fields)
}
