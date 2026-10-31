//! Checks the Cashu core against vectors produced with the `cashu` 0.18.3 crate
//! (`devtools-cashu/src/bin/golden.rs`): every positive vector is reproduced byte for byte, and
//! every case Prime must refuse is refused with the expected code, by the expected rule.

use bitcoin::secp256k1::{schnorr, Message, PublicKey, Secp256k1};
use fps_cashu_core::keys::{MintKeys, SCHEME};
use fps_cashu_core::keyset::{id_v00, id_v01};
use fps_cashu_core::message::{check_one_reading, output_readings};
use fps_cashu_core::{analyze, dhke, dleq, secret, sign_spend, url, validate_mint};
use fps_protocol::{CashuDleq, CashuMintDescriptor, CashuSignRequest, FixedBytes, RejectCode};
use serde_json::Value;

fn vectors() -> Value {
    serde_json::from_str(include_str!("vectors/cashu-0.18.3.json")).expect("valid json")
}

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().expect("hex string")).expect("valid hex")
}

fn array<const N: usize>(value: &Value) -> [u8; N] {
    bytes(value).try_into().expect("right length")
}

fn text(value: &Value) -> &str {
    value.as_str().expect("a string")
}

fn cases<'a>(value: &'a Value, key: &str) -> &'a Vec<Value> {
    let cases = value[key].as_array().expect("array of cases");
    assert!(!cases.is_empty(), "no {key} vectors");
    cases
}

fn cbor<T: serde::de::DeserializeOwned>(value: &Value) -> T {
    ciborium::from_reader(bytes(value).as_slice()).expect("valid CBOR")
}

fn code(value: &Value) -> RejectCode {
    let name = text(value);
    [
        RejectCode::InvalidDescriptor,
        RejectCode::InvalidTransaction,
        RejectCode::ForeignInput,
        RejectCode::InvalidNote,
        RejectCode::UnexpectedOutput,
        RejectCode::InvoiceMismatch,
        RejectCode::PolicyLimit,
        RejectCode::TooLarge,
        RejectCode::Unsupported,
    ]
    .into_iter()
    .find(|code| format!("{code:?}") == name)
    .unwrap_or_else(|| panic!("unknown code {name}"))
}

/// The fixture mint and keys every spend vector uses.
fn mint(vectors: &Value) -> (fps_cashu_core::PinnedMint, MintKeys) {
    let mint = &vectors["mint"];
    let descriptor: CashuMintDescriptor = cbor(&mint["descriptor"]);
    assert_eq!(descriptor.url, text(&mint["url"]));
    let pinned = validate_mint(&descriptor, None).expect("the fixture mint validates");
    let keys = MintKeys::derive(&array(&mint["app_seed"]), &descriptor.url).expect("derives");
    (pinned, keys)
}

#[test]
fn vectors_come_from_the_pinned_cashu_release() {
    let vectors = vectors();
    assert_eq!(vectors["cashu_version"], "0.18.3");
    assert_eq!(vectors["scheme"], SCHEME);
}

#[test]
fn hash_to_curve_matches_cashu() {
    let vectors = vectors();
    for case in cases(&vectors, "hash_to_curve") {
        let point = dhke::hash_to_curve(&bytes(&case["message"])).expect("a point");
        assert_eq!(point.serialize().to_vec(), bytes(&case["point"]), "{case}");
    }
}

#[test]
fn blinding_matches_cashu() {
    let vectors = vectors();
    for case in cases(&vectors, "blind") {
        let r = bitcoin::secp256k1::SecretKey::from_slice(&bytes(&case["r"])).expect("scalar");
        let b = dhke::blind(&bytes(&case["secret"]), &r).expect("blinds");
        assert_eq!(b.serialize().to_vec(), bytes(&case["b"]), "{case}");
    }
}

#[test]
fn keyset_ids_match_cashu() {
    let vectors = vectors();
    for case in cases(&vectors, "keyset_ids") {
        let keys: Vec<(u64, PublicKey)> = case["keys"]
            .as_array()
            .expect("keys")
            .iter()
            .map(|pair| {
                (
                    pair[0].as_u64().expect("amount"),
                    PublicKey::from_slice(&bytes(&pair[1])).expect("point"),
                )
            })
            .collect();
        assert_eq!(id_v00(&keys).to_vec(), bytes(&case["id_v00"]), "{case}");
        let id = id_v01(
            &keys,
            text(&case["unit"]),
            case["input_fee_ppk"].as_u64().expect("fee"),
            case["final_expiry"].as_u64(),
        );
        assert_eq!(id.to_vec(), bytes(&case["id_v01"]), "{case}");
    }
}

#[test]
fn canonical_urls_match_cashu_s_mint_url() {
    let vectors = vectors();
    for case in cases(&vectors, "mint_urls") {
        let input = text(&case["input"]);
        let expected = case["canonical"].as_str();
        assert_eq!(url::canonicalize(input).as_deref(), expected, "{input:?}");
        // Prime accepts exactly the URLs cashu leaves unchanged.
        assert_eq!(
            url::is_canonical(input),
            expected == Some(input),
            "{input:?}"
        );
    }
}

#[test]
fn the_key_scheme_reproduces_its_frozen_derivations() {
    let vectors = vectors();
    for case in cases(&vectors, "key_scheme") {
        let keys =
            MintKeys::derive(&array(&case["app_seed"]), text(&case["mint_url"])).expect("derives");
        let phone = keys.phone();
        assert_eq!(keys.mint_id().to_vec(), bytes(&case["mint_id"]));
        assert_eq!(
            phone.lock_root().serialize().to_vec(),
            bytes(&case["lock_root"])
        );
        assert_eq!(phone.view_key().to_vec(), bytes(&case["view_key"]));
        for epoch in cases(case, "epochs") {
            let e = u32::try_from(epoch["epoch"].as_u64().expect("epoch")).expect("u32");
            assert_eq!(
                phone.lock_tweak(e).unwrap().secret_bytes().to_vec(),
                bytes(&epoch["lock_tweak"])
            );
            assert_eq!(
                phone.lock_pubkey(e).unwrap().serialize().to_vec(),
                bytes(&epoch["lock_pubkey"])
            );
            assert_eq!(keys.lock_pubkey(e).unwrap(), phone.lock_pubkey(e).unwrap());
            let signing = keys.signing_key(e).unwrap();
            assert_eq!(
                signing.secret_bytes().to_vec(),
                bytes(&epoch["signing_secret"])
            );
            assert_eq!(signing.public_key(), phone.lock_pubkey(e).unwrap());
        }
        for output in cases(case, "outputs") {
            let e = u32::try_from(output["epoch"].as_u64().expect("epoch")).expect("u32");
            let keyset = bytes(&output["keyset"]);
            let counter = output["counter"].as_u64().expect("counter");
            let (nonce, r) = phone.output(e, &keyset, counter).unwrap();
            assert_eq!(nonce, text(&output["nonce"]));
            assert_eq!(r.secret_bytes().to_vec(), bytes(&output["r"]));
            let secret_text = phone.lock_secret(e, &keyset, counter).unwrap();
            assert_eq!(secret_text, text(&output["secret"]));
            let derived = phone.blinded_output(e, &keyset, counter).unwrap();
            assert_eq!(derived.secret, secret_text);
            assert_eq!(derived.r, r);
            assert_eq!(derived.b.serialize().to_vec(), bytes(&output["b"]));
            // The template reads back as this nonce, locked to P_e.
            let parsed = secret::parse(&secret_text).expect("a template secret");
            assert_eq!(hex::encode(parsed.nonce), nonce);
            assert_eq!(parsed.lock_key, phone.lock_pubkey(e).unwrap());
        }
    }
}

#[test]
fn dleq_verification_matches_cashu() {
    let vectors = vectors();
    let mut valid = 0;
    let mut invalid = 0;
    for case in cases(&vectors, "dleq") {
        let proof = CashuDleq {
            e: FixedBytes(array(&case["e"])),
            s: FixedBytes(array(&case["s"])),
            r: FixedBytes(array(&case["r"])),
        };
        let c = PublicKey::from_slice(&bytes(&case["c"])).expect("point");
        let mint_key = PublicKey::from_slice(&bytes(&case["mint_key"])).expect("point");
        let expected = case["valid"].as_bool().expect("bool");
        let secret = text(&case["secret"]);
        assert_eq!(
            dleq::verify_proof(secret.as_bytes(), &c, &proof, &mint_key),
            expected,
            "{case}"
        );
        if expected {
            valid += 1;
        } else {
            invalid += 1;
        }
    }
    assert!(
        valid >= 4 && invalid >= 4,
        "{valid} valid, {invalid} invalid"
    );
}

#[test]
fn spends_reproduce_cashu_s_message_and_the_signature() {
    let vectors = vectors();
    let (pinned, keys) = mint(&vectors);
    let secp = Secp256k1::verification_only();
    for case in cases(&vectors, "spends") {
        let name = text(&case["name"]);
        let request: CashuSignRequest = cbor(&case["request"]);
        let analysis = analyze(&request, &pinned, &keys).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            format!("{:?}", analysis.kind),
            text(&case["kind"]),
            "{name}"
        );
        assert_eq!(analysis.review.title, text(&case["title"]), "{name}");
        assert_eq!(
            analysis.review.amount_msat,
            case["amount_msat"].as_u64().unwrap(),
            "{name}"
        );
        assert_eq!(
            analysis.outflow_msat,
            case["outflow_msat"].as_u64().unwrap(),
            "{name}"
        );
        // The message cashu's sig_all_msg_to_sign builds, and its hash.
        assert_eq!(analysis.message, text(&case["message"]), "{name}");
        assert_eq!(
            analysis.message_hash.to_vec(),
            bytes(&case["message_hash"]),
            "{name}"
        );
        // The signature cashu's verify_spending_conditions accepted, byte for byte.
        let signature = sign_spend(&keys, &analysis, array(&case["aux_rand"])).expect("signs");
        assert_eq!(signature.to_vec(), bytes(&case["signature"]), "{name}");
        let lock_key = keys.lock_pubkey(request.epoch).unwrap();
        secp.verify_schnorr(
            &schnorr::Signature::from_slice(&signature).unwrap(),
            &Message::from_digest(analysis.message_hash),
            &lock_key.x_only_public_key().0,
        )
        .expect("verifies under P_e");
    }
}

#[test]
fn collision_transcripts_have_two_readings_and_are_refused() {
    let vectors = vectors();
    for case in cases(&vectors["refused"], "transcripts") {
        let transcript = text(&case["transcript"]);
        for list in ["signed", "posted"] {
            let joined: String = case[list]
                .as_array()
                .expect("outputs")
                .iter()
                .map(|pair| format!("{}{}", pair[0].as_u64().unwrap(), text(&pair[1])))
                .collect();
            assert_eq!(joined, transcript, "{}: {list}", case["name"]);
        }
        assert_eq!(output_readings(transcript), 2, "{}", case["name"]);
        let error = check_one_reading(transcript).unwrap_err();
        assert_eq!(
            error.code,
            RejectCode::InvalidTransaction,
            "{}",
            case["name"]
        );
    }
}

/// The refusal is the vector's: its code, and a message naming the rule that refused it.
fn assert_refused_as(case: &Value, error: &fps_signer_core::error::SignerError) {
    assert_eq!(
        error.code,
        code(&case["code"]),
        "{}: {}",
        case["name"],
        error.message
    );
    assert!(
        error.message.contains(text(&case["reason"])),
        "{}: {:?} is not refused for {:?}",
        case["name"],
        error.message,
        case["reason"]
    );
}

#[test]
fn refused_spends_are_refused_by_their_rules() {
    let vectors = vectors();
    let (pinned, keys) = mint(&vectors);
    for case in cases(&vectors["refused"], "spends") {
        let request: CashuSignRequest = cbor(&case["request"]);
        let error = analyze(&request, &pinned, &keys).expect_err(text(&case["name"]));
        assert_refused_as(case, &error);
    }
}

#[test]
fn refused_mints_are_refused_by_their_rules() {
    let vectors = vectors();
    for case in cases(&vectors["refused"], "mints") {
        let descriptor: CashuMintDescriptor = cbor(&case["descriptor"]);
        let error = validate_mint(&descriptor, None).expect_err(text(&case["name"]));
        assert_refused_as(case, &error);
    }
}
