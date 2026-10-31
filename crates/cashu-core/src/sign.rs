//! Signing an analysed spend.

use bitcoin::secp256k1::{Message, Secp256k1};
use fps_protocol::RejectCode;
use fps_signer_core::error::{SignerError, SignerResult};

use crate::analysis::CashuAnalysis;
use crate::keys::MintKeys;
use crate::message::message_hash;

/// The `SIG_ALL` signature of a spend: BIP-340 by `k_e`, the lock key of the analysis's epoch, over
/// the SHA-256 of the message the analysis built, as NUT-11 and `cashu`'s `SecretKey::sign` do. It
/// goes in the witness of the request's first input.
///
/// The digest is always computed here from Prime's own message, never taken from the phone.
/// `aux_rand` supplies 32 bytes of fresh randomness; on the device it reads the TRNG. The signature
/// is verified against `P_e`, the key the mint checks, before it is returned.
pub fn sign_spend(
    keys: &MintKeys,
    analysis: &CashuAnalysis,
    aux_rand: [u8; 32],
) -> SignerResult<[u8; 64]> {
    let internal = |message: &str| SignerError::new(RejectCode::Internal, message);
    let digest = message_hash(&analysis.message);
    if digest != analysis.message_hash {
        return Err(internal("the message hash does not match the message"));
    }
    let keypair = keys.signing_key(analysis.epoch)?;
    if Ok(keypair.public_key()) != keys.lock_pubkey(analysis.epoch) {
        return Err(internal(
            "the signing key does not match the epoch's lock key",
        ));
    }
    let secp = Secp256k1::new();
    let message = Message::from_digest(digest);
    let signature = secp.sign_schnorr_with_aux_rand(&message, &keypair, &aux_rand);
    secp.verify_schnorr(&signature, &message, &keypair.x_only_public_key().0)
        .map_err(|_| internal("the signature failed self-verification"))?;
    Ok(signature.serialize())
}
