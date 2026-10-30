//! Signing an analysed transaction.

use bitcoin::secp256k1::{Message, Secp256k1};
use fps_protocol::RejectCode;

use crate::analysis::{Analysis, LightningPayment, SigningKey};
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

/// The payer's authorisation for an LNv2 gateway to pay `payment`: a BIP-340 signature over the
/// invoice's [`gateway_auth_message`](crate::invoice::InvoiceDetails::gateway_auth_message) with the
/// contract's refund key. The gateway refuses a payment request without it.
///
/// Only for a payment the analysis checked: the contract pays this invoice's hash and refunds to
/// this device, and the message is a hash the device computed from the invoice itself, so it can
/// never be a transaction id spending the contract.
pub fn gateway_auth(
    keys: &FederationKeys,
    payment: &LightningPayment,
    aux_rand: [u8; 32],
) -> SignerResult<[u8; 64]> {
    let keypair = keys.refund_keypair(&payment.contract.ephemeral_pk)?;
    if keypair.public_key() != payment.contract.refund_pk {
        return Err(SignerError::new(
            RejectCode::Internal,
            "the contract's refund key is not this device's",
        ));
    }
    let secp = Secp256k1::new();
    let message = Message::from_digest(payment.invoice.gateway_auth_message());
    let signature = secp.sign_schnorr_with_aux_rand(&message, &keypair, &aux_rand);
    secp.verify_schnorr(&signature, &message, &keypair.x_only_public_key().0)
        .map_err(|_| {
            SignerError::new(
                RejectCode::Internal,
                "the gateway authorisation failed self-verification",
            )
        })?;
    Ok(signature.serialize())
}
