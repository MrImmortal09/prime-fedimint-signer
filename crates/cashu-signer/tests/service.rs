//! The Cashu service end to end through `Device`: mints added and updated, swaps and melts signed
//! with approvals, replays, cancels, the ring of stored answers, the spending policy, the limits on
//! what is pinned, restarts, failed writes, a Cashu state read only on first use, and one that
//! cannot be read.

mod common;

use bitcoin::secp256k1::{schnorr, Message, Secp256k1};
use common::mint::{self, keyset_of, Fixture, TestKeyset, A, B, C};
use common::*;
use fps_cashu_core::keys::MintKeys;
use fps_cashu_core::validate_mint;
use fps_cashu_signer::{
    pinned_descriptor, CashuOutflow, CashuResult, CashuState, MAX_CASHU_RESULTS, MAX_MINTS,
    MAX_PINNED_KEYS,
};
use fps_policy::PolicyConfig;
use fps_protocol::*;
use fps_signer::{ApprovalKind, Outcome, ServiceError, Signer};

/// Checks a Cashu signature as the mint does: BIP-340 by the lock key of the spend's epoch, over
/// the hash of the message Prime built for it.
fn assert_signs(fixture: &Fixture, spend: &CashuSignRequest, signed: &CashuSigned) {
    let analysis = fixture.analyze(spend).unwrap();
    assert_eq!(signed.message_hash.0, analysis.message_hash);
    let lock_key = fixture.keys.lock_pubkey(spend.epoch).unwrap();
    Secp256k1::verification_only()
        .verify_schnorr(
            &schnorr::Signature::from_slice(&signed.signature.0).unwrap(),
            &Message::from_digest(signed.message_hash.0),
            &lock_key.x_only_public_key().0,
        )
        .expect("the signature verifies against the epoch's lock key");
}

/// The descriptor of the test mint with only some of its keysets.
fn with_keysets(fixture: &Fixture, keysets: &[usize]) -> CashuMintDescriptor {
    CashuMintDescriptor {
        keysets: keysets
            .iter()
            .map(|index| fixture.keysets[*index].wire())
            .collect(),
        ..fixture.descriptor.clone()
    }
}

fn with_policy(device: &mut TestDevice, policy: PolicyConfig) {
    // The app changes the policy through `Deref`, on the signer.
    device.set_policy(policy).unwrap();
}

#[test]
fn info_names_the_scheme_and_needs_nothing() {
    let (mut device, fedimint, cashu) = cashu_device();
    let info = cashu_info(&mut device, id(1));
    assert_eq!(info.scheme, "fps-cashu-v1");
    assert!(info.mints.is_empty());
    // Not stored, and nothing written.
    assert_eq!(
        rejection(send(
            &mut device,
            id(2),
            Request::GetResult { request: id(1) }
        ))
        .code,
        RejectCode::NotFound
    );
    assert_eq!((fedimint.write_count(), cashu.write_count()), (0, 0));
}

#[test]
fn adding_a_mint_asks_every_time_and_answers_its_lock_material() {
    let fixture = Fixture::new();
    let (mut device, fedimint, cashu) = cashu_device();

    let waiting = approval(send(&mut device, id(1), add_mint(&fixture.descriptor)));
    assert_eq!(waiting.kind, ApprovalKind::Cashu);
    assert_eq!(waiting.review.title, "Cashu: add mint");
    assert!(!waiting.require_pin);
    assert!(waiting
        .review
        .lines
        .iter()
        .any(|line| line.label == "Mint" && line.value == mint::URL));
    assert_eq!(device.pending(), Some(&waiting));

    let material =
        match cashu_answer(decode_response(&device.approve(&id(1), false).unwrap().reply).unwrap())
        {
            CashuResponse::MintAdded(material) => material,
            other => panic!("expected lock material, got {other:?}"),
        };
    let keys = MintKeys::derive(&mint::APP_SEED, mint::URL).unwrap();
    assert_eq!(material, keys.lock_material());
    assert_eq!(material.scheme, "fps-cashu-v1");
    assert!(device.pending().is_none());

    // Pinned durably, as `validate_mint` returned it.
    assert_eq!(device.cashu_mints(), vec![fixture.mint.summary()]);
    assert_eq!(
        cashu_info(&mut device, id(2)).mints,
        vec![fixture.mint.summary()]
    );
    assert_eq!(cashu.write_count(), 1);
    assert_eq!(
        cashu.cashu_state().mints,
        vec![pinned_descriptor(&fixture.mint)]
    );
    assert!(
        cashu.cashu_state().results.is_empty(),
        "lock material is never stored"
    );

    // The same request again asks again: the lock material is never replayed.
    let again = approval(send(&mut device, id(1), add_mint(&fixture.descriptor)));
    assert_eq!(again.review.title, "Cashu: update mint keys");
    assert!(again
        .review
        .lines
        .iter()
        .any(|line| line.label == "New keysets" && line.value == "0"));
    // A rejection is stored and replayed, as the signer's are.
    let rejected = device.reject(&id(1)).unwrap();
    assert_eq!(
        decode_response(&rejected).unwrap().body,
        Response::Rejected(Rejection {
            code: RejectCode::UserRejected,
            message: "rejected on the device".to_owned()
        })
    );
    assert_eq!(
        reply_bytes(send(&mut device, id(1), add_mint(&fixture.descriptor))),
        rejected
    );
    assert_eq!(
        fedimint.write_count(),
        0,
        "Cashu never writes the signer's state"
    );
}

#[test]
fn an_update_only_adds_keysets_and_keeps_what_the_mint_stops_listing() {
    let fixture = Fixture::new();
    let (mut device, _, cashu) = cashu_device();
    // B's id (version 00) does not commit to its `active` flag.
    let mut first = with_keysets(&fixture, &[A, B]);
    first.keysets[1].active = true;
    added(&mut device, id(1), &first);
    assert_eq!(device.cashu_mints()[0].keysets, 2);
    assert!(cashu.cashu_state().mints[0].keysets[1].active);

    // The mint now lists A and C: C is new, and B, no longer listed, stays pinned as inactive.
    let update = with_keysets(&fixture, &[A, C]);
    let waiting = approval(send(&mut device, id(2), add_mint(&update)));
    assert_eq!(waiting.review.title, "Cashu: update mint keys");
    assert!(waiting
        .review
        .lines
        .iter()
        .any(|line| line.label == "New keysets" && line.value == "1"));
    device.approve(&id(2), false).unwrap();
    assert_eq!(device.cashu_mints()[0].keysets, 3);
    let stored = &cashu.cashu_state().mints[0];
    let ids: Vec<(Vec<u8>, bool)> = stored
        .keysets
        .iter()
        .map(|keyset| (keyset.id.0.clone(), keyset.active))
        .collect();
    assert_eq!(
        ids,
        vec![
            (fixture.id(A), true),
            (fixture.id(C), true),
            (fixture.id(B), false)
        ]
    );

    // A proof of B is still spendable, into C.
    let combine = fixture.swap(
        0,
        vec![fixture.proof(B, 0, 40, 8)],
        vec![fixture.change(C, 0, 41, 8)],
    );
    let waiting = approval(send(&mut device, id(3), sign(combine.clone())));
    assert_eq!(waiting.review.title, "Cashu: combine ecash");
    let signed = signed_after(&mut device, id(3));
    assert_signs(&fixture, &combine, &signed);
}

#[test]
fn an_update_cannot_change_what_is_pinned() {
    let fixture = Fixture::new();
    let (mut device, _, cashu) = cashu_device();
    added(&mut device, id(1), &fixture.descriptor);
    let before = cashu.bytes();

    // B's id (version 00) commits to its keys only, so a changed fee keeps the id.
    let mut refee = fixture.descriptor.clone();
    refee.keysets[B].input_fee_ppk = 50;
    // A new keyset listing A's keys under other amounts.
    let mut rotated = fixture.keysets[A].public_keys();
    let amounts: Vec<u64> = rotated.iter().map(|(amount, _)| *amount).collect();
    for (index, (amount, _)) in rotated.iter_mut().enumerate() {
        *amount = amounts[(index + 1) % amounts.len()];
    }
    rotated.sort_by_key(|(amount, _)| *amount);
    let mut relabel = fixture.descriptor.clone();
    relabel.keysets.push(keyset_of(&rotated, 0));
    let mut usd = fixture.descriptor.clone();
    usd.unit = "usd".to_owned();
    let mut spelled = fixture.descriptor.clone();
    spelled.url = "https://Mint.test.example/".to_owned();

    for (n, (descriptor, code, words)) in [
        (refee, RejectCode::InvalidDescriptor, "keys or fee differ"),
        (
            relabel,
            RejectCode::InvalidDescriptor,
            "a key stands for one amount",
        ),
        (usd, RejectCode::Unsupported, "unit"),
        (spelled, RejectCode::InvalidDescriptor, "canonical"),
    ]
    .into_iter()
    .enumerate()
    {
        let refusal = rejection(send(&mut device, id(10 + n as u8), add_mint(&descriptor)));
        assert_eq!(refusal.code, code, "{}", refusal.message);
        assert!(refusal.message.contains(words), "{}", refusal.message);
    }
    assert_eq!(cashu.bytes(), before, "nothing changed");
    assert_eq!(device.cashu_mints(), vec![fixture.mint.summary()]);
}

#[test]
fn a_send_is_signed_after_approval_and_replayed_after() {
    let fixture = Fixture::new();
    let (mut device, fedimint, cashu) = cashu_device();
    added(&mut device, id(1), &fixture.descriptor);

    let spend = mint::send(&fixture);
    let waiting = approval(send(&mut device, id(2), sign(spend.clone())));
    assert_eq!(waiting.kind, ApprovalKind::Cashu);
    assert_eq!(waiting.review.title, "Cashu: send ecash");
    assert_eq!(waiting.review.amount_msat, 8_000);
    assert!(!waiting.require_pin);
    assert_eq!(
        cashu.write_count(),
        1,
        "nothing is written before the approval"
    );

    let reply = device.approve(&id(2), false).unwrap().reply;
    let CashuResponse::Signed(signed) = cashu_answer(decode_response(&reply).unwrap()) else {
        panic!("expected a signature");
    };
    assert_signs(&fixture, &spend, &signed);

    // Stored with the outflow before it left: 8 sats leave and 1 is the mint's fee.
    let state = cashu.cashu_state();
    assert_eq!(state.stored_result(&id(2)), Some(reply.as_slice()));
    assert_eq!(
        state.outflow,
        CashuOutflow {
            day: Some(NOW / DAY),
            outflow_msat: 9_000
        }
    );
    // The same request id gets the stored answer, without a review or a second signature.
    assert_eq!(
        reply_bytes(send(&mut device, id(2), sign(spend.clone()))),
        reply
    );
    assert_eq!(
        reply_bytes(send(
            &mut device,
            id(3),
            Request::GetResult { request: id(2) }
        )),
        reply
    );
    assert_eq!(fedimint.write_count(), 0);
}

#[test]
fn a_melt_is_signed_after_approval() {
    let fixture = Fixture::new();
    let (mut device, _, cashu) = cashu_device();
    added(&mut device, id(1), &fixture.descriptor);

    let spend = mint::melt(&fixture);
    let waiting = approval(send(&mut device, id(2), sign(spend.clone())));
    assert_eq!(waiting.review.title, "Cashu: pay Lightning invoice");
    assert_eq!(waiting.review.amount_msat, 101_000);
    assert!(!waiting.review.warnings.is_empty());
    let signed = signed_after(&mut device, id(2));
    assert_signs(&fixture, &spend, &signed);
    // Everything a melt spends counts as leaving.
    assert_eq!(cashu.cashu_state().outflow.outflow_msat, 104_000);
}

fn signed_after(device: &mut TestDevice, request: RequestId) -> CashuSigned {
    match cashu_answer(decode_response(&device.approve(&request, true).unwrap().reply).unwrap()) {
        CashuResponse::Signed(signed) => signed,
        other => panic!("expected a signature, got {other:?}"),
    }
}

#[test]
fn a_rejected_spend_is_answered_remembered_and_not_counted() {
    let fixture = Fixture::new();
    let (mut device, _, cashu) = cashu_device();
    added(&mut device, id(1), &fixture.descriptor);
    let spend = sign(mint::send(&fixture));

    approval(send(&mut device, id(2), spend.clone()));
    let rejected = device.reject(&id(2)).unwrap();
    assert_eq!(
        rejection(Outcome::Reply(rejected.clone())).code,
        RejectCode::UserRejected
    );
    assert!(device.pending().is_none());
    assert_eq!(reply_bytes(send(&mut device, id(2), spend)), rejected);
    assert_eq!(cashu.cashu_state().outflow, CashuOutflow::default());
    assert_eq!(
        device.approve(&id(2), true).unwrap_err(),
        ServiceError::NoSuchRequest
    );
}

#[test]
fn one_cashu_review_waits_at_a_time_while_info_and_stored_answers_come_back() {
    let fixture = Fixture::new();
    let (mut device, _, _) = cashu_device();
    added(&mut device, id(1), &fixture.descriptor);
    let done = {
        approval(send(&mut device, id(8), sign(mint::send(&fixture))));
        device.approve(&id(8), true).unwrap().reply
    };
    approval(send(&mut device, id(2), sign(mint::send(&fixture))));

    let busy = format!(
        "request {} is waiting for approval on the device",
        id(2).to_hex()
    );
    for (n, body) in [
        sign(mint::melt(&fixture)),
        add_mint(&fixture.descriptor),
        // The waiting request itself, sent again.
        sign(mint::send(&fixture)),
    ]
    .into_iter()
    .enumerate()
    {
        let request = if n == 2 { id(2) } else { id(3 + n as u8) };
        assert_eq!(
            rejection(send(&mut device, request, body)),
            Rejection {
                code: RejectCode::Busy,
                message: busy.clone()
            }
        );
    }
    // A request answered before gets its answer, not `Busy`.
    assert_eq!(
        reply_bytes(send(&mut device, id(8), sign(mint::send(&fixture)))),
        done
    );
    assert_eq!(cashu_info(&mut device, id(9)).mints.len(), 1);
    assert_eq!(device.pending().map(|waiting| waiting.request), Some(id(2)));
}

#[test]
fn each_signature_draws_fresh_randomness() {
    let fixture = Fixture::new();
    let (mut device, _, _) = cashu_device();
    added(&mut device, id(1), &fixture.descriptor);
    let spend = mint::send(&fixture);
    let first = signed(&mut device, id(2), spend.clone());
    let second = signed(&mut device, id(3), spend.clone());
    // One message, two BIP-340 signatures: the auxiliary randomness comes from the platform.
    assert_eq!(first.message_hash, second.message_hash);
    assert_ne!(first.signature, second.signature);
    assert_signs(&fixture, &spend, &first);
    assert_signs(&fixture, &spend, &second);
}

#[test]
fn the_ring_keeps_the_last_32_cashu_answers() {
    let fixture = Fixture::new();
    let (mut device, fedimint, cashu) = cashu_device();
    added(&mut device, id(100), &fixture.descriptor);
    let total = MAX_CASHU_RESULTS as u8 + 1;
    let mut answers = Vec::new();
    for n in 1..=total {
        approval(send(&mut device, id(n), sign(mint::send(&fixture))));
        answers.push(device.reject(&id(n)).unwrap());
    }

    // The oldest answer is dropped, and the rest are kept in order.
    let kept: Vec<RequestId> = cashu
        .cashu_state()
        .results
        .iter()
        .map(|stored| stored.request)
        .collect();
    assert_eq!(kept, (2..=total).map(id).collect::<Vec<_>>());
    assert_eq!(
        reply_bytes(send(&mut device, id(2), sign(mint::send(&fixture)))),
        answers[1]
    );
    assert_eq!(
        reply_bytes(send(&mut device, id(total), sign(mint::send(&fixture)))),
        answers[usize::from(total) - 1]
    );
    // The request whose answer was dropped asks again.
    approval(send(&mut device, id(1), sign(mint::send(&fixture))));
    drop(device);

    // A full ring reads back.
    let mut reopened = open_device(mint::APP_SEED, &fedimint, &cashu, NOW);
    assert_eq!(reopened.cashu_off(), None);
    assert_eq!(
        reply_bytes(send(&mut reopened, id(total), sign(mint::send(&fixture)))),
        answers[usize::from(total) - 1]
    );
}

#[test]
fn the_cashu_state_is_read_on_first_use_not_when_the_device_opens() {
    let fixture = Fixture::new();
    let golden = Golden::load();
    let mut valid = CashuState::new();
    valid.pin(&fixture.mint);
    let bytes = valid.encode().unwrap();

    let cashu = Watched::holding(&bytes);
    let mut device = open_device(golden.app_seed, &Watched::default(), &cashu, NOW);
    assert_eq!(
        cashu.read_count(),
        0,
        "opening reads the signer's state only"
    );
    // Fedimint traffic does not read it, approvals included.
    approved(
        &mut device,
        id(1),
        Request::ImportFederation(golden.descriptor.clone()),
    );
    reply(send(&mut device, id(2), Request::GetInfo));
    reply(send(&mut device, id(3), Request::GetStatus));
    reply(send(
        &mut device,
        id(4),
        Request::GetResult { request: id(1) },
    ));
    // No Cashu request can wait before the state is read, so a `Cancel` need not read it.
    rejection(send(
        &mut device,
        id(5),
        Request::Cancel { request: id(99) },
    ));
    assert_eq!(cashu.read_count(), 0);
    // The first Cashu request reads it, once.
    assert_eq!(
        cashu_info(&mut device, id(6)).mints,
        vec![fixture.mint.summary()]
    );
    cashu_info(&mut device, id(7));
    assert_eq!(cashu.read_count(), 1);

    // So does a `GetResult` the signer has nothing for, which may name a Cashu request.
    let cashu = Watched::holding(&bytes);
    let mut device = open_device(golden.app_seed, &Watched::default(), &cashu, NOW);
    rejection(send(
        &mut device,
        id(8),
        Request::GetResult { request: id(99) },
    ));
    assert_eq!(cashu.read_count(), 1);
}

#[test]
fn spends_at_an_unknown_mint_are_refused() {
    let fixture = Fixture::new();
    let (mut device, _, cashu) = cashu_device();
    let refusal = rejection(send(&mut device, id(1), sign(mint::send(&fixture))));
    assert_eq!(refusal.code, RejectCode::UnknownFederation);
    assert!(refusal.message.contains("not added"), "{}", refusal.message);
    assert_eq!(cashu.write_count(), 0);
}

#[test]
fn the_pin_threshold_applies_as_for_fedimint() {
    let fixture = Fixture::new();
    let (mut device, _, _) = cashu_device();
    added(&mut device, id(1), &fixture.descriptor);

    // 9,000 msat leave; a threshold of 9,000 is not exceeded.
    with_policy(
        &mut device,
        PolicyConfig {
            pin_above_msat: Some(9_000),
            ..PolicyConfig::default()
        },
    );
    assert!(!approval(send(&mut device, id(2), sign(mint::send(&fixture)))).require_pin);
    device.reject(&id(2)).unwrap();

    with_policy(
        &mut device,
        PolicyConfig {
            pin_above_msat: Some(8_999),
            ..PolicyConfig::default()
        },
    );
    let spend = mint::send(&fixture);
    assert!(approval(send(&mut device, id(3), sign(spend.clone()))).require_pin);
    assert_eq!(
        device.approve(&id(3), false).unwrap_err(),
        ServiceError::PinRequired
    );
    assert!(
        device.pending().is_some(),
        "a missing PIN leaves the request waiting"
    );
    let signed = signed_after(&mut device, id(3));
    assert_signs(&fixture, &spend, &signed);
}

#[test]
fn per_request_and_daily_limits_apply_and_count_only_approvals() {
    let fixture = Fixture::new();
    let (mut device, fedimint, cashu) = cashu_device();
    added(&mut device, id(1), &fixture.descriptor);
    let spend = || sign(mint::send(&fixture));

    with_policy(
        &mut device,
        PolicyConfig {
            max_outflow_per_request_msat: Some(8_999),
            ..PolicyConfig::default()
        },
    );
    let refusal = rejection(send(&mut device, id(2), spend()));
    assert_eq!(refusal.code, RejectCode::PolicyLimit);
    assert!(
        refusal.message.contains("per-request limit"),
        "{}",
        refusal.message
    );

    with_policy(
        &mut device,
        PolicyConfig {
            max_outflow_per_day_msat: Some(20_000),
            ..PolicyConfig::default()
        },
    );
    approved(&mut device, id(3), spend());
    // A rejected spend is not counted.
    approval(send(&mut device, id(4), spend()));
    device.reject(&id(4)).unwrap();
    approved(&mut device, id(5), spend());
    assert_eq!(cashu.cashu_state().outflow.outflow_msat, 18_000);
    let refusal = rejection(send(&mut device, id(6), spend()));
    assert_eq!(refusal.code, RejectCode::PolicyLimit);
    assert!(
        refusal.message.contains("27000 msat") && refusal.message.contains("daily limit"),
        "{}",
        refusal.message
    );
    drop(device);

    // The next day starts a new count.
    let mut tomorrow = open_device(mint::APP_SEED, &fedimint, &cashu, NOW + DAY);
    approved(&mut tomorrow, id(6), spend());
    assert_eq!(
        cashu.cashu_state().outflow,
        CashuOutflow {
            day: Some(NOW / DAY + 1),
            outflow_msat: 9_000
        }
    );
}

/// A send from one proof of 65,536 sats of keyset C, which charges no fee: `leaving` sats leave
/// and the rest comes back as change, both in powers of two. Counters from `counter` up.
fn large_send(fixture: &Fixture, counter: u64, leaving: u64) -> CashuSignRequest {
    let powers = |total: u64| {
        (0..64)
            .rev()
            .map(|bit| 1u64 << bit)
            .filter(move |power| total & power != 0)
    };
    let mut outputs: Vec<CashuOutput> = powers(leaving)
        .enumerate()
        .map(|(n, amount)| fixture.leaving(C, amount, (counter as u8).wrapping_add(n as u8)))
        .collect();
    outputs.extend(
        powers(65_536 - leaving)
            .enumerate()
            .map(|(n, amount)| fixture.change(C, 0, counter + 1 + n as u64, amount)),
    );
    fixture.swap(0, vec![fixture.proof(C, 0, counter, 65_536)], outputs)
}

#[test]
fn the_daily_limit_counts_fedimint_and_cashu_together() {
    let fixture = Fixture::new();
    // The frozen Fedimint state: 552,067,136 msat out today, a daily limit of 600,000,000 and a PIN
    // above 10,000,000.
    let (fedimint, cashu) = (Watched::holding(FROZEN_STATE), Watched::default());
    let mut device = open_device(mint::APP_SEED, &fedimint, &cashu, NOW);
    assert_eq!(device.policy(), &compat_policy());
    assert_eq!(
        device.state().policy_state.outflow_msat,
        FROZEN_OUTFLOW_MSAT
    );
    added(&mut device, id(1), &fixture.descriptor);

    // 32,768 sats fit beside Fedimint's count; 49,152 do not.
    let refusal = rejection(send(
        &mut device,
        id(2),
        sign(large_send(&fixture, 50, 49_152)),
    ));
    assert_eq!(refusal.code, RejectCode::PolicyLimit);
    assert!(
        refusal.message.contains("601219136 msat"),
        "{}",
        refusal.message
    );
    let waiting = approval(send(
        &mut device,
        id(3),
        sign(large_send(&fixture, 60, 32_768)),
    ));
    assert!(waiting.require_pin);
    signed_after(&mut device, id(3));
    // Now Cashu's own count adds to Fedimint's.
    let refusal = rejection(send(
        &mut device,
        id(4),
        sign(large_send(&fixture, 70, 16_384)),
    ));
    assert_eq!(refusal.code, RejectCode::PolicyLimit);
    assert!(
        refusal.message.contains("601219136 msat"),
        "{}",
        refusal.message
    );
    // Only read: the signer's state is as it was.
    assert_eq!(fedimint.write_count(), 0);
    assert_eq!(fedimint.bytes().unwrap(), FROZEN_STATE);
    drop(device);

    // On its own the first spend fits.
    let alone = Watched::default();
    let mut fresh = open_device(mint::APP_SEED, &alone, &Watched::default(), NOW);
    with_policy(&mut fresh, compat_policy());
    added(&mut fresh, id(1), &fixture.descriptor);
    assert!(
        approval(send(
            &mut fresh,
            id(2),
            sign(large_send(&fixture, 50, 49_152))
        ))
        .require_pin
    );

    // The next day neither count carries over.
    let mut tomorrow = open_device(mint::APP_SEED, &fedimint, &cashu, NOW + DAY);
    approval(send(
        &mut tomorrow,
        id(5),
        sign(large_send(&fixture, 50, 49_152)),
    ));
}

#[test]
fn a_melts_fee_reserve_is_held_to_the_routing_fee_limit() {
    let fixture = Fixture::new();
    let (mut device, _, _) = cashu_device();
    added(&mut device, id(1), &fixture.descriptor);
    // 320 sats in, 1 of fee, a quote of 101 sats with 200 of fee reserve, 8 blanks for the rest.
    let melt = fixture.melt(
        0,
        vec![fixture.proof(A, 0, 60, 256), fixture.proof(A, 0, 61, 64)],
        fixture.blanks(A, 0, 70, 8),
        Some(100_500),
        101,
        200,
    );
    fixture.analyze(&melt).expect("the melt itself is sound");

    // The default limit for 100,500 msat: 100 sats plus 1.5%, 101,507 msat.
    let refusal = rejection(send(&mut device, id(2), sign(melt.clone())));
    assert_eq!(refusal.code, RejectCode::PolicyLimit);
    assert_eq!(
        refusal.message,
        "the mint's fee reserve of 200000 msat is above the routing fee limit of 101507 msat"
    );
    // The usual reserve of 2 sats is well under it.
    approval(send(&mut device, id(3), sign(mint::melt(&fixture))));
    device.reject(&id(3)).unwrap();

    // A user who allows 200 sats plus 2% allows this reserve.
    with_policy(&mut device, compat_policy());
    let waiting = approval(send(&mut device, id(4), sign(melt)));
    assert_eq!(waiting.review.title, "Cashu: pay Lightning invoice");
}

#[test]
fn get_result_and_cancel_reach_cashu_requests() {
    let fixture = Fixture::new();
    let (mut device, fedimint, cashu) = cashu_device();
    added(&mut device, id(1), &fixture.descriptor);
    let spend = sign(mint::send(&fixture));
    approval(send(&mut device, id(2), spend.clone()));

    let waiting = reply(send(
        &mut device,
        id(3),
        Request::GetResult { request: id(2) },
    ));
    assert_eq!((waiting.id, waiting.body), (id(3), Response::Pending));
    let cancelled = reply(send(&mut device, id(4), Request::Cancel { request: id(2) }));
    assert_eq!((cancelled.id, cancelled.body), (id(4), Response::Cancelled));
    assert!(device.pending().is_none());
    let stored = reply(send(
        &mut device,
        id(5),
        Request::GetResult { request: id(2) },
    ));
    assert_eq!((stored.id, stored.body), (id(2), Response::Cancelled));
    assert_eq!(
        reply(send(&mut device, id(2), spend)).body,
        Response::Cancelled
    );
    assert_eq!(
        device.approve(&id(2), true).unwrap_err(),
        ServiceError::NoSuchRequest
    );

    // Neither backend knows these.
    assert_eq!(
        rejection(send(
            &mut device,
            id(6),
            Request::GetResult { request: id(99) }
        ))
        .code,
        RejectCode::NotFound
    );
    assert_eq!(
        rejection(send(
            &mut device,
            id(7),
            Request::Cancel { request: id(99) }
        ))
        .code,
        RejectCode::NotFound
    );
    assert_eq!(cashu.cashu_state().results.len(), 1);
    assert_eq!(fedimint.write_count(), 0);
}

#[test]
fn the_cashu_state_survives_a_restart() {
    let fixture = Fixture::new();
    let (mut device, fedimint, cashu) = cashu_device();
    added(&mut device, id(1), &fixture.descriptor);
    let send_reply = {
        approval(send(&mut device, id(2), sign(mint::send(&fixture))));
        device.approve(&id(2), true).unwrap().reply
    };
    approval(send(&mut device, id(3), sign(mint::melt(&fixture))));
    let rejected = device.reject(&id(3)).unwrap();
    drop(device);

    let mut reopened = open_device(mint::APP_SEED, &fedimint, &cashu, NOW);
    assert!(reopened.cashu_off().is_none());
    assert_eq!(reopened.cashu_mints(), vec![fixture.mint.summary()]);
    assert_eq!(
        reply_bytes(send(&mut reopened, id(2), sign(mint::send(&fixture)))),
        send_reply
    );
    assert_eq!(
        reply_bytes(send(&mut reopened, id(3), sign(mint::melt(&fixture)))),
        rejected
    );
    assert_eq!(cashu.cashu_state().outflow.outflow_msat, 9_000);
    // And signs again from the reloaded mint.
    let spend = mint::send(&fixture);
    approval(send(&mut reopened, id(4), sign(spend.clone())));
    assert_signs(&fixture, &spend, &signed_after(&mut reopened, id(4)));
}

#[test]
fn a_failed_write_keeps_an_approval_or_a_cancel_waiting_and_still_ends_a_rejection() {
    let fixture = Fixture::new();
    let (mut device, _, cashu) = cashu_device();

    approval(send(&mut device, id(1), add_mint(&fixture.descriptor)));
    cashu.fail_writes(true);
    assert!(matches!(
        device.approve(&id(1), false),
        Err(ServiceError::Storage(_))
    ));
    assert!(device.cashu_mints().is_empty(), "nothing is pinned");
    assert_eq!(device.pending().map(|waiting| waiting.request), Some(id(1)));
    cashu.fail_writes(false);
    device.approve(&id(1), false).unwrap();
    assert_eq!(device.cashu_mints().len(), 1);

    // An approval or a cancel whose write fails commits nothing, and the review waits on.
    approval(send(&mut device, id(2), sign(mint::send(&fixture))));
    cashu.fail_writes(true);
    assert!(matches!(
        device.approve(&id(2), true),
        Err(ServiceError::Storage(_))
    ));
    let refusal = rejection(send(&mut device, id(4), Request::Cancel { request: id(2) }));
    assert_eq!(refusal.code, RejectCode::Internal);
    assert_eq!(
        reply(send(
            &mut device,
            id(3),
            Request::GetResult { request: id(2) }
        ))
        .body,
        Response::Pending
    );
    assert_eq!(device.pending().map(|waiting| waiting.request), Some(id(2)));
    cashu.fail_writes(false);
    signed_after(&mut device, id(2));
    assert_eq!(cashu.cashu_state().outflow.outflow_msat, 9_000);

    // A rejection ends the review even when its write fails, as the signer's does, since the app
    // stops waiting for a review the user rejected. Nothing is stored, so the phone finds no answer
    // and asks again.
    approval(send(&mut device, id(5), sign(mint::send(&fixture))));
    cashu.fail_writes(true);
    assert!(matches!(
        device.reject(&id(5)),
        Err(ServiceError::Storage(_))
    ));
    assert!(device.pending().is_none());
    assert_eq!(
        device.approve(&id(5), true).unwrap_err(),
        ServiceError::NoSuchRequest
    );
    cashu.fail_writes(false);
    assert_eq!(
        rejection(send(
            &mut device,
            id(6),
            Request::GetResult { request: id(5) }
        ))
        .code,
        RejectCode::NotFound
    );
    approval(send(&mut device, id(5), sign(mint::send(&fixture))));
    assert_eq!(cashu.cashu_state().outflow.outflow_msat, 9_000);
    assert!(cashu.cashu_state().stored_result(&id(5)).is_none());
}

#[test]
fn at_most_sixteen_mints_and_2048_keys_are_pinned() {
    let fixture = Fixture::new();
    let (mut device, fedimint, cashu) = cashu_device();
    let at = |n: usize| CashuMintDescriptor {
        url: format!("https://mint{n}.test.example"),
        ..fixture.descriptor.clone()
    };
    // Sixteen copies of the test mint, 128 keys each, reach both limits exactly.
    assert_eq!(MAX_MINTS * 128, MAX_PINNED_KEYS);
    for n in 0..MAX_MINTS {
        added(&mut device, id(n as u8 + 1), &at(n));
    }
    let refusal = rejection(send(&mut device, id(100), add_mint(&at(MAX_MINTS))));
    assert_eq!(refusal.code, RejectCode::TooLarge);
    // A mint already pinned can still be updated, counted once.
    let waiting = approval(send(&mut device, id(101), add_mint(&at(0))));
    assert_eq!(waiting.review.title, "Cashu: update mint keys");
    device.reject(&id(101)).unwrap();
    // But not to more keys than the limit.
    let before = cashu.bytes();
    let mut more = at(0);
    more.keysets
        .push(TestKeyset::new(3, 1, true, 0, true, None).wire());
    let refusal = rejection(send(&mut device, id(102), add_mint(&more)));
    assert_eq!(refusal.code, RejectCode::TooLarge);
    assert!(
        refusal.message.contains("2048") && refusal.message.contains("2049"),
        "{}",
        refusal.message
    );
    assert_eq!(cashu.bytes(), before);
    drop(device);

    // The full file reads back.
    let mut reopened = open_device(mint::APP_SEED, &fedimint, &cashu, NOW);
    assert_eq!(reopened.cashu_off(), None);
    assert_eq!(reopened.cashu_mints().len(), MAX_MINTS);
    assert_eq!(cashu.cashu_state().pinned_keys(), MAX_PINNED_KEYS);
}

/// A Cashu state that cannot be read, and what the service says about it.
fn unreadable_states(fixture: &Fixture) -> Vec<(&'static str, Option<Vec<u8>>, &'static str)> {
    let mut valid = CashuState::new();
    valid.pin(&fixture.mint);
    let encoded = valid.encode().unwrap();

    let mut newer = valid.clone();
    newer.version = 2;
    let mut extra: ciborium::Value = ciborium::from_reader(encoded.as_slice()).unwrap();
    if let ciborium::Value::Map(fields) = &mut extra {
        fields.push(("note".into(), 1.into()));
    }
    let mut extra_bytes = Vec::new();
    ciborium::into_writer(&extra, &mut extra_bytes).unwrap();
    let mut trailing = encoded.clone();
    trailing.push(0);
    let mut tampered = valid.clone();
    tampered.mints[0].keysets[0].id.0[5] ^= 1;
    let mut twice = valid.clone();
    twice.mints.push(twice.mints[0].clone());
    let mut many = CashuState::new();
    for n in 0..=MAX_MINTS {
        let mut descriptor = pinned_descriptor(&fixture.mint);
        descriptor.url = format!("https://mint{n}.test.example");
        many.mints.push(descriptor);
    }
    // Sixteen mints with one key more than the limit.
    let mut heavy = many.clone();
    heavy.mints.truncate(MAX_MINTS);
    heavy.mints[0]
        .keysets
        .push(TestKeyset::new(3, 1, true, 0, true, None).wire());
    let mut answers = valid.clone();
    answers.results = (0..=MAX_CASHU_RESULTS as u8)
        .map(|n| CashuResult {
            request: id(n),
            response: vec![n].into(),
        })
        .collect();

    vec![
        (
            "not CBOR",
            Some(b"\xff\x00 not cbor".to_vec()),
            "does not decode",
        ),
        (
            "a later version",
            Some(newer.encode().unwrap()),
            "version 2",
        ),
        ("an unknown field", Some(extra_bytes), "does not decode"),
        ("bytes after it", Some(trailing), "after its end"),
        (
            "a tampered keyset id",
            Some(tampered.encode().unwrap()),
            "not valid",
        ),
        (
            "a mint stored twice",
            Some(twice.encode().unwrap()),
            "twice",
        ),
        (
            "too many mints",
            Some(many.encode().unwrap()),
            "more than 16",
        ),
        (
            "too many keys",
            Some(heavy.encode().unwrap()),
            "pins 2049 keys, more than 2048",
        ),
        (
            "too many answers",
            Some(answers.encode().unwrap()),
            "holds 33 answers, more than 32",
        ),
        ("a failed read", None, "flash read failed"),
    ]
}

#[test]
fn an_unreadable_cashu_state_turns_cashu_off_leaves_it_alone_and_spares_fedimint() {
    let fixture = Fixture::new();
    let golden = Golden::load();
    for (case, bytes, reason) in unreadable_states(&fixture) {
        let cashu = match &bytes {
            Some(bytes) => Watched::holding(bytes),
            None => {
                let failing = Watched::holding(b"anything");
                failing.fail_reads(true);
                failing
            }
        };
        let before = cashu.bytes();
        let fedimint = Watched::default();
        let mut device = open_device(mint::APP_SEED, &fedimint, &cashu, NOW);
        let off = device
            .cashu_off()
            .unwrap_or_else(|| panic!("{case}: Cashu is on"));
        assert!(off.contains(reason), "{case}: {off}");

        // Info answers with no mints; everything else is refused, and says why.
        let info = cashu_info(&mut device, id(1));
        assert_eq!(info.scheme, "fps-cashu-v1", "{case}");
        assert!(info.mints.is_empty(), "{case}");
        for (n, body) in [add_mint(&fixture.descriptor), sign(mint::send(&fixture))]
            .into_iter()
            .enumerate()
        {
            let refusal = rejection(send(&mut device, id(2 + n as u8), body));
            assert_eq!(refusal.code, RejectCode::Internal, "{case}");
            assert!(
                refusal.message.contains("Cashu is off"),
                "{case}: {}",
                refusal.message
            );
            assert!(
                refusal.message.contains(reason),
                "{case}: {}",
                refusal.message
            );
        }

        // Fedimint works.
        let waiting = approval(send(
            &mut device,
            id(4),
            Request::ImportFederation(golden.descriptor.clone()),
        ));
        assert_eq!(waiting.review.title, "Add federation", "{case}");
        let imported = decode_response(&device.approve(&id(4), false).unwrap().reply).unwrap();
        assert!(
            matches!(imported.body, Response::FederationImported(_)),
            "{case}"
        );
        assert_eq!(device.federations().len(), 1, "{case}");
        assert_eq!(fedimint.write_count(), 1, "{case}");

        // The Cashu file is never rewritten.
        assert_eq!(cashu.write_count(), 0, "{case}");
        assert_eq!(cashu.bytes(), before, "{case}");
    }
}

#[test]
fn opening_fails_only_as_the_signer_does() {
    let fixture = Fixture::new();
    let mut valid = CashuState::new();
    valid.pin(&fixture.mint);
    let cashu = Watched::holding(&valid.encode().unwrap());

    let corrupt = Watched::holding(b"\xa1not a state");
    let unreadable = Watched::default();
    unreadable.fail_reads(true);
    for fedimint in [corrupt, unreadable] {
        let expected = match Signer::open([7; 32], fedimint.clone(), TestPlatform::at(NOW)) {
            Ok(_) => panic!("the signer opens"),
            Err(error) => error,
        };
        let got = match fps_cashu_signer::Device::open(
            [7; 32],
            fedimint.clone(),
            TestPlatform::at(NOW),
            cashu.clone(),
            TestPlatform::at(NOW),
        ) {
            Ok(_) => panic!("the device opens"),
            Err(error) => error,
        };
        assert_eq!(got, expected);
    }

    // A Cashu state written by this build is read back as it was.
    let mut device = open_device([7; 32], &Watched::default(), &cashu, NOW);
    assert!(device.cashu_off().is_none());
    assert_eq!(device.cashu_mints(), vec![fixture.mint.summary()]);
    assert_eq!(validate_mint(&valid.mints[0], None).unwrap(), fixture.mint);
}

/// Sizes and host timings for docs/PHASES.md:
/// `cargo test --release -p fps-cashu-signer --test service -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn measure() {
    use std::time::Instant;

    fn timed<T>(runs: u32, mut f: impl FnMut() -> T) -> f64 {
        let start = Instant::now();
        for _ in 0..runs {
            std::hint::black_box(f());
        }
        start.elapsed().as_secs_f64() * 1e3 / f64::from(runs)
    }

    let fixture = Fixture::new();
    let keys: usize = fixture
        .mint
        .keysets
        .iter()
        .map(|keyset| keyset.keys.len())
        .sum();

    // One mint, then the most mints, each with the test mint's keysets, and a full ring of answers.
    let mut one = CashuState::new();
    one.pin(&fixture.mint);
    let mut full = CashuState::new();
    for n in 0..MAX_MINTS {
        let mut mint = fixture.mint.clone();
        mint.url = format!("https://mint{n}.test.example");
        full.pin(&mint);
    }
    let (mut device, _, cashu) = cashu_device();
    added(&mut device, id(1), &fixture.descriptor);
    for n in 0..40u8 {
        signed(&mut device, id(10 + n), mint::send(&fixture));
    }
    let answers = cashu.cashu_state().results;
    // The largest file the limits allow: the most mints, each with the most keysets, of 8 keys
    // each, so 2,048 keys in all, with version 01 ids, final expiries and a full ring of answers.
    let mut largest = CashuState::new();
    for n in 0..MAX_MINTS {
        let descriptor = CashuMintDescriptor {
            url: format!("https://mint{n}.test.example"),
            unit: "sat".to_owned(),
            keysets: (0..16)
                .map(|k| {
                    TestKeyset::new((n * 16 + k) as u8, 8, true, 100, true, Some(1_900_000_000))
                        .wire()
                })
                .collect(),
        };
        largest.pin(&validate_mint(&descriptor, None).unwrap());
    }
    assert_eq!(largest.pinned_keys(), MAX_PINNED_KEYS);
    largest.results = answers.clone();
    full.results = answers.clone();
    one.results = answers;
    let (one, full, largest) = (
        one.encode().unwrap(),
        full.encode().unwrap(),
        largest.encode().unwrap(),
    );
    println!(
        "fps-cashu.cbor: {} bytes with 1 mint of {} keysets and {keys} keys and 32 answers, {} bytes with {MAX_MINTS} such mints, {} bytes at most",
        one.len(),
        fixture.mint.keysets.len(),
        full.len(),
        largest.len()
    );

    // Opening reads only the signer's state; the first Cashu request reads and checks Cashu's.
    let golden = Golden::load();
    for (label, bytes) in [
        ("1 mint", &one),
        ("16 mints", &full),
        ("the largest file", &largest),
    ] {
        let open = timed(20, || {
            open_device(
                golden.app_seed,
                &Watched::holding(FROZEN_STATE),
                &Watched::holding(bytes),
                NOW,
            )
        });
        let first = timed(20, || {
            let mut device = open_device(
                golden.app_seed,
                &Watched::holding(FROZEN_STATE),
                &Watched::holding(bytes),
                NOW,
            );
            cashu_info(&mut device, id(1))
        });
        println!(
            "with {label}: open {open:.2} ms; open, then the first Cashu request, which reads the file: {first:.2} ms"
        );
    }
    let bare = timed(20, || {
        open_signer(golden.app_seed, &Watched::holding(FROZEN_STATE), NOW)
    });
    println!("the bare signer over the frozen state: open {bare:.2} ms");

    let spend = mint::send(&fixture);
    let melt = mint::melt(&fixture);
    let mut n = 0u8;
    for (label, spend) in [
        ("send, 3 inputs and 2 outputs", &spend),
        ("melt, 3 inputs and 2 blanks", &melt),
    ] {
        let ms = timed(50, || {
            n = n.wrapping_add(1);
            let request = FixedBytes([n; 16]);
            approval(send(&mut device, request, sign(spend.clone())));
            device.approve(&request, true).unwrap()
        });
        println!("{label}: handle, approve, sign and store: {ms:.2} ms");
    }

    // The decode `Device` adds in front of `Signer::handle`, on the largest golden request.
    let largest = [
        "reissue",
        "pay_invoice",
        "refund",
        "withdraw",
        "prepare_send",
        "reclaim",
    ]
    .into_iter()
    .map(|case| message(id(1), golden.sign(case)))
    .max_by_key(Vec::len)
    .unwrap();
    let us = timed(2_000, || decode_request(&largest).unwrap()) * 1e3;
    println!(
        "decoding the largest golden request ({} bytes) again: {us:.1} µs",
        largest.len()
    );
}
