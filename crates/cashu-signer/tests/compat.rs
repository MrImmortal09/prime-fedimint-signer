//! Compatibility across versions: an old phone talking to a Prime with Cashu, a new phone asking an
//! old Prime for Cashu, and a Prime whose app has the new protocol crate but no Cashu service.

mod common;

use std::collections::BTreeMap;

use common::mint::Fixture;
use common::*;
use fps_protocol::*;
use fps_signer::Outcome;
use serde::de::IgnoredAny;
use serde::Deserialize;

/// Every v1 message of the wire format pins, by name: `crates/protocol/tests/compat`.
fn v1_samples() -> BTreeMap<String, Vec<u8>> {
    let fixture: BTreeMap<String, String> =
        serde_json::from_str(include_str!("../../protocol/tests/compat/v1-messages.json")).unwrap();
    fixture
        .into_iter()
        .map(|(name, hex)| (name, hex::decode(hex).unwrap()))
        .collect()
}

/// What one message did to a `Prime`, approved where it asked.
#[derive(Debug, PartialEq)]
enum Answer {
    Reply(Vec<u8>),
    Approved(fps_signer::Approval, Vec<u8>),
    Released(Vec<u8>),
}

fn answer<T: Prime>(prime: &mut T, bytes: &[u8]) -> Answer {
    match prime.handle(bytes) {
        Outcome::Reply(reply) => Answer::Reply(reply),
        Outcome::NeedsApproval(approval) => {
            let reply = prime.approve(&approval.request, true).unwrap().reply;
            Answer::Approved(approval, reply)
        }
        Outcome::Released { reply, .. } => Answer::Released(reply),
    }
}

#[test]
fn an_old_phone_gets_the_same_answers_from_a_device_as_from_the_signer() {
    let samples = v1_samples();
    assert_eq!(samples.len(), 53);
    let golden = Golden::load();
    // Twice: on fresh devices, and on devices that hold the golden federation and Cashu state.
    for populated in [false, true] {
        let (bare, fedimint) = (Watched::default(), Watched::default());
        let cashu = if populated {
            Watched::holding(&populated_cashu_state())
        } else {
            Watched::default()
        };
        let mut signer = open_signer(golden.app_seed, &bare, NOW);
        let mut device = open_device(golden.app_seed, &fedimint, &cashu, NOW);
        if populated {
            let import = message(id(1), Request::ImportFederation(golden.descriptor.clone()));
            assert!(matches!(answer(&mut signer, &import), Answer::Approved(..)));
            assert!(matches!(answer(&mut device, &import), Answer::Approved(..)));
        }
        // Requests and responses alike: a response sent as a request is malformed for both.
        for (name, bytes) in &samples {
            assert_eq!(
                answer(&mut device, bytes),
                answer(&mut signer, bytes),
                "{name} (populated: {populated})"
            );
        }
        assert!(fedimint.writes() == bare.writes());
        assert_eq!(cashu.write_count(), 0);
    }
}

/// The request enum of protocol version 1 as an old Prime app has it: every variant up to `Cancel`
/// and no `Cashu`, with the nested types it shares with this version.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
enum V1Request {
    GetInfo,
    ImportFederation(FederationDescriptor),
    ExportViewMaterial {
        federation: FederationId,
    },
    DeriveOutgoingNonces {
        federation: FederationId,
        notes: Vec<NoteRef>,
    },
    SignTransaction(SignRequest),
    ReleaseNotes(ReleaseRequest),
    GetResult {
        request: RequestId,
    },
    GetStatus,
    Cancel {
        request: RequestId,
    },
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct V1Envelope {
    v: u16,
    id: RequestId,
    body: V1Request,
}

#[derive(Deserialize)]
struct V1Header {
    v: u16,
    id: RequestId,
    #[allow(dead_code)]
    body: IgnoredAny,
}

/// What an old Prime app answers, as its `decode_request` and `Signer::handle` do: the header
/// first, then the whole message, refused as malformed under the request id when it does not decode.
fn old_prime_answer(bytes: &[u8]) -> Result<RequestId, (RequestId, Rejection)> {
    let header: V1Header = ciborium::from_reader(bytes).expect("a readable header");
    assert_eq!(header.v, PROTOCOL_VERSION);
    match ciborium::from_reader::<V1Envelope, _>(bytes) {
        Ok(envelope) => Ok(envelope.id),
        Err(error) => Err((
            header.id,
            Rejection {
                code: RejectCode::Malformed,
                message: format!("invalid message body: {error}"),
            },
        )),
    }
}

#[test]
fn an_old_prime_refuses_a_cashu_request_as_malformed() {
    // The copy reads every v1 request, as the old app did.
    for (name, bytes) in v1_samples() {
        if name.starts_with("request.") {
            assert!(old_prime_answer(&bytes).is_ok(), "{name}");
        }
    }

    let fixture = Fixture::new();
    for (n, body) in [
        info(),
        add_mint(&fixture.descriptor),
        sign(common::mint::send(&fixture)),
        sign(common::mint::melt(&fixture)),
    ]
    .into_iter()
    .enumerate()
    {
        let request = id(n as u8 + 1);
        let bytes = message(request, body);
        let Err((answered, refusal)) = old_prime_answer(&bytes) else {
            panic!("an old Prime read a Cashu request");
        };
        assert_eq!(answered, request);
        assert_eq!(refusal.code, RejectCode::Malformed);
        assert!(
            refusal.message.contains("unknown variant `Cashu`"),
            "{}",
            refusal.message
        );
        // That answer is a v1 response, which the new phone reads as "no Cashu here".
        let reply = encode(&Envelope::new(
            answered,
            Response::Rejected(refusal.clone()),
        ));
        let read = decode_response(&reply).unwrap();
        assert_eq!((read.id, read.body), (request, Response::Rejected(refusal)));
    }
}

#[test]
fn a_signer_without_the_cashu_service_answers_unsupported() {
    let fixture = Fixture::new();
    let storage = Watched::default();
    let mut signer = open_signer(common::mint::APP_SEED, &storage, NOW);
    for (n, body) in [
        info(),
        add_mint(&fixture.descriptor),
        sign(common::mint::send(&fixture)),
    ]
    .into_iter()
    .enumerate()
    {
        let request = id(n as u8 + 1);
        let answer = reply(send_signer(&mut signer, request, body));
        assert_eq!(
            (answer.id, answer.body),
            (
                request,
                Response::Rejected(Rejection {
                    code: RejectCode::Unsupported,
                    message: "this device has no Cashu service".to_owned()
                })
            )
        );
        // Not stored, and nothing waits.
        assert_eq!(
            rejection(send_signer(
                &mut signer,
                id(10),
                Request::GetResult { request }
            ))
            .code,
            RejectCode::NotFound
        );
        assert!(signer.pending().is_none());
    }
    assert_eq!(storage.write_count(), 0);

    // The same requests on a device are Cashu's.
    let (mut device, _, _) = cashu_device();
    assert_eq!(cashu_info(&mut device, id(1)).scheme, "fps-cashu-v1");
    assert!(matches!(
        send(&mut device, id(2), add_mint(&fixture.descriptor)),
        Outcome::NeedsApproval(_)
    ));
}
