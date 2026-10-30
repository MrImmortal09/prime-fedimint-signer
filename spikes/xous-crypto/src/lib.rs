//! Phase 0 spike.
//!
//! Exercises every cryptographic and encoding primitive the Prime-side signer core depends on, so
//! that one build for `armv7a-unknown-xous-elf` proves the whole dependency set compiles for the
//! device, and one host run gives a cost baseline. Nothing here is used by the signer itself; the
//! production implementations live in `fps-signer-core` from Phase 1 on.
//!
//! The two hash-to-curve functions reproduce fedimint's `tbs` crate exactly: mint v1 notes hash
//! their nonce with SHA3-256 under `TBS_BLS12-381_`, mint v2 notes with SHA-256 under
//! `FEDIMINT_TBS_BLS12_381_MESSAGE`. Both seed a ChaCha20 RNG and sample a random G1 point.

use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::secp256k1::{schnorr, Keypair, Message, Secp256k1, XOnlyPublicKey};
use bls12_381::{pairing, G1Affine, G1Projective, G2Affine, G2Projective, Scalar};
use group::{Curve, Group};
use rand_chacha::ChaChaRng;
use rand_core::SeedableRng;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha3::Digest;

/// Domain tag of fedimint `tbs::Message::from_bytes` (SHA3 variant, mint v1).
pub const TBS_SHA3_TAG: &[u8] = b"TBS_BLS12-381_";
/// Domain tag of fedimint `tbs::Message::from_bytes_sha256` (mint v2).
pub const TBS_SHA256_TAG: &[u8] = b"FEDIMINT_TBS_BLS12_381_MESSAGE";

/// Maps bytes to a G1 point the way fedimint's `tbs::Message::from_bytes` does.
pub fn hash_to_g1_sha3(data: &[u8]) -> G1Affine {
    let mut hasher = sha3::Sha3_256::new();
    hasher.update(TBS_SHA3_TAG);
    hasher.update(data);
    let seed: [u8; 32] = hasher.finalize().into();
    G1Projective::random(&mut ChaChaRng::from_seed(seed)).to_affine()
}

/// Maps bytes to a G1 point the way fedimint's `tbs::Message::from_bytes_sha256` does.
pub fn hash_to_g1_sha256(data: &[u8]) -> G1Affine {
    let mut engine = sha256::HashEngine::default();
    engine.input(TBS_SHA256_TAG);
    engine.input(data);
    let seed = sha256::Hash::from_engine(engine).to_byte_array();
    G1Projective::random(&mut ChaChaRng::from_seed(seed)).to_affine()
}

/// Signs `sig = msg * sk` and checks `e(msg, sk·G2) == e(sig, G2)`, the tbs verification equation.
pub fn bls_sign_and_verify(secret: u64, data: &[u8]) -> bool {
    let sk = Scalar::from(secret);
    let pk = (G2Projective::generator() * sk).to_affine();
    let msg = hash_to_g1_sha256(data);
    let sig = (msg * sk).to_affine();
    bls_verify(&msg, &sig, &pk)
}

/// The tbs verification equation.
pub fn bls_verify(msg: &G1Affine, sig: &G1Affine, pk: &G2Affine) -> bool {
    pairing(msg, pk) == pairing(sig, &G2Affine::generator())
}

/// Blinds a message with a scalar, as `tbs::blind_message` does.
pub fn blind(msg: &G1Affine, blinding: &Scalar) -> G1Affine {
    (msg * blinding).to_affine()
}

/// BIP-340 signature over a 32-byte digest with caller-supplied auxiliary randomness.
pub fn schnorr_sign(secret: &[u8; 32], digest: &[u8; 32], aux: &[u8; 32]) -> Option<[u8; 64]> {
    let secp = Secp256k1::signing_only();
    let keypair = Keypair::from_seckey_slice(&secp, secret).ok()?;
    let sig = secp.sign_schnorr_with_aux_rand(&Message::from_digest(*digest), &keypair, aux);
    Some(sig.serialize())
}

/// Verifies a BIP-340 signature against an x-only public key.
pub fn schnorr_verify(xonly: &[u8; 32], digest: &[u8; 32], signature: &[u8; 64]) -> bool {
    let secp = Secp256k1::verification_only();
    let (Ok(pk), Ok(sig)) = (
        XOnlyPublicKey::from_slice(xonly),
        schnorr::Signature::from_slice(signature),
    ) else {
        return false;
    };
    secp.verify_schnorr(&sig, &Message::from_digest(*digest), &pk)
        .is_ok()
}

/// Serialises to CBOR and back, returning the decoded value and the encoded size.
pub fn cbor_roundtrip<T: Serialize + DeserializeOwned>(value: &T) -> Option<(T, usize)> {
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).ok()?;
    let decoded = ciborium::from_reader(bytes.as_slice()).ok()?;
    Some((decoded, bytes.len()))
}

/// Parses a BOLT11 invoice, checks its signature, and returns the payment hash and amount.
pub fn bolt11_summary(invoice: &str) -> Option<([u8; 32], Option<u64>)> {
    let invoice: lightning_invoice::Bolt11Invoice = invoice.parse().ok()?;
    invoice.check_signature().ok()?;
    Some((
        invoice.payment_hash().to_byte_array(),
        invoice.amount_milli_satoshis(),
    ))
}
