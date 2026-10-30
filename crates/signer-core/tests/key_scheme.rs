//! The `fps-v1` key scheme: frozen vectors, and the properties the design depends on.

use bitcoin::secp256k1::PublicKey;
use fps_protocol::Branch;
use fps_signer_core::keys::{FederationKeys, ViewKeys, SCHEME};
use serde_json::Value;

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap()).unwrap()
}

fn array<const N: usize>(value: &Value) -> [u8; N] {
    bytes(value).try_into().unwrap()
}

#[test]
fn derivations_match_the_frozen_vectors() {
    let vectors: Value =
        serde_json::from_str(include_str!("vectors/key-scheme-fps-v1.json")).unwrap();
    assert_eq!(vectors["scheme"], SCHEME);
    let cases = vectors["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 3);
    for case in cases {
        let keys =
            FederationKeys::derive(&array(&case["app_seed"]), &array(&case["federation_id"]))
                .unwrap();
        let view = keys.view();
        assert_eq!(
            view.spend_key().serialize().to_vec(),
            bytes(&case["spend_key"])
        );
        assert_eq!(view.chain_code().to_vec(), bytes(&case["chain_code"]));
        assert_eq!(view.view_secret().to_vec(), bytes(&case["view_secret"]));
        assert_eq!(view.tweak_filter().to_vec(), bytes(&case["tweak_filter"]));
        for note in case["notes"].as_array().unwrap() {
            let denomination = note["denomination"].as_u64().unwrap() as u8;
            let tweak: [u8; 16] = array(&note["tweak"]);
            assert_eq!(
                view.tweak_passes_filter(&tweak),
                note["passes_filter"].as_bool().unwrap()
            );
            for (branch, prefix) in [(Branch::Spend, "spend"), (Branch::Outgoing, "outgoing")] {
                let keypair = keys.note_keypair(branch, denomination, &tweak).unwrap();
                assert_eq!(
                    keypair.public_key().serialize().to_vec(),
                    bytes(&note[format!("{prefix}_nonce")])
                );
                assert_eq!(
                    keypair.secret_bytes().to_vec(),
                    bytes(&note[format!("{prefix}_secret")])
                );
                assert_eq!(
                    view.blinding_key(branch, denomination, &tweak)
                        .to_bytes()
                        .to_vec(),
                    bytes(&note[format!("{prefix}_blinding_key")])
                );
                assert_eq!(
                    view.blinded_nonce(branch, &keypair.public_key(), denomination, &tweak)
                        .to_compressed()
                        .to_vec(),
                    bytes(&note[format!("{prefix}_blinded_nonce")])
                );
            }
        }
        for refund in case["refunds"].as_array().unwrap() {
            let ephemeral = PublicKey::from_slice(&bytes(&refund["ephemeral"])).unwrap();
            let keypair = keys.refund_keypair(&ephemeral).unwrap();
            assert_eq!(
                keypair.public_key().serialize().to_vec(),
                bytes(&refund["refund_key"])
            );
            assert_eq!(
                keypair.secret_bytes().to_vec(),
                bytes(&refund["refund_secret"])
            );
            assert_eq!(view.refund_key(&ephemeral).unwrap(), keypair.public_key());
        }
    }
}

#[test]
fn the_view_material_alone_derives_every_spend_branch_nonce() {
    let keys = FederationKeys::derive(&[1; 32], &[2; 32]).unwrap();
    // Rebuild the phone's half from exported bytes only.
    let view = ViewKeys::new(
        keys.view().spend_key(),
        keys.view().chain_code(),
        keys.view().view_secret(),
    );
    for i in 0..50u8 {
        let tweak = [i; 16];
        let denomination = i % 42;
        let private = keys
            .note_keypair(Branch::Spend, denomination, &tweak)
            .unwrap();
        assert_eq!(
            view.note_nonce(denomination, &tweak).unwrap(),
            private.public_key()
        );
        assert_eq!(
            view.blinding_key(Branch::Spend, denomination, &tweak),
            keys.view()
                .blinding_key(Branch::Spend, denomination, &tweak)
        );
    }
}

#[test]
fn branches_federations_and_seeds_never_share_keys() {
    let keys = FederationKeys::derive(&[1; 32], &[2; 32]).unwrap();
    let other_federation = FederationKeys::derive(&[1; 32], &[3; 32]).unwrap();
    let other_seed = FederationKeys::derive(&[4; 32], &[2; 32]).unwrap();
    let tweak = [9; 16];
    let spend = keys.note_nonce(Branch::Spend, 20, &tweak).unwrap();
    let outgoing = keys.note_nonce(Branch::Outgoing, 20, &tweak).unwrap();
    assert_ne!(spend, outgoing);
    assert_ne!(spend, keys.note_nonce(Branch::Spend, 21, &tweak).unwrap());
    assert_ne!(
        spend,
        other_federation
            .note_nonce(Branch::Spend, 20, &tweak)
            .unwrap()
    );
    assert_ne!(
        spend,
        other_seed.note_nonce(Branch::Spend, 20, &tweak).unwrap()
    );
    assert_ne!(keys.view().spend_key(), other_federation.view().spend_key());
    assert_ne!(keys.view().view_secret(), other_seed.view().view_secret());
    assert_ne!(
        keys.view().blinding_key(Branch::Spend, 20, &tweak),
        keys.view().blinding_key(Branch::Outgoing, 20, &tweak)
    );
}

#[test]
fn a_released_outgoing_key_reveals_nothing_about_the_spend_root() {
    // The attack D11 prevents: with non-hardened derivation, child secret minus its public tweak
    // is the root. Outgoing keys are hardened, so the same subtraction yields garbage.
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    let keys = FederationKeys::derive(&[5; 32], &[6; 32]).unwrap();
    let tweak = [1; 16];
    let secp = Secp256k1::new();

    let spend_child = keys.note_keypair(Branch::Spend, 12, &tweak).unwrap();
    let t = keys.view().note_tweak(12, &tweak).unwrap();
    let recovered = SecretKey::from_slice(&spend_child.secret_bytes())
        .unwrap()
        .add_tweak(&bitcoin::secp256k1::Scalar::from_be_bytes(negate(t.to_be_bytes())).unwrap())
        .unwrap();
    assert_eq!(
        PublicKey::from_secret_key(&secp, &recovered),
        keys.view().spend_key(),
        "a spend-branch key plus the xpub does reveal the root, which is why it is never released"
    );

    let outgoing_child = keys.note_keypair(Branch::Outgoing, 12, &tweak).unwrap();
    let attempt = SecretKey::from_slice(&outgoing_child.secret_bytes())
        .unwrap()
        .add_tweak(&bitcoin::secp256k1::Scalar::from_be_bytes(negate(t.to_be_bytes())).unwrap())
        .unwrap();
    assert_ne!(
        PublicKey::from_secret_key(&secp, &attempt),
        keys.view().spend_key()
    );
}

/// `n - x` for a scalar below the secp256k1 group order.
fn negate(x: [u8; 32]) -> [u8; 32] {
    const ORDER: [u8; 32] = [
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xfe, 0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36,
        0x41, 0x41,
    ];
    let mut out = [0u8; 32];
    let mut borrow = 0i16;
    for i in (0..32).rev() {
        let mut digit = i16::from(ORDER[i]) - i16::from(x[i]) - borrow;
        borrow = 0;
        if digit < 0 {
            digit += 256;
            borrow = 1;
        }
        out[i] = digit as u8;
    }
    out
}

#[test]
fn the_tweak_filter_passes_about_one_tweak_in_sixty_five_thousand() {
    let keys = FederationKeys::derive(&[7; 32], &[8; 32]).unwrap();
    let passing = (0u32..400_000)
        .filter(|i| {
            let mut tweak = [0u8; 16];
            tweak[..4].copy_from_slice(&i.to_be_bytes());
            keys.view().tweak_passes_filter(&tweak)
        })
        .count();
    // Expected about 6; a broken filter gives 0 or thousands.
    assert!((1..=20).contains(&passing), "{passing} of 400000 passed");
}
