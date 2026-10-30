//! BOLT11 invoices, parsed and signature-checked on the device.

use std::str::FromStr;

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::PublicKey;
use fps_protocol::RejectCode;
use lightning_invoice::{Bolt11Invoice, Bolt11InvoiceDescriptionRef, Currency};

use crate::encoding::Writer;
use crate::error::{SignerError, SignerResult};

/// Longest invoice accepted; BOLT11 invoices with long route hints stay well under this.
pub const MAX_INVOICE_CHARS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceDetails {
    pub payment_hash: [u8; 32],
    pub amount_msat: Option<u64>,
    /// The node that will be paid: the payee field, or the key recovered from the signature.
    pub payee: PublicKey,
    /// The description, or `None` when the invoice commits to a description hash.
    pub description: Option<String>,
    pub currency: Currency,
    pub timestamp_unix: u64,
    pub expiry_secs: u64,
    /// The invoice re-serialised from what was parsed: the form fedimint encodes and hashes.
    pub canonical: String,
}

impl InvoiceDetails {
    /// Whether the invoice is for the network the federation runs on.
    pub fn matches_network(&self, network: bitcoin::Network) -> bool {
        let expected = match network {
            bitcoin::Network::Bitcoin => Currency::Bitcoin,
            bitcoin::Network::Testnet | bitcoin::Network::Testnet4 => Currency::BitcoinTestnet,
            bitcoin::Network::Signet => Currency::Signet,
            bitcoin::Network::Regtest => Currency::Regtest,
        };
        self.currency == expected
    }

    pub fn expires_at_unix(&self) -> u64 {
        self.timestamp_unix.saturating_add(self.expiry_secs)
    }

    /// The message an LNv2 gateway checks the payer's authorisation against before it pays: the
    /// SHA-256 of fedimint's consensus encoding of `LightningInvoice::Bolt11(invoice)`. Signed with
    /// the contract's refund key, it shows the gateway the payment was asked for by whoever funded
    /// the contract.
    pub fn gateway_auth_message(&self) -> [u8; 32] {
        let mut writer = Writer::new();
        // `LightningInvoice::Bolt11` is variant 0; a `Bolt11Invoice` encodes as its string form.
        writer.write_variant(0, |fields| fields.write_string(&self.canonical));
        sha256::Hash::hash(&writer.into_bytes()).to_byte_array()
    }
}

pub fn parse_invoice(invoice: &str) -> SignerResult<InvoiceDetails> {
    let mismatch = |message: String| SignerError::new(RejectCode::InvoiceMismatch, message);
    let invoice = invoice.trim();
    let invoice = invoice
        .strip_prefix("lightning:")
        .or_else(|| invoice.strip_prefix("LIGHTNING:"))
        .unwrap_or(invoice);
    if invoice.len() > MAX_INVOICE_CHARS {
        return Err(mismatch(format!(
            "invoice is longer than {MAX_INVOICE_CHARS} characters"
        )));
    }
    let parsed = Bolt11Invoice::from_str(invoice)
        .map_err(|error| mismatch(format!("invalid invoice: {error}")))?;
    parsed
        .check_signature()
        .map_err(|error| mismatch(format!("invoice signature does not verify: {error}")))?;
    let description = match parsed.description() {
        Bolt11InvoiceDescriptionRef::Direct(description) => Some(description.to_string()),
        Bolt11InvoiceDescriptionRef::Hash(_) => None,
    };
    Ok(InvoiceDetails {
        payment_hash: parsed.payment_hash().to_byte_array(),
        amount_msat: parsed.amount_milli_satoshis(),
        payee: parsed.get_payee_pub_key(),
        description,
        currency: parsed.currency(),
        timestamp_unix: parsed.duration_since_epoch().as_secs(),
        expiry_secs: parsed.expiry_time().as_secs(),
        canonical: parsed.to_string(),
    })
}
