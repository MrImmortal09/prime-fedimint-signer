//! The `fps-cashu-v1` key scheme. Specified in `docs/KEY_SCHEME_CASHU.md`; this is the reference
//! implementation, shared by Prime and the phone.
//!
//! Per mint, Prime derives a lock root `p` and a view key `v` from its app seed and the mint's
//! canonical URL. The phone receives `(P = p·G, v)`:
//!
//! - the lock key of epoch `e` is `P_e = P + t_e·G` with `t_e` from `v`, so the phone derives every
//!   lock key, and only Prime holds `k_e = p + t_e`;
//! - the secret nonce and blinding factor of every output come from `v`, so receiving, change and
//!   restoring need no Prime.
//!
//! The tweak is not hardened: `k_e` minus the public `t_e` is `p`. That is safe because Prime never
//! releases a Cashu private key; decision D11 is about keys that leave the device.

use bitcoin::hashes::{hmac, sha256, sha512, Hash, HashEngine};
use bitcoin::secp256k1::{self, Keypair, PublicKey, Scalar, Secp256k1, SecretKey};
use fps_protocol::{CashuLockMaterial, FixedBytes, CASHU_KEY_SCHEME};
use zeroize::Zeroizing;

use crate::{dhke, hex, secret};

/// Scheme name reported to the phone.
pub const SCHEME: &str = CASHU_KEY_SCHEME;

/// Per-mint root: `HMAC512(app_seed, TAG_MINT || mint_id)`.
pub const TAG_MINT: &[u8] = b"fps-cashu-v1/mint";
/// Lock key tweak of an epoch: `HMAC512(v, TAG_LOCK || ser32(e))`.
pub const TAG_LOCK: &[u8] = b"fps-cashu-v1/lock";
/// Nonce and blinding factor of an output: `HMAC512(v, TAG_OUTPUT || ser32(e) || len(K) || K ||
/// ser64(c))`.
pub const TAG_OUTPUT: &[u8] = b"fps-cashu-v1/output";
/// Every tag of the scheme. Each starts with `fps-cashu-v1/`, which differs from the Fedimint
/// scheme's `fps-v1/` at byte 4, so no Cashu derivation shares an HMAC message with a Fedimint one.
pub const TAGS: [&[u8]; 3] = [TAG_MINT, TAG_LOCK, TAG_OUTPUT];

/// Why a derivation produced no key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// A scalar was 0 or not below the group order, or a key landed on the point at infinity.
    /// Happens with probability about 2^-128; the phone treats that position as unusable.
    InvalidScalar,
    /// `len(K)` is one byte, so a keyset id may have at most 255 bytes. Real ids have 8 or 33.
    KeysetIdTooLong,
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidScalar => f.write_str("key derivation produced an invalid scalar"),
            Self::KeysetIdTooLong => f.write_str("a keyset id longer than 255 bytes"),
        }
    }
}

impl std::error::Error for KeyError {}

impl From<secp256k1::Error> for KeyError {
    fn from(_: secp256k1::Error) -> Self {
        Self::InvalidScalar
    }
}

/// The mint id of a canonical mint URL: its SHA-256.
pub fn mint_id(canonical_url: &str) -> [u8; 32] {
    sha256::Hash::hash(canonical_url.as_bytes()).to_byte_array()
}

fn hmac512(key: &[u8], parts: &[&[u8]]) -> Zeroizing<[u8; 64]> {
    let mut engine = hmac::HmacEngine::<sha512::Hash>::new(key);
    for part in parts {
        engine.input(part);
    }
    Zeroizing::new(hmac::Hmac::<sha512::Hash>::from_engine(engine).to_byte_array())
}

fn first_half(bytes: &[u8; 64]) -> Zeroizing<[u8; 32]> {
    let mut half = Zeroizing::new([0u8; 32]);
    half.iter_mut()
        .zip(bytes)
        .for_each(|(slot, byte)| *slot = *byte);
    half
}

fn second_half(bytes: &[u8; 64]) -> Zeroizing<[u8; 32]> {
    let mut half = Zeroizing::new([0u8; 32]);
    half.iter_mut()
        .zip(bytes.iter().skip(32))
        .for_each(|(slot, byte)| *slot = *byte);
    half
}

/// `parse256`: 32 bytes big-endian as a scalar, failing on 0 and on anything not below `n`.
fn parse256(bytes: &[u8; 32]) -> Result<SecretKey, KeyError> {
    SecretKey::from_slice(bytes).map_err(|_| KeyError::InvalidScalar)
}

/// The output of `fps-cashu-v1` at one position, opened: what a change output must be.
#[derive(Clone, PartialEq, Eq)]
pub struct DerivedOutput {
    /// The template secret, locked to the epoch's lock key.
    pub secret: String,
    /// The blinding factor.
    pub r: SecretKey,
    /// `B_ = hash_to_curve(secret) + r·G`.
    pub b: PublicKey,
}

/// The watch-only half: what the phone holds for a mint.
#[derive(Clone)]
pub struct PhoneMintKeys {
    lock_root: PublicKey,
    view_key: Zeroizing<[u8; 32]>,
}

impl PhoneMintKeys {
    pub fn new(lock_root: PublicKey, view_key: [u8; 32]) -> Self {
        Self {
            lock_root,
            view_key: Zeroizing::new(view_key),
        }
    }

    /// `P`.
    pub fn lock_root(&self) -> PublicKey {
        self.lock_root
    }

    /// `v`.
    pub fn view_key(&self) -> [u8; 32] {
        *self.view_key
    }

    /// `t_e`, the public tweak from `P` to the lock key of epoch `e`.
    pub fn lock_tweak(&self, epoch: u32) -> Result<SecretKey, KeyError> {
        let i = hmac512(&*self.view_key, &[TAG_LOCK, &epoch.to_be_bytes()]);
        parse256(&first_half(&i))
    }

    /// `P_e = P + t_e·G`, the key every proof of epoch `e` is locked to.
    pub fn lock_pubkey(&self, epoch: u32) -> Result<PublicKey, KeyError> {
        let t = self.lock_tweak(epoch)?;
        Ok(self
            .lock_root
            .add_exp_tweak(&Secp256k1::verification_only(), &Scalar::from(t))?)
    }

    /// The nonce's bytes and `r`.
    pub(crate) fn output_parts(
        &self,
        epoch: u32,
        keyset_id: &[u8],
        counter: u64,
    ) -> Result<(Zeroizing<[u8; 32]>, SecretKey), KeyError> {
        let length = u8::try_from(keyset_id.len()).map_err(|_| KeyError::KeysetIdTooLong)?;
        let x = hmac512(
            &*self.view_key,
            &[
                TAG_OUTPUT,
                &epoch.to_be_bytes(),
                &[length],
                keyset_id,
                &counter.to_be_bytes(),
            ],
        );
        let r = parse256(&second_half(&x))?;
        Ok((first_half(&x), r))
    }

    /// The secret nonce (64 lowercase hex characters) and blinding factor `r` of output `counter`
    /// of keyset `keyset_id` in epoch `epoch`.
    pub fn output(
        &self,
        epoch: u32,
        keyset_id: &[u8],
        counter: u64,
    ) -> Result<(String, SecretKey), KeyError> {
        let (nonce, r) = self.output_parts(epoch, keyset_id, counter)?;
        Ok((hex::encode(&*nonce), r))
    }

    /// The template secret of that output, locked to `P_e`.
    pub fn lock_secret(
        &self,
        epoch: u32,
        keyset_id: &[u8],
        counter: u64,
    ) -> Result<String, KeyError> {
        let (nonce, _) = self.output_parts(epoch, keyset_id, counter)?;
        Ok(secret::build(&nonce, &self.lock_pubkey(epoch)?))
    }

    /// That output opened and blinded: its secret, `r` and `B_`.
    pub fn blinded_output(
        &self,
        epoch: u32,
        keyset_id: &[u8],
        counter: u64,
    ) -> Result<DerivedOutput, KeyError> {
        let (nonce, r) = self.output_parts(epoch, keyset_id, counter)?;
        let secret = secret::build(&nonce, &self.lock_pubkey(epoch)?);
        let b = dhke::blind(secret.as_bytes(), &r).ok_or(KeyError::InvalidScalar)?;
        Ok(DerivedOutput { secret, r, b })
    }
}

/// Everything Prime derives for one mint.
pub struct MintKeys {
    mint_id: [u8; 32],
    lock_root: SecretKey,
    phone: PhoneMintKeys,
}

impl MintKeys {
    /// Derives the keys of the mint at `canonical_url`, which must already be in canonical form
    /// (see [`url`](crate::url)): another spelling of the same mint gives other keys.
    pub fn derive(app_seed: &[u8; 32], canonical_url: &str) -> Result<Self, KeyError> {
        Self::derive_for_mint_id(app_seed, &mint_id(canonical_url))
    }

    /// Derives the keys of the mint whose id is `mint_id`.
    pub fn derive_for_mint_id(app_seed: &[u8; 32], mint_id: &[u8; 32]) -> Result<Self, KeyError> {
        let m = hmac512(app_seed, &[TAG_MINT, mint_id]);
        let lock_root = parse256(&first_half(&m))?;
        let lock_pubkey = PublicKey::from_secret_key(&Secp256k1::signing_only(), &lock_root);
        Ok(Self {
            mint_id: *mint_id,
            lock_root,
            phone: PhoneMintKeys::new(lock_pubkey, *second_half(&m)),
        })
    }

    pub fn mint_id(&self) -> [u8; 32] {
        self.mint_id
    }

    /// The watch-only half the phone receives.
    pub fn phone(&self) -> &PhoneMintKeys {
        &self.phone
    }

    /// `P_e`.
    pub fn lock_pubkey(&self, epoch: u32) -> Result<PublicKey, KeyError> {
        self.phone.lock_pubkey(epoch)
    }

    /// `k_e = p + t_e mod n`, the key that signs spends of epoch `e`. Never leaves Prime.
    pub fn signing_key(&self, epoch: u32) -> Result<Keypair, KeyError> {
        let t = self.phone.lock_tweak(epoch)?;
        let secret = self.lock_root.add_tweak(&Scalar::from(t))?;
        Ok(Keypair::from_secret_key(
            &Secp256k1::signing_only(),
            &secret,
        ))
    }

    /// The nonce and blinding factor of an output, as [`PhoneMintKeys::output`].
    pub fn output(
        &self,
        epoch: u32,
        keyset_id: &[u8],
        counter: u64,
    ) -> Result<(String, SecretKey), KeyError> {
        self.phone.output(epoch, keyset_id, counter)
    }

    /// The answer to an approved `AddMint`: the lock root and the view key.
    pub fn lock_material(&self) -> CashuLockMaterial {
        CashuLockMaterial {
            mint: FixedBytes(self.mint_id),
            scheme: SCHEME.to_owned(),
            lock_pubkey: FixedBytes(self.phone.lock_root.serialize()),
            view_key: FixedBytes(self.phone.view_key()),
        }
    }
}
