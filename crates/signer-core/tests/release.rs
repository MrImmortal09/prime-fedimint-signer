//! Releasing outgoing-branch notes as ecash tokens.

mod common;

use common::*;
use fps_protocol::{Branch, Delivery, FixedBytes, RejectCode, ReleaseNote, ReleaseRequest};
use fps_signer_core::encoding::Reader;
use fps_signer_core::fedimint::base32;
use fps_signer_core::{plan_release, EmptyLedger, SignerError};

fn release_note(fixture: &mut Fixture, denomination: u8) -> ReleaseNote {
    let tweak = fixture.tweak();
    let note = fixture.note(Branch::Outgoing, denomination, tweak);
    ReleaseNote {
        denomination,
        tweak: FixedBytes(tweak),
        signature: FixedBytes(note.signature.to_compressed()),
    }
}

fn request(
    fixture: &Fixture,
    notes: Vec<ReleaseNote>,
    include_invite: bool,
    delivery: Delivery,
) -> ReleaseRequest {
    ReleaseRequest {
        federation: FixedBytes(fixture.id()),
        notes,
        include_invite,
        delivery,
    }
}

fn refused(
    fixture: &Fixture,
    request: &ReleaseRequest,
    ledger: &dyn fps_signer_core::Ledger,
) -> RejectCode {
    let result: Result<_, SignerError> =
        plan_release(&fixture.federation, &fixture.keys, request, ledger, true);
    result.err().expect("must be refused").code
}

#[test]
fn a_release_builds_a_token_holding_the_outgoing_keys() {
    let mut fixture = Fixture::default();
    let notes = vec![
        release_note(&mut fixture, 10),
        release_note(&mut fixture, 15),
    ];
    let plan = plan_release(
        &fixture.federation,
        &fixture.keys,
        &request(&fixture, notes.clone(), false, Delivery::ShowOnDevice),
        &EmptyLedger,
        true,
    )
    .unwrap();
    assert_eq!(plan.amount_msat, (1 << 10) + (1 << 15));
    assert_eq!(plan.token.notes.len(), 2);
    for (spendable, note) in plan.token.notes.iter().zip(&notes) {
        let expected = fixture
            .keys
            .note_keypair(Branch::Outgoing, note.denomination, &note.tweak.0)
            .unwrap();
        assert_eq!(spendable.secret_key, expected.secret_bytes());
    }
    let token = plan.token.to_token_string();
    assert!(token.starts_with("fedimint"));
    // Field count: the mint plus two notes, no invite.
    let bytes = base32::decode_prefixed(&token).unwrap();
    assert_eq!(Reader::new(&bytes).read_bigsize("fields").unwrap(), 3);
    assert_eq!(plan.review.title, "Send ecash");
    assert!(plan.review.warnings.is_empty());
}

#[test]
fn the_invite_uses_the_devices_own_guardian_list() {
    let mut fixture = Fixture::default();
    let notes = vec![release_note(&mut fixture, 12)];
    let plan = plan_release(
        &fixture.federation,
        &fixture.keys,
        &request(&fixture, notes, true, Delivery::ReturnToPhone),
        &EmptyLedger,
        true,
    )
    .unwrap();
    let invite = plan.token.invite.as_ref().unwrap();
    assert_eq!(invite.peer, fixture.federation.guardians[0].peer);
    assert_eq!(invite.url, fixture.federation.guardians[0].url);
    assert!(plan.review.warnings.iter().any(|w| w.contains("phone")));
}

#[test]
fn released_and_repeated_notes_are_refused() {
    let mut fixture = Fixture::default();
    let note = release_note(&mut fixture, 11);
    let mut ledger = TestLedger::default();
    ledger
        .released
        .push((fixture.id(), note.denomination, note.tweak.0));
    let again = request(&fixture, vec![note], false, Delivery::ShowOnDevice);
    assert_eq!(
        refused(&fixture, &again, &ledger),
        RejectCode::AlreadyReleased
    );

    let twice = request(&fixture, vec![note, note], false, Delivery::ShowOnDevice);
    assert_eq!(
        refused(&fixture, &twice, &EmptyLedger),
        RejectCode::InvalidTransaction
    );
}

#[test]
fn notes_with_bad_signatures_are_not_released() {
    let mut fixture = Fixture::default();
    let mut note = release_note(&mut fixture, 13);
    note.signature = release_note(&mut fixture, 14).signature;
    let bad = request(&fixture, vec![note], false, Delivery::ShowOnDevice);
    assert_eq!(
        refused(&fixture, &bad, &EmptyLedger),
        RejectCode::InvalidNote
    );
    // Checking can be turned off by policy.
    assert!(plan_release(
        &fixture.federation,
        &fixture.keys,
        &bad,
        &EmptyLedger,
        false
    )
    .is_ok());

    let mut not_a_point = release_note(&mut fixture, 13);
    not_a_point.signature = FixedBytes([0x01; 48]);
    let invalid = request(&fixture, vec![not_a_point], false, Delivery::ShowOnDevice);
    assert_eq!(
        refused(&fixture, &invalid, &EmptyLedger),
        RejectCode::InvalidNote
    );
}

#[test]
fn malformed_release_requests_are_refused() {
    let mut fixture = Fixture::default();
    assert_eq!(
        refused(
            &fixture,
            &request(&fixture, vec![], false, Delivery::ShowOnDevice),
            &EmptyLedger
        ),
        RejectCode::InvalidTransaction
    );

    let mut unknown = release_note(&mut fixture, 9);
    unknown.denomination = 60;
    assert_eq!(
        refused(
            &fixture,
            &request(&fixture, vec![unknown], false, Delivery::ShowOnDevice),
            &EmptyLedger
        ),
        RejectCode::InvalidNote
    );

    let note = release_note(&mut fixture, 9);
    let many = vec![note; 65];
    assert_eq!(
        refused(
            &fixture,
            &request(&fixture, many, false, Delivery::ShowOnDevice),
            &EmptyLedger
        ),
        RejectCode::TooLarge
    );

    let mut elsewhere = request(&fixture, vec![note], false, Delivery::ShowOnDevice);
    elsewhere.federation = FixedBytes([1; 32]);
    assert_eq!(
        refused(&fixture, &elsewhere, &EmptyLedger),
        RejectCode::UnknownFederation
    );
}
