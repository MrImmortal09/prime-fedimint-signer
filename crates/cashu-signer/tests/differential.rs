//! With Cashu idle, `Device` is the bare `Signer`, byte for byte.
//!
//! One driver plays the Fedimint golden scenario of `crates/signer/tests/compat_state.rs`
//! (`run_scenario`, which leaves exactly the frozen `fps-state-v1.cbor`) and then everything else
//! the signer answers: info and status, every stored answer and replays, nonces, malformed, foreign
//! and oversized messages, `Busy` with each request that needs approval and what never waits
//! (nonces, results, cancels and malformed bytes) while a review does, a PIN, a release without a
//! second review, a failed write, forgetting and importing again. It runs three times: on a bare
//! `Signer`, on a `Device` with no Cashu state, and on a `Device` with a populated Cashu state,
//! whose answers sit under ids the driver sends too. Every answer, every approval asked for, every
//! result of approving, rejecting and changing the policy, and every write of `fps-state.cbor`
//! must be the same, and the Cashu state never written.

mod common;

use common::*;
use fps_policy::PolicyConfig;
use fps_protocol::*;
use fps_signer::{Approval, Outcome, ServiceError};

/// Everything the driver sees.
#[derive(Debug, PartialEq)]
enum Event {
    Reply(Vec<u8>),
    Approval(Approval),
    Released(Vec<u8>, Option<String>),
    Approved(Result<(Vec<u8>, Option<String>), ServiceError>),
    Rejected(Result<Vec<u8>, ServiceError>),
    Policy(Result<(), ServiceError>),
    Forgot(Result<(), ServiceError>),
    Pending(Option<Approval>),
    Federations(Vec<FederationSummary>),
    /// How many times the signer had written its state at this point.
    Writes(usize),
}

struct Driver<'a, T: Prime> {
    prime: T,
    fedimint: &'a Watched,
    events: Vec<Event>,
}

impl<T: Prime> Driver<'_, T> {
    fn raw(&mut self, bytes: &[u8]) {
        let event = match self.prime.handle(bytes) {
            Outcome::Reply(reply) => Event::Reply(reply),
            Outcome::NeedsApproval(approval) => Event::Approval(approval),
            Outcome::Released { reply, show_token } => {
                Event::Released(reply, show_token.map(|token| token.to_string()))
            }
        };
        self.events.push(event);
    }

    fn send(&mut self, request: u8, body: Request) {
        self.raw(&message(id(request), body));
    }

    fn approve(&mut self, request: u8, pin_verified: bool) {
        let approved = self
            .prime
            .approve(&id(request), pin_verified)
            .map(|approved| {
                (
                    approved.reply,
                    approved.show_token.map(|token| token.to_string()),
                )
            });
        self.events.push(Event::Approved(approved));
    }

    /// Sends a request that needs approval and approves it with the PIN.
    fn approved(&mut self, request: u8, body: Request) {
        self.send(request, body);
        assert!(
            matches!(self.events.last(), Some(Event::Approval(_))),
            "request {request} did not ask for approval: {:?}",
            self.events.last()
        );
        self.approve(request, true);
    }

    fn reject(&mut self, request: u8) {
        let rejected = self.prime.reject(&id(request));
        self.events.push(Event::Rejected(rejected));
    }

    fn policy(&mut self, policy: PolicyConfig) {
        let changed = self.prime.set_policy(policy);
        self.events.push(Event::Policy(changed));
    }

    fn snapshot(&mut self) {
        self.events.push(Event::Pending(self.prime.pending()));
        self.events
            .push(Event::Federations(self.prime.federations()));
        self.events.push(Event::Writes(self.fedimint.write_count()));
    }
}

fn play<T: Prime>(driver: &mut Driver<T>, golden: &Golden) {
    let federation = golden.federation();

    // `run_scenario` of compat_state.rs, step for step.
    driver.approved(1, Request::ImportFederation(golden.descriptor.clone()));
    for (n, case) in [
        (2, "reissue"),
        (3, "pay_invoice"),
        (4, "refund"),
        (5, "withdraw"),
        (6, "prepare_send"),
    ] {
        driver.approved(n, golden.sign(case));
    }
    driver.approved(7, golden.release("reclaim", Delivery::ReturnToPhone));
    driver.approved(8, golden.sign("reclaim"));
    driver.send(9, Request::ExportViewMaterial { federation });
    driver.reject(9);
    driver.send(10, golden.sign("reissue"));
    driver.send(11, Request::Cancel { request: id(10) });
    driver.policy(compat_policy());
    driver.snapshot();

    // What a device that holds that state answers.
    driver.send(20, Request::GetInfo);
    driver.send(21, Request::GetStatus);
    for stored in 1..=10 {
        driver.send(
            22,
            Request::GetResult {
                request: id(stored),
            },
        );
    }
    driver.send(23, Request::GetResult { request: id(99) });
    driver.send(24, Request::Cancel { request: id(99) });
    driver.send(3, golden.sign("pay_invoice"));
    driver.send(7, golden.release("reclaim", Delivery::ReturnToPhone));
    driver.send(25, golden.release("reclaim", Delivery::ShowOnDevice));
    // Over the daily limit of the frozen policy.
    driver.send(26, golden.sign("pay_invoice"));
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
    driver.send(
        27,
        Request::DeriveOutgoingNonces {
            federation,
            notes: notes.clone(),
        },
    );
    driver.send(
        28,
        Request::DeriveOutgoingNonces {
            federation,
            notes: vec![notes[0]; 65],
        },
    );
    driver.send(
        29,
        Request::ExportViewMaterial {
            federation: FixedBytes([0x55; 32]),
        },
    );
    driver.raw(b"\xff\x00garbage");
    driver.raw(&encode(&Envelope {
        version: 2,
        id: id(30),
        body: Request::GetInfo,
    }));
    driver.raw(&vec![0; MAX_MESSAGE_SIZE + 1]);
    let mut truncated = message(id(31), golden.sign("withdraw"));
    truncated.truncate(truncated.len() - 3);
    driver.raw(&truncated);
    driver.snapshot();

    // One review waits; every other request that needs approval is Busy.
    driver.send(40, Request::ExportViewMaterial { federation });
    driver.send(41, Request::ImportFederation(golden.descriptor.clone()));
    driver.send(42, Request::ExportViewMaterial { federation });
    driver.send(43, golden.sign("reissue"));
    driver.send(44, golden.release("prepare_send", Delivery::ReturnToPhone));
    driver.send(2, golden.sign("reissue"));
    driver.send(45, Request::GetResult { request: id(40) });
    driver.send(46, Request::GetStatus);
    driver.send(47, Request::GetInfo);
    // What never waits for approval is answered while a review does.
    driver.send(
        48,
        Request::DeriveOutgoingNonces {
            federation,
            notes: vec![notes[0]],
        },
    );
    driver.send(49, Request::GetResult { request: id(99) });
    driver.send(49, Request::Cancel { request: id(99) });
    driver.raw(b"\xff\x00garbage");
    driver.snapshot();
    driver.approve(99, true);
    driver.reject(99);
    driver.approve(40, false);
    // View material is never stored: the same request asks again.
    driver.send(40, Request::ExportViewMaterial { federation });
    driver.reject(40);

    // A PIN, a release without a second review, and a failed write.
    driver.policy(PolicyConfig {
        pin_above_msat: Some(1),
        ..PolicyConfig::default()
    });
    driver.send(50, golden.sign("reissue"));
    driver.approve(50, false);
    driver.snapshot();
    driver.approve(50, true);
    driver.policy(PolicyConfig::default());
    driver.send(51, golden.release("prepare_send", Delivery::ShowOnDevice));
    driver.send(52, golden.sign("withdraw"));
    driver.fedimint.fail_writes(true);
    driver.approve(52, true);
    driver.snapshot();
    driver.fedimint.fail_writes(false);
    driver.approve(52, true);

    // Forgetting a federation and importing it again.
    let forgot = driver.prime.forget_federation(&federation);
    driver.events.push(Event::Forgot(forgot));
    driver.send(53, Request::ExportViewMaterial { federation });
    let again = driver.prime.forget_federation(&federation);
    driver.events.push(Event::Forgot(again));
    driver.approved(54, Request::ImportFederation(golden.descriptor.clone()));
    driver.snapshot();
}

/// The driver's events and the signer's writes, and the `Prime` it drove.
fn transcript<T: Prime>(
    prime: T,
    fedimint: &Watched,
    golden: &Golden,
) -> (T, Vec<Event>, Vec<Vec<u8>>) {
    let mut driver = Driver {
        prime,
        fedimint,
        events: Vec::new(),
    };
    play(&mut driver, golden);
    (driver.prime, driver.events, fedimint.writes())
}

#[test]
fn the_bare_signer_leaves_the_frozen_state() {
    // The first part of the driver is compat_state.rs's scenario: its last write is the frozen state.
    let golden = Golden::load();
    let fedimint = Watched::default();
    let (_, events, writes) = transcript(
        open_signer(golden.app_seed, &fedimint, NOW),
        &fedimint,
        &golden,
    );
    let Some(Event::Writes(at_freeze)) = events
        .iter()
        .find(|event| matches!(event, Event::Writes(_)))
    else {
        panic!("no snapshot");
    };
    assert!(
        writes[at_freeze - 1] == FROZEN_STATE,
        "the scenario no longer writes the frozen state"
    );
    // The driver went where it meant to: a release without review, a PIN asked for, a failed write.
    assert!(events
        .iter()
        .any(|event| matches!(event, Event::Released(_, Some(_)))));
    assert!(events
        .iter()
        .any(|event| matches!(event, Event::Approved(Err(ServiceError::PinRequired)))));
    assert!(events
        .iter()
        .any(|event| matches!(event, Event::Approved(Err(ServiceError::Storage(_))))));
    let busy = events
        .iter()
        .filter(|event| match event {
            Event::Reply(bytes) => matches!(
                decode_response(bytes).map(|envelope| envelope.body),
                Ok(Response::Rejected(Rejection {
                    code: RejectCode::Busy,
                    ..
                }))
            ),
            _ => false,
        })
        .count();
    assert_eq!(
        busy, 4,
        "each request that needs approval met a waiting review"
    );
}

fn assert_same_as_the_signer(cashu: Watched) {
    let golden = Golden::load();
    let cashu_before = cashu.bytes();

    let bare = Watched::default();
    let (_, expected_events, expected_writes) =
        transcript(open_signer(golden.app_seed, &bare, NOW), &bare, &golden);

    let fedimint = Watched::default();
    let device = open_device(golden.app_seed, &fedimint, &cashu, NOW);
    let (mut device, events, writes) = transcript(device, &fedimint, &golden);

    assert_eq!(events.len(), expected_events.len());
    for (n, (event, expected)) in events.iter().zip(&expected_events).enumerate() {
        assert_eq!(event, expected, "event {n} differs");
    }
    assert_eq!(writes.len(), expected_writes.len());
    assert!(
        writes == expected_writes,
        "fps-state.cbor was written differently"
    );
    assert_eq!(cashu.write_count(), 0, "the Cashu state was written");
    assert_eq!(cashu.bytes(), cashu_before);
    // The Cashu state was read, once, when a `GetResult` found nothing on the Fedimint side, and
    // Cashu was on all along.
    assert_eq!(cashu.read_count(), 1);
    assert!(device.cashu_off().is_none());
}

#[test]
fn a_device_without_cashu_state_answers_as_the_bare_signer() {
    assert_same_as_the_signer(Watched::default());
}

#[test]
fn a_device_with_cashu_state_answers_as_the_bare_signer() {
    assert_same_as_the_signer(Watched::holding(&populated_cashu_state()));
}
