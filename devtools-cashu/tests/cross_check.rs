//! Seeded randomized cross-checks of every fps-cashu-core primitive against the `cashu` 0.18.3
//! crate, ending with the check that matters: Prime's signatures over random swaps and melts pass
//! `cashu`'s own `verify_spending_conditions`, the check a CDK mint runs.
//!
//!   cargo test --release --manifest-path devtools-cashu/Cargo.toml

use std::collections::BTreeMap;
use std::str::FromStr;

use bitcoin::secp256k1;
use cashu::dhke;
use cashu::nuts::nut02::KeySetVersion;
use cashu::nuts::{
    BlindedMessage, Conditions, CurrencyUnit, Id, Keys, MeltRequest, Nut10Secret, SigFlag,
    SpendingConditionVerification, SpendingConditions, SwapRequest,
};
use cashu::secret::Secret;
use cashu::{Amount, MintUrl, PublicKey, SecretKey};
use fps_cashu_core::keys::MintKeys;
use fps_cashu_core::keyset::{id_v00, id_v01};
use fps_cashu_core::message::SigAllMessage;
use fps_cashu_core::{analyze, dleq, secret, sign_spend, url, validate_mint, SpendKind};
use fps_devtools_cashu::*;
use fps_protocol::{CashuDleq, CashuSpendKind, FixedBytes, RejectCode};
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;

fn rng(seed: u64) -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(seed)
}

#[test]
fn hash_to_curve_and_blinding_match_cashu() {
    let mut rng = rng(1);
    for _ in 0..500 {
        let mut message = vec![0u8; rng.gen_range(0..300)];
        rng.fill(message.as_mut_slice());
        let ours = fps_cashu_core::dhke::hash_to_curve(&message).expect("a point");
        let theirs = dhke::hash_to_curve(&message).expect("a point");
        assert_eq!(ours, *theirs);

        let r = random_scalar(&mut rng);
        let ours =
            fps_cashu_core::dhke::blind(&message, &secp256k1::SecretKey::from_slice(&r).unwrap());
        let (theirs, _) =
            dhke::blind_message(&message, Some(SecretKey::from_slice(&r).unwrap())).unwrap();
        assert_eq!(ours, Some(*theirs));
    }
}

fn random_keys(rng: &mut ChaCha20Rng) -> Vec<(u64, secp256k1::PublicKey)> {
    let mut bits: Vec<u32> = (0..64).filter(|_| rng.gen_bool(0.5)).collect();
    if bits.is_empty() {
        bits.push(rng.gen_range(0..64));
    }
    bits.into_iter()
        .map(|bit| (1u64 << bit, *random_point(rng)))
        .collect()
}

#[test]
fn keyset_ids_match_cashu() {
    let mut rng = rng(2);
    for _ in 0..300 {
        let keys = random_keys(&mut rng);
        let fee = *[0u64, 0, 100, 1_000, rng.gen()].choose(&mut rng).unwrap();
        let expiry = *[
            None,
            Some(0),
            Some(rng.gen()),
            Some(rng.gen_range(1..u64::from(u32::MAX))),
        ]
        .choose(&mut rng)
        .unwrap();
        let map = Keys::new(
            keys.iter()
                .map(|(amount, key)| (Amount::from(*amount), PublicKey::from(*key)))
                .collect::<BTreeMap<_, _>>(),
        );
        // Ours sorts by amount as cashu does, whatever order the keys come in.
        let mut shuffled = keys.clone();
        shuffled.shuffle(&mut rng);
        assert_eq!(
            id_v00(&shuffled).to_vec(),
            Id::v1_from_keys(&map).to_bytes()
        );
        assert_eq!(
            id_v01(&shuffled, "sat", fee, expiry).to_vec(),
            Id::v2_from_data(&map, &CurrencyUnit::Sat, fee, expiry).to_bytes()
        );
    }
}

#[test]
fn mints_generated_by_cashu_validate_and_altered_ones_do_not() {
    let mut rng = rng(3);
    for round in 0..40u32 {
        let count = rng.gen_range(1..=4);
        let keysets: Vec<TestKeyset> = (0..count)
            .map(|index| {
                let amounts: Vec<u64> = (0..rng.gen_range(1..=64)).map(|bit| 1u64 << bit).collect();
                let version = if rng.gen_bool(0.5) {
                    KeySetVersion::Version00
                } else {
                    KeySetVersion::Version01
                };
                let expiry = [None, Some(0), Some(1_900_000_000)][rng.gen_range(0..3)];
                TestKeyset::generate(
                    format!("mint {round} keyset {index}").as_bytes(),
                    &amounts,
                    rng.gen_range(0..=1_000),
                    expiry,
                    version,
                    index == 0 || rng.gen_bool(0.5),
                )
            })
            .collect();
        let mint = TestMint {
            url: format!("https://mint-{round}.example"),
            keysets,
        };
        let pinned = validate_mint(&mint.descriptor(), None).expect("cashu's keysets validate");
        assert_eq!(pinned.keysets.len(), count);
        for keyset in &mint.keysets {
            let ours = pinned.keyset(&keyset.id_bytes()).expect("pinned");
            for (amount, key) in keyset.keys().iter() {
                assert_eq!(ours.key(amount.to_u64()), Some(&**key));
            }
        }
        // The same keysets again update nothing.
        assert_eq!(
            validate_mint(&mint.descriptor(), Some(&pinned)).unwrap(),
            pinned
        );
        let mut altered = mint.descriptor();
        let keyset = rng.gen_range(0..count);
        let byte = rng.gen_range(1..altered.keysets[keyset].id.0.len());
        altered.keysets[keyset].id.0[byte] ^= 1 << rng.gen_range(0..8);
        let error = validate_mint(&altered, None).unwrap_err();
        assert_eq!(error.code, RejectCode::InvalidDescriptor);

        // A keyset of the mint's own keys under rotated amounts, so that no key keeps its amount,
        // has a valid version 01 id, and is refused beside the keys it relabels, in one
        // descriptor or as an update.
        let mut keys: Vec<(u64, secp256k1::PublicKey)> = mint.keysets[keyset]
            .keys()
            .iter()
            .map(|(amount, key)| (amount.to_u64(), **key))
            .collect();
        if keys.len() > 1 {
            let amounts: Vec<u64> = keys.iter().map(|(amount, _)| *amount).collect();
            let shift = 1 + rng.gen_range(0..keys.len() - 1);
            keys.rotate_left(shift);
            let relabelled: Vec<(u64, secp256k1::PublicKey)> = amounts
                .into_iter()
                .zip(keys.into_iter().map(|(_, key)| key))
                .collect();
            let mut descriptor = mint.descriptor();
            descriptor.keysets.push(fps_protocol::CashuKeyset {
                id: fps_protocol::ByteBuf(id_v01(&relabelled, "sat", 0, None).to_vec()),
                unit: "sat".to_owned(),
                input_fee_ppk: 0,
                final_expiry: None,
                active: true,
                keys: relabelled
                    .iter()
                    .map(|(amount, key)| fps_protocol::CashuKey {
                        amount: *amount,
                        pubkey: FixedBytes(key.serialize()),
                    })
                    .collect(),
            });
            for existing in [None, Some(&pinned)] {
                let error = validate_mint(&descriptor, existing).unwrap_err();
                assert_eq!(error.code, RejectCode::InvalidDescriptor);
                assert!(error.message.contains("a key stands for one amount"));
            }
        }
    }
}

#[test]
fn template_secrets_read_the_same_in_both_crates() {
    let mut rng = rng(4);
    let sig_all = || Conditions::new(None, None, None, None, Some(SigFlag::SigAll), None).unwrap();
    for _ in 0..200 {
        // Ours, read and rewritten by cashu.
        let key = *random_point(&mut rng);
        let text = secret::build(&random_bytes(&mut rng), &key);
        let parsed = Nut10Secret::try_from(&Secret::new(text.clone())).unwrap();
        assert_eq!(Secret::try_from(parsed).unwrap().to_string(), text);
        assert_eq!(
            SpendingConditions::try_from(&Secret::new(text.clone())).unwrap(),
            SpendingConditions::new_p2pk(PublicKey::from(key), Some(sig_all()))
        );

        // cashu's own single-key SIG_ALL lock, read by ours.
        let conditions = SpendingConditions::new_p2pk(PublicKey::from(key), Some(sig_all()));
        let theirs: Secret = Nut10Secret::try_from(conditions)
            .unwrap()
            .try_into()
            .unwrap();
        let read = secret::parse(&theirs.to_string()).expect("ours reads cashu's template");
        assert_eq!(read.lock_key, key);

        // Any other lock cashu writes is not the template: SIG_INPUTS, another key, a locktime,
        // a signature count.
        let locktime = 4_000_000_000 + rng.gen_range(0..1_000_000);
        let other = match rng.gen_range(0..4) {
            0 => Conditions::new(None, None, None, None, Some(SigFlag::SigInputs), None),
            1 => Conditions::new(
                None,
                Some(vec![random_point(&mut rng)]),
                None,
                None,
                Some(SigFlag::SigAll),
                None,
            ),
            2 => Conditions::new(
                Some(locktime),
                None,
                Some(vec![random_point(&mut rng)]),
                None,
                Some(SigFlag::SigAll),
                None,
            ),
            _ => Conditions::new(
                None,
                Some(vec![random_point(&mut rng)]),
                None,
                Some(2),
                Some(SigFlag::SigAll),
                None,
            ),
        }
        .unwrap();
        let conditions = SpendingConditions::new_p2pk(PublicKey::from(key), Some(other));
        let theirs: Secret = Nut10Secret::try_from(conditions)
            .unwrap()
            .try_into()
            .unwrap();
        assert!(secret::parse(&theirs.to_string()).is_none(), "{theirs}");
    }
}

fn random_secret(rng: &mut ChaCha20Rng) -> String {
    match rng.gen_range(0..3) {
        0 => hex::encode(random_bytes(rng)),
        1 => secret::build(&random_bytes(rng), &random_point(rng)),
        _ => (0..rng.gen_range(1..80))
            .map(|_| char::from(rng.gen_range(0x20u8..0x7f)))
            .collect(),
    }
}

#[test]
fn messages_match_cashu_s_sig_all_message() {
    let mut rng = rng(5);
    let id = Id::from_str("00bfa73302d12ffd").unwrap();
    for _ in 0..300 {
        let inputs: Vec<(String, [u8; 33])> = (0..rng.gen_range(1..10))
            .map(|_| (random_secret(&mut rng), random_point(&mut rng).to_bytes()))
            .collect();
        let outputs: Vec<(u64, [u8; 33])> = (0..rng.gen_range(0..10))
            .map(|_| {
                let amount = *[0, 1, 1 << rng.gen_range(0..64), rng.gen()]
                    .choose(&mut rng)
                    .unwrap();
                (amount, random_point(&mut rng).to_bytes())
            })
            .collect();
        let proofs: Vec<cashu::nuts::Proof> = inputs
            .iter()
            .map(|(text, c)| {
                cashu::nuts::Proof::new(
                    Amount::from(1),
                    id,
                    Secret::new(text.clone()),
                    PublicKey::from_slice(c).unwrap(),
                )
            })
            .collect();
        let blinded: Vec<BlindedMessage> = outputs
            .iter()
            .map(|(amount, b)| {
                BlindedMessage::new(Amount::from(*amount), id, PublicKey::from_slice(b).unwrap())
            })
            .collect();
        let input_refs: Vec<(&str, &[u8; 33])> =
            inputs.iter().map(|(text, c)| (text.as_str(), c)).collect();
        let output_refs: Vec<(u64, &[u8; 33])> =
            outputs.iter().map(|(amount, b)| (*amount, b)).collect();

        let swap = SigAllMessage::build(&input_refs, &output_refs, None);
        assert_eq!(
            swap.as_str(),
            SwapRequest::new(proofs.clone(), blinded.clone()).sig_all_msg_to_sign()
        );

        let quote = quote_like(&mut rng);
        let melt = SigAllMessage::build(&input_refs, &output_refs, Some(&quote));
        assert_eq!(
            melt.as_str(),
            MeltRequest::new(quote.clone(), proofs, Some(blinded)).sig_all_msg_to_sign()
        );
    }
}

fn quote_like(rng: &mut ChaCha20Rng) -> String {
    let bytes = random_bytes(rng);
    if rng.gen_bool(0.5) {
        let text = hex::encode(&bytes[..16]);
        format!(
            "{}-{}-{}-{}-{}",
            &text[..8],
            &text[8..12],
            &text[12..16],
            &text[16..20],
            &text[20..]
        )
    } else {
        bytes
            .iter()
            .take(30)
            .map(|byte| {
                const ALPHABET: &[u8] =
                    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
                char::from(ALPHABET[usize::from(byte % 64)])
            })
            .collect()
    }
}

#[test]
fn dleq_verification_agrees_with_cashu() {
    let mut rng = rng(6);
    let amounts = standard_amounts();
    let keyset = TestKeyset::generate(
        b"dleq keyset",
        &amounts,
        0,
        None,
        KeySetVersion::Version01,
        true,
    );
    for _ in 0..100 {
        let amount = *amounts.choose(&mut rng).unwrap();
        let text = random_secret(&mut rng);
        let proof = keyset.issue(amount, &text, &random_scalar(&mut rng));
        let dleq_proof = proof.dleq.clone().unwrap();
        let mut wire = CashuDleq {
            e: FixedBytes(dleq_proof.e.to_secret_bytes()),
            s: FixedBytes(dleq_proof.s.to_secret_bytes()),
            r: FixedBytes(dleq_proof.r.to_secret_bytes()),
        };
        let key = keyset.mint_key(amount);
        assert!(dleq::verify_proof(text.as_bytes(), &proof.c, &wire, &key));
        assert!(proof.verify_dleq(PublicKey::from(key)).is_ok());

        // Tamper with one field, or ask for another amount's key: both refuse.
        match rng.gen_range(0..4) {
            0 => wire.e.0[rng.gen_range(0..32)] ^= 1 << rng.gen_range(0..8),
            1 => wire.s.0[rng.gen_range(0..32)] ^= 1 << rng.gen_range(0..8),
            2 => wire.r.0[rng.gen_range(0..32)] ^= 1 << rng.gen_range(0..8),
            _ => {}
        }
        let other_key = keyset.mint_key(if amount == 1 { 2 } else { 1 });
        let key = if wire.e == FixedBytes(dleq_proof.e.to_secret_bytes())
            && wire.s == FixedBytes(dleq_proof.s.to_secret_bytes())
            && wire.r == FixedBytes(dleq_proof.r.to_secret_bytes())
        {
            other_key
        } else {
            key
        };
        let ours = dleq::verify_proof(text.as_bytes(), &proof.c, &wire, &key);
        let mut probe = proof.clone();
        let theirs = match (
            SecretKey::from_slice(&wire.e.0),
            SecretKey::from_slice(&wire.s.0),
            SecretKey::from_slice(&wire.r.0),
        ) {
            (Ok(e), Ok(s), Ok(r)) => {
                probe.dleq = Some(cashu::nuts::ProofDleq::new(e, s, r));
                probe.verify_dleq(PublicKey::from(key)).is_ok()
            }
            _ => false,
        };
        assert!(!ours);
        assert_eq!(ours, theirs);
    }
}

#[test]
fn canonical_urls_match_cashu_s_mint_url() {
    let mut rng = rng(7);
    let schemes = ["https", "http", "HTTPS", "Http", "", "ftp", "https:"];
    let hosts = [
        "mint.example",
        "MINT.Example",
        "127.0.0.1:3338",
        "",
        "İstanbul.example",
        "a@b",
    ];
    let paths = [
        "", "/", "//", "/Path", "/a/b/", "/a//b", "/x://y", "/?q=1", "/#f", "///",
    ];
    let mut checked = 0;
    for _ in 0..2_000 {
        let scheme = schemes.choose(&mut rng).unwrap();
        let separator = ["://", ":/", "//", ""].choose(&mut rng).unwrap();
        let host = hosts.choose(&mut rng).unwrap();
        let path = paths.choose(&mut rng).unwrap();
        let input = format!("{scheme}{separator}{host}{path}");
        let theirs = MintUrl::from_str(&input).ok().map(|url| url.to_string());
        assert_eq!(url::canonicalize(&input), theirs, "{input:?}");
        assert_eq!(
            url::is_canonical(&input),
            theirs.as_deref() == Some(input.as_str()),
            "{input:?}"
        );
        if url::check_mint_url(&input).is_ok() {
            // Whatever Prime pins is a URL cashu renders unchanged.
            assert_eq!(theirs.as_deref(), Some(input.as_str()));
            checked += 1;
        }
    }
    assert!(checked >= 10, "{checked}");
}

/// A mint of three keysets with random fees: active, inactive with a version 00 id, active.
fn random_mint(rng: &mut ChaCha20Rng, round: u32) -> TestMint {
    let amounts = standard_amounts();
    let fee = |rng: &mut ChaCha20Rng| *[0u64, 100, 1_000].choose(rng).unwrap();
    TestMint {
        url: format!("https://mint-{round}.example"),
        keysets: vec![
            TestKeyset::generate(
                format!("{round}/1").as_bytes(),
                &amounts,
                fee(rng),
                None,
                KeySetVersion::Version01,
                true,
            ),
            TestKeyset::generate(
                format!("{round}/2").as_bytes(),
                &amounts,
                fee(rng),
                None,
                KeySetVersion::Version00,
                false,
            ),
            TestKeyset::generate(
                format!("{round}/3").as_bytes(),
                &amounts,
                fee(rng),
                Some(1_900_000_000),
                KeySetVersion::Version01,
                true,
            ),
        ],
    }
}

fn fee_sat(mint: &TestMint, keysets: &[usize]) -> u64 {
    keysets
        .iter()
        .map(|index| mint.keysets[*index].keyset.input_fee_ppk)
        .sum::<u64>()
        .div_ceil(1000)
}

/// Powers of two summing to `amount`.
fn split(amount: u64) -> Vec<u64> {
    (0..64)
        .map(|bit| 1u64 << bit)
        .filter(|part| amount & part != 0)
        .collect()
}

/// Builds the spend with change counters from `offset`; an honest phone whose outputs read two ways
/// moves to the next counters, and so does this.
#[allow(clippy::too_many_arguments)]
fn signed_spend(
    rng: &mut ChaCha20Rng,
    mint: &TestMint,
    keys: &MintKeys,
    epoch: u32,
    build: &dyn Fn(&mut Spend, &mut ChaCha20Rng, u64),
    kind: CashuSpendKind,
) -> (Spend, fps_cashu_core::CashuAnalysis, [u8; 64]) {
    let pinned = validate_mint(&mint.descriptor(), None).unwrap();
    for offset in 0..32u64 {
        let mut spend = Spend::default();
        build(&mut spend, rng, 1_000 + offset * 100);
        let request = spend.request(keys.mint_id(), epoch, kind.clone());
        match analyze(&request, &pinned, keys) {
            Ok(analysis) => {
                let signature = sign_spend(keys, &analysis, random_bytes(rng)).unwrap();
                return (spend, analysis, signature);
            }
            Err(error) if error.message.contains("more than one way") => continue,
            Err(error) => panic!("{error}"),
        }
    }
    panic!("no counters gave a single reading")
}

#[test]
fn random_swaps_signed_by_prime_pass_cashu_s_verification() {
    let mut rng = rng(8);
    let mut kinds = BTreeMap::new();
    for round in 0..60u32 {
        let mint = random_mint(&mut rng, round);
        let keys = MintKeys::derive(&random_bytes(&mut rng), &mint.url).unwrap();
        let epoch = rng.gen_range(0..5);
        // Inputs from any keyset, until they pay more than their fee.
        let (amounts, keysets) = loop {
            let count = rng.gen_range(1..=8);
            let amounts: Vec<u64> = (0..count).map(|_| 1u64 << rng.gen_range(0..21)).collect();
            let keysets: Vec<usize> = (0..count).map(|_| rng.gen_range(0..3)).collect();
            if amounts.iter().sum::<u64>() > fee_sat(&mint, &keysets) {
                break (amounts, keysets);
            }
        };
        let total = amounts.iter().sum::<u64>() - fee_sat(&mint, &keysets);
        // A send (change at random epochs ahead, or leaving), a combine (change at the input epoch
        // only) or a move to new keys (change, the first output at a later epoch), each output in
        // an active keyset, in random order.
        let mode = round % 3;
        let mut plan: Vec<(u64, Option<u32>, usize)> = split(total)
            .into_iter()
            .enumerate()
            .map(|(index, amount)| {
                let change = match mode {
                    0 => rng.gen_bool(0.6).then(|| epoch + rng.gen_range(0..=3)),
                    1 => Some(epoch),
                    _ if index == 0 => Some(epoch + rng.gen_range(1..=3)),
                    _ => Some(epoch + rng.gen_range(0..=3)),
                };
                (amount, change, [0usize, 2][rng.gen_range(0..2)])
            })
            .collect();
        plan.shuffle(&mut rng);
        let phone = keys.phone().clone();
        let build = |spend: &mut Spend, rng: &mut ChaCha20Rng, offset: u64| {
            for (index, (amount, keyset)) in amounts.iter().zip(&keysets).enumerate() {
                spend.input(
                    &phone,
                    &mint.keysets[*keyset],
                    epoch,
                    index as u64,
                    *amount,
                    rng.gen_bool(0.5),
                );
            }
            for (index, (amount, change, keyset)) in plan.iter().enumerate() {
                match change {
                    Some(at) => spend.change(
                        &phone,
                        &mint.keysets[*keyset],
                        *at,
                        offset + index as u64,
                        *amount,
                    ),
                    None => spend.leaving(&mint.keysets[*keyset], *amount, rng),
                }
            }
        };
        let (spend, analysis, signature) =
            signed_spend(&mut rng, &mint, &keys, epoch, &build, CashuSpendKind::Swap);
        *kinds.entry(format!("{:?}", analysis.kind)).or_insert(0) += 1;
        let swap = spend.swap();
        assert_eq!(analysis.message, swap.sig_all_msg_to_sign());
        assert!(swap_accepts(&swap, &signature), "round {round}");

        // The signature covers the outputs: moving one sat between two of them breaks it.
        let mut tampered = swap.clone();
        if let Some(output) = tampered.outputs_mut().first_mut() {
            output.amount = Amount::from(output.amount.to_u64() ^ 1);
            assert!(!swap_accepts(&tampered, &signature));
        }
        let expected = match analysis.kind {
            SpendKind::Send => analysis.totals.leaving_sat + analysis.totals.fee_sat,
            _ => analysis.totals.fee_sat,
        };
        assert_eq!(analysis.outflow_msat, expected * 1000);
    }
    assert_eq!(kinds.len(), 3, "{kinds:?}");
}

#[test]
fn random_melts_signed_by_prime_pass_cashu_s_verification() {
    let mut rng = rng(9);
    for round in 0..40u32 {
        let mint = random_mint(&mut rng, round);
        let keys = MintKeys::derive(&random_bytes(&mut rng), &mint.url).unwrap();
        let epoch = rng.gen_range(0..5);
        let invoice_msat = rng.gen_range(1_000..10_000_000u64);
        let amount = if rng.gen_bool(0.5) {
            invoice_msat / 1000
        } else {
            invoice_msat.div_ceil(1000)
        };
        let fee_reserve = rng.gen_range(0..200);
        // Inputs from the active keysets, covering the quote, the reserve and their own fee.
        let mut inputs: Vec<(u64, usize)> = Vec::new();
        loop {
            let keysets: Vec<usize> = inputs.iter().map(|(_, keyset)| *keyset).collect();
            let have: u64 = inputs.iter().map(|(amount, _)| amount).sum();
            if !inputs.is_empty() && have >= amount + fee_reserve + fee_sat(&mint, &keysets) {
                break;
            }
            inputs.push((
                1u64 << rng.gen_range(5..24),
                [0usize, 2][rng.gen_range(0..2)],
            ));
        }
        let keysets: Vec<usize> = inputs.iter().map(|(_, keyset)| *keyset).collect();
        let have: u64 = inputs.iter().map(|(amount, _)| amount).sum();
        let returnable = have - amount - fee_sat(&mint, &keysets);
        let blanks = u64::from(64 - returnable.leading_zeros()) + rng.gen_range(0..3);
        let quote = quote_like(&mut rng);
        let kind = CashuSpendKind::Melt {
            quote: quote.clone(),
            invoice: invoice(invoice_msat, "random melt", random_bytes(&mut rng)),
            amount,
            fee_reserve,
        };
        let phone = keys.phone().clone();
        let build = |spend: &mut Spend, _: &mut ChaCha20Rng, offset: u64| {
            for (index, (amount, keyset)) in inputs.iter().enumerate() {
                spend.input(
                    &phone,
                    &mint.keysets[*keyset],
                    epoch,
                    index as u64,
                    *amount,
                    true,
                );
            }
            for counter in 0..blanks {
                spend.change(
                    &phone,
                    &mint.keysets[(counter % 2 * 2) as usize],
                    epoch + (counter % 4) as u32,
                    offset + counter,
                    0,
                );
            }
        };
        let (spend, analysis, signature) =
            signed_spend(&mut rng, &mint, &keys, epoch, &build, kind);
        assert_eq!(analysis.kind, SpendKind::Melt);
        assert_eq!(analysis.outflow_msat, have * 1000);
        let melt = spend.melt(&quote);
        assert_eq!(analysis.message, melt.sig_all_msg_to_sign());
        assert!(melt_accepts(&melt, &signature), "round {round}");
        // The signature is bound to the quote.
        let other = MeltRequest::new(
            format!("{quote}x"),
            spend.proofs.clone(),
            Some(spend.blinded.clone()),
        );
        assert!(!melt_accepts(&other, &signature));
    }
}
