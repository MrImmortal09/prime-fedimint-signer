//! Compatibility pins for wire protocol version 1, frozen against the Fedimint-only code (`main` at
//! `3455d17`) before the Cashu backend was added.
//!
//! `compat/v1-messages.json` maps every sample below to the lowercase hex of its CBOR envelope. The
//! samples cover every `Request` and `Response` variant, every `RejectCode`, every variant of the
//! nested enums, and both arms of every optional field. A diff in this file or in its fixture means
//! the Fedimint wire format changed, so a paired phone and an installed Prime would stop
//! understanding each other. Fix the code, never the fixture.

use std::collections::{BTreeMap, BTreeSet};

use ciborium::Value;
use fps_protocol::*;

const FIXTURE: &str = include_str!("compat/v1-messages.json");

/// How many samples the fixture holds.
const SAMPLE_COUNT: usize = 53;

/// The request id of every sample envelope.
const ID: RequestId = FixedBytes([
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
]);

/// Every variant of version 1, by enum, as its tag on the wire.
const V1_VARIANTS: [(&str, &[&str]); 8] = [
    (
        "Request",
        &[
            "GetInfo",
            "ImportFederation",
            "ExportViewMaterial",
            "DeriveOutgoingNonces",
            "SignTransaction",
            "ReleaseNotes",
            "GetResult",
            "GetStatus",
            "Cancel",
        ],
    ),
    (
        "Response",
        &[
            "Info",
            "FederationImported",
            "ViewMaterial",
            "OutgoingNonces",
            "Signed",
            "Released",
            "Status",
            "Pending",
            "Cancelled",
            "Rejected",
        ],
    ),
    (
        "Intent",
        &[
            "PayInvoice",
            "Withdraw",
            "PrepareSend",
            "Reissue",
            "Reclaim",
            "Refund",
        ],
    ),
    ("InputHint", &["Note", "Refund"]),
    ("Branch", &["Spend", "Outgoing"]),
    ("Delivery", &["ShowOnDevice", "ReturnToPhone"]),
    (
        "Network",
        &["Bitcoin", "Testnet", "Testnet4", "Signet", "Regtest"],
    ),
    (
        "RejectCode",
        &[
            "Malformed",
            "UnsupportedVersion",
            "UnknownFederation",
            "FederationIdMismatch",
            "InvalidDescriptor",
            "InvalidTransaction",
            "Unsupported",
            "ForeignInput",
            "InvalidNote",
            "UnexpectedOutput",
            "IntentMismatch",
            "InvoiceMismatch",
            "AlreadyReleased",
            "PolicyLimit",
            "UserRejected",
            "Busy",
            "NotFound",
            "TooLarge",
            "Internal",
        ],
    ),
];

const NETWORKS: [(&str, Network); 5] = [
    ("Bitcoin", Network::Bitcoin),
    ("Testnet", Network::Testnet),
    ("Testnet4", Network::Testnet4),
    ("Signet", Network::Signet),
    ("Regtest", Network::Regtest),
];

const REJECT_CODES: [(&str, RejectCode); 19] = [
    ("Malformed", RejectCode::Malformed),
    ("UnsupportedVersion", RejectCode::UnsupportedVersion),
    ("UnknownFederation", RejectCode::UnknownFederation),
    ("FederationIdMismatch", RejectCode::FederationIdMismatch),
    ("InvalidDescriptor", RejectCode::InvalidDescriptor),
    ("InvalidTransaction", RejectCode::InvalidTransaction),
    ("Unsupported", RejectCode::Unsupported),
    ("ForeignInput", RejectCode::ForeignInput),
    ("InvalidNote", RejectCode::InvalidNote),
    ("UnexpectedOutput", RejectCode::UnexpectedOutput),
    ("IntentMismatch", RejectCode::IntentMismatch),
    ("InvoiceMismatch", RejectCode::InvoiceMismatch),
    ("AlreadyReleased", RejectCode::AlreadyReleased),
    ("PolicyLimit", RejectCode::PolicyLimit),
    ("UserRejected", RejectCode::UserRejected),
    ("Busy", RejectCode::Busy),
    ("NotFound", RejectCode::NotFound),
    ("TooLarge", RejectCode::TooLarge),
    ("Internal", RejectCode::Internal),
];

enum Sample {
    Request(Request),
    Response(Response),
}

impl Sample {
    fn encode(&self) -> Vec<u8> {
        match self {
            Self::Request(body) => encode(&Envelope::new(ID, body.clone())),
            Self::Response(body) => encode(&Envelope::new(ID, body.clone())),
        }
    }
}

/// `N` bytes counting up from `first`, so every field has its own pattern.
fn bytes<const N: usize>(first: u8) -> FixedBytes<N> {
    FixedBytes(std::array::from_fn(|i| first.wrapping_add(i as u8)))
}

/// `len` bytes counting up from `first`.
fn buf(len: usize, first: u8) -> ByteBuf {
    (0..len)
        .map(|i| first.wrapping_add(i as u8))
        .collect::<Vec<_>>()
        .into()
}

fn module(instance: u16, kind: &str, major: u32, minor: u32) -> ModuleInfo {
    ModuleInfo {
        instance,
        kind: kind.to_owned(),
        major,
        minor,
    }
}

/// Every optional part present, with integers of every CBOR width.
fn full_descriptor() -> FederationDescriptor {
    FederationDescriptor {
        federation: bytes(0x10),
        api_endpoints: buf(24, 0x40),
        name: Some("Compat Fédération".to_owned()),
        modules: vec![
            module(0, "mintv2", 1, 0),
            module(1, "lnv2", 1, 0),
            module(2, "walletv2", 1, 0),
            module(3, "meta", 0, 1),
        ],
        mint: Some(MintParams {
            instance: 0,
            keys: vec![
                DenominationKey {
                    denomination: 0,
                    key: bytes(0x80),
                },
                DenominationKey {
                    denomination: 30,
                    key: bytes(0xe0),
                },
            ],
            fee_base_msat: 100,
            fee_ppm: 100,
            amount_unit: 0,
        }),
        lightning: Some(LightningParams {
            instance: 1,
            network: Network::Signet,
            fee_base_msat: 1_000,
            fee_ppm: 100,
        }),
        wallet: Some(WalletParams {
            instance: 2,
            network: Network::Signet,
            dust_limit_sat: 546,
            fee_base_msat: 100_000,
            fee_ppm: 0,
        }),
    }
}

/// Every optional part absent and every collection empty.
fn minimal_descriptor() -> FederationDescriptor {
    FederationDescriptor {
        federation: bytes(0x10),
        api_endpoints: ByteBuf::default(),
        name: None,
        modules: vec![],
        mint: None,
        lightning: None,
        wallet: None,
    }
}

fn descriptor_on(network: Network) -> FederationDescriptor {
    FederationDescriptor {
        federation: bytes(0x10),
        api_endpoints: buf(8, 0x40),
        name: None,
        modules: vec![module(1, "lnv2", 1, 0), module(2, "walletv2", 1, 0)],
        mint: None,
        lightning: Some(LightningParams {
            instance: 1,
            network,
            fee_base_msat: 0,
            fee_ppm: 0,
        }),
        wallet: Some(WalletParams {
            instance: 2,
            network,
            dust_limit_sat: 330,
            fee_base_msat: 0,
            fee_ppm: 0,
        }),
    }
}

fn note(tweak: u8, branch: Branch) -> InputHint {
    InputHint::Note {
        tweak: bytes(tweak),
        branch,
    }
}

fn sign(intent: Intent, inputs: Vec<InputHint>) -> Request {
    Request::SignTransaction(SignRequest {
        federation: bytes(0x10),
        transaction: buf(40, 0x90),
        inputs,
        intent,
    })
}

fn release_note(denomination: u8, tweak: u8) -> ReleaseNote {
    ReleaseNote {
        denomination,
        tweak: bytes(tweak),
        signature: bytes(0xa0),
    }
}

fn named_summary() -> FederationSummary {
    FederationSummary {
        federation: bytes(0x10),
        name: Some("Compat Fédération".to_owned()),
        guardians: 4,
        modules: vec![
            "mintv2".to_owned(),
            "lnv2".to_owned(),
            "walletv2".to_owned(),
            "meta".to_owned(),
        ],
    }
}

fn request(name: &str, body: Request) -> (String, Sample) {
    (format!("request.{name}"), Sample::Request(body))
}

fn response(name: &str, body: Response) -> (String, Sample) {
    (format!("response.{name}"), Sample::Response(body))
}

fn samples() -> Vec<(String, Sample)> {
    let mut samples = vec![
        request("GetInfo", Request::GetInfo),
        request(
            "ImportFederation.full",
            Request::ImportFederation(full_descriptor()),
        ),
        request(
            "ImportFederation.minimal",
            Request::ImportFederation(minimal_descriptor()),
        ),
        request(
            "ExportViewMaterial",
            Request::ExportViewMaterial {
                federation: bytes(0x10),
            },
        ),
        request(
            "DeriveOutgoingNonces",
            Request::DeriveOutgoingNonces {
                federation: bytes(0x10),
                notes: vec![
                    NoteRef {
                        denomination: 0,
                        tweak: bytes(0x60),
                    },
                    NoteRef {
                        denomination: 40,
                        tweak: bytes(0x70),
                    },
                ],
            },
        ),
        request(
            "SignTransaction.PayInvoice",
            sign(
                Intent::PayInvoice {
                    invoice: "lnbcrt500u1compat".to_owned(),
                },
                vec![note(0x60, Branch::Spend), note(0x70, Branch::Spend)],
            ),
        ),
        request(
            "SignTransaction.Withdraw",
            sign(Intent::Withdraw, vec![note(0x60, Branch::Spend)]),
        ),
        request(
            "SignTransaction.PrepareSend",
            sign(
                Intent::PrepareSend {
                    amount_msat: 1 << 32,
                },
                vec![note(0x60, Branch::Spend)],
            ),
        ),
        request(
            "SignTransaction.Reissue",
            sign(
                Intent::Reissue,
                vec![note(0x60, Branch::Spend), note(0x70, Branch::Spend)],
            ),
        ),
        request(
            "SignTransaction.Reclaim",
            sign(
                Intent::Reclaim,
                vec![note(0x60, Branch::Outgoing), note(0x70, Branch::Outgoing)],
            ),
        ),
        request(
            "SignTransaction.Refund",
            sign(
                Intent::Refund,
                vec![InputHint::Refund {
                    contract: buf(48, 0xb0),
                }],
            ),
        ),
        request(
            "ReleaseNotes.ShowOnDevice",
            Request::ReleaseNotes(ReleaseRequest {
                federation: bytes(0x10),
                notes: vec![release_note(23, 0x60), release_note(20, 0x70)],
                include_invite: true,
                delivery: Delivery::ShowOnDevice,
            }),
        ),
        request(
            "ReleaseNotes.ReturnToPhone",
            Request::ReleaseNotes(ReleaseRequest {
                federation: bytes(0x10),
                notes: vec![release_note(9, 0x60)],
                include_invite: false,
                delivery: Delivery::ReturnToPhone,
            }),
        ),
        request(
            "GetResult",
            Request::GetResult {
                request: bytes(0x50),
            },
        ),
        request("GetStatus", Request::GetStatus),
        request(
            "Cancel",
            Request::Cancel {
                request: bytes(0x50),
            },
        ),
        response(
            "Info.idle",
            Response::Info(DeviceInfo {
                protocol: 1,
                key_scheme: "fps-v1".to_owned(),
                federations: vec![
                    named_summary(),
                    FederationSummary {
                        federation: bytes(0x30),
                        name: None,
                        guardians: 1,
                        modules: vec![],
                    },
                ],
                busy: false,
            }),
        ),
        response(
            "Info.busy",
            Response::Info(DeviceInfo {
                protocol: 1,
                key_scheme: "fps-v1".to_owned(),
                federations: vec![],
                busy: true,
            }),
        ),
        response(
            "FederationImported",
            Response::FederationImported(named_summary()),
        ),
        response(
            "ViewMaterial",
            Response::ViewMaterial(ViewMaterial {
                federation: bytes(0x10),
                key_scheme: "fps-v1".to_owned(),
                spend_key: bytes(0x02),
                chain_code: bytes(0xc0),
                view_secret: bytes(0xd0),
            }),
        ),
        response(
            "OutgoingNonces",
            Response::OutgoingNonces {
                nonces: vec![bytes(0x02), bytes(0x03)],
            },
        ),
        response(
            "Signed.without_gateway_auth",
            Response::Signed(SignedTransaction {
                txid: bytes(0x20),
                signatures: vec![bytes(0xa0), bytes(0xe0)],
                gateway_auth: None,
            }),
        ),
        response(
            "Signed.with_gateway_auth",
            Response::Signed(SignedTransaction {
                txid: bytes(0x20),
                signatures: vec![bytes(0xa0)],
                gateway_auth: Some(bytes(0x30)),
            }),
        ),
        response(
            "Released.token",
            Response::Released(Released {
                amount_msat: 9_437_184,
                notes: 2,
                token: Some("fedimint1compat".to_owned()),
            }),
        ),
        response(
            "Released.no_token",
            Response::Released(Released {
                amount_msat: 98_304,
                notes: 2,
                token: None,
            }),
        ),
        response(
            "Status.pending",
            Response::Status(StatusInfo {
                released: vec![ReleasedNote {
                    federation: bytes(0x10),
                    note: NoteRef {
                        denomination: 23,
                        tweak: bytes(0x60),
                    },
                }],
                recent: vec![bytes(0x50), ID],
                outflow_today_msat: 5_000_000_000,
                pending: Some(bytes(0x50)),
            }),
        ),
        response(
            "Status.idle",
            Response::Status(StatusInfo {
                released: vec![],
                recent: vec![],
                outflow_today_msat: 0,
                pending: None,
            }),
        ),
        response("Pending", Response::Pending),
        response("Cancelled", Response::Cancelled),
    ];
    for (name, network) in NETWORKS {
        samples.push(request(
            &format!("ImportFederation.{name}"),
            Request::ImportFederation(descriptor_on(network)),
        ));
    }
    for (name, code) in REJECT_CODES {
        samples.push(response(
            &format!("Rejected.{name}"),
            Response::Rejected(Rejection {
                code,
                message: format!("refused: {name}"),
            }),
        ));
    }
    samples
}

fn fixture() -> BTreeMap<String, String> {
    serde_json::from_str(FIXTURE).expect("the fixture is a JSON object of strings")
}

fn frozen<'a>(fixture: &'a BTreeMap<String, String>, name: &str) -> &'a str {
    fixture
        .get(name)
        .unwrap_or_else(|| panic!("{name} is missing from the fixture"))
}

/// The variant name of an externally tagged enum value.
fn tag(value: &Value) -> String {
    match value {
        Value::Text(name) => name.clone(),
        Value::Map(entries) if entries.len() == 1 => {
            entries[0].0.as_text().expect("a variant name").to_owned()
        }
        other => panic!("not an enum value: {other:?}"),
    }
}

/// Collects the tag of every nested enum value, by enum. Version 1 gives each enum-typed field its
/// own name, so the field name says which enum it is.
fn collect_tags(value: &Value, found: &mut BTreeMap<&'static str, BTreeSet<String>>) {
    match value {
        Value::Map(entries) => {
            for (key, value) in entries {
                let field = key.as_text().unwrap_or_default();
                let kind = match field {
                    "intent" => Some("Intent"),
                    "branch" => Some("Branch"),
                    "delivery" => Some("Delivery"),
                    "network" => Some("Network"),
                    "code" => Some("RejectCode"),
                    _ => None,
                };
                if let Some(kind) = kind {
                    found.entry(kind).or_default().insert(tag(value));
                }
                if field == "inputs" {
                    for hint in value.as_array().expect("inputs is an array") {
                        found.entry("InputHint").or_default().insert(tag(hint));
                    }
                }
                collect_tags(value, found);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_tags(item, found);
            }
        }
        _ => {}
    }
}

#[test]
fn every_sample_encodes_to_its_frozen_bytes() {
    let fixture = fixture();
    let changed: Vec<String> = samples()
        .into_iter()
        .filter_map(|(name, sample)| {
            let actual = hex::encode(sample.encode());
            let expected = frozen(&fixture, &name);
            (actual != expected)
                .then(|| format!("{name}\n  frozen  {expected}\n  now     {actual}"))
        })
        .collect();
    assert!(
        changed.is_empty(),
        "the v1 wire format changed:\n{}",
        changed.join("\n")
    );
}

#[test]
fn frozen_bytes_decode_to_the_samples_and_encode_back_unchanged() {
    let fixture = fixture();
    for (name, sample) in samples() {
        let bytes = hex::decode(frozen(&fixture, &name)).expect("lowercase hex");
        let again = match sample {
            Sample::Request(body) => {
                let envelope =
                    decode_request(&bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
                assert_eq!(envelope, Envelope::new(ID, body), "{name}");
                encode(&envelope)
            }
            Sample::Response(body) => {
                let envelope =
                    decode_response(&bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
                assert_eq!(envelope, Envelope::new(ID, body), "{name}");
                encode(&envelope)
            }
        };
        assert!(again == bytes, "{name} does not encode back to its bytes");
    }
}

#[test]
fn the_fixture_holds_exactly_the_samples() {
    let names: Vec<String> = samples().into_iter().map(|(name, _)| name).collect();
    let expected: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    assert_eq!(expected.len(), names.len(), "two samples share a name");
    assert_eq!(
        expected.len(),
        SAMPLE_COUNT,
        "a sample was added or dropped"
    );

    let fixture = fixture();
    let held: BTreeSet<&str> = fixture.keys().map(String::as_str).collect();
    let missing: Vec<&&str> = expected.difference(&held).collect();
    let unexpected: Vec<&&str> = held.difference(&expected).collect();
    assert!(
        missing.is_empty() && unexpected.is_empty(),
        "missing from the fixture: {missing:?}; in the fixture without a sample: {unexpected:?}"
    );
}

#[test]
fn the_samples_use_every_v1_variant() {
    let mut found: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    for (_, sample) in samples() {
        let envelope: Value = ciborium::from_reader(sample.encode().as_slice()).unwrap();
        let body = envelope
            .as_map()
            .and_then(|fields| fields.iter().find(|(key, _)| key.as_text() == Some("body")))
            .map(|(_, body)| body)
            .expect("an envelope has a body");
        let kind = match sample {
            Sample::Request(_) => "Request",
            Sample::Response(_) => "Response",
        };
        found.entry(kind).or_default().insert(tag(body));
        collect_tags(body, &mut found);
    }
    let expected: BTreeMap<&str, BTreeSet<String>> = V1_VARIANTS
        .iter()
        .map(|(kind, variants)| (*kind, variants.iter().map(|v| v.to_string()).collect()))
        .collect();
    assert_eq!(found, expected);
}

#[test]
fn version_one_constants_are_unchanged() {
    assert_eq!(PROTOCOL_VERSION, 1);
    assert_eq!(KEY_SCHEME, "fps-v1");
    assert_eq!(MAX_MESSAGE_SIZE, 64 * 1024);
    assert_eq!(UR_TYPE_REQUEST, "fps-request");
    assert_eq!(UR_TYPE_RESPONSE, "fps-response");
}
