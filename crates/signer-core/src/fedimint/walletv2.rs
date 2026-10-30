//! The `walletv2` module (`fedimint-walletv2-common` 0.13): peg-out outputs.

use bitcoin::secp256k1::XOnlyPublicKey;
use bitcoin::{Address, Network, ScriptBuf};

use super::primitives::read_xonly_key;
use crate::encoding::{Decode, DecodeError, DecodeResult, Encode, Reader, Writer};

pub const KIND: &str = "walletv2";

/// `StandardScript`: the destination of a peg-out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardScript {
    P2pkh([u8; 20]),
    P2sh([u8; 20]),
    P2wpkh([u8; 20]),
    P2wsh([u8; 32]),
    P2tr(XOnlyPublicKey),
}

impl StandardScript {
    pub fn script_pubkey(&self) -> ScriptBuf {
        use bitcoin::hashes::Hash;
        match self {
            Self::P2pkh(hash) => ScriptBuf::new_p2pkh(&bitcoin::PubkeyHash::from_byte_array(*hash)),
            Self::P2sh(hash) => ScriptBuf::new_p2sh(&bitcoin::ScriptHash::from_byte_array(*hash)),
            Self::P2wpkh(hash) => {
                ScriptBuf::new_p2wpkh(&bitcoin::WPubkeyHash::from_byte_array(*hash))
            }
            Self::P2wsh(hash) => {
                ScriptBuf::new_p2wsh(&bitcoin::WScriptHash::from_byte_array(*hash))
            }
            Self::P2tr(key) => ScriptBuf::new_p2tr_tweaked(
                bitcoin::key::TweakedPublicKey::dangerous_assume_tweaked(*key),
            ),
        }
    }

    /// The address a user would recognise, on `network`.
    pub fn address(&self, network: Network) -> Option<String> {
        Address::from_script(&self.script_pubkey(), network)
            .ok()
            .map(|address| address.to_string())
    }
}

impl Encode for StandardScript {
    fn encode(&self, writer: &mut Writer) {
        match self {
            Self::P2pkh(hash) => writer.write_variant(0, |fields| fields.write_raw(hash)),
            Self::P2sh(hash) => writer.write_variant(1, |fields| fields.write_raw(hash)),
            Self::P2wpkh(hash) => writer.write_variant(2, |fields| fields.write_raw(hash)),
            Self::P2wsh(hash) => writer.write_variant(3, |fields| fields.write_raw(hash)),
            Self::P2tr(key) => writer.write_variant(4, |fields| fields.write_raw(&key.serialize())),
        }
    }
}

impl Decode for StandardScript {
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        let (variant, mut fields) = reader.read_variant("peg-out destination")?;
        let script = match variant {
            0 => Self::P2pkh(fields.read_array("p2pkh hash")?),
            1 => Self::P2sh(fields.read_array("p2sh hash")?),
            2 => Self::P2wpkh(fields.read_array("p2wpkh hash")?),
            3 => Self::P2wsh(fields.read_array("p2wsh hash")?),
            4 => Self::P2tr(read_xonly_key(&mut fields, "p2tr key")?),
            variant => {
                return Err(DecodeError::UnknownVariant {
                    what: "peg-out destination",
                    variant,
                })
            }
        };
        fields.finish("peg-out destination")?;
        Ok(script)
    }
}

/// `WalletOutput::V0(WalletOutputV0 { destination, value, fee })`. The module charges its fee on
/// `value + fee`, which is also the amount the output takes from the transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalletOutput {
    pub destination: StandardScript,
    pub value_sat: u64,
    /// On-chain fee the federation's peg-out transaction pays.
    pub fee_sat: u64,
}

impl WalletOutput {
    /// What the output consumes from the transaction, in msat.
    pub fn amount_msat(&self) -> Option<u64> {
        self.value_sat
            .checked_add(self.fee_sat)
            .and_then(|sat| sat.checked_mul(1000))
    }
}

impl Encode for WalletOutput {
    fn encode(&self, writer: &mut Writer) {
        writer.write_variant(0, |fields| {
            self.destination.encode(fields);
            fields.write_bigsize(self.value_sat);
            fields.write_bigsize(self.fee_sat);
        });
    }
}

impl Decode for WalletOutput {
    fn decode(reader: &mut Reader<'_>) -> DecodeResult<Self> {
        let (variant, mut fields) = reader.read_variant("wallet output")?;
        if variant != 0 {
            return Err(DecodeError::UnknownVariant {
                what: "wallet output",
                variant,
            });
        }
        let output = Self {
            destination: StandardScript::decode(&mut fields)?,
            value_sat: fields.read_u64("peg-out value")?,
            fee_sat: fields.read_u64("peg-out fee")?,
        };
        fields.finish("wallet output")?;
        Ok(output)
    }
}
