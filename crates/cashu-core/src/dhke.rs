//! NUT-00 blind Diffie-Hellman on secp256k1: `hash_to_curve`, and the blinded message
//! `B_ = Y + r·G` an output carries.

use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::secp256k1::{Parity, PublicKey, Secp256k1, SecretKey, XOnlyPublicKey};

/// NUT-00's domain separator.
pub const DOMAIN_SEPARATOR: &[u8] = b"Secp256k1_HashToCurve_Cashu_";

/// The counter runs below 2^16, as in NUT-00. Each try succeeds with probability about 1/2.
pub const MAX_TRIES: u32 = 1 << 16;

/// `Y = hash_to_curve(message)`: `h = SHA-256(DOMAIN_SEPARATOR || message)`, then the first
/// `SHA-256(h || counter)` (counter a little-endian `u32` from 0) that is the x coordinate of a
/// point, taken with even y. `message` is the UTF-8 of a proof's secret, verbatim.
pub fn hash_to_curve(message: &[u8]) -> Option<PublicKey> {
    let mut engine = sha256::HashEngine::default();
    engine.input(DOMAIN_SEPARATOR);
    engine.input(message);
    let message_hash = sha256::Hash::from_engine(engine).to_byte_array();
    for counter in 0..MAX_TRIES {
        let mut engine = sha256::HashEngine::default();
        engine.input(&message_hash);
        engine.input(&counter.to_le_bytes());
        let candidate = sha256::Hash::from_engine(engine).to_byte_array();
        if let Ok(x) = XOnlyPublicKey::from_slice(&candidate) {
            return Some(PublicKey::from_x_only_public_key(x, Parity::Even));
        }
    }
    None
}

/// `B_ = hash_to_curve(secret) + r·G`: the blinded message of an output with this opening.
pub fn blind(secret: &[u8], r: &SecretKey) -> Option<PublicKey> {
    let y = hash_to_curve(secret)?;
    y.combine(&r.public_key(&Secp256k1::signing_only())).ok()
}
