//! Pinning a mint: the URL rules, the keyset checks, and the template secret.

mod common;

use bitcoin::secp256k1::{PublicKey, Secp256k1};
use common::*;
use fps_cashu_core::keyset::{id_v00, id_v01, MAX_INPUT_FEE_PPK, MAX_KEYS, MAX_KEYSETS, UNIT};
use fps_cashu_core::url::{canonicalize, check_mint_url, is_canonical, MAX_URL_LEN};
use fps_cashu_core::{secret, validate_mint};
use fps_protocol::{ByteBuf, CashuKey, CashuMintDescriptor, FixedBytes, RejectCode};

fn refused(descriptor: &CashuMintDescriptor, code: RejectCode, words: &str) {
    let error = validate_mint(descriptor, None).expect_err(words);
    assert_eq!(error.code, code, "{}", error.message);
    assert!(
        error.message.contains(words),
        "expected {words:?} in {:?}",
        error.message
    );
}

#[test]
fn canonical_urls_are_cdk_s_mint_url_form() {
    for (input, expected) in [
        ("https://mint.example", Some("https://mint.example")),
        ("HTTPS://Mint.Example//", Some("https://mint.example")),
        (
            "https://mint.example/Path/To/",
            Some("https://mint.example/Path/To"),
        ),
        (
            "https://mint.example:3338",
            Some("https://mint.example:3338"),
        ),
        ("mint.example", None),
        ("", None),
    ] {
        assert_eq!(canonicalize(input).as_deref(), expected, "{input}");
        if let Some(expected) = expected {
            assert!(is_canonical(expected));
        }
    }
    assert!(!is_canonical("https://mint.example/"));
    assert!(!is_canonical("https://Mint.example"));
}

#[test]
fn prime_pins_only_canonical_urls_it_can_show_honestly() {
    for url in [
        "https://testnut.cashu.space",
        "https://mint.minibits.cash/Bitcoin",
        "http://127.0.0.1:3338",
        "https://mint.example:8085/a//b",
    ] {
        check_mint_url(url).unwrap_or_else(|error| panic!("{url}: {error}"));
    }
    // The longest URL accepted, then one byte more.
    let longest = format!("https://{}.example", "a".repeat(MAX_URL_LEN - 16));
    assert_eq!(longest.len(), MAX_URL_LEN);
    check_mint_url(&longest).unwrap();
    let long = format!("https://{}.example", "a".repeat(MAX_URL_LEN - 15));
    for (url, words) in [
        ("https://mint.example/", "canonical"),
        ("https://Mint.example", "canonical"),
        ("HTTPS://mint.example", "is not https or http"),
        ("ftp://mint.example", "is not https or http"),
        ("mint.example", "is not https or http"),
        ("https://", "has no host"),
        ("https:///path", "has no host"),
        ("https://mint example", "spaces"),
        ("https://mint.example/\u{7}", "control"),
        ("https://mïnt.example", "non-ASCII"),
        ("https://wallet.example@evil.example", "user info"),
        ("https://mint.example/?x=1", "query"),
        ("https://mint.example/#x", "fragment"),
        (long.as_str(), "longer than 256"),
    ] {
        let error = check_mint_url(url).expect_err(url);
        assert_eq!(error.code, RejectCode::InvalidDescriptor, "{url}");
        assert!(error.message.contains(words), "{url}: {}", error.message);
    }
}

#[test]
fn a_valid_descriptor_is_pinned_as_given() {
    let fixture = Fixture::new();
    let mint = &fixture.mint;
    assert_eq!(mint.id, fps_cashu_core::keys::mint_id(URL));
    assert_eq!(mint.url, URL);
    assert_eq!(mint.unit, UNIT);
    assert_eq!(mint.keysets.len(), 3);
    let a = mint.keyset(&fixture.id(A)).unwrap();
    assert!(a.active && !a.is_v00());
    assert_eq!(a.input_fee_ppk, 100);
    assert!(mint.keyset(&fixture.id(B)).unwrap().is_v00());
    assert_eq!(
        mint.keyset(&fixture.id(C)).unwrap().final_expiry,
        Some(1_900_000_000)
    );
    assert!(a.key(1 << 31).is_some());
    assert!(a.key(3).is_none() && a.key(0).is_none() && a.key(1 << 32).is_none());
    let summary = mint.summary();
    assert_eq!(summary.mint.0, mint.id);
    assert_eq!(summary.keysets, 3);
    assert_eq!(summary.unit, "sat");
}

#[test]
fn only_sat_is_accepted() {
    let fixture = Fixture::new();
    let mut descriptor = fixture.descriptor.clone();
    descriptor.unit = "usd".to_owned();
    refused(
        &descriptor,
        RejectCode::Unsupported,
        "\"usd\" is not \"sat\"",
    );
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets[A].unit = "msat".to_owned();
    refused(&descriptor, RejectCode::Unsupported, "\"msat\"");
}

#[test]
fn keyset_counts_and_ids_are_bounded_and_distinct() {
    let fixture = Fixture::new();
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets.clear();
    refused(&descriptor, RejectCode::InvalidDescriptor, "no keysets");

    // The most keysets accepted, then one more.
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets = (0..MAX_KEYSETS as u8)
        .map(|index| TestKeyset::new(10 + index, 1, true, 0, true, None).wire())
        .collect();
    validate_mint(&descriptor, None).unwrap();
    let keyset = descriptor.keysets[0].clone();
    descriptor.keysets = vec![keyset; MAX_KEYSETS + 1];
    refused(
        &descriptor,
        RejectCode::InvalidDescriptor,
        "more than 16 keysets",
    );

    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets.push(descriptor.keysets[A].clone());
    refused(&descriptor, RejectCode::InvalidDescriptor, "listed twice");

    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets[A].active = false;
    descriptor.keysets[C].active = false;
    refused(
        &descriptor,
        RejectCode::InvalidDescriptor,
        "no active keyset",
    );
}

#[test]
fn keyset_ids_must_be_derived_from_their_keys() {
    let fixture = Fixture::new();
    for keyset in [A, B] {
        let mut descriptor = fixture.descriptor.clone();
        descriptor.keysets[keyset].id.0[3] ^= 1;
        refused(
            &descriptor,
            RejectCode::InvalidDescriptor,
            "not derived from its keys",
        );
    }
    // A version 01 id commits to the fee and the expiry; a version 00 id does not.
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets[A].input_fee_ppk = 200;
    refused(
        &descriptor,
        RejectCode::InvalidDescriptor,
        "not derived from its keys",
    );
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets[C].final_expiry = None;
    refused(
        &descriptor,
        RejectCode::InvalidDescriptor,
        "not derived from its keys",
    );
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets[B].input_fee_ppk = 200;
    descriptor.keysets[B].final_expiry = Some(5);
    validate_mint(&descriptor, None).unwrap();

    // Unknown versions and lengths.
    for id in [vec![], vec![0; 9], vec![1; 8], vec![2; 33], vec![0; 33]] {
        let mut descriptor = fixture.descriptor.clone();
        descriptor.keysets[A].id = ByteBuf(id);
        refused(
            &descriptor,
            RejectCode::InvalidDescriptor,
            "neither 8 bytes",
        );
    }
}

#[test]
fn keys_are_valid_points_in_increasing_powers_of_two() {
    let fixture = Fixture::new();
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets[A].keys.clear();
    refused(&descriptor, RejectCode::InvalidDescriptor, "0 keys");

    let mut descriptor = fixture.descriptor.clone();
    let key = descriptor.keysets[A].keys[0].clone();
    descriptor.keysets[A].keys = vec![key; MAX_KEYS + 1];
    refused(&descriptor, RejectCode::InvalidDescriptor, "65 keys");

    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets[A].keys.swap(0, 1);
    refused(
        &descriptor,
        RejectCode::InvalidDescriptor,
        "increasing order",
    );

    let mut descriptor = fixture.descriptor.clone();
    let second = descriptor.keysets[A].keys[1].clone();
    descriptor.keysets[A].keys.insert(1, second);
    refused(
        &descriptor,
        RejectCode::InvalidDescriptor,
        "increasing order",
    );

    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets[A].keys[2].amount = 3;
    refused(&descriptor, RejectCode::InvalidDescriptor, "and 3 is not");

    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets[A].keys[0].amount = 0;
    refused(&descriptor, RejectCode::InvalidDescriptor, "and 0 is not");

    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets[A].keys[4].pubkey = FixedBytes([4; 33]);
    refused(
        &descriptor,
        RejectCode::InvalidDescriptor,
        "the key for 16 is not a valid point",
    );
}

#[test]
fn a_key_stands_for_one_amount_only() {
    // A melt's message does not commit to input amounts: Prime counts each input at the amount
    // whose pinned key its DLEQ proof names. So no key may be pinned under two amounts.
    let fixture = Fixture::new();
    let error_words = "a key stands for one amount";

    // One key under the amounts 1 and 2, with an id derived from just that.
    let key = scalar(b"one key").public_key(&Secp256k1::new());
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets.push(keyset_of(&[(1, key), (2, key)], 0));
    refused(
        &descriptor,
        RejectCode::InvalidDescriptor,
        "its key for 2 is the key for 1",
    );

    // Keyset A's own keys relabelled, 2^i under the key of 2^(31 − i), with a valid id. Pinned,
    // it would let a melt claim a real proof of 2^20 as one of 2^11.
    let real = fixture.keysets[A].public_keys();
    let relabelled: Vec<(u64, PublicKey)> = real
        .iter()
        .zip(real.iter().rev())
        .map(|((amount, _), (_, key))| (*amount, *key))
        .collect();
    let relabelled = keyset_of(&relabelled, 0);
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets.push(relabelled.clone());
    refused(&descriptor, RejectCode::InvalidDescriptor, error_words);

    // As an update beside the pinned keyset A: in place of A, ...
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets[A] = relabelled.clone();
    let error = validate_mint(&descriptor, Some(&fixture.mint)).unwrap_err();
    assert_eq!(error.code, RejectCode::InvalidDescriptor);
    assert!(error.message.contains(error_words), "{}", error.message);

    // ... or after an update that no longer lists A, which keeps it pinned.
    let mut without_a = fixture.descriptor.clone();
    without_a.keysets.remove(A);
    let pinned = validate_mint(&without_a, Some(&fixture.mint)).unwrap();
    let mut descriptor = without_a.clone();
    descriptor.keysets.push(relabelled);
    let error = validate_mint(&descriptor, Some(&pinned)).unwrap_err();
    assert_eq!(error.code, RejectCode::InvalidDescriptor);
    assert!(error.message.contains(error_words), "{}", error.message);

    // The same key under the same amount in two keysets names nothing new: A's keys under a
    // version 00 id.
    let mut descriptor = fixture.descriptor.clone();
    let mut copy = descriptor.keysets[A].clone();
    copy.id = ByteBuf(id_v00(&real).to_vec());
    descriptor.keysets.push(copy);
    validate_mint(&descriptor, None).unwrap();
}

#[test]
fn an_update_adds_keysets_and_changes_nothing_pinned() {
    let fixture = Fixture::new();
    let update = |descriptor: &CashuMintDescriptor| validate_mint(descriptor, Some(&fixture.mint));

    // The same descriptor again pins the same.
    assert_eq!(update(&fixture.descriptor).unwrap(), fixture.mint);

    // A new keyset is added, and the active flags follow the mint.
    let d = TestKeyset::new(3, 8, true, 0, true, None);
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets[A].active = false;
    descriptor.keysets.push(d.wire());
    let pinned = update(&descriptor).unwrap();
    assert_eq!(pinned.keysets.len(), 4);
    assert!(!pinned.keyset(&fixture.id(A)).unwrap().active);
    assert!(pinned.keyset(&d.id).unwrap().active);

    // A keyset the mint no longer lists stays pinned, after the listed ones, as inactive.
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets.remove(C);
    let pinned = update(&descriptor).unwrap();
    let ids: Vec<Vec<u8>> = pinned.keysets.iter().map(|k| k.id.clone()).collect();
    assert_eq!(ids, [fixture.id(A), fixture.id(B), fixture.id(C)]);
    let kept = pinned.keyset(&fixture.id(C)).unwrap();
    let before = fixture.mint.keyset(&fixture.id(C)).unwrap();
    assert!(!kept.active);
    assert_eq!((&kept.keys, kept.input_fee_ppk), (&before.keys, 0));

    // A version 00 id does not commit to the fee: the first pin takes any, an update none other.
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets[B].input_fee_ppk = 250;
    validate_mint(&descriptor, None).unwrap();
    let error = update(&descriptor).unwrap_err();
    assert_eq!(error.code, RejectCode::InvalidDescriptor);
    assert!(
        error.message.contains("keys or fee differ"),
        "{}",
        error.message
    );

    // Other keys under a pinned id, as a version 00 id that collides would bring.
    let mut colliding = fixture.mint.clone();
    let other = scalar(b"collision").public_key(&Secp256k1::new());
    colliding.keysets[B].keys.insert(1, other);
    let error = validate_mint(&fixture.descriptor, Some(&colliding)).unwrap_err();
    assert!(
        error.message.contains("keys or fee differ"),
        "{}",
        error.message
    );

    // At most 16 keysets, counting those kept: 14 new ones beside the 3 pinned are too many.
    let mut descriptor = fixture.descriptor.clone();
    descriptor.keysets = (0..14u8)
        .map(|index| TestKeyset::new(10 + index, 1, true, 0, true, None).wire())
        .collect();
    let error = update(&descriptor).unwrap_err();
    assert_eq!(error.code, RejectCode::InvalidDescriptor);
    assert!(
        error.message.contains("counting those pinned before"),
        "{}",
        error.message
    );
    descriptor.keysets.pop();
    assert_eq!(update(&descriptor).unwrap().keysets.len(), MAX_KEYSETS);

    // The mint pinned before must be this one.
    let mut elsewhere = fixture.descriptor.clone();
    elsewhere.url = "https://other.example".to_owned();
    let error = validate_mint(&elsewhere, Some(&fixture.mint)).unwrap_err();
    assert_eq!(error.code, RejectCode::Internal);
}

#[test]
fn the_input_fee_is_bounded() {
    // A keyset with one key and the highest accepted fee, then one ppk more.
    let secp = Secp256k1::new();
    let key = scalar(b"fee keyset").public_key(&secp);
    let keyset = |fee: u64| {
        let keys: Vec<(u64, PublicKey)> = vec![(1, key)];
        fps_protocol::CashuKeyset {
            id: ByteBuf(id_v01(&keys, "sat", fee, None).to_vec()),
            unit: "sat".to_owned(),
            input_fee_ppk: fee,
            final_expiry: None,
            active: true,
            keys: vec![CashuKey {
                amount: 1,
                pubkey: FixedBytes(key.serialize()),
            }],
        }
    };
    let descriptor = |fee| CashuMintDescriptor {
        url: URL.to_owned(),
        unit: "sat".to_owned(),
        keysets: vec![keyset(fee)],
    };
    validate_mint(&descriptor(MAX_INPUT_FEE_PPK), None).unwrap();
    refused(
        &descriptor(MAX_INPUT_FEE_PPK + 1),
        RejectCode::InvalidDescriptor,
        "above the limit",
    );
    // The version 00 id of the same keys.
    assert_eq!(id_v00(&[(1, key)])[0], 0);
}

#[test]
fn the_template_is_exactly_what_cdk_writes_and_nothing_else_parses() {
    let secp = Secp256k1::new();
    let key = scalar(b"lock").public_key(&secp);
    let text = secret::build(&[0xab; 32], &key);
    assert_eq!(text.len(), secret::TEMPLATE_LEN);
    assert_eq!(
        text,
        format!(
            r#"["P2PK",{{"nonce":"{}","data":"{}","tags":[["sigflag","SIG_ALL"]]}}]"#,
            "ab".repeat(32),
            hex::encode(key.serialize())
        )
    );
    let parsed = secret::parse(&text).unwrap();
    assert_eq!(parsed.nonce, [0xab; 32]);
    assert_eq!(parsed.lock_key, key);

    // A data field that is not a point, an uncompressed prefix, stray characters.
    let not_a_point = text.replace(
        &hex::encode(key.serialize()),
        &format!("02{}", "f".repeat(64)),
    );
    let uncompressed = text.replace(&hex::encode(key.serialize())[..2], "04");
    for bad in [
        not_a_point,
        uncompressed,
        format!("{text} "),
        format!(" {}", &text[1..]),
        text.replace("SIG_ALL", "SIG_ALX"),
        text.replacen('a', "g", 1),
    ] {
        assert!(secret::parse(&bad).is_none(), "{bad}");
    }
}
