//! BOLT11 parsing on the device.

mod common;

use common::invoice;
use fps_protocol::RejectCode;
use fps_signer_core::invoice::parse_invoice;
use lightning_invoice::Currency;

#[test]
fn invoices_parse_with_their_details() {
    let text = invoice([4; 32], Some(12_345_000), Currency::Regtest);
    let details = parse_invoice(&text).unwrap();
    assert_eq!(details.payment_hash, [4; 32]);
    assert_eq!(details.amount_msat, Some(12_345_000));
    assert_eq!(details.description.as_deref(), Some("fixture payment"));
    assert_eq!(details.timestamp_unix, 1_800_000_000);
    assert!(details.expires_at_unix() > details.timestamp_unix);
    assert!(details.matches_network(bitcoin::Network::Regtest));
    assert!(!details.matches_network(bitcoin::Network::Bitcoin));

    // The payee is the node that signed it.
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    let node = PublicKey::from_secret_key(
        &Secp256k1::new(),
        &SecretKey::from_slice(&[0x77; 32]).unwrap(),
    );
    assert_eq!(details.payee, node);
}

#[test]
fn uri_prefixes_and_whitespace_are_accepted() {
    let text = invoice([5; 32], Some(1_000), Currency::Regtest);
    assert!(parse_invoice(&format!(" lightning:{text}\n")).is_ok());
    assert!(parse_invoice(&format!("LIGHTNING:{text}")).is_ok());
}

#[test]
fn testnet_invoices_match_testnet_and_testnet4() {
    let details = parse_invoice(&invoice([6; 32], Some(1_000), Currency::BitcoinTestnet)).unwrap();
    assert!(details.matches_network(bitcoin::Network::Testnet));
    assert!(details.matches_network(bitcoin::Network::Testnet4));
    assert!(!details.matches_network(bitcoin::Network::Signet));
}

#[test]
fn corrupted_and_oversized_invoices_are_refused() {
    let text = invoice([7; 32], Some(1_000), Currency::Regtest);
    let mut corrupted = text.clone().into_bytes();
    let index = corrupted.len() - 10;
    corrupted[index] = if corrupted[index] == b'q' { b'p' } else { b'q' };
    let error = parse_invoice(std::str::from_utf8(&corrupted).unwrap()).unwrap_err();
    assert_eq!(error.code, RejectCode::InvoiceMismatch);

    let error = parse_invoice(&"lnbcrt1".repeat(1_000)).unwrap_err();
    assert_eq!(error.code, RejectCode::InvoiceMismatch);
    assert!(error.message.contains("longer"));
}
