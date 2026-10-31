//! NUT-12 DLEQ proofs: offline evidence that the mint signed a proof with a given key.
//!
//! A melt's signature does not commit to the amounts of its inputs, and the mint keeps whatever its
//! blank outputs cannot return. So for a melt Prime checks each input's DLEQ proof against the
//! pinned key for the amount the phone claims. A valid proof shows the mint signed the proof with
//! that key; it does not show the proof is unspent.

use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::secp256k1::{PublicKey, Scalar, Secp256k1, SecretKey};
use fps_protocol::CashuDleq;

use crate::{dhke, hex};

/// NUT-12 `hash_e`: SHA-256 over the lowercase hex of each point in uncompressed form,
/// concatenated.
pub fn hash_e(points: &[PublicKey]) -> [u8; 32] {
    let mut engine = sha256::HashEngine::default();
    for point in points {
        engine.input(hex::encode(&point.serialize_uncompressed()).as_bytes());
    }
    sha256::Hash::from_engine(engine).to_byte_array()
}

/// Whether `dleq` proves that `c` is the mint's signature, under `mint_key`, on `secret`
/// (`Proof::verify_dleq` in `cashu` 0.18.3). With `r` the proof's blinding factor:
///
/// ```text
/// C' = C + r·A          B' = hash_to_curve(secret) + r·G
/// R1 = s·G − e·A        R2 = s·B' − e·C'
/// valid ⇔ e = hash_e(R1, R2, A, C')
/// ```
///
/// `e`, `s` and `r` must be non-zero scalars below the group order, as `cashu` requires.
pub fn verify_proof(secret: &[u8], c: &PublicKey, dleq: &CashuDleq, mint_key: &PublicKey) -> bool {
    check(secret, c, dleq, mint_key).unwrap_or(false)
}

fn check(secret: &[u8], c: &PublicKey, dleq: &CashuDleq, mint_key: &PublicKey) -> Option<bool> {
    let secp = Secp256k1::new();
    let e = SecretKey::from_slice(&dleq.e.0).ok()?;
    let s = SecretKey::from_slice(&dleq.s.0).ok()?;
    let r = SecretKey::from_slice(&dleq.r.0).ok()?;
    let y = dhke::hash_to_curve(secret)?;
    let blinded_signature = c
        .combine(&mint_key.mul_tweak(&secp, &Scalar::from(r)).ok()?)
        .ok()?;
    let blinded_message = y.combine(&r.public_key(&secp)).ok()?;
    let e_a = mint_key.mul_tweak(&secp, &Scalar::from(e)).ok()?;
    let r1 = s.public_key(&secp).combine(&e_a.negate(&secp)).ok()?;
    let e_c = blinded_signature.mul_tweak(&secp, &Scalar::from(e)).ok()?;
    let r2 = blinded_message
        .mul_tweak(&secp, &Scalar::from(s))
        .ok()?
        .combine(&e_c.negate(&secp))
        .ok()?;
    Some(hash_e(&[r1, r2, *mint_key, blinded_signature]) == dleq.e.0)
}
