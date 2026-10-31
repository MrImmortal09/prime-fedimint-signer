//! One review at a time across both backends, and requests that reach across them: `Busy` both
//! ways, `GetResult` and `Cancel` finding either backend's requests, an id both backends hold, and
//! approvals reaching the service whose review waits.
//!
//! A waiting Cashu review must look to a Fedimint phone exactly like a waiting Fedimint review, so
//! the Fedimint answers are compared byte for byte with a bare `Signer` whose own review waits under
//! the same request id.

mod common;

use common::mint::{self, Fixture};
use common::*;
use fps_protocol::*;
use fps_signer::{ApprovalKind, Outcome, ServiceError};

/// The id the waiting review has in both setups.
const WAITING: u8 = 50;

/// A Fedimint history to answer from: the golden federation, a signed reissue (id 2) and a
/// prepared send whose notes may be released without a second review (id 3).
fn history<T: Prime>(prime: &mut T, golden: &Golden) {
    for (request, body) in [
        (1, Request::ImportFederation(golden.descriptor.clone())),
        (2, golden.sign("reissue")),
        (3, golden.sign("prepare_send")),
    ] {
        match prime.handle(&message(id(request), body)) {
            Outcome::NeedsApproval(_) => {}
            _ => panic!("request {request} did not ask"),
        }
        prime.approve(&id(request), true).unwrap();
    }
}

/// The Fedimint requests a phone may send while a review waits, by request id.
fn fedimint_requests(golden: &Golden) -> Vec<(u8, Request)> {
    let federation = golden.federation();
    vec![
        // The four that need approval.
        (60, Request::ImportFederation(golden.descriptor.clone())),
        (61, Request::ExportViewMaterial { federation }),
        (62, golden.sign("withdraw")),
        // Prepared notes, which an idle device would release without asking.
        (63, golden.release("prepare_send", Delivery::ReturnToPhone)),
        // Requests with a stored answer.
        (2, golden.sign("reissue")),
        (1, Request::ImportFederation(golden.descriptor.clone())),
        // Requests that never need approval.
        (64, Request::GetInfo),
        (65, Request::GetStatus),
        (
            66,
            Request::DeriveOutgoingNonces {
                federation,
                notes: vec![NoteRef {
                    denomination: 12,
                    tweak: FixedBytes([9; 16]),
                }],
            },
        ),
        (67, Request::GetResult { request: id(2) }),
    ]
}

#[test]
fn a_waiting_cashu_review_holds_fedimint_requests_as_the_signer_does() {
    let golden = Golden::load();
    let fixture = Fixture::new();

    // The bare signer, its own review waiting under WAITING.
    let bare = Watched::default();
    let mut signer = open_signer(golden.app_seed, &bare, NOW);
    history(&mut signer, &golden);
    approval(send_signer(
        &mut signer,
        id(WAITING),
        Request::ExportViewMaterial {
            federation: golden.federation(),
        },
    ));
    // The same signer with nothing waiting, for the requests whose answers describe Fedimint only.
    let idle_storage = Watched::default();
    let mut idle = open_signer(golden.app_seed, &idle_storage, NOW);
    history(&mut idle, &golden);

    // The device, with the same history and a Cashu review waiting under WAITING.
    let (fedimint, cashu) = (Watched::default(), Watched::default());
    let mut device = open_device(golden.app_seed, &fedimint, &cashu, NOW);
    history(&mut device, &golden);
    let waiting = approval(send(
        &mut device,
        id(WAITING),
        add_mint(&fixture.descriptor),
    ));
    assert_eq!(waiting.kind, ApprovalKind::Cashu);
    let writes = fedimint.write_count();

    for (request, body) in fedimint_requests(&golden) {
        let bytes = message(id(request), body);
        let answer = reply_bytes(device.handle(&bytes));
        let expected = match request {
            // `busy` and `pending` describe Fedimint only: a Cashu review is not Fedimint's.
            64 | 65 => reply_bytes(idle.handle(&bytes)),
            _ => reply_bytes(signer.handle(&bytes)),
        };
        assert!(
            answer == expected,
            "request {request}: {:?}, the signer says {:?}",
            decode_response(&answer).unwrap().body,
            decode_response(&expected).unwrap().body
        );
        if (60..=63).contains(&request) {
            assert_eq!(rejection(Outcome::Reply(answer)).code, RejectCode::Busy);
        }
    }
    assert_eq!(
        fedimint.write_count(),
        writes,
        "nothing reached the signer's state"
    );
    assert_eq!(device.pending(), Some(&waiting));

    // Once the Cashu review is decided, the same requests go through.
    device.reject(&id(WAITING)).unwrap();
    let Outcome::Released { reply, .. } = send(
        &mut device,
        id(63),
        golden.release("prepare_send", Delivery::ReturnToPhone),
    ) else {
        panic!("prepared notes are released without a second review");
    };
    assert!(matches!(
        decode_response(&reply).unwrap().body,
        Response::Released(_)
    ));
}

#[test]
fn a_waiting_fedimint_review_holds_cashu_requests_but_not_info() {
    let golden = Golden::load();
    let fixture = Fixture::new();
    let (mut device, fedimint, cashu) = cashu_device();
    added(&mut device, id(1), &fixture.descriptor);
    let done = {
        approval(send(&mut device, id(2), sign(mint::send(&fixture))));
        device.approve(&id(2), true).unwrap().reply
    };
    let cashu_writes = cashu.write_count();

    // A Fedimint review waits.
    let waiting = approval(send(
        &mut device,
        id(WAITING),
        Request::ImportFederation(golden.descriptor.clone()),
    ));
    assert_eq!(waiting.kind, ApprovalKind::ImportFederation);
    // The bare signer's Busy, for comparison: its review under the same id.
    let mut signer = open_signer(mint::APP_SEED, &Watched::default(), NOW);
    approval(send_signer(
        &mut signer,
        id(WAITING),
        Request::ImportFederation(golden.descriptor.clone()),
    ));
    let signers_busy = reply_bytes(send_signer(
        &mut signer,
        id(60),
        Request::ExportViewMaterial {
            federation: golden.federation(),
        },
    ));

    let busy = reply_bytes(send(&mut device, id(60), sign(mint::melt(&fixture))));
    assert!(busy == signers_busy, "the Busy answer is the signer's");
    assert_eq!(
        rejection(send(&mut device, id(61), add_mint(&fixture.descriptor))).code,
        RejectCode::Busy
    );
    // Info still answers, and a stored answer is still served.
    assert_eq!(cashu_info(&mut device, id(62)).mints.len(), 1);
    assert_eq!(
        reply_bytes(send(&mut device, id(2), sign(mint::send(&fixture)))),
        done
    );
    assert_eq!(cashu.write_count(), cashu_writes);
    assert_eq!(device.pending(), Some(&waiting));

    // Once it is decided, Cashu goes on.
    device.reject(&id(WAITING)).unwrap();
    approval(send(&mut device, id(60), sign(mint::melt(&fixture))));
    assert!(fedimint.write_count() > 0);
}

#[test]
fn get_result_and_cancel_find_either_backends_requests() {
    let golden = Golden::load();
    let fixture = Fixture::new();
    let (mut device, fedimint, cashu) = cashu_device();
    let mut signer = open_signer(mint::APP_SEED, &Watched::default(), NOW);

    // Stored answers on both sides.
    let imported = {
        approval(send(
            &mut device,
            id(1),
            Request::ImportFederation(golden.descriptor.clone()),
        ));
        device.approve(&id(1), false).unwrap().reply
    };
    added(&mut device, id(2), &fixture.descriptor);
    let signed = {
        approval(send(&mut device, id(3), sign(mint::send(&fixture))));
        device.approve(&id(3), true).unwrap().reply
    };
    for (request, expected) in [(1, &imported), (3, &signed)] {
        assert_eq!(
            &reply_bytes(send(
                &mut device,
                id(10),
                Request::GetResult {
                    request: id(request)
                }
            )),
            expected
        );
    }

    // A waiting Cashu request: Pending, then cancelled and stored on the Cashu side only.
    approval(send(&mut device, id(4), sign(mint::melt(&fixture))));
    let fedimint_writes = fedimint.write_count();
    let pending = reply(send(
        &mut device,
        id(11),
        Request::GetResult { request: id(4) },
    ));
    assert_eq!((pending.id, pending.body), (id(11), Response::Pending));
    let cancelled = reply(send(
        &mut device,
        id(12),
        Request::Cancel { request: id(4) },
    ));
    assert_eq!(
        (cancelled.id, cancelled.body),
        (id(12), Response::Cancelled)
    );
    let stored = reply(send(
        &mut device,
        id(13),
        Request::GetResult { request: id(4) },
    ));
    assert_eq!((stored.id, stored.body), (id(4), Response::Cancelled));
    assert_eq!(fedimint.write_count(), fedimint_writes);
    assert!(cashu.cashu_state().stored_result(&id(4)).is_some());

    // A waiting Fedimint request: the signer's own answers.
    let federation = golden.federation();
    approval(send(
        &mut device,
        id(5),
        Request::ExportViewMaterial { federation },
    ));
    let cashu_writes = cashu.write_count();
    let pending = reply(send(
        &mut device,
        id(14),
        Request::GetResult { request: id(5) },
    ));
    assert_eq!((pending.id, pending.body), (id(14), Response::Pending));
    let cancelled = reply(send(
        &mut device,
        id(15),
        Request::Cancel { request: id(5) },
    ));
    assert_eq!(
        (cancelled.id, cancelled.body),
        (id(15), Response::Cancelled)
    );
    assert!(device.state().stored_result(&id(5)).is_some());
    assert_eq!(cashu.write_count(), cashu_writes);

    // Neither has it: the signer's answer, byte for byte.
    for body in [
        Request::GetResult { request: id(99) },
        Request::Cancel { request: id(99) },
        // Stored, but no longer waiting.
        Request::Cancel { request: id(4) },
        Request::Cancel { request: id(3) },
    ] {
        let bytes = message(id(16), body);
        assert_eq!(
            reply_bytes(device.handle(&bytes)),
            reply_bytes(signer.handle(&bytes))
        );
    }
}

/// A phone may use one id on both backends. Each request is replayed from its own backend's
/// answers; `GetResult` and `Cancel` look in Fedimint first, and a Fedimint answer stored under a
/// message's own id is what the signer replays, whatever the message asks.
#[test]
fn ids_shared_by_both_backends_replay_per_backend_and_resolve_fedimint_first() {
    let golden = Golden::load();
    let fixture = Fixture::new();
    let (mut device, fedimint, _) = cashu_device();
    let import = Request::ImportFederation(golden.descriptor.clone());
    approval(send(&mut device, id(1), import.clone()));
    let imported = device.approve(&id(1), false).unwrap().reply;
    added(&mut device, id(9), &fixture.descriptor);
    // The same id on the Cashu side: a review and an answer of Cashu's own.
    approval(send(&mut device, id(1), sign(mint::send(&fixture))));
    let signed = device.approve(&id(1), true).unwrap().reply;
    assert_ne!(signed, imported);

    assert_eq!(reply_bytes(send(&mut device, id(1), import)), imported);
    assert_eq!(
        reply_bytes(send(&mut device, id(1), sign(mint::send(&fixture)))),
        signed
    );
    assert_eq!(
        reply_bytes(send(
            &mut device,
            id(10),
            Request::GetResult { request: id(1) }
        )),
        imported
    );

    // A `Cancel` sent under an id Fedimint has answered gets that answer, and the Cashu review it
    // names keeps waiting.
    approval(send(&mut device, id(2), sign(mint::melt(&fixture))));
    let writes = fedimint.write_count();
    assert_eq!(
        reply_bytes(send(&mut device, id(1), Request::Cancel { request: id(2) })),
        imported
    );
    assert_eq!(device.pending().map(|waiting| waiting.request), Some(id(2)));
    assert_eq!(fedimint.write_count(), writes);
}

#[test]
fn approving_and_rejecting_reach_the_service_whose_review_waits() {
    let golden = Golden::load();
    let fixture = Fixture::new();
    let (mut device, fedimint, cashu) = cashu_device();
    added(&mut device, id(1), &fixture.descriptor);
    let federation_writes = fedimint.write_count();

    // Cashu's review.
    approval(send(&mut device, id(2), sign(mint::send(&fixture))));
    assert_eq!(
        device.pending().map(|waiting| waiting.kind),
        Some(ApprovalKind::Cashu)
    );
    assert_eq!(
        device.approve(&id(9), true).unwrap_err(),
        ServiceError::NoSuchRequest
    );
    assert_eq!(
        device.reject(&id(9)).unwrap_err(),
        ServiceError::NoSuchRequest
    );
    let cashu_writes = cashu.write_count();
    device.reject(&id(2)).unwrap();
    assert_eq!(cashu.write_count(), cashu_writes + 1);
    assert_eq!(fedimint.write_count(), federation_writes);
    assert!(device.pending().is_none());

    // Fedimint's review.
    approval(send(
        &mut device,
        id(3),
        Request::ImportFederation(golden.descriptor.clone()),
    ));
    assert_eq!(
        device.pending().map(|waiting| waiting.kind),
        Some(ApprovalKind::ImportFederation)
    );
    let cashu_writes = cashu.write_count();
    let answer = decode_response(&device.approve(&id(3), false).unwrap().reply).unwrap();
    assert!(matches!(answer.body, Response::FederationImported(_)));
    assert_eq!(fedimint.write_count(), federation_writes + 1);
    assert_eq!(cashu.write_count(), cashu_writes);

    // And Cashu's again, approved.
    approval(send(&mut device, id(4), sign(mint::send(&fixture))));
    let answer = decode_response(&device.approve(&id(4), true).unwrap().reply).unwrap();
    assert!(matches!(
        answer.body,
        Response::Cashu(CashuResponse::Signed(_))
    ));
    assert_eq!(fedimint.write_count(), federation_writes + 1);
}
