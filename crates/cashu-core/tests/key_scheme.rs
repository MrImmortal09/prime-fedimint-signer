//! The `fps-cashu-v1` key scheme: its frozen derivations and the relations between them, the
//! properties the design depends on, and its separation from the Fedimint scheme `fps-v1`.

use std::collections::BTreeSet;

use bitcoin::secp256k1::{PublicKey, Scalar, Secp256k1};
use fps_cashu_core::keys::{mint_id, KeyError, MintKeys, PhoneMintKeys, SCHEME, TAGS};
use fps_cashu_core::secret;
use fps_signer_core::keys::FederationKeys;
use serde_json::Value;

/// Every tag of `fps-v1`, as docs/KEY_SCHEME.md lists them (the constants are private to
/// fps-signer-core).
const FPS_V1_TAGS: [&str; 9] = [
    "fps-v1/federation",
    "fps-v1/spend-root",
    "fps-v1/view",
    "fps-v1/outgoing",
    "fps-v1/mintv2-note",
    "fps-v1/lnv2-refund",
    "fps-v1/mintv2-outgoing",
    "fps-v1/mintv2-blind",
    "fps-v1/mintv2-tweak-filter",
];

/// v00 and v01 keyset ids of the shape real mints use.
fn keyset_ids() -> [Vec<u8>; 2] {
    let mut v00 = vec![0u8; 8];
    v00[1..].copy_from_slice(&[0xab; 7]);
    let mut v01 = vec![1u8; 33];
    v01[1..].copy_from_slice(&[0xcd; 32]);
    [v00, v01]
}

fn hex32(value: &Value) -> [u8; 32] {
    hex::decode(value.as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap()
}

fn point(value: &Value) -> PublicKey {
    PublicKey::from_slice(&hex::decode(value.as_str().unwrap()).unwrap()).unwrap()
}

#[test]
fn the_frozen_derivations_hold_their_relations() {
    // The key_scheme vectors (devtools-cashu), re-derived, and each value checked against the
    // formulas of docs/KEY_SCHEME_CASHU.md with plain secp256k1 operations.
    let vectors: Value = serde_json::from_str(include_str!("vectors/cashu-0.18.3.json")).unwrap();
    assert_eq!(vectors["scheme"], SCHEME);
    let secp = Secp256k1::new();
    let cases = vectors["key_scheme"].as_array().unwrap();
    assert_eq!(cases.len(), 3);
    for case in cases {
        let url = case["mint_url"].as_str().unwrap();
        let keys = MintKeys::derive(&hex32(&case["app_seed"]), url).unwrap();
        assert_eq!(keys.mint_id(), hex32(&case["mint_id"]));
        assert_eq!(keys.mint_id(), mint_id(url));
        let lock_root = point(&case["lock_root"]);
        assert_eq!(keys.phone().lock_root(), lock_root);
        assert_eq!(keys.phone().view_key(), hex32(&case["view_key"]));
        for epoch in case["epochs"].as_array().unwrap() {
            let e = u32::try_from(epoch["epoch"].as_u64().unwrap()).unwrap();
            // P_e = P + t_e·G, and k_e·G = P_e.
            let t = Scalar::from_be_bytes(hex32(&epoch["lock_tweak"])).unwrap();
            let lock_key = point(&epoch["lock_pubkey"]);
            assert_eq!(lock_root.add_exp_tweak(&secp, &t).unwrap(), lock_key);
            let k = bitcoin::secp256k1::SecretKey::from_slice(&hex32(&epoch["signing_secret"]))
                .unwrap();
            assert_eq!(k.public_key(&secp), lock_key);
            assert_eq!(keys.signing_key(e).unwrap().secret_key(), k);
        }
        for output in case["outputs"].as_array().unwrap() {
            let e = u32::try_from(output["epoch"].as_u64().unwrap()).unwrap();
            let keyset = hex::decode(output["keyset"].as_str().unwrap()).unwrap();
            let counter = output["counter"].as_u64().unwrap();
            let derived = keys.phone().blinded_output(e, &keyset, counter).unwrap();
            assert_eq!(derived.secret, output["secret"].as_str().unwrap());
            assert_eq!(derived.r.secret_bytes(), hex32(&output["r"]));
            // The secret is the template of this nonce and P_e; B_ = hash_to_curve(secret) + r·G.
            let nonce = hex32(&output["nonce"]);
            let lock_key = keys.lock_pubkey(e).unwrap();
            assert_eq!(secret::build(&nonce, &lock_key), derived.secret);
            let y = fps_cashu_core::dhke::hash_to_curve(derived.secret.as_bytes()).unwrap();
            let b = y.combine(&derived.r.public_key(&secp)).unwrap();
            assert_eq!(b, point(&output["b"]));
        }
    }
}

#[test]
fn no_tag_of_one_scheme_is_a_prefix_of_a_tag_of_the_other() {
    let cashu: Vec<&[u8]> = TAGS.to_vec();
    for tag in &cashu {
        assert!(
            tag.starts_with(b"fps-cashu-v1/"),
            "{}",
            String::from_utf8_lossy(tag)
        );
    }
    for fedimint in FPS_V1_TAGS {
        for tag in &cashu {
            assert!(!fedimint.as_bytes().starts_with(tag), "{fedimint}");
            assert!(!tag.starts_with(fedimint.as_bytes()), "{fedimint}");
        }
    }
    // They part at byte 4: `fps-c…` against `fps-v…`.
    assert_eq!(&b"fps-cashu-v1/"[..4], &b"fps-v1/"[..4]);
    assert_ne!(b"fps-cashu-v1/"[4], b"fps-v1/"[4]);
    // Within the scheme, no tag is a prefix of another either.
    for (i, a) in cashu.iter().enumerate() {
        for (j, b) in cashu.iter().enumerate() {
            assert!(i == j || !a.starts_with(b), "tags {i} and {j}");
        }
    }
}

/// Every hex value in an `fps-v1` vector case, and the x coordinate of every key among them.
fn fps_v1_values(case: &Value, out: &mut BTreeSet<Vec<u8>>) {
    match case {
        Value::String(text) => {
            if let Ok(bytes) = hex::decode(text) {
                if bytes.len() == 33 {
                    out.insert(bytes[1..].to_vec());
                }
                out.insert(bytes);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| fps_v1_values(item, out)),
        Value::Object(fields) => fields.values().for_each(|field| fps_v1_values(field, out)),
        _ => {}
    }
}

/// Everything the Cashu scheme derives for a mint, public and private, with the x coordinates of
/// its keys.
fn cashu_values(keys: &MintKeys, out: &mut BTreeSet<Vec<u8>>) {
    let phone = keys.phone();
    let add_key = |key: PublicKey, out: &mut BTreeSet<Vec<u8>>| {
        out.insert(key.serialize().to_vec());
        out.insert(key.x_only_public_key().0.serialize().to_vec());
    };
    add_key(phone.lock_root(), out);
    out.insert(phone.view_key().to_vec());
    for epoch in [0, 1, 2, 3, 4, 100] {
        add_key(keys.lock_pubkey(epoch).unwrap(), out);
        out.insert(keys.signing_key(epoch).unwrap().secret_bytes().to_vec());
        out.insert(phone.lock_tweak(epoch).unwrap().secret_bytes().to_vec());
        for keyset in keyset_ids() {
            for counter in [0, 1, 2] {
                let (nonce, r) = phone.output(epoch, &keyset, counter).unwrap();
                out.insert(hex::decode(nonce).unwrap());
                out.insert(r.secret_bytes().to_vec());
            }
        }
    }
}

#[test]
fn no_cashu_key_equals_a_fedimint_key() {
    // The seeds and federation ids of the frozen fps-v1 vectors. The Cashu keys are derived for
    // the federation id itself as a mint id, the closest the two schemes' inputs can come, and for
    // ordinary URLs.
    let vectors: Value = serde_json::from_str(include_str!(
        "../../signer-core/tests/vectors/key-scheme-fps-v1.json"
    ))
    .unwrap();
    assert_eq!(vectors["scheme"], "fps-v1");
    let cases = vectors["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 3);
    for case in cases {
        let seed: [u8; 32] = hex::decode(case["app_seed"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let federation: [u8; 32] = hex::decode(case["federation_id"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();

        let mut fedimint = BTreeSet::new();
        fps_v1_values(case, &mut fedimint);
        // The frozen vectors hold the federation's keys; check they are what fps-v1 derives now.
        let federation_keys = FederationKeys::derive(&seed, &federation).unwrap();
        assert!(fedimint.contains(federation_keys.view().spend_key().serialize().as_slice()));
        assert!(fedimint.contains(federation_keys.view().view_secret().as_slice()));
        // The inputs are no derived values.
        fedimint.remove(seed.as_slice());
        fedimint.remove(federation.as_slice());

        let mut cashu = BTreeSet::new();
        cashu_values(
            &MintKeys::derive_for_mint_id(&seed, &federation).unwrap(),
            &mut cashu,
        );
        for url in ["https://testnut.cashu.space", "http://127.0.0.1:3338"] {
            cashu_values(&MintKeys::derive(&seed, url).unwrap(), &mut cashu);
        }
        let shared: Vec<String> = fedimint.intersection(&cashu).map(hex::encode).collect();
        assert!(shared.is_empty(), "shared values: {shared:?}");
        assert!(fedimint.len() > 40 && cashu.len() > 200);
    }
}

#[test]
fn the_phone_half_derives_every_public_value_without_prime() {
    let keys = MintKeys::derive(&[3; 32], "https://mint.example").unwrap();
    // Rebuild the phone's half from the exported lock material only.
    let material = keys.lock_material();
    assert_eq!(material.scheme, SCHEME);
    assert_eq!(material.mint.0, keys.mint_id());
    let phone = PhoneMintKeys::new(
        PublicKey::from_slice(&material.lock_pubkey.0).unwrap(),
        material.view_key.0,
    );
    for epoch in [0, 1, 9, u32::MAX] {
        let lock_key = phone.lock_pubkey(epoch).unwrap();
        assert_eq!(lock_key, keys.lock_pubkey(epoch).unwrap());
        // Only Prime's half has the private key, and it matches.
        assert_eq!(keys.signing_key(epoch).unwrap().public_key(), lock_key);
        for keyset in keyset_ids() {
            for counter in [0, 7, u64::MAX] {
                let (nonce, r) = phone.output(epoch, &keyset, counter).unwrap();
                assert_eq!(
                    (nonce.clone(), r),
                    keys.output(epoch, &keyset, counter).unwrap()
                );
                let text = phone.lock_secret(epoch, &keyset, counter).unwrap();
                let parsed = secret::parse(&text).unwrap();
                assert_eq!(hex::encode(parsed.nonce), nonce);
                assert_eq!(parsed.lock_key, lock_key);
                let blinded = phone.blinded_output(epoch, &keyset, counter).unwrap();
                assert_eq!(blinded.secret, text);
                assert_eq!(
                    Some(blinded.b),
                    fps_cashu_core::dhke::blind(text.as_bytes(), &r)
                );
            }
        }
    }
}

#[test]
fn mints_seeds_epochs_keysets_and_counters_never_share_keys() {
    let keys = MintKeys::derive(&[1; 32], "https://a.example").unwrap();
    let other_mint = MintKeys::derive(&[1; 32], "https://b.example").unwrap();
    let other_seed = MintKeys::derive(&[2; 32], "https://a.example").unwrap();
    assert_ne!(keys.phone().lock_root(), other_mint.phone().lock_root());
    assert_ne!(keys.phone().lock_root(), other_seed.phone().lock_root());
    assert_ne!(keys.phone().view_key(), other_mint.phone().view_key());
    assert_ne!(keys.phone().view_key(), other_seed.phone().view_key());

    let mut lock_keys = BTreeSet::new();
    for epoch in 0..64 {
        assert!(lock_keys.insert(keys.lock_pubkey(epoch).unwrap().serialize()));
    }
    assert!(!lock_keys.contains(&keys.phone().lock_root().serialize()));

    let mut outputs = BTreeSet::new();
    for epoch in 0..4 {
        for keyset in keyset_ids() {
            for counter in 0..16 {
                let (nonce, r) = keys.output(epoch, &keyset, counter).unwrap();
                assert!(outputs.insert(nonce));
                assert!(outputs.insert(hex::encode(r.secret_bytes())));
            }
        }
    }
    // The keyset id is length-prefixed: an id and the same bytes plus a counter byte differ.
    let short = keys.output(0, &[0, 1, 2, 3, 4, 5, 6, 7], 0).unwrap();
    let long = keys.output(0, &[0, 1, 2, 3, 4, 5, 6, 7, 0], 0).unwrap();
    assert_ne!(short.0, long.0);
}

#[test]
fn the_mint_id_is_the_hash_of_the_canonical_url_and_spellings_matter() {
    use bitcoin::hashes::{sha256, Hash};
    let url = "https://testnut.cashu.space";
    assert_eq!(
        mint_id(url),
        sha256::Hash::hash(url.as_bytes()).to_byte_array()
    );
    let keys = MintKeys::derive(&[5; 32], url).unwrap();
    assert_eq!(keys.mint_id(), mint_id(url));
    // Another spelling of the same mint is another mint to the key scheme, which is why Prime
    // takes only the canonical form.
    let other = MintKeys::derive(&[5; 32], "https://testnut.cashu.space/").unwrap();
    assert_ne!(keys.phone().lock_root(), other.phone().lock_root());
}

#[test]
fn a_released_cashu_key_would_reveal_the_lock_root() {
    // The tweak is not hardened: k_e − t_e = p. Harmless only because Prime never releases k_e.
    let keys = MintKeys::derive(&[6; 32], "https://mint.example").unwrap();
    let secp = Secp256k1::new();
    let k = keys.signing_key(4).unwrap().secret_key();
    let t = keys.phone().lock_tweak(4).unwrap();
    let p = k.add_tweak(&Scalar::from(t.negate())).unwrap();
    assert_eq!(
        PublicKey::from_secret_key(&secp, &p),
        keys.phone().lock_root()
    );
}

#[test]
fn keyset_ids_longer_than_a_length_byte_are_refused() {
    let keys = MintKeys::derive(&[7; 32], "https://mint.example").unwrap();
    assert!(keys.output(0, &[1; 255], 0).is_ok());
    assert_eq!(
        keys.output(0, &[1; 256], 0).map(|_| ()),
        Err(KeyError::KeysetIdTooLong)
    );
    assert!(keys.phone().blinded_output(0, &[1; 256], 0).is_err());
}
