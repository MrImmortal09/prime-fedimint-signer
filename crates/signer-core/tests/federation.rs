//! Federation descriptor validation.

mod common;

use common::*;
use fps_protocol::{FederationDescriptor, FixedBytes, Network, RejectCode};
use fps_signer_core::encoding::Writer;
use fps_signer_core::federation::{Federation, KnownModule};
use fps_signer_core::fedimint::endpoints::federation_id;

fn refused(descriptor: &FederationDescriptor) -> RejectCode {
    Federation::from_descriptor(descriptor)
        .expect_err("must be refused")
        .code
}

#[test]
fn a_valid_descriptor_imports() {
    let descriptor = descriptor(&guardians(3));
    let federation = Federation::from_descriptor(&descriptor).unwrap();
    assert_eq!(federation.id, descriptor.federation.0);
    assert_eq!(federation.guardians.len(), 3);
    assert_eq!(federation.display_name(), "Fixture Federation");
    assert_eq!(federation.mint.keys.len(), 42);
    assert_eq!(federation.known_module(MINT), Some(KnownModule::Mint));
    assert_eq!(
        federation.known_module(LIGHTNING),
        Some(KnownModule::Lightning)
    );
    assert_eq!(federation.known_module(WALLET), Some(KnownModule::Wallet));
    assert_eq!(federation.known_module(META), None);
    assert_eq!(federation.module_kind(META), Some("meta"));
    let summary = federation.summary();
    assert_eq!(summary.guardians, 3);
    assert_eq!(summary.modules, ["mintv2", "lnv2", "walletv2", "meta"]);
}

#[test]
fn the_federation_id_must_match_the_endpoints() {
    let mut descriptor = descriptor(&guardians(2));
    descriptor.federation = FixedBytes([0xab; 32]);
    assert_eq!(refused(&descriptor), RejectCode::FederationIdMismatch);

    // Endpoints changed after the fact.
    let mut descriptor = descriptor_with_endpoints(&guardians(2));
    let mut tampered = descriptor.api_endpoints.0.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    descriptor.api_endpoints = tampered.into();
    assert_eq!(refused(&descriptor), RejectCode::FederationIdMismatch);
}

fn descriptor_with_endpoints(
    guardians: &[fps_signer_core::fedimint::endpoints::Guardian],
) -> FederationDescriptor {
    descriptor(guardians)
}

/// A descriptor whose endpoints are `bytes`, with the id recomputed to match.
fn with_raw_endpoints(bytes: Vec<u8>) -> FederationDescriptor {
    let mut descriptor = descriptor(&guardians(1));
    descriptor.federation = FixedBytes(federation_id(&bytes));
    descriptor.api_endpoints = bytes.into();
    descriptor
}

#[test]
fn endpoints_must_be_well_formed() {
    // No guardians.
    let mut writer = Writer::new();
    writer.write_bigsize(0);
    assert_eq!(
        refused(&with_raw_endpoints(writer.into_bytes())),
        RejectCode::InvalidDescriptor
    );

    // Peers out of order.
    let mut writer = Writer::new();
    writer.write_bigsize(2);
    for peer in [3u64, 1] {
        writer.write_bigsize(peer);
        writer.write_string("wss://g.example/");
        writer.write_string("g");
    }
    assert_eq!(
        refused(&with_raw_endpoints(writer.into_bytes())),
        RejectCode::InvalidDescriptor
    );

    // Garbage.
    assert_eq!(
        refused(&with_raw_endpoints(vec![0x05, 0x01])),
        RejectCode::InvalidDescriptor
    );
}

#[test]
fn module_lists_must_be_consistent() {
    let base = descriptor(&guardians(1));

    let mut duplicate = base.clone();
    duplicate.modules.push(duplicate.modules[0].clone());
    assert_eq!(refused(&duplicate), RejectCode::InvalidDescriptor);

    let mut wrong_kind = base.clone();
    wrong_kind.modules[0].kind = "mint".to_owned();
    assert_eq!(refused(&wrong_kind), RejectCode::InvalidDescriptor);

    let mut missing = base.clone();
    missing.modules.retain(|m| m.instance != LIGHTNING);
    assert_eq!(refused(&missing), RejectCode::InvalidDescriptor);

    let mut shared = base.clone();
    shared.wallet.as_mut().unwrap().instance = LIGHTNING;
    assert_eq!(refused(&shared), RejectCode::InvalidDescriptor);

    let mut no_mint = base.clone();
    no_mint.mint = None;
    assert_eq!(refused(&no_mint), RejectCode::Unsupported);

    // Lightning and wallet are optional.
    let mut minimal = base.clone();
    minimal.lightning = None;
    minimal.wallet = None;
    let federation = Federation::from_descriptor(&minimal).unwrap();
    assert!(federation.lightning.is_none() && federation.wallet.is_none());
}

#[test]
fn mint_parameters_are_checked() {
    let base = descriptor(&guardians(1));

    let mut unit = base.clone();
    unit.mint.as_mut().unwrap().amount_unit = 1;
    assert_eq!(refused(&unit), RejectCode::Unsupported);

    let mut bad_key = base.clone();
    bad_key.mint.as_mut().unwrap().keys[3].key = FixedBytes([0x12; 96]);
    assert_eq!(refused(&bad_key), RejectCode::InvalidDescriptor);

    let mut high = base.clone();
    high.mint.as_mut().unwrap().keys[0].denomination = 42;
    assert_eq!(refused(&high), RejectCode::InvalidDescriptor);

    let mut twice = base.clone();
    let mint = twice.mint.as_mut().unwrap();
    mint.keys[1].denomination = mint.keys[0].denomination;
    assert_eq!(refused(&twice), RejectCode::InvalidDescriptor);

    let mut none = base.clone();
    none.mint.as_mut().unwrap().keys.clear();
    assert_eq!(refused(&none), RejectCode::InvalidDescriptor);

    let mut fee = base.clone();
    fee.mint.as_mut().unwrap().fee_ppm = 1_001;
    assert_eq!(refused(&fee), RejectCode::InvalidDescriptor);
}

#[test]
fn lightning_and_wallet_must_agree_on_the_network() {
    let mut descriptor = descriptor(&guardians(1));
    descriptor.wallet.as_mut().unwrap().network = Network::Signet;
    assert_eq!(refused(&descriptor), RejectCode::InvalidDescriptor);
}

#[test]
fn names_are_cleaned_and_bounded() {
    let mut descriptor = descriptor(&guardians(1));
    descriptor.name = Some("  Club\u{7}house \n".to_owned());
    assert_eq!(
        Federation::from_descriptor(&descriptor)
            .unwrap()
            .name
            .as_deref(),
        Some("Clubhouse")
    );

    descriptor.name = Some("   ".to_owned());
    let federation = Federation::from_descriptor(&descriptor).unwrap();
    assert_eq!(federation.name, None);
    assert!(federation.display_name().starts_with("Federation "));

    descriptor.name = Some("x".repeat(65));
    assert_eq!(refused(&descriptor), RejectCode::InvalidDescriptor);
}
