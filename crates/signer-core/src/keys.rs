//! The `fps-v1` key scheme. Specified in `docs/KEY_SCHEME.md`; this is the reference
//! implementation.
//!
//! Per federation, the device derives a spend root `(k, c)`, a view secret `v` and a hardened
//! outgoing secret `o` from its app seed. The phone receives `(K = k·G, c, v)`:
//!
//! - spend-branch note keys are `K + t·G` with `t = HMAC-SHA512(c, tag || K || payload)[..32]`, so the
//!   phone can derive every nonce but no private key;
//! - LNv2 refund keys use the same construction keyed by the contract's ephemeral point;
//! - outgoing-branch note keys are `HMAC-SHA512(o, tag || payload)[..32]`, device-only, and are the
//!   only keys ever released (decision D11);
//! - blinding keys and the tweak filter come from `v`.
//!
//! One released spend-branch private key plus `(K, c)` would reveal `k`. That is why spend-branch
//! keys are only ever used to sign.

use bitcoin::hashes::{hmac, sha256, sha512, Hash, HashEngine};
use bitcoin::secp256k1::{self, Keypair, PublicKey, Scalar as Tweak, Secp256k1, SecretKey};
use bls12_381::{G1Affine, Scalar};
use fps_protocol::Branch;
use zeroize::Zeroizing;

use crate::tbs;

/// Scheme name reported to the phone.
pub const SCHEME: &str = "fps-v1";

const TAG_FEDERATION: &[u8] = b"fps-v1/federation";
const TAG_SPEND_ROOT: &[u8] = b"fps-v1/spend-root";
const TAG_VIEW: &[u8] = b"fps-v1/view";
const TAG_OUTGOING: &[u8] = b"fps-v1/outgoing";
const TAG_NOTE: &[u8] = b"fps-v1/mintv2-note";
const TAG_REFUND: &[u8] = b"fps-v1/lnv2-refund";
const TAG_OUTGOING_NOTE: &[u8] = b"fps-v1/mintv2-outgoing";
const TAG_BLIND: &[u8] = b"fps-v1/mintv2-blind";
const TAG_TWEAK_FILTER: &[u8] = b"fps-v1/mintv2-tweak-filter";

/// A derivation produced an out-of-range scalar. Happens with probability about 2^-128; callers
/// treat that tweak as unusable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyError;

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("key derivation produced an invalid scalar")
    }
}

impl std::error::Error for KeyError {}

impl From<secp256k1::Error> for KeyError {
    fn from(_: secp256k1::Error) -> Self {
        Self
    }
}

fn hmac512(key: &[u8], parts: &[&[u8]]) -> Zeroizing<[u8; 64]> {
    let mut engine = hmac::HmacEngine::<sha512::Hash>::new(key);
    for part in parts {
        engine.input(part);
    }
    Zeroizing::new(hmac::Hmac::<sha512::Hash>::from_engine(engine).to_byte_array())
}

fn first_half(bytes: &[u8; 64]) -> Zeroizing<[u8; 32]> {
    Zeroizing::new(bytes[..32].try_into().expect("64 bytes"))
}

fn branch_byte(branch: Branch) -> u8 {
    match branch {
        Branch::Spend => 0,
        Branch::Outgoing => 1,
    }
}

/// The watch-only half: what the phone holds.
#[derive(Clone)]
pub struct ViewKeys {
    spend_key: PublicKey,
    chain_code: [u8; 32],
    view_secret: Zeroizing<[u8; 32]>,
}

impl ViewKeys {
    pub fn new(spend_key: PublicKey, chain_code: [u8; 32], view_secret: [u8; 32]) -> Self {
        Self {
            spend_key,
            chain_code,
            view_secret: Zeroizing::new(view_secret),
        }
    }

    pub fn spend_key(&self) -> PublicKey {
        self.spend_key
    }

    pub fn chain_code(&self) -> [u8; 32] {
        self.chain_code
    }

    pub fn view_secret(&self) -> [u8; 32] {
        *self.view_secret
    }

    fn child_tweak(&self, tag: &[u8], payload: &[&[u8]]) -> Result<Tweak, KeyError> {
        let spend_key = self.spend_key.serialize();
        let mut parts: Vec<&[u8]> = vec![tag, &spend_key];
        parts.extend_from_slice(payload);
        let i = hmac512(&self.chain_code, &parts);
        Tweak::from_be_bytes(*first_half(&i)).map_err(|_| KeyError)
    }

    /// `t` for the spend-branch note at `(denomination, tweak)`.
    pub fn note_tweak(&self, denomination: u8, tweak: &[u8; 16]) -> Result<Tweak, KeyError> {
        self.child_tweak(TAG_NOTE, &[&[denomination], tweak])
    }

    /// Public key (nonce) of the spend-branch note at `(denomination, tweak)`.
    pub fn note_nonce(&self, denomination: u8, tweak: &[u8; 16]) -> Result<PublicKey, KeyError> {
        let t = self.note_tweak(denomination, tweak)?;
        Ok(self
            .spend_key
            .add_exp_tweak(&Secp256k1::verification_only(), &t)?)
    }

    /// `t` for the LNv2 refund key of a contract with this ephemeral point.
    pub fn refund_tweak(&self, ephemeral: &PublicKey) -> Result<Tweak, KeyError> {
        self.child_tweak(TAG_REFUND, &[&ephemeral.serialize()])
    }

    /// The refund key a contract must carry for its refund to come back to this device.
    pub fn refund_key(&self, ephemeral: &PublicKey) -> Result<PublicKey, KeyError> {
        let t = self.refund_tweak(ephemeral)?;
        Ok(self
            .spend_key
            .add_exp_tweak(&Secp256k1::verification_only(), &t)?)
    }

    /// Blinding key of the note at `(branch, denomination, tweak)`.
    pub fn blinding_key(&self, branch: Branch, denomination: u8, tweak: &[u8; 16]) -> Scalar {
        let wide = hmac512(
            &*self.view_secret,
            &[TAG_BLIND, &[branch_byte(branch)], &[denomination], tweak],
        );
        Scalar::from_bytes_wide(&wide)
    }

    /// The blinded nonce message a note output must carry: `nonce_message(nonce) * blinding_key`.
    pub fn blinded_nonce(
        &self,
        branch: Branch,
        nonce: &PublicKey,
        denomination: u8,
        tweak: &[u8; 16],
    ) -> G1Affine {
        tbs::blind(
            &tbs::nonce_message(nonce),
            &self.blinding_key(branch, denomination, tweak),
        )
    }

    /// The secret seed of mint v2's tweak filter (`tweak_filter` upstream).
    pub fn tweak_filter(&self) -> [u8; 32] {
        *first_half(&hmac512(&*self.view_secret, &[TAG_TWEAK_FILTER]))
    }

    /// Mint v2's `check_tweak`: SHA-256 of `(tweak, filter)` starts with two zero bytes. Wallets
    /// grind tweaks until this holds so recovery can skip almost every foreign output cheaply.
    pub fn tweak_passes_filter(&self, tweak: &[u8; 16]) -> bool {
        let mut engine = sha256::HashEngine::default();
        engine.input(tweak);
        engine.input(&self.tweak_filter());
        let digest = sha256::Hash::from_engine(engine).to_byte_array();
        digest[0] == 0 && digest[1] == 0
    }
}

/// Everything the device derives for one federation.
pub struct FederationKeys {
    federation: [u8; 32],
    spend_secret: SecretKey,
    outgoing_secret: Zeroizing<[u8; 32]>,
    view: ViewKeys,
}

impl FederationKeys {
    pub fn derive(app_seed: &[u8; 32], federation: &[u8; 32]) -> Result<Self, KeyError> {
        let federation_secret = hmac512(app_seed, &[TAG_FEDERATION, federation]);
        let root = hmac512(&*federation_secret, &[TAG_SPEND_ROOT]);
        let spend_secret = SecretKey::from_slice(&*first_half(&root))?;
        let chain_code: [u8; 32] = root[32..].try_into().expect("64 bytes");
        let view_secret = first_half(&hmac512(&*federation_secret, &[TAG_VIEW]));
        let outgoing_secret = first_half(&hmac512(&*federation_secret, &[TAG_OUTGOING]));
        let spend_key = PublicKey::from_secret_key(&Secp256k1::signing_only(), &spend_secret);
        Ok(Self {
            federation: *federation,
            spend_secret,
            outgoing_secret,
            view: ViewKeys::new(spend_key, chain_code, *view_secret),
        })
    }

    pub fn federation(&self) -> [u8; 32] {
        self.federation
    }

    pub fn view(&self) -> &ViewKeys {
        &self.view
    }

    /// The keypair of the note at `(branch, denomination, tweak)`.
    pub fn note_keypair(
        &self,
        branch: Branch,
        denomination: u8,
        tweak: &[u8; 16],
    ) -> Result<Keypair, KeyError> {
        let secret = match branch {
            Branch::Spend => {
                let t = self.view.note_tweak(denomination, tweak)?;
                self.spend_secret.add_tweak(&t)?
            }
            Branch::Outgoing => {
                let i = hmac512(
                    &*self.outgoing_secret,
                    &[TAG_OUTGOING_NOTE, &[denomination], tweak],
                );
                SecretKey::from_slice(&*first_half(&i))?
            }
        };
        Ok(Keypair::from_secret_key(
            &Secp256k1::signing_only(),
            &secret,
        ))
    }

    /// The nonce of the note at `(branch, denomination, tweak)`.
    pub fn note_nonce(
        &self,
        branch: Branch,
        denomination: u8,
        tweak: &[u8; 16],
    ) -> Result<PublicKey, KeyError> {
        match branch {
            Branch::Spend => self.view.note_nonce(denomination, tweak),
            Branch::Outgoing => Ok(self.note_keypair(branch, denomination, tweak)?.public_key()),
        }
    }

    /// The refund keypair for a contract with this ephemeral point.
    pub fn refund_keypair(&self, ephemeral: &PublicKey) -> Result<Keypair, KeyError> {
        let t = self.view.refund_tweak(ephemeral)?;
        let secret = self.spend_secret.add_tweak(&t)?;
        Ok(Keypair::from_secret_key(
            &Secp256k1::signing_only(),
            &secret,
        ))
    }
}
