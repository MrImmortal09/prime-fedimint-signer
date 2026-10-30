//! The signer service end to end, driven with the fedimint-built scenarios from the signer core's
//! golden vectors.

use std::cell::Cell;
use std::rc::Rc;

use fps_policy::PolicyConfig;
use fps_protocol::*;
use fps_signer::{ApprovalKind, MemoryStorage, Outcome, Platform, ServiceError, Signer, Storage};
use fps_signer_core::FederationKeys;
use serde_json::Value;

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../signer-core/tests/vectors/fedimint-0.13.0-beta.1.json"
    ))
    .unwrap()
}

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap()).unwrap()
}

struct Scenario {
    app_seed: [u8; 32],
    descriptor: FederationDescriptor,
    transactions: Vec<(String, SignRequest)>,
}

fn scenario() -> Scenario {
    let vectors = vectors();
    let scenario = &vectors["scenario"];
    Scenario {
        app_seed: bytes(&scenario["app_seed"]).try_into().unwrap(),
        descriptor: ciborium::from_reader(bytes(&scenario["descriptor"]).as_slice()).unwrap(),
        transactions: scenario["transactions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| {
                (
                    case["name"].as_str().unwrap().to_owned(),
                    ciborium::from_reader(bytes(&case["request"]).as_slice()).unwrap(),
                )
            })
            .collect(),
    }
}

impl Scenario {
    fn request(&self, name: &str) -> SignRequest {
        self.transactions
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, request)| request.clone())
            .unwrap()
    }
}

#[derive(Default)]
struct TestPlatform {
    counter: u8,
    now: Option<u64>,
}

impl Platform for TestPlatform {
    fn random_32(&mut self) -> [u8; 32] {
        self.counter = self.counter.wrapping_add(1);
        [self.counter; 32]
    }

    fn now_unix(&self) -> Option<u64> {
        self.now
    }
}

/// Storage that can be told to fail, shared with the test through `fail`.
#[derive(Clone, Default)]
struct FlakyStorage {
    inner: Rc<std::cell::RefCell<MemoryStorage>>,
    fail: Rc<Cell<bool>>,
}

impl Storage for FlakyStorage {
    fn load(&mut self) -> Result<Option<Vec<u8>>, String> {
        self.inner.borrow_mut().load()
    }

    fn store(&mut self, bytes: &[u8]) -> Result<(), String> {
        if self.fail.get() {
            return Err("flash write failed".to_owned());
        }
        self.inner.borrow_mut().store(bytes)
    }
}

type TestSigner = Signer<FlakyStorage, TestPlatform>;

fn open(storage: FlakyStorage, app_seed: [u8; 32]) -> TestSigner {
    Signer::open(app_seed, storage, TestPlatform::default()).unwrap()
}

fn id(byte: u8) -> RequestId {
    FixedBytes([byte; 16])
}

fn send(signer: &mut TestSigner, request: RequestId, body: Request) -> Outcome {
    signer.handle(&encode(&Envelope::new(request, body)))
}

fn response(bytes: &[u8]) -> ResponseEnvelope {
    decode_response(bytes).unwrap()
}

/// Expects an immediate reply and returns it.
fn reply(outcome: Outcome) -> ResponseEnvelope {
    match outcome {
        Outcome::Reply(bytes) => response(&bytes),
        Outcome::NeedsApproval(approval) => {
            panic!("unexpected approval: {:?}", approval.review.title)
        }
        Outcome::Released { .. } => panic!("unexpected release"),
    }
}

fn rejection(outcome: Outcome) -> RejectCode {
    match reply(outcome).body {
        Response::Rejected(rejection) => rejection.code,
        other => panic!("expected a rejection, got {other:?}"),
    }
}

/// Sends a request that needs approval, approves it, and returns the reply.
fn approved(signer: &mut TestSigner, request: RequestId, body: Request) -> ResponseEnvelope {
    match send(signer, request, body) {
        Outcome::NeedsApproval(approval) => {
            assert_eq!(approval.request, request);
            let approved = signer.approve(&request, true).unwrap();
            response(&approved.reply)
        }
        Outcome::Reply(bytes) => panic!("expected an approval, got {:?}", response(&bytes).body),
        Outcome::Released { .. } => panic!("expected an approval, got a release"),
    }
}

fn imported(scenario: &Scenario) -> (TestSigner, FlakyStorage) {
    let storage = FlakyStorage::default();
    let mut signer = open(storage.clone(), scenario.app_seed);
    let answer = approved(
        &mut signer,
        id(1),
        Request::ImportFederation(scenario.descriptor.clone()),
    );
    assert!(matches!(answer.body, Response::FederationImported(_)));
    (signer, storage)
}

fn lenient_releases(signer: &mut TestSigner) {
    // Most release tests use made-up signatures, so they switch off the mint-signature check;
    // `prepared_notes_release_with_their_real_mint_signatures` runs with it on.
    signer
        .set_policy(PolicyConfig {
            verify_release_signatures: false,
            ..PolicyConfig::default()
        })
        .unwrap();
}

/// The mint signatures the scenario federation issued for a case's outgoing notes.
fn issued(case: &str) -> Vec<ReleaseNote> {
    let vectors = vectors();
    let case = vectors["scenario"]["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == case)
        .unwrap()
        .clone();
    case["issued"]
        .as_array()
        .unwrap()
        .iter()
        .map(|note| ReleaseNote {
            denomination: note["denomination"].as_u64().unwrap() as u8,
            tweak: FixedBytes(bytes(&note["tweak"]).try_into().unwrap()),
            signature: FixedBytes(bytes(&note["signature"]).try_into().unwrap()),
        })
        .collect()
}

fn dummy_signature() -> FixedBytes<48> {
    FixedBytes(bls12_381_generator())
}

fn bls12_381_generator() -> [u8; 48] {
    // The compressed G1 generator: a valid point, not a valid note signature.
    let mut bytes = [0u8; 48];
    hex::decode_to_slice(
        "97f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb",
        &mut bytes,
    )
    .unwrap();
    bytes
}

#[test]
fn a_fresh_device_reports_its_protocol_and_no_federations() {
    let mut signer = open(FlakyStorage::default(), [1; 32]);
    let Response::Info(info) = reply(send(&mut signer, id(1), Request::GetInfo)).body else {
        panic!("expected info");
    };
    assert_eq!(info.protocol, PROTOCOL_VERSION);
    assert_eq!(info.key_scheme, KEY_SCHEME);
    assert!(info.federations.is_empty());
    assert!(!info.busy);
}

#[test]
fn importing_a_federation_needs_approval_once() {
    let scenario = scenario();
    let mut signer = open(FlakyStorage::default(), scenario.app_seed);
    let Outcome::NeedsApproval(approval) = send(
        &mut signer,
        id(1),
        Request::ImportFederation(scenario.descriptor.clone()),
    ) else {
        panic!("import must ask first");
    };
    assert_eq!(approval.kind, ApprovalKind::ImportFederation);
    assert_eq!(approval.review.title, "Add federation");
    assert!(approval
        .review
        .lines
        .iter()
        .any(|l| l.label == "Guardians" && l.value == "4"));
    signer.approve(&id(1), false).unwrap();
    assert_eq!(signer.federations().len(), 1);

    // The same descriptor again is answered without asking.
    let answer = reply(send(
        &mut signer,
        id(2),
        Request::ImportFederation(scenario.descriptor.clone()),
    ));
    assert!(matches!(answer.body, Response::FederationImported(_)));
    // A changed descriptor for the same federation asks again, as an update.
    let mut changed = scenario.descriptor.clone();
    changed.name = Some("Renamed".to_owned());
    let Outcome::NeedsApproval(approval) =
        send(&mut signer, id(3), Request::ImportFederation(changed))
    else {
        panic!("an update must ask");
    };
    assert_eq!(approval.review.title, "Update federation");
}

#[test]
fn view_material_matches_the_key_scheme_and_is_never_replayed() {
    let scenario = scenario();
    let (mut signer, _) = imported(&scenario);
    let federation = scenario.descriptor.federation;
    let answer = approved(
        &mut signer,
        id(5),
        Request::ExportViewMaterial { federation },
    );
    let Response::ViewMaterial(material) = answer.body else {
        panic!("expected view material");
    };
    let keys = FederationKeys::derive(&scenario.app_seed, &federation.0).unwrap();
    assert_eq!(material.spend_key.0, keys.view().spend_key().serialize());
    assert_eq!(material.chain_code.0, keys.view().chain_code());
    assert_eq!(material.view_secret.0, keys.view().view_secret());
    // Sending the same request again asks again rather than replaying the secret.
    assert!(matches!(
        send(
            &mut signer,
            id(5),
            Request::ExportViewMaterial { federation }
        ),
        Outcome::NeedsApproval(_)
    ));
}

#[test]
fn signing_needs_approval_and_a_replay_returns_the_same_signatures() {
    let scenario = scenario();
    let (mut signer, _) = imported(&scenario);
    let request = Request::SignTransaction(scenario.request("pay_invoice"));
    let Outcome::NeedsApproval(approval) = send(&mut signer, id(7), request.clone()) else {
        panic!("signing must ask");
    };
    assert_eq!(approval.kind, ApprovalKind::SignTransaction);
    assert_eq!(approval.review.title, "Pay Lightning invoice");
    let first = signer.approve(&id(7), false).unwrap().reply;
    let Response::Signed(signed) = response(&first).body else {
        panic!("expected signatures");
    };
    assert_eq!(signed.signatures.len(), 1);

    // The same request id gets the stored answer, byte for byte, without a new review.
    let Outcome::Reply(again) = send(&mut signer, id(7), request) else {
        panic!("a replay must not ask again");
    };
    assert_eq!(again, first);
}

#[test]
fn rejected_and_cancelled_requests_are_answered_and_remembered() {
    let scenario = scenario();
    let (mut signer, _) = imported(&scenario);
    let request = Request::SignTransaction(scenario.request("reissue"));

    assert!(matches!(
        send(&mut signer, id(8), request.clone()),
        Outcome::NeedsApproval(_)
    ));
    let rejected = signer.reject(&id(8)).unwrap();
    assert!(matches!(
        response(&rejected).body,
        Response::Rejected(Rejection {
            code: RejectCode::UserRejected,
            ..
        })
    ));
    assert_eq!(
        rejection(send(&mut signer, id(8), request.clone())),
        RejectCode::UserRejected
    );

    assert!(matches!(
        send(&mut signer, id(9), request),
        Outcome::NeedsApproval(_)
    ));
    assert_eq!(
        reply(send(
            &mut signer,
            id(10),
            Request::Cancel { request: id(9) }
        ))
        .body,
        Response::Cancelled
    );
    assert!(signer.pending().is_none());
    let stored = reply(send(
        &mut signer,
        id(11),
        Request::GetResult { request: id(9) },
    ));
    assert_eq!((stored.id, stored.body), (id(9), Response::Cancelled));
    assert_eq!(
        signer.approve(&id(9), true).unwrap_err(),
        ServiceError::NoSuchRequest
    );
}

#[test]
fn one_request_waits_at_a_time() {
    let scenario = scenario();
    let (mut signer, _) = imported(&scenario);
    assert!(matches!(
        send(
            &mut signer,
            id(20),
            Request::SignTransaction(scenario.request("reissue"))
        ),
        Outcome::NeedsApproval(_)
    ));
    assert_eq!(
        rejection(send(
            &mut signer,
            id(21),
            Request::SignTransaction(scenario.request("withdraw"))
        )),
        RejectCode::Busy
    );
    // Status requests still work while waiting.
    let Response::Status(status) = reply(send(&mut signer, id(22), Request::GetStatus)).body else {
        panic!("expected status");
    };
    assert_eq!(status.pending, Some(id(20)));
    assert_eq!(
        reply(send(
            &mut signer,
            id(23),
            Request::GetResult { request: id(20) }
        ))
        .body,
        Response::Pending
    );
    assert_eq!(
        rejection(send(
            &mut signer,
            id(24),
            Request::GetResult { request: id(99) }
        )),
        RejectCode::NotFound
    );
}

#[test]
fn large_amounts_need_the_pin_and_limits_refuse_before_review() {
    let scenario = scenario();
    let (mut signer, _) = imported(&scenario);
    signer
        .set_policy(PolicyConfig {
            pin_above_msat: Some(1_000_000),
            ..PolicyConfig::default()
        })
        .unwrap();
    let Outcome::NeedsApproval(approval) = send(
        &mut signer,
        id(30),
        Request::SignTransaction(scenario.request("withdraw")),
    ) else {
        panic!("expected an approval");
    };
    assert!(approval.require_pin);
    assert_eq!(
        signer.approve(&id(30), false).unwrap_err(),
        ServiceError::PinRequired
    );
    assert!(
        signer.pending().is_some(),
        "a missing PIN leaves the request waiting"
    );
    assert!(matches!(
        response(&signer.approve(&id(30), true).unwrap().reply).body,
        Response::Signed(_)
    ));

    signer
        .set_policy(PolicyConfig {
            max_outflow_per_day_msat: Some(10_000_000),
            ..PolicyConfig::default()
        })
        .unwrap();
    assert_eq!(
        rejection(send(
            &mut signer,
            id(31),
            Request::SignTransaction(scenario.request("pay_invoice"))
        )),
        RejectCode::PolicyLimit
    );
}

#[test]
fn prepared_notes_release_without_a_second_review_exactly_once() {
    let scenario = scenario();
    let (mut signer, _) = imported(&scenario);
    lenient_releases(&mut signer);
    let federation = scenario.descriptor.federation;
    let answer = approved(
        &mut signer,
        id(40),
        Request::SignTransaction(scenario.request("prepare_send")),
    );
    assert!(matches!(answer.body, Response::Signed(_)));
    let prepared: Vec<NoteRef> = signer.state().prepared.iter().map(|p| p.note).collect();
    assert_eq!(prepared.len(), 2);

    let release = |delivery| {
        Request::ReleaseNotes(ReleaseRequest {
            federation,
            notes: prepared
                .iter()
                .map(|note| ReleaseNote {
                    denomination: note.denomination,
                    tweak: note.tweak,
                    signature: dummy_signature(),
                })
                .collect(),
            include_invite: true,
            delivery,
        })
    };
    let Outcome::Released { reply, show_token } =
        send(&mut signer, id(41), release(Delivery::ShowOnDevice))
    else {
        panic!("prepared notes release without asking");
    };
    let token = show_token.expect("shown on the device");
    assert!(token.starts_with("fedimint"));
    let Response::Released(released) = response(&reply).body else {
        panic!("expected a release");
    };
    assert_eq!(released.amount_msat, (1 << 23) + (1 << 20));
    assert_eq!(
        released.token, None,
        "the phone never sees a token shown on the device"
    );
    assert!(signer.state().prepared.is_empty());
    assert_eq!(signer.state().released.len(), 2);

    assert_eq!(
        rejection(send(&mut signer, id(42), release(Delivery::ReturnToPhone))),
        RejectCode::AlreadyReleased
    );
}

#[test]
fn prepared_notes_release_with_their_real_mint_signatures() {
    let scenario = scenario();
    let (mut signer, _) = imported(&scenario);
    assert!(signer.policy().verify_release_signatures);
    approved(
        &mut signer,
        id(70),
        Request::SignTransaction(scenario.request("prepare_send")),
    );
    let release = |notes: Vec<ReleaseNote>| {
        Request::ReleaseNotes(ReleaseRequest {
            federation: scenario.descriptor.federation,
            notes,
            include_invite: false,
            delivery: Delivery::ReturnToPhone,
        })
    };
    let notes = issued("prepare_send");
    assert_eq!(notes.len(), 2);

    // Each signature is valid, but for the other note: nothing is released.
    let mut swapped = notes.clone();
    let first = swapped[0].signature;
    swapped[0].signature = swapped[1].signature;
    swapped[1].signature = first;
    assert_eq!(
        rejection(send(&mut signer, id(71), release(swapped))),
        RejectCode::InvalidNote
    );
    assert_eq!(signer.state().prepared.len(), 2);

    let Outcome::Released { reply, show_token } = send(&mut signer, id(72), release(notes)) else {
        panic!("prepared notes release without asking");
    };
    assert!(show_token.is_none(), "the token goes to the phone");
    let Response::Released(released) = response(&reply).body else {
        panic!("expected a release");
    };
    assert_eq!(released.amount_msat, (1 << 23) + (1 << 20));
    assert!(released.token.unwrap().starts_with("fedimint"));
    assert!(signer.state().prepared.is_empty());
    assert_eq!(signer.state().released.len(), 2);
}

#[test]
fn an_approved_release_unlocks_the_reclaim_of_those_notes() {
    let scenario = scenario();
    let (mut signer, _) = imported(&scenario);
    lenient_releases(&mut signer);
    let reclaim = scenario.request("reclaim");
    let notes: Vec<ReleaseNote> = reclaim
        .inputs
        .iter()
        .zip([16u8, 15])
        .map(|(hint, denomination)| {
            let InputHint::Note { tweak, .. } = hint else {
                panic!("note hint")
            };
            ReleaseNote {
                denomination,
                tweak: *tweak,
                signature: dummy_signature(),
            }
        })
        .collect();

    // Before release these are ordinary outgoing notes: a reclaim has nothing to reclaim.
    assert_eq!(
        rejection(send(
            &mut signer,
            id(50),
            Request::SignTransaction(reclaim.clone())
        )),
        RejectCode::IntentMismatch
    );

    let release = Request::ReleaseNotes(ReleaseRequest {
        federation: scenario.descriptor.federation,
        notes,
        include_invite: false,
        delivery: Delivery::ReturnToPhone,
    });
    let answer = approved(&mut signer, id(51), release);
    let Response::Released(released) = answer.body else {
        panic!("expected a release");
    };
    assert!(released.token.unwrap().starts_with("fedimint"));

    let answer = approved(&mut signer, id(52), Request::SignTransaction(reclaim));
    assert!(matches!(answer.body, Response::Signed(_)));
}

#[test]
fn a_funded_contract_verifies_its_later_refund() {
    let scenario = scenario();
    let (mut signer, _) = imported(&scenario);
    let Outcome::NeedsApproval(approval) = send(
        &mut signer,
        id(60),
        Request::SignTransaction(scenario.request("refund")),
    ) else {
        panic!("expected an approval");
    };
    assert!(
        !approval.review.warnings.is_empty(),
        "an unknown contract is flagged"
    );
    signer.reject(&id(60)).unwrap();

    approved(
        &mut signer,
        id(61),
        Request::SignTransaction(scenario.request("pay_invoice")),
    );
    assert_eq!(signer.state().contracts.len(), 1);
    let Outcome::NeedsApproval(approval) = send(
        &mut signer,
        id(62),
        Request::SignTransaction(scenario.request("refund")),
    ) else {
        panic!("expected an approval");
    };
    assert!(
        approval.review.warnings.is_empty(),
        "the funded contract is recognised"
    );
}

#[test]
fn state_and_answers_survive_a_restart() {
    let scenario = scenario();
    let (mut signer, storage) = imported(&scenario);
    let first = approved(
        &mut signer,
        id(70),
        Request::SignTransaction(scenario.request("reissue")),
    );
    drop(signer);

    let mut reopened = open(storage, scenario.app_seed);
    assert_eq!(reopened.federations().len(), 1);
    let again = reply(send(
        &mut reopened,
        id(70),
        Request::SignTransaction(scenario.request("reissue")),
    ));
    assert_eq!(again, first);
}

#[test]
fn a_failed_write_leaks_nothing_and_keeps_the_request_waiting() {
    let scenario = scenario();
    let (mut signer, storage) = imported(&scenario);
    lenient_releases(&mut signer);
    approved(
        &mut signer,
        id(80),
        Request::SignTransaction(scenario.request("prepare_send")),
    );
    let notes: Vec<ReleaseNote> = signer
        .state()
        .prepared
        .iter()
        .map(|p| ReleaseNote {
            denomination: p.note.denomination,
            tweak: p.note.tweak,
            signature: dummy_signature(),
        })
        .collect();
    signer
        .set_policy(PolicyConfig {
            verify_release_signatures: false,
            release_prepared_without_review: false,
            ..PolicyConfig::default()
        })
        .unwrap();
    let release = Request::ReleaseNotes(ReleaseRequest {
        federation: scenario.descriptor.federation,
        notes,
        include_invite: false,
        delivery: Delivery::ReturnToPhone,
    });
    assert!(matches!(
        send(&mut signer, id(81), release),
        Outcome::NeedsApproval(_)
    ));

    storage.fail.set(true);
    assert!(matches!(
        signer.approve(&id(81), true),
        Err(ServiceError::Storage(_))
    ));
    assert!(signer.state().released.is_empty(), "nothing is recorded");
    assert!(signer.pending().is_some(), "the user can try again");

    storage.fail.set(false);
    let approved = signer.approve(&id(81), true).unwrap();
    let Response::Released(released) = response(&approved.reply).body else {
        panic!("expected a release");
    };
    assert!(released.token.is_some());
    assert_eq!(signer.state().released.len(), 2);
}

#[test]
fn malformed_and_misaddressed_messages_are_rejected() {
    let scenario = scenario();
    let (mut signer, _) = imported(&scenario);
    let Outcome::Reply(bytes) = signer.handle(b"\xff\x00garbage") else {
        panic!("garbage gets a reply");
    };
    let answer = response(&bytes);
    assert_eq!(answer.id, FixedBytes([0; 16]));
    assert!(matches!(
        answer.body,
        Response::Rejected(Rejection {
            code: RejectCode::Malformed,
            ..
        })
    ));

    let mut elsewhere = scenario.request("reissue");
    elsewhere.federation = FixedBytes([0x55; 32]);
    assert_eq!(
        rejection(send(
            &mut signer,
            id(90),
            Request::SignTransaction(elsewhere)
        )),
        RejectCode::UnknownFederation
    );
    assert_eq!(
        rejection(send(
            &mut signer,
            id(91),
            Request::ExportViewMaterial {
                federation: FixedBytes([0x55; 32])
            }
        )),
        RejectCode::UnknownFederation
    );
}

#[test]
fn outgoing_nonces_come_from_the_hardened_branch_without_approval() {
    let scenario = scenario();
    let (mut signer, _) = imported(&scenario);
    let federation = scenario.descriptor.federation;
    let notes = vec![
        NoteRef {
            denomination: 20,
            tweak: FixedBytes([1; 16]),
        },
        NoteRef {
            denomination: 9,
            tweak: FixedBytes([2; 16]),
        },
    ];
    let Response::OutgoingNonces { nonces } = reply(send(
        &mut signer,
        id(95),
        Request::DeriveOutgoingNonces {
            federation,
            notes: notes.clone(),
        },
    ))
    .body
    else {
        panic!("expected nonces");
    };
    let keys = FederationKeys::derive(&scenario.app_seed, &federation.0).unwrap();
    for (nonce, note) in nonces.iter().zip(&notes) {
        let expected = keys
            .note_nonce(Branch::Outgoing, note.denomination, &note.tweak.0)
            .unwrap();
        assert_eq!(nonce.0, expected.serialize());
        assert_ne!(
            nonce.0,
            keys.note_nonce(Branch::Spend, note.denomination, &note.tweak.0)
                .unwrap()
                .serialize()
        );
    }
    let too_many = vec![notes[0]; 65];
    assert_eq!(
        rejection(send(
            &mut signer,
            id(96),
            Request::DeriveOutgoingNonces {
                federation,
                notes: too_many
            }
        )),
        RejectCode::TooLarge
    );
}
