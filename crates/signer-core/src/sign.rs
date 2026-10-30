//! Signing an analysed transaction.

use bitcoin::secp256k1::{Message, Secp256k1};
use fps_protocol::RejectCode;

use crate::analysis::{Analysis, SigningKey};
use crate::error::{SignerError, SignerResult};
use crate::keys::FederationKeys;

/// One BIP-340 signature over the transaction id per input, in input order, as fedimint's
/// `TransactionSignature::NaiveMultisig` expects.
///
/// `aux_rand` supplies 32 bytes of fresh randomness per signature; on the device it reads the TRNG.
/// Every signature is verified against the key the federation will use before it is returned.
pub fn sign_transaction(
    keys: &FederationKeys,
    analysis: &Analysis,
    aux_rand: &mut dyn FnMut() -> [u8; 32],
) -> SignerResult<Vec<[u8; 64]>> {
    let secp = Secp256k1::new();
    let message = Message::from_digest(analysis.txid);
    let mut signatures = Vec::with_capacity(analysis.inputs.len());
    for (index, input) in analysis.inputs.iter().enumerate() {
        let keypair = match input.key {
            SigningKey::Note {
                branch,
                denomination,
                tweak,
            } => keys.note_keypair(branch, denomination, &tweak)?,
            SigningKey::Refund { ephemeral } => keys.refund_keypair(&ephemeral)?,
        };
        if keypair.public_key() != input.public_key {
            return Err(SignerError::new(
                RejectCode::Internal,
                format!("input {index}: derived key does not match the analysed key"),
            ));
        }
        let signature = secp.sign_schnorr_with_aux_rand(&message, &keypair, &aux_rand());
        secp.verify_schnorr(&signature, &message, &keypair.x_only_public_key().0)
            .map_err(|_| {
                SignerError::new(
                    RejectCode::Internal,
                    format!("input {index}: signature failed self-verification"),
                )
            })?;
        signatures.push(signature.serialize());
    }
    Ok(signatures)
}
