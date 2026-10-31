//! Writes the golden vectors fps-cashu-core's tests check against.
//!
//!   cargo run --release --manifest-path devtools-cashu/Cargo.toml --bin golden
//!
//! One file, `crates/cashu-core/tests/vectors/cashu-0.18.3.json`:
//!
//! - values the `cashu` 0.18.3 crate computes: `hash_to_curve` and blinding (the NUT-00 vectors
//!   and generated ones), keyset ids of both versions, canonical mint URLs, and DLEQ proofs made by
//!   its mint-side functions;
//! - the `fps-cashu-v1` derivations, frozen, with every template secret parsed and re-serialised by
//!   `cashu` and every `B_` computed by it;
//! - full swaps and melts built from those keys, whose message is `cashu`'s `sig_all_msg_to_sign`
//!   and whose signature, made by fps-cashu-core, passes `cashu`'s `verify_spending_conditions`;
//! - what Prime must refuse, each with its code and words of its reason: the two known
//!   output-transcript collisions, a collision built from the wallet's own change, a tampered
//!   opening, a foreign lock key, change with a stranger's secret under the wallet's own `r`, a melt
//!   input without a DLEQ proof, change beyond the epoch window, a melt with too few blanks, a
//!   non-canonical mint URL, a wrong keyset id, and a keyset that relabels another's keys.
//!
//! Everything is seeded, so re-running produces an identical file. Every claim above is asserted
//! before the file is written.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1;
use cashu::dhke;
use cashu::nuts::nut02::KeySetVersion;
use cashu::nuts::{
    BlindedMessage, Conditions, CurrencyUnit, Id, Keys, Nut10Secret, SigFlag,
    SpendingConditionVerification, SpendingConditions,
};
use cashu::secret::Secret;
use cashu::{Amount, MintUrl, PublicKey, SecretKey};
use fps_cashu_core::analysis::MAX_EPOCHS_AHEAD;
use fps_cashu_core::keys::{MintKeys, PhoneMintKeys};
use fps_cashu_core::keyset::id_v01;
use fps_cashu_core::{analyze, message, secret, sign_spend, validate_mint};
use fps_devtools_cashu::*;
use fps_protocol::{
    ByteBuf, CashuKey, CashuKeyset, CashuMintDescriptor, CashuOutputHint, CashuSignRequest,
    CashuSpendKind, FixedBytes, RejectCode,
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use serde::Serialize;

const CASHU_VERSION: &str = "0.18.3";
const APP_SEED: [u8; 32] = [0x5e; 32];
const MINT_URL: &str = "https://mint.fixture.example";

fn h(bytes: impl AsRef<[u8]>) -> String {
    hex::encode(bytes)
}

/// Plain CBOR, as fps-protocol serialises message bodies.
fn cbor<T: Serialize>(value: &T) -> String {
    let mut out = Vec::new();
    ciborium::into_writer(value, &mut out).expect("serialises");
    h(out)
}

#[derive(Serialize)]
struct Vectors {
    generator: String,
    cashu_version: String,
    scheme: String,
    hash_to_curve: Vec<HashCase>,
    blind: Vec<BlindCase>,
    keyset_ids: Vec<KeysetIdCase>,
    mint_urls: Vec<UrlCase>,
    key_scheme: Vec<KeySchemeCase>,
    dleq: Vec<DleqCase>,
    /// The mint and app seed every spend uses.
    mint: MintCase,
    spends: Vec<SpendCase>,
    refused: Refused,
}

#[derive(Serialize)]
struct MintCase {
    app_seed: String,
    url: String,
    /// CBOR of the `CashuMintDescriptor`.
    descriptor: String,
}

#[derive(Serialize)]
struct HashCase {
    message: String,
    point: String,
}

#[derive(Serialize)]
struct BlindCase {
    secret: String,
    r: String,
    b: String,
}

#[derive(Serialize)]
struct KeysetIdCase {
    keys: Vec<(u64, String)>,
    unit: String,
    input_fee_ppk: u64,
    final_expiry: Option<u64>,
    id_v00: String,
    id_v01: String,
}

#[derive(Serialize)]
struct UrlCase {
    input: String,
    /// `MintUrl::from_str(input).to_string()`, or `None` where `cashu` refuses it.
    canonical: Option<String>,
}

#[derive(Serialize)]
struct KeySchemeCase {
    app_seed: String,
    mint_url: String,
    mint_id: String,
    lock_root: String,
    view_key: String,
    epochs: Vec<EpochCase>,
    outputs: Vec<OutputCase>,
}

#[derive(Serialize)]
struct EpochCase {
    epoch: u32,
    lock_tweak: String,
    lock_pubkey: String,
    signing_secret: String,
}

#[derive(Serialize)]
struct OutputCase {
    epoch: u32,
    keyset: String,
    counter: u64,
    nonce: String,
    r: String,
    secret: String,
    b: String,
}

#[derive(Serialize)]
struct DleqCase {
    secret: String,
    c: String,
    e: String,
    s: String,
    r: String,
    mint_key: String,
    valid: bool,
}

#[derive(Serialize)]
struct SpendCase {
    name: String,
    /// CBOR of the `CashuSignRequest`.
    request: String,
    kind: String,
    title: String,
    amount_msat: u64,
    outflow_msat: u64,
    message: String,
    message_hash: String,
    aux_rand: String,
    signature: String,
}

#[derive(Serialize)]
struct Refused {
    transcripts: Vec<TranscriptCase>,
    spends: Vec<RefusedSpend>,
    mints: Vec<RefusedMint>,
}

#[derive(Serialize)]
struct TranscriptCase {
    name: String,
    /// The outputs Prime would review.
    signed: Vec<(u64, String)>,
    /// Other outputs with the same message, which the phone could post instead.
    posted: Vec<(u64, String)>,
    transcript: String,
}

#[derive(Serialize)]
struct RefusedSpend {
    name: String,
    request: String,
    code: String,
    /// Words the refusal's message contains, which tell the rule that refused it.
    reason: String,
}

#[derive(Serialize)]
struct RefusedMint {
    name: String,
    descriptor: String,
    code: String,
    reason: String,
}

fn main() {
    let mut rng = ChaCha20Rng::seed_from_u64(0xca5b_0001);
    let mint = fixture_mint();
    let vectors = Vectors {
        generator: "devtools-cashu/src/bin/golden.rs".to_owned(),
        cashu_version: CASHU_VERSION.to_owned(),
        scheme: fps_cashu_core::keys::SCHEME.to_owned(),
        hash_to_curve: hash_to_curve_cases(&mut rng),
        blind: blind_cases(&mut rng),
        keyset_ids: keyset_id_cases(&mut rng),
        mint_urls: url_cases(),
        key_scheme: key_scheme_cases(&mut rng, &mint),
        dleq: dleq_cases(&mut rng, &mint),
        mint: MintCase {
            app_seed: h(APP_SEED),
            url: MINT_URL.to_owned(),
            descriptor: cbor(&mint.descriptor()),
        },
        spends: spend_cases(&mut rng, &mint),
        refused: Refused {
            transcripts: vec![demo_one(), demo_two()],
            spends: refused_spends(&mut rng, &mint),
            mints: refused_mints(&mint),
        },
    };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/cashu-core/tests/vectors/cashu-0.18.3.json");
    let text = serde_json::to_string_pretty(&vectors).expect("serialises") + "\n";
    std::fs::write(&path, text).expect("writes the vectors");
    println!("wrote {}", path.display());
}

/// The mint the key scheme, DLEQ and spend cases use: a fee-charging active keyset with a final
/// expiry, an inactive keyset with a version 00 id, and a free active keyset.
fn fixture_mint() -> TestMint {
    let amounts = standard_amounts();
    TestMint {
        url: MINT_URL.to_owned(),
        keysets: vec![
            TestKeyset::generate(
                b"fixture keyset 1",
                &amounts,
                100,
                Some(2_000_000_000),
                KeySetVersion::Version01,
                true,
            ),
            TestKeyset::generate(
                b"fixture keyset 2",
                &amounts,
                0,
                None,
                KeySetVersion::Version00,
                false,
            ),
            TestKeyset::generate(
                b"fixture keyset 3",
                &amounts,
                0,
                None,
                KeySetVersion::Version01,
                true,
            ),
        ],
    }
}

fn cdk_hash_to_curve(message: &[u8]) -> PublicKey {
    dhke::hash_to_curve(message).expect("a point")
}

fn hash_to_curve_cases(rng: &mut ChaCha20Rng) -> Vec<HashCase> {
    // NUT-00's vectors, then random byte strings of several lengths, then template secrets.
    let mut messages: Vec<Vec<u8>> = (0u8..3)
        .map(|last| {
            let mut message = vec![0u8; 32];
            message[31] = last;
            message
        })
        .collect();
    for length in [0usize, 1, 31, 32, 33, 64, 100, 192] {
        let mut message = vec![0u8; length];
        rng.fill(message.as_mut_slice());
        messages.push(message);
    }
    for _ in 0..3 {
        let nonce = random_bytes(rng);
        let key = *random_point(rng);
        messages.push(secret::build(&nonce, &key).into_bytes());
    }
    messages
        .into_iter()
        .map(|message| HashCase {
            point: h(cdk_hash_to_curve(&message).to_bytes()),
            message: h(message),
        })
        .collect()
}

fn blind_cases(rng: &mut ChaCha20Rng) -> Vec<BlindCase> {
    // NUT-00's two vectors, then random secrets and blinding factors.
    let mut cases: Vec<(Vec<u8>, [u8; 32])> = [
        (
            "d341ee4871f1f889041e63cf0d3823c713eea6aff01e80f1719f08f9e5be98f6",
            "99fce58439fc37412ab3468b73db0569322588f62fb3a49182d67e23d877824a",
        ),
        (
            "f1aaf16c2239746f369572c0784d9dd3d032d952c2d992175873fb58fae31a60",
            "f78476ea7cc9ade20f9e05e58a804cf19533f03ea805ece5fee88c8e2874ba50",
        ),
    ]
    .iter()
    .map(|(secret, r)| {
        (
            hex::decode(secret).expect("hex"),
            hex::decode(r).expect("hex").try_into().expect("32 bytes"),
        )
    })
    .collect();
    for _ in 0..6 {
        cases.push((
            hex::encode(random_bytes(rng)).into_bytes(),
            random_scalar(rng),
        ));
    }
    cases
        .into_iter()
        .map(|(secret, r)| {
            let blinding = SecretKey::from_slice(&r).expect("valid r");
            let (b, _) = dhke::blind_message(&secret, Some(blinding)).expect("blinds");
            BlindCase {
                secret: h(secret),
                r: h(r),
                b: h(b.to_bytes()),
            }
        })
        .collect()
}

fn keyset_id_cases(rng: &mut ChaCha20Rng) -> Vec<KeysetIdCase> {
    let mut cases = Vec::new();
    // NUT-02's own examples: four keys, as version 00 and as version 01 with a fee and an expiry.
    let nut02 = [
        "03a40f20667ed53513075dc51e715ff2046cad64eb68960632269ba7f0210e38bc",
        "03fd4ce5a16b65576145949e6f99f445f8249fee17c606b688b504a849cdc452de",
        "02648eccfa4c026960966276fa5a4cae46ce0fd432211a4f449bf84f13aa5f8303",
        "02fdfd6796bfeac490cbee12f778f867f0a2c68f6508d17c649759ea0dc3547528",
    ];
    let keys: Vec<(u64, PublicKey)> = nut02
        .iter()
        .enumerate()
        .map(|(bit, key)| (1u64 << bit, PublicKey::from_hex(key).expect("point")))
        .collect();
    let case = keyset_id_case(&keys, 100, Some(2_059_210_353));
    assert_eq!(case.id_v00, "00456a94ab4e1c46", "NUT-02 version 00 vector");
    assert_eq!(
        case.id_v01, "015ba18a8adcd02e715a58358eb618da4a4b3791151a4bee5e968bb88406ccf76a",
        "NUT-02 version 01 vector"
    );
    cases.push(case);
    // Random keysets: every combination of fee and expiry, including an explicit zero expiry,
    // which `cashu` leaves out of the id.
    let settings = [
        (0, None),
        (100, None),
        (0, Some(1_900_000_000)),
        (1_000, Some(1_900_000_000)),
        (100, Some(0)),
        (rng.gen_range(1..100_000), Some(rng.gen())),
    ];
    for (fee, expiry) in settings {
        let count = rng.gen_range(1..=64);
        let keys: Vec<(u64, PublicKey)> = (0..count)
            .map(|bit| (1u64 << bit, random_point(rng)))
            .collect();
        cases.push(keyset_id_case(&keys, fee, expiry));
    }
    cases
}

fn keyset_id_case(keys: &[(u64, PublicKey)], fee: u64, expiry: Option<u64>) -> KeysetIdCase {
    let map: BTreeMap<Amount, PublicKey> = keys
        .iter()
        .map(|(amount, key)| (Amount::from(*amount), *key))
        .collect();
    let keys_map = Keys::new(map);
    KeysetIdCase {
        keys: keys
            .iter()
            .map(|(amount, key)| (*amount, h(key.to_bytes())))
            .collect(),
        unit: "sat".to_owned(),
        input_fee_ppk: fee,
        final_expiry: expiry,
        id_v00: Id::v1_from_keys(&keys_map).to_string(),
        id_v01: Id::v2_from_data(&keys_map, &CurrencyUnit::Sat, fee, expiry).to_string(),
    }
}

fn url_cases() -> Vec<UrlCase> {
    [
        "https://testnut.cashu.space",
        "https://testnut.cashu.space/",
        "https://testnut.cashu.space////",
        "HTTPS://TestNut.Cashu.Space",
        "https://mint.minibits.cash/Bitcoin",
        "https://mint.minibits.cash/Bitcoin/",
        "https://MINT.minibits.cash/Bitcoin/Path",
        "http://127.0.0.1:3338",
        "http://localhost:8085/",
        "https://mint.example:443/a//b/",
        "https://mint.example/a?b=c",
        "https://mint.example/path://more",
        "mint.example",
        "",
        "/",
        "://",
        "https://",
        "https:///path",
        "HTTPS://MİNT.example",
    ]
    .into_iter()
    .map(|input| UrlCase {
        input: input.to_owned(),
        canonical: MintUrl::from_str(input).ok().map(|url| url.to_string()),
    })
    .collect()
}

/// The template secret checked by `cashu`: it parses as a P2PK lock to `lock_key` with `SIG_ALL`
/// and nothing else, and re-serialises to exactly the same string.
fn assert_cashu_reads_template(text: &str, lock_key: &secp256k1::PublicKey) {
    let secret = Secret::new(text);
    let parsed = Nut10Secret::try_from(&secret).expect("cashu parses the template");
    let written = Secret::try_from(parsed).expect("cashu writes it back");
    assert_eq!(
        written.to_string(),
        text,
        "cashu re-serialises the template unchanged"
    );
    let conditions = SpendingConditions::try_from(&secret).expect("spending conditions");
    let expected = SpendingConditions::new_p2pk(
        PublicKey::from(*lock_key),
        Some(Conditions::new(None, None, None, None, Some(SigFlag::SigAll), None).expect("valid")),
    );
    assert_eq!(conditions, expected, "a single-key SIG_ALL lock");
}

fn key_scheme_cases(rng: &mut ChaCha20Rng, mint: &TestMint) -> Vec<KeySchemeCase> {
    let keysets: Vec<Vec<u8>> = vec![mint.keysets[0].id_bytes(), mint.keysets[1].id_bytes()];
    let seeds = [[0u8; 32], APP_SEED, random_bytes(rng)];
    let urls = [
        MINT_URL,
        "https://testnut.cashu.space",
        "http://127.0.0.1:3338",
    ];
    seeds
        .iter()
        .zip(urls)
        .map(|(seed, url)| {
            let keys = MintKeys::derive(seed, url).expect("derives");
            let phone = keys.phone();
            let epochs = [0u32, 1, 2, 7, u32::MAX]
                .into_iter()
                .map(|epoch| EpochCase {
                    epoch,
                    lock_tweak: h(phone.lock_tweak(epoch).expect("tweak").secret_bytes()),
                    lock_pubkey: h(keys.lock_pubkey(epoch).expect("key").serialize()),
                    signing_secret: h(keys.signing_key(epoch).expect("key").secret_bytes()),
                })
                .collect();
            let mut outputs = Vec::new();
            for epoch in [0u32, 3] {
                for keyset in &keysets {
                    for counter in [0u64, 1, 99, u64::MAX] {
                        outputs.push(output_case(phone, epoch, keyset, counter));
                    }
                }
            }
            KeySchemeCase {
                app_seed: h(seed),
                mint_url: url.to_owned(),
                mint_id: h(keys.mint_id()),
                lock_root: h(phone.lock_root().serialize()),
                view_key: h(phone.view_key()),
                epochs,
                outputs,
            }
        })
        .collect()
}

fn output_case(phone: &PhoneMintKeys, epoch: u32, keyset: &[u8], counter: u64) -> OutputCase {
    let (nonce, r) = phone.output(epoch, keyset, counter).expect("derives");
    let text = phone.lock_secret(epoch, keyset, counter).expect("derives");
    let lock_key = phone.lock_pubkey(epoch).expect("key");
    assert_cashu_reads_template(&text, &lock_key);
    let blinding = SecretKey::from_slice(&r.secret_bytes()).expect("valid r");
    let (b, _) = dhke::blind_message(text.as_bytes(), Some(blinding)).expect("blinds");
    OutputCase {
        epoch,
        keyset: h(keyset),
        counter,
        nonce,
        r: h(r.secret_bytes()),
        secret: text,
        b: h(b.to_bytes()),
    }
}

fn dleq_cases(rng: &mut ChaCha20Rng, mint: &TestMint) -> Vec<DleqCase> {
    let mut cases = Vec::new();
    // NUT-12's proof vector, signed by the mint key A = G.
    cases.push(DleqCase {
        secret: "daf4dd00a2b68a0858a80450f52c8a7d2ccf87d375e43e216e0c571f089f63e9".to_owned(),
        c: "024369d2d22a80ecf78f3937da9d5f30c1b9f74f0c32684d583cca0fa6a61cdcfc".to_owned(),
        e: "b31e58ac6527f34975ffab13e70a48b6d2b0d35abc4b03f0151f09ee1a9763d4".to_owned(),
        s: "8fbae004c59e754d71df67e392b6ae4e29293113ddc2ec86592a0431d16306d8".to_owned(),
        r: "a6d13fcd7a18442e6076f5e1e7c887ad5de40a019824bdfa9fe740d302e8d861".to_owned(),
        mint_key: "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798".to_owned(),
        valid: true,
    });
    let keys = MintKeys::derive(&APP_SEED, MINT_URL).expect("derives");
    let phone = keys.phone();
    for (index, amount) in [1u64, 8, 1024, 1 << 31].into_iter().enumerate() {
        let keyset = &mint.keysets[index % 2];
        // Template secrets, as the wallet's proofs carry, and a plain one.
        let (text, r) = if index == 3 {
            (hex::encode(random_bytes(rng)), random_scalar(rng))
        } else {
            let output = phone
                .blinded_output(0, &keyset.id_bytes(), index as u64)
                .expect("derives");
            (output.secret, output.r.secret_bytes())
        };
        let proof = keyset.issue(amount, &text, &r);
        let dleq = proof.dleq.clone().expect("the mint adds a DLEQ proof");
        let mint_key = keyset.mint_key(amount);
        let case = |e: [u8; 32], s: [u8; 32], key: secp256k1::PublicKey| {
            let mut probe = proof.clone();
            probe.dleq = Some(cashu::nuts::ProofDleq::new(
                SecretKey::from_slice(&e).expect("scalar"),
                SecretKey::from_slice(&s).expect("scalar"),
                dleq.r.clone(),
            ));
            DleqCase {
                secret: text.clone(),
                c: h(proof.c.to_bytes()),
                e: h(e),
                s: h(s),
                r: h(dleq.r.to_secret_bytes()),
                mint_key: h(key.serialize()),
                valid: probe.verify_dleq(PublicKey::from(key)).is_ok(),
            }
        };
        let e = dleq.e.to_secret_bytes();
        let s = dleq.s.to_secret_bytes();
        let valid = case(e, s, mint_key);
        assert!(valid.valid, "cashu accepts its own DLEQ proof");
        cases.push(valid);
        let mut tampered = s;
        tampered[31] ^= 1;
        let tampered = case(e, tampered, mint_key);
        assert!(!tampered.valid, "cashu refuses a tampered DLEQ proof");
        cases.push(tampered);
        // The key of another amount: a proof claiming the wrong amount.
        let other = if amount == 1 { 2 } else { 1 };
        let wrong = case(e, s, keyset.mint_key(other));
        assert!(
            !wrong.valid,
            "cashu refuses the DLEQ proof under another key"
        );
        cases.push(wrong);
    }
    cases
}

fn mint_id() -> [u8; 32] {
    fps_cashu_core::keys::mint_id(MINT_URL)
}

fn melt_kind(quote: &str, amount_msat: u64, amount: u64, fee_reserve: u64) -> CashuSpendKind {
    CashuSpendKind::Melt {
        quote: quote.to_owned(),
        invoice: invoice(
            amount_msat,
            "fixture melt",
            sha256::Hash::hash(quote.as_bytes()).to_byte_array(),
        ),
        amount,
        fee_reserve,
    }
}

/// A UUID-shaped quote id, as CDK mints issue.
fn quote_id(rng: &mut ChaCha20Rng) -> String {
    let bytes = random_bytes(rng);
    let text = hex::encode(&bytes[..16]);
    format!(
        "{}-{}-{}-{}-{}",
        &text[..8],
        &text[8..12],
        &text[12..16],
        &text[16..20],
        &text[20..32]
    )
}

/// Analyses, checks the message against `cashu`'s, signs, and checks `cashu` accepts the
/// signature.
fn spend_case(
    rng: &mut ChaCha20Rng,
    mint: &TestMint,
    name: &str,
    spend: &Spend,
    epoch: u32,
    kind: CashuSpendKind,
) -> SpendCase {
    let descriptor = mint.descriptor();
    let pinned = validate_mint(&descriptor, None).expect("the fixture mint validates");
    let keys = MintKeys::derive(&APP_SEED, MINT_URL).expect("derives");
    let request = spend.request(mint_id(), epoch, kind.clone());
    let analysis =
        analyze(&request, &pinned, &keys).unwrap_or_else(|error| panic!("{name}: {error}"));
    let cashu_message = match &kind {
        CashuSpendKind::Swap => spend.swap().sig_all_msg_to_sign(),
        CashuSpendKind::Melt { quote, .. } => spend.melt(quote).sig_all_msg_to_sign(),
    };
    assert_eq!(
        analysis.message, cashu_message,
        "{name}: the message is cashu's"
    );
    let aux_rand = random_bytes(rng);
    let signature = sign_spend(&keys, &analysis, aux_rand).expect("signs");
    let accepted = match &kind {
        CashuSpendKind::Swap => swap_accepts(&spend.swap(), &signature),
        CashuSpendKind::Melt { quote, .. } => melt_accepts(&spend.melt(quote), &signature),
    };
    assert!(
        accepted,
        "{name}: cashu's verify_spending_conditions accepts the signature"
    );
    SpendCase {
        name: name.to_owned(),
        request: cbor(&request),
        kind: format!("{:?}", analysis.kind),
        title: analysis.review.title.clone(),
        amount_msat: analysis.review.amount_msat,
        outflow_msat: analysis.outflow_msat,
        message: analysis.message.clone(),
        message_hash: h(analysis.message_hash),
        aux_rand: h(aux_rand),
        signature: h(signature),
    }
}

fn spend_cases(rng: &mut ChaCha20Rng, mint: &TestMint) -> Vec<SpendCase> {
    let keys = MintKeys::derive(&APP_SEED, MINT_URL).expect("derives");
    let phone = keys.phone();
    let [fee_keyset, old_keyset, free_keyset] =
        [&mint.keysets[0], &mint.keysets[1], &mint.keysets[2]];
    let mut cases = Vec::new();

    // Send 8 sats, with 4 of change; three inputs at 100 ppk cost 1 sat.
    let mut send = Spend::default();
    send.input(phone, fee_keyset, 0, 0, 8, false);
    send.input(phone, fee_keyset, 0, 1, 4, false);
    send.input(phone, fee_keyset, 0, 2, 1, false);
    send.change(phone, fee_keyset, 0, 10, 4);
    send.leaving(fee_keyset, 8, rng);
    cases.push(spend_case(
        rng,
        mint,
        "send",
        &send,
        0,
        CashuSpendKind::Swap,
    ));

    // Combine three proofs of the inactive version 00 keyset into one in an active keyset.
    let mut combine = Spend::default();
    combine.input(phone, old_keyset, 0, 0, 2, true);
    combine.input(phone, old_keyset, 0, 1, 2, true);
    combine.input(phone, old_keyset, 0, 2, 4, true);
    combine.change(phone, free_keyset, 0, 0, 8);
    cases.push(spend_case(
        rng,
        mint,
        "combine",
        &combine,
        0,
        CashuSpendKind::Swap,
    ));

    // Move two proofs of epoch 1 to the key of epoch 2.
    let mut rotate = Spend::default();
    rotate.input(phone, free_keyset, 1, 5, 16, false);
    rotate.input(phone, free_keyset, 1, 6, 16, false);
    rotate.change(phone, free_keyset, 2, 0, 32);
    cases.push(spend_case(
        rng,
        mint,
        "rotate",
        &rotate,
        1,
        CashuSpendKind::Swap,
    ));

    // Send from epoch 3 with change at epochs 3 and 6, inputs from two keysets.
    let mut ahead = Spend::default();
    ahead.input(phone, old_keyset, 3, 0, 64, false);
    ahead.input(phone, free_keyset, 3, 0, 32, false);
    ahead.leaving(free_keyset, 16, rng);
    ahead.change(phone, free_keyset, 3, 1, 16);
    ahead.change(phone, free_keyset, 6, 0, 64);
    cases.push(spend_case(
        rng,
        mint,
        "send-change-ahead",
        &ahead,
        3,
        CashuSpendKind::Swap,
    ));

    // Pay 100,500 msat: a quote of 101 sats, 2 of fee reserve, 1 of input fee, two blanks.
    let quote = quote_id(rng);
    let mut melt = Spend::default();
    melt.input(phone, fee_keyset, 0, 20, 64, true);
    melt.input(phone, fee_keyset, 0, 21, 32, true);
    melt.input(phone, fee_keyset, 0, 22, 8, true);
    melt.change(phone, fee_keyset, 0, 30, 0);
    melt.change(phone, fee_keyset, 0, 31, 0);
    let kind = melt_kind(&quote, 100_500, 101, 2);
    cases.push(spend_case(rng, mint, "melt", &melt, 0, kind));

    // Pay 21,000 msat from one proof of 128: seven blanks return up to 106 sats.
    let quote = quote_id(rng);
    let mut overshoot = Spend::default();
    overshoot.input(phone, fee_keyset, 0, 40, 128, true);
    for counter in 41..48 {
        overshoot.change(phone, fee_keyset, 0, counter, 0);
    }
    let kind = melt_kind(&quote, 21_000, 21, 1);
    cases.push(spend_case(rng, mint, "melt-overshoot", &overshoot, 0, kind));
    cases
}

fn refused_spends(rng: &mut ChaCha20Rng, mint: &TestMint) -> Vec<RefusedSpend> {
    let keys = MintKeys::derive(&APP_SEED, MINT_URL).expect("derives");
    let phone = keys.phone();
    let pinned = validate_mint(&mint.descriptor(), None).expect("validates");
    let [fee_keyset, _, free_keyset] = [&mint.keysets[0], &mint.keysets[1], &mint.keysets[2]];
    let mut cases = Vec::new();
    let mut refused = |name: &str, request: CashuSignRequest, code: RejectCode, reason: &str| {
        let error = analyze(&request, &pinned, &keys).expect_err(name);
        assert_eq!(error.code, code, "{name}: {error}");
        assert!(error.message.contains(reason), "{name}: {error}");
        cases.push(RefusedSpend {
            name: name.to_owned(),
            request: cbor(&request),
            code: format!("{code:?}"),
            reason: reason.to_owned(),
        });
    };

    // The second demo's collision, made from the wallet's own change by grinding counters: Prime
    // would show 4 sats leaving while the phone could post 1,024 to the same output.
    let collision = own_change_collision(rng, phone, free_keyset);
    refused(
        "collision-from-own-change",
        collision.request(mint_id(), 0, CashuSpendKind::Swap),
        RejectCode::InvalidTransaction,
        "can be read in more than one way",
    );

    // A change output whose blinding factor was changed after it was blinded.
    let mut tampered = Spend::default();
    tampered.input(phone, fee_keyset, 0, 0, 8, false);
    tampered.input(phone, fee_keyset, 0, 1, 4, false);
    tampered.input(phone, fee_keyset, 0, 2, 1, false);
    tampered.change(phone, fee_keyset, 0, 10, 4);
    tampered.leaving(fee_keyset, 8, rng);
    let mut request = tampered.request(mint_id(), 0, CashuSpendKind::Swap);
    request.outputs[0].opening.r.0[31] ^= 1;
    refused(
        "tampered-opening",
        request,
        RejectCode::UnexpectedOutput,
        "output 0: B_ is not hash_to_curve(secret) + r·G",
    );

    // Inputs locked to the key of epoch 1, in a request for epoch 0.
    let mut foreign = Spend::default();
    foreign.input(phone, free_keyset, 1, 0, 8, false);
    foreign.change(phone, free_keyset, 0, 0, 8);
    refused(
        "foreign-lock-key",
        foreign.request(mint_id(), 0, CashuSpendKind::Swap),
        RejectCode::ForeignInput,
        "input 0 is not locked to this wallet's key for epoch 0",
    );

    // A send dressed as change: the hint names the wallet's output at counter 10, and `r` is that
    // output's, but the secret is a stranger's.
    let mut disguised = Spend::default();
    disguised.input(phone, fee_keyset, 0, 0, 8, false);
    disguised.input(phone, fee_keyset, 0, 1, 4, false);
    disguised.input(phone, fee_keyset, 0, 2, 1, false);
    let own = phone
        .blinded_output(0, &fee_keyset.id_bytes(), 10)
        .expect("derives");
    let hint = CashuOutputHint {
        epoch: 0,
        keyset: ByteBuf(fee_keyset.id_bytes()),
        counter: 10,
    };
    disguised.push_output(
        fee_keyset,
        4,
        hex::encode(random_bytes(rng)),
        own.r.secret_bytes(),
        Some(hint),
    );
    disguised.leaving(fee_keyset, 8, rng);
    refused(
        "change-foreign-secret-own-r",
        disguised.request(mint_id(), 0, CashuSpendKind::Swap),
        RejectCode::UnexpectedOutput,
        "output 0: it is not this wallet's output at epoch 0, counter 10",
    );

    // A melt with one input whose DLEQ proof is missing, so Prime cannot know what it is worth.
    let quote = quote_id(rng);
    let mut undisclosed = Spend::default();
    undisclosed.input(phone, fee_keyset, 0, 20, 64, true);
    undisclosed.input(phone, fee_keyset, 0, 21, 32, false);
    undisclosed.input(phone, fee_keyset, 0, 22, 8, true);
    undisclosed.change(phone, fee_keyset, 0, 30, 0);
    undisclosed.change(phone, fee_keyset, 0, 31, 0);
    refused(
        "melt-without-dleq",
        undisclosed.request(mint_id(), 0, melt_kind(&quote, 100_500, 101, 2)),
        RejectCode::InvalidNote,
        "input 1: a melt needs a DLEQ proof on every input",
    );

    // Change one epoch beyond the window, where a restore stops looking.
    let mut beyond = Spend::default();
    beyond.input(phone, free_keyset, 1, 0, 8, false);
    beyond.change(phone, free_keyset, 1 + MAX_EPOCHS_AHEAD + 1, 0, 8);
    refused(
        "change-epoch-out-of-window",
        beyond.request(mint_id(), 1, CashuSpendKind::Swap),
        RejectCode::UnexpectedOutput,
        "output 0: change at epoch 5 is outside epochs 1 to 1 + 3",
    );

    // One proof of 128 for 21 sats: up to 106 come back, which takes seven blanks, not six.
    let quote = quote_id(rng);
    let mut short = Spend::default();
    short.input(phone, fee_keyset, 0, 40, 128, true);
    for counter in 41..47 {
        short.change(phone, fee_keyset, 0, counter, 0);
    }
    refused(
        "melt-too-few-blanks",
        short.request(mint_id(), 0, melt_kind(&quote, 21_000, 21, 1)),
        RejectCode::PolicyLimit,
        "6 blank outputs cannot return up to 106 sats of change; at least 7 are needed",
    );
    cases
}

/// Grinds counters until the wallet's own change outputs reproduce the second demo: a 1,024 output
/// whose `B_` ends in `402` or `403` and a 1 output whose `B_` ends in `102`, each leaving a valid
/// point when the digits shift.
fn own_change_collision(
    rng: &mut ChaCha20Rng,
    phone: &PhoneMintKeys,
    keyset: &TestKeyset,
) -> Spend {
    let id = keyset.id_bytes();
    let output_hex = |counter: u64| {
        h(phone
            .blinded_output(0, &id, counter)
            .expect("derives")
            .b
            .serialize())
    };
    let c1 = (1_000u64..)
        .find(|counter| {
            let h1 = output_hex(*counter);
            matches!(&h1[63..], "402" | "403") && is_point_hex(&format!("024{}", &h1[..63]))
        })
        .expect("a counter");
    let h1 = output_hex(c1);
    let c2 = (100_000u64..)
        .find(|counter| {
            let h2 = output_hex(*counter);
            &h2[63..] == "102" && is_point_hex(&format!("{}1{}", &h1[64..], &h2[..63]))
        })
        .expect("a counter");
    let mut spend = Spend::default();
    spend.input(phone, keyset, 0, 0, 1024, false);
    spend.input(phone, keyset, 0, 1, 4, false);
    spend.input(phone, keyset, 0, 2, 1, false);
    spend.change(phone, keyset, 0, c1, 1024);
    spend.change(phone, keyset, 0, c2, 1);
    spend.leaving(keyset, 4, rng);

    // The alternative really signs the same: build it with cashu and compare the messages.
    let h2 = h(spend.outputs[1].b.0);
    let h3 = h(spend.outputs[2].b.0);
    let j1 = format!("024{}", &h1[..63]);
    let j2 = format!("{}1{}", &h1[64..], &h2[..63]);
    let posted: Vec<BlindedMessage> = [(1u64, j1), (4, j2), (1024, h3)]
        .into_iter()
        .map(|(amount, key)| {
            BlindedMessage::new(
                Amount::from(amount),
                keyset.id(),
                PublicKey::from_hex(&key).expect("point"),
            )
        })
        .collect();
    let alternative = cashu::nuts::SwapRequest::new(spend.proofs.clone(), posted);
    assert_eq!(
        alternative.sig_all_msg_to_sign(),
        spend.swap().sig_all_msg_to_sign(),
        "the ground outputs collide"
    );
    spend
}

fn refused_mints(mint: &TestMint) -> Vec<RefusedMint> {
    let mut cases = Vec::new();
    let mut refused =
        |name: &str, descriptor: CashuMintDescriptor, code: RejectCode, reason: &str| {
            let error = validate_mint(&descriptor, None).expect_err(name);
            assert_eq!(error.code, code, "{name}: {error}");
            assert!(error.message.contains(reason), "{name}: {error}");
            cases.push(RefusedMint {
                name: name.to_owned(),
                descriptor: cbor(&descriptor),
                code: format!("{code:?}"),
                reason: reason.to_owned(),
            });
        };
    let mut descriptor = mint.descriptor();
    descriptor.url = "https://Mint.Fixture.Example/".to_owned();
    assert_eq!(
        MintUrl::from_str(&descriptor.url)
            .expect("parses")
            .to_string(),
        MINT_URL,
        "cashu renders the fixture URL differently"
    );
    refused(
        "non-canonical-url",
        descriptor,
        RejectCode::InvalidDescriptor,
        "is not in canonical form",
    );

    let mut descriptor = mint.descriptor();
    descriptor.keysets[0].id.0[5] ^= 1;
    refused(
        "wrong-keyset-id",
        descriptor,
        RejectCode::InvalidDescriptor,
        "the id is not derived from its keys",
    );

    // The fee keyset's own keys under reversed amounts, with a valid id, beside the keyset itself:
    // pinned, it would let a melt count a real proof of 2^20 as one of 2^11.
    let real = &mint.keysets[0];
    let amounts = standard_amounts();
    let relabelled: Vec<(u64, secp256k1::PublicKey)> = amounts
        .iter()
        .zip(amounts.iter().rev())
        .map(|(amount, other)| (*amount, real.mint_key(*other)))
        .collect();
    let mut descriptor = mint.descriptor();
    descriptor.keysets.push(CashuKeyset {
        id: ByteBuf(id_v01(&relabelled, "sat", 0, None).to_vec()),
        unit: "sat".to_owned(),
        input_fee_ppk: 0,
        final_expiry: None,
        active: false,
        keys: relabelled
            .iter()
            .map(|(amount, key)| CashuKey {
                amount: *amount,
                pubkey: FixedBytes(key.serialize()),
            })
            .collect(),
    });
    refused(
        "key-under-two-amounts",
        descriptor,
        RejectCode::InvalidDescriptor,
        "a key stands for one amount",
    );
    cases
}

/// The generator `G`.
fn generator() -> secp256k1::PublicKey {
    point_of(1)
}

/// `Y + r·G` for `r` from 1 up, until `accept` takes the point's hex: the demos' grinding loop.
fn grind(start: secp256k1::PublicKey, accept: impl Fn(&str) -> bool) -> String {
    let mut point = start.combine(&generator()).expect("not infinity");
    loop {
        let text = h(point.serialize());
        if accept(&text) {
            return text;
        }
        point = point.combine(&generator()).expect("not infinity");
    }
}

fn demo_template(nonce: &[u8; 32], lock: &secp256k1::PublicKey) -> String {
    secret::build(nonce, lock)
}

fn transcript(outputs: &[(u64, String)]) -> String {
    outputs
        .iter()
        .map(|(amount, key)| format!("{amount}{key}"))
        .collect()
}

/// `review-crypto/demo.py`: Prime would see 1,021 of change and 4 leaving; the same message posts
/// 1 and 1,024 leaving. (1,021 is no power of two, so the analysis refuses it earlier as well.)
fn demo_one() -> TranscriptCase {
    let lock = point_of(0xC0FFEE);
    let nonce = sha256::Hash::hash(b"nonce").to_byte_array();
    let y = *cdk_hash_to_curve(demo_template(&nonce, &lock).as_bytes());
    let h1 = grind(y, |text| {
        text.ends_with("102") && is_point_hex(&format!("021{}", &text[..63]))
    });
    let h2 = h(point_of(0xBADBEEF).serialize());
    let j1 = format!("021{}", &h1[..63]);
    let case = transcript_case(
        "demo-1",
        vec![(1021, h1), (4, h2.clone())],
        vec![(1, j1), (1024, h2)],
    );
    assert_eq!(
        h(sha256::Hash::hash(case.transcript.as_bytes()).to_byte_array()),
        "bf5638f3277b7941c11ca79674ade7a534d4ce72ce8eceb3497de73b7395da20",
        "the transcript demo.py prints"
    );
    case
}

/// `review-crypto/demo2.py`: every amount a power of two. Prime would see 1,025 of change and 4
/// leaving; the same message posts 1 and 4 to points nobody can open and 1,024 leaving.
fn demo_two() -> TranscriptCase {
    let lock = point_of(0xC0FFEE);
    let template = |tag: &[u8]| demo_template(&sha256::Hash::hash(tag).to_byte_array(), &lock);
    let y1 = *cdk_hash_to_curve(template(b"n1").as_bytes());
    let h1 = grind(y1, |text| {
        matches!(&text[63..], "402" | "403") && is_point_hex(&format!("024{}", &text[..63]))
    });
    let y2 = *cdk_hash_to_curve(template(b"n2").as_bytes());
    let h2 = grind(y2, |text| {
        &text[63..] == "102" && is_point_hex(&format!("{}1{}", &h1[64..], &text[..63]))
    });
    let h3 = h(point_of(0xBADBEEF).serialize());
    let j1 = format!("024{}", &h1[..63]);
    let j2 = format!("{}1{}", &h1[64..], &h2[..63]);
    transcript_case(
        "demo-2",
        vec![(1024, h1), (1, h2), (4, h3.clone())],
        vec![(1, j1), (4, j2), (1024, h3)],
    )
}

fn transcript_case(
    name: &str,
    signed: Vec<(u64, String)>,
    posted: Vec<(u64, String)>,
) -> TranscriptCase {
    let text = transcript(&signed);
    assert_eq!(
        text,
        transcript(&posted),
        "{name}: the two lists read the same"
    );
    assert!(
        message::output_readings(&text) >= 2,
        "{name}: fps-cashu-core sees two readings"
    );
    // cashu builds the same message for both lists, so one signature would cover both.
    let id = Id::from_str("00bfa73302d12ffd").expect("id");
    let as_outputs = |list: &[(u64, String)]| -> Vec<BlindedMessage> {
        list.iter()
            .map(|(amount, key)| {
                BlindedMessage::new(
                    Amount::from(*amount),
                    id,
                    PublicKey::from_hex(key).expect("point"),
                )
            })
            .collect()
    };
    let message = |list: &[(u64, String)]| {
        cashu::nuts::SwapRequest::new(vec![], as_outputs(list)).sig_all_msg_to_sign()
    };
    assert_eq!(
        message(&signed),
        message(&posted),
        "{name}: cashu signs both the same"
    );
    TranscriptCase {
        name: name.to_owned(),
        signed,
        posted,
        transcript: text,
    }
}
