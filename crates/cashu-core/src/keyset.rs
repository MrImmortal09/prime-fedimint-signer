//! Mint keysets: NUT-02 keyset ids, and validating a mint's descriptor before Prime pins it.
//!
//! Mint keys are trusted on first use from the paired phone, as federation configs are (decision
//! D12). What Prime checks is that every keyset id is derived from its keys, so the keys it pins
//! are exactly the ones the ids, which proofs carry, name. A version `01` id also commits to the
//! unit, the fee and the final expiry; a version `00` id commits to the keys only.
//!
//! Neither version ties a key to its amount: an id is derived from whatever amount each key is
//! listed under. A melt's limit depends on that tie, because its message does not commit to input
//! amounts and Prime counts each input at the amount its DLEQ proof's key is pinned for. So once a
//! key is pinned for an amount it keeps it: no key may stand for two amounts, an update cannot
//! change a pinned keyset, and a keyset the mint stops listing stays pinned.

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::PublicKey;
use fps_protocol::{CashuKeyset, CashuMintDescriptor, CashuMintSummary, FixedBytes, RejectCode};
use fps_signer_core::error::{SignerError, SignerResult};

use crate::{hex, keys, url};

/// The only unit this version accepts.
pub const UNIT: &str = "sat";
/// Most keysets a mint descriptor may carry.
pub const MAX_KEYSETS: usize = 16;
/// Most keys a keyset may have: one per power of two in a `u64`.
pub const MAX_KEYS: usize = 64;
/// Highest input fee accepted: 100 sats per input. Mints charge around 0.1.
pub const MAX_INPUT_FEE_PPK: u64 = 100_000;

/// Version byte and length of a version `00` keyset id.
pub const VERSION_00: u8 = 0x00;
pub const ID_LEN_00: usize = 8;
/// Version byte and length of a version `01` keyset id.
pub const VERSION_01: u8 = 0x01;
pub const ID_LEN_01: usize = 33;

/// A version `00` id: `00` and the first 7 bytes of SHA-256 over the compressed keys, in increasing
/// order of amount.
pub fn id_v00(keys: &[(u64, PublicKey)]) -> [u8; ID_LEN_00] {
    let mut sorted = keys.to_vec();
    sorted.sort_by_key(|(amount, _)| *amount);
    let concatenated: Vec<u8> = sorted.iter().flat_map(|(_, key)| key.serialize()).collect();
    versioned(
        VERSION_00,
        &sha256::Hash::hash(&concatenated).to_byte_array(),
    )
}

/// The version byte, then as much of the hash as fits.
fn versioned<const N: usize>(version: u8, hash: &[u8; 32]) -> [u8; N] {
    let mut id = [version; N];
    id.iter_mut()
        .skip(1)
        .zip(hash)
        .for_each(|(slot, byte)| *slot = *byte);
    id
}

/// A version `01` id: `01` and the SHA-256 of `amount:key_hex,…|unit:<unit>`, followed by
/// `|input_fee_ppk:<fee>` when the fee is not zero and `|final_expiry:<time>` when there is a
/// non-zero final expiry, keys in increasing order of amount (`Id::v2_from_data` in `cashu` 0.18.3).
pub fn id_v01(
    keys: &[(u64, PublicKey)],
    unit: &str,
    input_fee_ppk: u64,
    final_expiry: Option<u64>,
) -> [u8; ID_LEN_01] {
    let mut sorted = keys.to_vec();
    sorted.sort_by_key(|(amount, _)| *amount);
    let mut preimage = String::new();
    for (index, (amount, key)) in sorted.iter().enumerate() {
        if index > 0 {
            preimage.push(',');
        }
        preimage.push_str(&amount.to_string());
        preimage.push(':');
        hex::push(&mut preimage, &key.serialize());
    }
    preimage.push_str("|unit:");
    preimage.push_str(unit);
    if input_fee_ppk > 0 {
        preimage.push_str(&format!("|input_fee_ppk:{input_fee_ppk}"));
    }
    if let Some(expiry) = final_expiry.filter(|expiry| *expiry > 0) {
        preimage.push_str(&format!("|final_expiry:{expiry}"));
    }
    versioned(
        VERSION_01,
        &sha256::Hash::hash(preimage.as_bytes()).to_byte_array(),
    )
}

/// A keyset Prime has pinned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedKeyset {
    pub id: Vec<u8>,
    pub input_fee_ppk: u64,
    pub final_expiry: Option<u64>,
    pub active: bool,
    /// Key per amount; every amount is a power of two.
    pub keys: BTreeMap<u64, PublicKey>,
}

impl PinnedKeyset {
    /// The key for `amount`, if it is a power of two this keyset signs.
    pub fn key(&self, amount: u64) -> Option<&PublicKey> {
        if !amount.is_power_of_two() {
            return None;
        }
        self.keys.get(&amount)
    }

    /// Whether the id is of version `00`, which commits to neither the unit nor the fee.
    pub fn is_v00(&self) -> bool {
        self.id.first() == Some(&VERSION_00)
    }
}

/// A mint as Prime pins it: the keysets of every descriptor approved for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedMint {
    /// SHA-256 of the canonical URL.
    pub id: [u8; 32],
    pub url: String,
    pub unit: String,
    /// In the latest descriptor's order, then the keysets pinned before that it no longer lists.
    pub keysets: Vec<PinnedKeyset>,
}

impl PinnedMint {
    /// The pinned keyset with this id.
    pub fn keyset(&self, id: &[u8]) -> Option<&PinnedKeyset> {
        self.keysets.iter().find(|keyset| keyset.id == id)
    }

    pub fn summary(&self) -> CashuMintSummary {
        CashuMintSummary {
            mint: FixedBytes(self.id),
            url: self.url.clone(),
            unit: self.unit.clone(),
            keysets: u32::try_from(self.keysets.len()).unwrap_or(u32::MAX),
        }
    }
}

fn invalid(message: impl Into<String>) -> SignerError {
    SignerError::new(RejectCode::InvalidDescriptor, message)
}

/// Short form of a keyset id for messages and reviews: its first 8 bytes in hex.
pub fn short_id(id: &[u8]) -> String {
    hex::encode(id.get(..8).unwrap_or(id))
}

/// Validates a mint descriptor and returns what Prime pins for the mint: a canonical URL, unit
/// `sat`, 1 to [`MAX_KEYSETS`] keysets with distinct ids, each with 1 to [`MAX_KEYS`] valid keys in
/// strictly increasing power-of-two amounts, a fee of at most [`MAX_INPUT_FEE_PPK`], and an id
/// derived from its keys; at least one keyset active; and no key under two amounts.
///
/// `existing` is what Prime has pinned for this mint already, when the descriptor updates it. An
/// update adds keysets and moves the `active` flags, nothing else: a keyset already pinned must come
/// back with the same keys and fee, one the mint no longer lists stays pinned as inactive, and no
/// key may stand for two amounts across all of them, counting at most [`MAX_KEYSETS`].
pub fn validate_mint(
    descriptor: &CashuMintDescriptor,
    existing: Option<&PinnedMint>,
) -> SignerResult<PinnedMint> {
    url::check_mint_url(&descriptor.url)?;
    let id = keys::mint_id(&descriptor.url);
    if existing.is_some_and(|existing| existing.id != id) {
        return Err(SignerError::new(
            RejectCode::Internal,
            "the mint pinned before is another mint",
        ));
    }
    if descriptor.unit != UNIT {
        return Err(unsupported_unit(&descriptor.unit));
    }
    if descriptor.keysets.is_empty() {
        return Err(invalid("the mint lists no keysets"));
    }
    if descriptor.keysets.len() > MAX_KEYSETS {
        return Err(invalid(format!("more than {MAX_KEYSETS} keysets")));
    }
    let mut keysets: Vec<PinnedKeyset> = Vec::with_capacity(descriptor.keysets.len());
    for keyset in &descriptor.keysets {
        let pinned = validate_keyset(keyset)?;
        if keysets.iter().any(|other| other.id == pinned.id) {
            return Err(invalid(format!(
                "keyset {} is listed twice",
                short_id(&pinned.id)
            )));
        }
        keysets.push(pinned);
    }
    if !keysets.iter().any(|keyset| keyset.active) {
        return Err(invalid("the mint has no active keyset"));
    }
    let pinned = existing.map_or(&[][..], |existing| existing.keysets.as_slice());
    for keyset in &keysets {
        let changed = pinned.iter().any(|old| {
            old.id == keyset.id
                && (old.keys != keyset.keys || old.input_fee_ppk != keyset.input_fee_ppk)
        });
        if changed {
            return Err(invalid(format!(
                "keyset {}: its keys or fee differ from those pinned under the same id",
                short_id(&keyset.id)
            )));
        }
    }
    let listed = |id: &[u8]| keysets.iter().any(|keyset| keyset.id == id);
    let unlisted = pinned.iter().filter(|old| !listed(&old.id)).count();
    if keysets.len() + unlisted > MAX_KEYSETS {
        return Err(invalid(format!(
            "more than {MAX_KEYSETS} keysets, counting those pinned before that the mint no longer lists"
        )));
    }
    // Every pinned keyset, then the new ones (a listed pinned keyset is the same), so a conflict
    // names the keyset that is new.
    let new = keysets
        .iter()
        .filter(|keyset| pinned.iter().all(|old| old.id != keyset.id));
    one_amount_per_key(pinned.iter().chain(new))?;
    for old in pinned {
        if keysets.iter().all(|keyset| keyset.id != old.id) {
            keysets.push(PinnedKeyset {
                active: false,
                ..old.clone()
            });
        }
    }
    Ok(PinnedMint {
        id,
        url: descriptor.url.clone(),
        unit: descriptor.unit.clone(),
        keysets,
    })
}

/// Refuses a key listed under two amounts, in one keyset or across keysets. Every proof's amount
/// is read off the key that signed it, so each key must name one amount.
fn one_amount_per_key<'a>(keysets: impl Iterator<Item = &'a PinnedKeyset>) -> SignerResult<()> {
    let mut amounts: BTreeMap<[u8; 33], (u64, &[u8])> = BTreeMap::new();
    for keyset in keysets {
        for (amount, key) in &keyset.keys {
            match amounts.entry(key.serialize()) {
                Entry::Vacant(slot) => {
                    slot.insert((*amount, &keyset.id));
                }
                Entry::Occupied(slot) => {
                    let (first, first_keyset) = *slot.get();
                    if first != *amount {
                        return Err(invalid(format!(
                            "keyset {}: its key for {amount} is the key for {first} in keyset {}, and a key stands for one amount",
                            short_id(&keyset.id),
                            short_id(first_keyset)
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

fn unsupported_unit(unit: &str) -> SignerError {
    let shown: String = unit.chars().filter(|c| !c.is_control()).take(16).collect();
    SignerError::new(
        RejectCode::Unsupported,
        format!("unit {shown:?} is not {UNIT:?}, the only unit this version supports"),
    )
}

fn validate_keyset(keyset: &CashuKeyset) -> SignerResult<PinnedKeyset> {
    let id = keyset.id.as_slice();
    let shown = short_id(id);
    if keyset.unit != UNIT {
        return Err(unsupported_unit(&keyset.unit));
    }
    match (id.first(), id.len()) {
        (Some(&VERSION_00), ID_LEN_00) | (Some(&VERSION_01), ID_LEN_01) => {}
        _ => {
            return Err(invalid(format!(
                "keyset {shown}: the id is neither 8 bytes of version 00 nor 33 bytes of version 01"
            )))
        }
    }
    if keyset.keys.is_empty() || keyset.keys.len() > MAX_KEYS {
        return Err(invalid(format!(
            "keyset {shown}: {} keys; between 1 and {MAX_KEYS} are accepted",
            keyset.keys.len()
        )));
    }
    if keyset.input_fee_ppk > MAX_INPUT_FEE_PPK {
        return Err(invalid(format!(
            "keyset {shown}: an input fee of {} ppk is above the limit of {MAX_INPUT_FEE_PPK}",
            keyset.input_fee_ppk
        )));
    }
    let mut keys = Vec::with_capacity(keyset.keys.len());
    let mut previous = 0u64;
    for key in &keyset.keys {
        if !key.amount.is_power_of_two() || key.amount <= previous {
            return Err(invalid(format!(
                "keyset {shown}: amounts must be powers of two in increasing order, and {} is not",
                key.amount
            )));
        }
        previous = key.amount;
        let point = PublicKey::from_slice(&key.pubkey.0).map_err(|_| {
            invalid(format!(
                "keyset {shown}: the key for {} is not a valid point",
                key.amount
            ))
        })?;
        keys.push((key.amount, point));
    }
    let derived = match id.first() {
        Some(&VERSION_00) => id_v00(&keys).to_vec(),
        _ => id_v01(
            &keys,
            &keyset.unit,
            keyset.input_fee_ppk,
            keyset.final_expiry,
        )
        .to_vec(),
    };
    if derived != id {
        return Err(invalid(format!(
            "keyset {shown}: the id is not derived from its keys"
        )));
    }
    Ok(PinnedKeyset {
        id: id.to_vec(),
        input_fee_ppk: keyset.input_fee_ppk,
        final_expiry: keyset.final_expiry,
        active: keyset.active,
        keys: keys.into_iter().collect(),
    })
}
