//! Compatibility pins for the signer's state file, `fps-state.cbor` version 1, frozen against the
//! Fedimint-only code (`main` at `3455d17`) before the Cashu backend was added.
//!
//! `compat/fps-state-v1.cbor` is the file the service wrote at the end of `run_scenario`: the golden
//! scenario plus a policy change, which populates every field of `State`. Every later build must
//! open that file and act on it, read and write it back byte for byte, and still write exactly it
//! for the same scenario. A diff in this file or in its fixture means the Fedimint state format or
//! the signer's behaviour changed, so an installed Prime's state would no longer open, or would open
//! differently. Fix the code, never the fixture.

use std::cell::RefCell;
use std::rc::Rc;

use fps_policy::{PolicyConfig, PolicyState};
use fps_protocol::*;
use fps_signer::{
    Approval, FederationNote, FundedContract, MemoryStorage, Outcome, Platform, Signer, State,
    Storage,
};
use serde_json::Value;

const FIXTURE: &[u8] = include_bytes!("compat/fps-state-v1.cbor");

/// The state a device starts from, with the default policy, as CBOR hex.
const FRESH_STATE: &str = concat!(
    "a86776657273696f6e016b66656465726174696f6e73806872656c6561736564806870726570617265648069",
    "636f6e7472616374738067726573756c74738066706f6c696379aa781c6d61785f6f7574666c6f775f706572",
    "5f726571756573745f6d736174f678186d61785f6f7574666c6f775f7065725f6461795f6d736174f66e7069",
    "6e5f61626f76655f6d7361741a05f5e10078196d61785f726f7574696e675f6665655f626173655f6d736174",
    "1a000186a0736d61785f726f7574696e675f6665655f70706d193a98767665726966795f6e6f74655f736967",
    "6e617475726573f578197665726966795f72656c656173655f7369676e617475726573f5781f72656c656173",
    "655f70726570617265645f776974686f75745f726576696577f56a6d61785f696e7075747318406b6d61785f",
    "6f75747075747318406c706f6c6963795f7374617465a263646179f66c6f7574666c6f775f6d73617400",
);

/// The clock of every signer here: 2026-01-01T00:00:00Z, UTC day 20454.
const NOW: u64 = 1_767_225_600;
const DAY: u64 = 86_400;

/// What the scenario's approvals let leave, in msat: the external outputs and fees of the six signed
/// transactions, and the released notes.
const OUTFLOW_MSAT: u64 = 552_067_136;

struct Golden {
    vectors: Value,
    app_seed: [u8; 32],
    descriptor: FederationDescriptor,
}

impl Golden {
    fn load() -> Self {
        let vectors: Value = serde_json::from_str(include_str!(
            "../../signer-core/tests/vectors/fedimint-0.13.0-beta.1.json"
        ))
        .unwrap();
        let scenario = &vectors["scenario"];
        let app_seed = hex_bytes(&scenario["app_seed"]).try_into().unwrap();
        let descriptor =
            ciborium::from_reader(hex_bytes(&scenario["descriptor"]).as_slice()).unwrap();
        Self {
            vectors,
            app_seed,
            descriptor,
        }
    }

    fn federation(&self) -> FederationId {
        self.descriptor.federation
    }

    fn case(&self, name: &str) -> &Value {
        self.vectors["scenario"]["transactions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == name)
            .unwrap_or_else(|| panic!("no golden case {name}"))
    }

    fn request(&self, name: &str) -> SignRequest {
        ciborium::from_reader(hex_bytes(&self.case(name)["request"]).as_slice()).unwrap()
    }

    /// The mint signatures the scenario federation issued for a case's outgoing notes.
    fn issued(&self, name: &str) -> Vec<ReleaseNote> {
        self.case(name)["issued"]
            .as_array()
            .unwrap()
            .iter()
            .map(|note| ReleaseNote {
                denomination: note["denomination"].as_u64().unwrap() as u8,
                tweak: FixedBytes(hex_bytes(&note["tweak"]).try_into().unwrap()),
                signature: FixedBytes(hex_bytes(&note["signature"]).try_into().unwrap()),
            })
            .collect()
    }

    /// A case's notes as the ledger records them.
    fn ledger_notes(&self, name: &str) -> Vec<FederationNote> {
        self.issued(name)
            .iter()
            .map(|note| FederationNote {
                federation: self.federation(),
                note: NoteRef {
                    denomination: note.denomination,
                    tweak: note.tweak,
                },
            })
            .collect()
    }

    fn release(&self, case: &str) -> Request {
        Request::ReleaseNotes(ReleaseRequest {
            federation: self.federation(),
            notes: self.issued(case),
            include_invite: true,
            delivery: Delivery::ReturnToPhone,
        })
    }
}

fn hex_bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap()).unwrap()
}

/// The platform of `service.rs`: randomness from a counter, so every run signs the same bytes, and a
/// fixed clock.
struct TestPlatform {
    counter: u8,
    now: u64,
}

impl Platform for TestPlatform {
    fn random_32(&mut self) -> [u8; 32] {
        self.counter = self.counter.wrapping_add(1);
        [self.counter; 32]
    }

    fn now_unix(&self) -> Option<u64> {
        Some(self.now)
    }
}

/// Memory storage the test can still read after the signer took it.
#[derive(Clone, Default)]
struct SharedStorage(Rc<RefCell<MemoryStorage>>);

impl SharedStorage {
    fn holding(bytes: &[u8]) -> Self {
        Self(Rc::new(RefCell::new(MemoryStorage {
            bytes: Some(bytes.to_vec()),
        })))
    }

    fn bytes(&self) -> Vec<u8> {
        self.0
            .borrow()
            .bytes
            .clone()
            .expect("the signer wrote its state")
    }
}

impl Storage for SharedStorage {
    fn load(&mut self) -> Result<Option<Vec<u8>>, String> {
        self.0.borrow_mut().load()
    }

    fn store(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.0.borrow_mut().store(bytes)
    }
}

type TestSigner = Signer<SharedStorage, TestPlatform>;

fn open(storage: SharedStorage, app_seed: [u8; 32], now: u64) -> TestSigner {
    Signer::open(app_seed, storage, TestPlatform { counter: 0, now }).unwrap()
}

/// A signer over the frozen state, and its storage.
fn frozen_signer(golden: &Golden, now: u64) -> (TestSigner, SharedStorage) {
    let storage = SharedStorage::holding(FIXTURE);
    (open(storage.clone(), golden.app_seed, now), storage)
}

fn id(byte: u8) -> RequestId {
    FixedBytes([byte; 16])
}

fn send(signer: &mut TestSigner, request: RequestId, body: Request) -> Outcome {
    signer.handle(&encode(&Envelope::new(request, body)))
}

/// Expects an immediate reply and returns it.
fn reply(outcome: Outcome) -> ResponseEnvelope {
    match outcome {
        Outcome::Reply(bytes) => decode_response(&bytes).unwrap(),
        Outcome::NeedsApproval(approval) => {
            panic!("unexpected approval: {}", approval.review.title)
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

fn approval(outcome: Outcome) -> Approval {
    match outcome {
        Outcome::NeedsApproval(approval) => approval,
        Outcome::Reply(bytes) => panic!(
            "expected an approval, got {:?}",
            decode_response(&bytes).unwrap().body
        ),
        Outcome::Released { .. } => panic!("expected an approval, got a release"),
    }
}

/// Sends a request that needs approval, approves it, and checks the answer is not a refusal.
fn approved(signer: &mut TestSigner, request: RequestId, body: Request) {
    approval(send(signer, request, body));
    let reply = signer.approve(&request, true).unwrap().reply;
    let answer = decode_response(&reply).unwrap().body;
    assert!(
        !matches!(answer, Response::Rejected(_)),
        "request {request:?} was refused: {answer:?}"
    );
}

/// Every setting differs from its default, so a setting that stops being read or written changes
/// the bytes.
fn compat_policy() -> PolicyConfig {
    PolicyConfig {
        max_outflow_per_request_msat: Some(1_000_000_000),
        max_outflow_per_day_msat: Some(600_000_000),
        pin_above_msat: Some(10_000_000),
        max_routing_fee_base_msat: 200_000,
        max_routing_fee_ppm: 20_000,
        verify_note_signatures: false,
        verify_release_signatures: false,
        release_prepared_without_review: false,
        max_inputs: 32,
        max_outputs: 48,
    }
}

/// Drives a fresh signer through the golden scenario, changes its policy, and returns the state file
/// it wrote last.
fn run_scenario(golden: &Golden) -> Vec<u8> {
    let storage = SharedStorage::default();
    let mut signer = open(storage.clone(), golden.app_seed, NOW);
    let federation = golden.federation();

    approved(
        &mut signer,
        id(1),
        Request::ImportFederation(golden.descriptor.clone()),
    );
    // Every golden transaction but the reclaim, whose notes are released first. The payment funds
    // the contract the refund spends; the send stages two outgoing notes.
    for (n, case) in [
        (2, "reissue"),
        (3, "pay_invoice"),
        (4, "refund"),
        (5, "withdraw"),
        (6, "prepare_send"),
    ] {
        approved(
            &mut signer,
            id(n),
            Request::SignTransaction(golden.request(case)),
        );
    }
    approved(&mut signer, id(7), golden.release("reclaim"));
    approved(
        &mut signer,
        id(8),
        Request::SignTransaction(golden.request("reclaim")),
    );
    // One request the user rejects, and one the phone cancels.
    approval(send(
        &mut signer,
        id(9),
        Request::ExportViewMaterial { federation },
    ));
    signer.reject(&id(9)).unwrap();
    approval(send(
        &mut signer,
        id(10),
        Request::SignTransaction(golden.request("reissue")),
    ));
    assert_eq!(
        reply(send(
            &mut signer,
            id(11),
            Request::Cancel { request: id(10) }
        ))
        .body,
        Response::Cancelled
    );

    signer.set_policy(compat_policy()).unwrap();
    storage.bytes()
}

/// The fields in which two states differ.
fn differing_fields(a: &State, b: &State) -> Vec<&'static str> {
    [
        ("version", a.version == b.version),
        ("federations", a.federations == b.federations),
        ("released", a.released == b.released),
        ("prepared", a.prepared == b.prepared),
        ("contracts", a.contracts == b.contracts),
        ("results", a.results == b.results),
        ("policy", a.policy == b.policy),
        ("policy_state", a.policy_state == b.policy_state),
    ]
    .into_iter()
    .filter(|(_, same)| !same)
    .map(|(field, _)| field)
    .collect()
}

#[test]
fn the_frozen_state_decodes_and_encodes_back_byte_for_byte() {
    let golden = Golden::load();
    let state = State::decode(FIXTURE).unwrap();
    assert!(
        state.encode() == FIXTURE,
        "the frozen state does not encode back to its bytes"
    );

    // Every field holds what the scenario put there.
    assert_eq!(state.version, 1);
    assert_eq!(state.federations, vec![golden.descriptor.clone()]);
    assert_eq!(state.released, golden.ledger_notes("reclaim"));
    assert_eq!(state.prepared, golden.ledger_notes("prepare_send"));
    let funded = &golden.case("refund")["funded"][0];
    assert_eq!(
        state.contracts,
        vec![FundedContract {
            federation: golden.federation(),
            txid: FixedBytes(hex_bytes(&funded["txid"]).try_into().unwrap()),
            out_idx: funded["out_idx"].as_u64().unwrap(),
            contract: hex_bytes(&funded["contract"]).into(),
        }]
    );
    let answers: Vec<(RequestId, &str)> = state
        .results
        .iter()
        .map(|stored| {
            let envelope = decode_response(stored.response.as_slice()).unwrap();
            assert_eq!(envelope.id, stored.request);
            let answer = match envelope.body {
                Response::FederationImported(_) => "imported",
                Response::Signed(_) => "signed",
                Response::Released(_) => "released",
                Response::Rejected(Rejection {
                    code: RejectCode::UserRejected,
                    ..
                }) => "rejected",
                Response::Cancelled => "cancelled",
                _ => "unexpected",
            };
            (stored.request, answer)
        })
        .collect();
    assert_eq!(
        answers,
        [
            "imported",
            "signed",
            "signed",
            "signed",
            "signed",
            "signed",
            "released",
            "signed",
            "rejected",
            "cancelled"
        ]
        .into_iter()
        .zip(1..)
        .map(|(answer, n)| (id(n), answer))
        .collect::<Vec<_>>()
    );
    assert_eq!(state.policy, compat_policy());
    assert_eq!(
        state.policy_state,
        PolicyState {
            day: Some(NOW / DAY),
            outflow_msat: OUTFLOW_MSAT,
        }
    );
    let default = PolicyConfig::default();
    let policy = &state.policy;
    let unchanged: Vec<&str> = [
        (
            "max_outflow_per_request_msat",
            policy.max_outflow_per_request_msat == default.max_outflow_per_request_msat,
        ),
        (
            "max_outflow_per_day_msat",
            policy.max_outflow_per_day_msat == default.max_outflow_per_day_msat,
        ),
        (
            "pin_above_msat",
            policy.pin_above_msat == default.pin_above_msat,
        ),
        (
            "max_routing_fee_base_msat",
            policy.max_routing_fee_base_msat == default.max_routing_fee_base_msat,
        ),
        (
            "max_routing_fee_ppm",
            policy.max_routing_fee_ppm == default.max_routing_fee_ppm,
        ),
        (
            "verify_note_signatures",
            policy.verify_note_signatures == default.verify_note_signatures,
        ),
        (
            "verify_release_signatures",
            policy.verify_release_signatures == default.verify_release_signatures,
        ),
        (
            "release_prepared_without_review",
            policy.release_prepared_without_review == default.release_prepared_without_review,
        ),
        ("max_inputs", policy.max_inputs == default.max_inputs),
        ("max_outputs", policy.max_outputs == default.max_outputs),
    ]
    .into_iter()
    .filter(|(_, same)| *same)
    .map(|(setting, _)| setting)
    .collect();
    assert!(unchanged.is_empty(), "left at the default: {unchanged:?}");
}

#[test]
fn the_frozen_state_opens_as_a_populated_signer() {
    let golden = Golden::load();
    let federation = golden.federation();
    let (mut signer, storage) = frozen_signer(&golden, NOW);
    assert_eq!(signer.state(), &State::decode(FIXTURE).unwrap());
    assert_eq!(signer.policy(), &compat_policy());
    // Saving it unchanged writes the same bytes.
    signer.set_policy(compat_policy()).unwrap();
    assert!(
        storage.bytes() == FIXTURE,
        "the signer saves the frozen state differently"
    );

    // Its federation is listed.
    let Response::Info(info) = reply(send(&mut signer, id(20), Request::GetInfo)).body else {
        panic!("expected info");
    };
    assert_eq!(info.protocol, 1);
    assert_eq!(info.key_scheme, "fps-v1");
    assert_eq!(info.federations, signer.federations());
    assert_eq!(info.federations.len(), 1);
    assert_eq!(info.federations[0].federation, federation);
    assert_eq!(info.federations[0].guardians, 4);
    assert!(!info.busy);

    // Its ledger, recent requests and today's outflow are reported.
    let Response::Status(status) = reply(send(&mut signer, id(21), Request::GetStatus)).body else {
        panic!("expected status");
    };
    let released: Vec<ReleasedNote> = golden
        .ledger_notes("reclaim")
        .into_iter()
        .map(|note| ReleasedNote {
            federation: note.federation,
            note: note.note,
        })
        .collect();
    assert_eq!(status.released, released);
    assert_eq!(status.recent, (1..=10).map(id).collect::<Vec<_>>());
    assert_eq!(status.outflow_today_msat, OUTFLOW_MSAT);
    assert_eq!(status.pending, None);

    // Every stored answer is served again byte for byte, and a repeated request gets its stored
    // answer instead of a second signature.
    for stored in signer.state().results.clone() {
        let Outcome::Reply(bytes) = send(
            &mut signer,
            id(22),
            Request::GetResult {
                request: stored.request,
            },
        ) else {
            panic!("a stored answer is served without a review");
        };
        assert!(bytes == stored.response.as_slice());
    }
    let Outcome::Reply(again) = send(
        &mut signer,
        id(3),
        Request::SignTransaction(golden.request("pay_invoice")),
    ) else {
        panic!("a repeated request is answered without a review");
    };
    assert!(again.as_slice() == signer.state().stored_result(&id(3)).unwrap());
    let Response::Signed(signed) = decode_response(&again).unwrap().body else {
        panic!("expected signatures");
    };
    assert!(signed.gateway_auth.is_some());

    // Released notes stay released.
    assert_eq!(
        rejection(send(&mut signer, id(23), golden.release("reclaim"))),
        RejectCode::AlreadyReleased
    );
    // The funded contract is recognised when its refund comes to be signed.
    let refund = approval(send(
        &mut signer,
        id(24),
        Request::SignTransaction(golden.request("refund")),
    ));
    assert!(
        refund.review.warnings.is_empty(),
        "the funded contract is recognised"
    );
}

#[test]
fn the_frozen_policy_and_counters_still_apply() {
    let golden = Golden::load();
    let pay = || Request::SignTransaction(golden.request("pay_invoice"));

    // Today's outflow is counted: a second payment today goes over the daily limit.
    let (mut signer, _) = frozen_signer(&golden, NOW);
    assert_eq!(
        rejection(send(&mut signer, id(30), pay())),
        RejectCode::PolicyLimit
    );

    // The next day starts a new count, and the payment is over the PIN threshold.
    let (mut signer, _) = frozen_signer(&golden, NOW + DAY);
    assert!(approval(send(&mut signer, id(30), pay())).require_pin);

    // The staged notes are still staged: once the policy allows it again, they are released
    // without a second review.
    let (mut signer, _) = frozen_signer(&golden, NOW);
    let release = golden.release("prepare_send");
    assert!(matches!(
        send(&mut signer, id(31), release.clone()),
        Outcome::NeedsApproval(_)
    ));
    let (mut signer, _) = frozen_signer(&golden, NOW);
    signer
        .set_policy(PolicyConfig {
            release_prepared_without_review: true,
            ..compat_policy()
        })
        .unwrap();
    let Outcome::Released { reply, .. } = send(&mut signer, id(31), release) else {
        panic!("staged notes are released without a second review");
    };
    let Response::Released(released) = decode_response(&reply).unwrap().body else {
        panic!("expected a release");
    };
    assert_eq!(released.amount_msat, (1 << 23) + (1 << 20));
}

#[test]
fn the_scenario_still_writes_exactly_the_frozen_state() {
    let golden = Golden::load();
    let written = run_scenario(&golden);
    let differing = differing_fields(
        &State::decode(&written).unwrap(),
        &State::decode(FIXTURE).unwrap(),
    );
    assert!(
        differing.is_empty(),
        "the scenario now leaves different {differing:?}"
    );
    assert!(
        written == FIXTURE,
        "the same state is now written as different bytes"
    );
}

#[test]
fn a_fresh_state_is_unchanged() {
    let signer = Signer::open(
        [0; 32],
        MemoryStorage::default(),
        TestPlatform {
            counter: 0,
            now: NOW,
        },
    )
    .unwrap();
    assert_eq!(signer.policy(), &PolicyConfig::default());
    assert_eq!(hex::encode(signer.state().encode()), FRESH_STATE);
}
