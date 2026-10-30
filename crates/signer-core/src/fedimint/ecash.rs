//! Mint v2 out-of-band ecash tokens (`fedimint-mintv2-client::ecash::ECash`).
//!
//! A token is `ECash(Vec<ECashField>)` with fields `Mint(FederationId)`, then one
//! `Note(SpendableNote)` per note, then optionally `Invite { peer_apis, federation_id }`. It travels
//! as `fedimint` + base32. The notes carry their secret keys: whoever holds the token can spend it.

use bls12_381::G1Affine;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::base32;
use crate::encoding::Writer;

/// `ECashField` variant indices, in declaration order.
const FIELD_MINT: u64 = 0;
const FIELD_NOTE: u64 = 1;
const FIELD_INVITE: u64 = 2;

/// `SpendableNote { denomination, keypair, signature }`; the keypair encodes as its secret key.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SpendableNote {
    pub denomination: u8,
    pub secret_key: [u8; 32],
    #[zeroize(skip)]
    pub signature: G1Affine,
}

/// The invite part: one guardian's API URL, so a receiver who has not joined can redeem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub peer: u16,
    pub url: String,
}

/// A mint v2 ecash token.
#[derive(Clone)]
pub struct EcashToken {
    pub federation: [u8; 32],
    pub notes: Vec<SpendableNote>,
    pub invite: Option<Invite>,
}

impl EcashToken {
    /// Total value in msat, `None` on an invalid denomination.
    pub fn amount_msat(&self) -> Option<u64> {
        self.notes.iter().try_fold(0u64, |total, note| {
            total.checked_add(super::mintv2::denomination_amount_msat(note.denomination)?)
        })
    }

    /// The consensus encoding. Treat the result as secret.
    pub fn to_bytes(&self) -> Vec<u8> {
        let fields = 1 + self.notes.len() + usize::from(self.invite.is_some());
        let mut writer = Writer::new();
        writer.write_bigsize(fields as u64);
        writer.write_variant(FIELD_MINT, |field| field.write_raw(&self.federation));
        for note in &self.notes {
            writer.write_variant(FIELD_NOTE, |field| {
                field.write_u8(note.denomination);
                field.write_raw(&note.secret_key);
                field.write_raw(&note.signature.to_compressed());
            });
        }
        if let Some(invite) = &self.invite {
            writer.write_variant(FIELD_INVITE, |field| {
                // `peer_apis: Vec<(PeerId, SafeUrl)>`, then `federation_id`.
                field.write_bigsize(1);
                field.write_bigsize(u64::from(invite.peer));
                field.write_string(&invite.url);
                field.write_raw(&self.federation);
            });
        }
        writer.into_bytes()
    }

    /// The `fedimint…` string a wallet redeems. Treat the result as secret.
    pub fn to_token_string(&self) -> String {
        let mut bytes = self.to_bytes();
        let token = base32::encode_prefixed(&bytes);
        bytes.zeroize();
        token
    }
}
