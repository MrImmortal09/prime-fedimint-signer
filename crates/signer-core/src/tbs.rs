//! fedimint's threshold blind signatures (`fedimint-tbs` 0.13), the parts a client needs.
//!
//! Messages are hashed to G1 by seeding a ChaCha20 RNG with a tagged hash and sampling a random
//! point, exactly as `tbs::Message::from_bytes` (SHA3-256, mint v1) and
//! `tbs::Message::from_bytes_sha256` (SHA-256, mint v2) do. Mint v2 notes use the second.

use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::secp256k1::PublicKey;
use bls12_381::{pairing, G1Affine, G1Projective, G2Affine, Scalar};
use group::{Curve, Group};
use rand_chacha::ChaChaRng;
use rand_core::SeedableRng;
use sha3::Digest;

/// Tag of `Message::from_bytes`.
pub const SHA3_TAG: &[u8] = b"TBS_BLS12-381_";
/// Tag of `Message::from_bytes_sha256`.
pub const SHA256_TAG: &[u8] = b"FEDIMINT_TBS_BLS12_381_MESSAGE";

/// `Message::from_bytes` (mint v1).
pub fn hash_to_g1_sha3(data: &[u8]) -> G1Affine {
    let mut hasher = sha3::Sha3_256::new();
    hasher.update(SHA3_TAG);
    hasher.update(data);
    let seed: [u8; 32] = hasher.finalize().into();
    G1Projective::random(&mut ChaChaRng::from_seed(seed)).to_affine()
}

/// `Message::from_bytes_sha256` (mint v2).
pub fn hash_to_g1_sha256(data: &[u8]) -> G1Affine {
    let mut engine = sha256::HashEngine::default();
    engine.input(SHA256_TAG);
    engine.input(data);
    let seed = sha256::Hash::from_engine(engine).to_byte_array();
    G1Projective::random(&mut ChaChaRng::from_seed(seed)).to_affine()
}

/// Mint v2 `nonce_message`: the message the mint signs for a note.
pub fn nonce_message(nonce: &PublicKey) -> G1Affine {
    hash_to_g1_sha256(&nonce.serialize())
}

/// `blind_message`.
pub fn blind(message: &G1Affine, blinding_key: &Scalar) -> G1Affine {
    (message * blinding_key).to_affine()
}

/// `unblind_signature`; `None` for a zero blinding key.
pub fn unblind(blinded_signature: &G1Affine, blinding_key: &Scalar) -> Option<G1Affine> {
    let inverse: Option<Scalar> = blinding_key.invert().into();
    Some((blinded_signature * inverse?).to_affine())
}

/// `verify`: `e(message, pk) == e(signature, G2)`.
pub fn verify(message: &G1Affine, signature: &G1Affine, public_key: &G2Affine) -> bool {
    pairing(message, public_key) == pairing(signature, &G2Affine::generator())
}
