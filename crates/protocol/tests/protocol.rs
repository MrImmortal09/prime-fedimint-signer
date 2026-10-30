//! The wire protocol: every message round trips, and decoding is strict.

use fps_protocol::*;

fn id(byte: u8) -> RequestId {
    FixedBytes([byte; 16])
}

fn descriptor() -> FederationDescriptor {
    FederationDescriptor {
        federation: FixedBytes([1; 32]),
        api_endpoints: vec![1, 0, 3, b'u', b'r', b'l', 1, b'n'].into(),
        name: Some("Name".to_owned()),
        modules: vec![ModuleInfo {
            instance: 0,
            kind: "mintv2".to_owned(),
            major: 1,
            minor: 0,
        }],
        mint: Some(MintParams {
            instance: 0,
            keys: vec![DenominationKey {
                denomination: 9,
                key: FixedBytes([2; 96]),
            }],
            fee_base_msat: 100,
            fee_ppm: 10,
            amount_unit: 0,
        }),
        lightning: Some(LightningParams {
            instance: 1,
            network: Network::Signet,
            fee_base_msat: 1_000,
            fee_ppm: 0,
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

fn requests() -> Vec<Request> {
    let note = NoteRef {
        denomination: 12,
        tweak: FixedBytes([3; 16]),
    };
    vec![
        Request::GetInfo,
        Request::ImportFederation(descriptor()),
        Request::ExportViewMaterial {
            federation: FixedBytes([1; 32]),
        },
        Request::DeriveOutgoingNonces {
            federation: FixedBytes([1; 32]),
            notes: vec![note, note],
        },
        Request::SignTransaction(SignRequest {
            federation: FixedBytes([1; 32]),
            transaction: vec![1, 2, 3].into(),
            inputs: vec![
                InputHint::Note {
                    tweak: FixedBytes([4; 16]),
                    branch: Branch::Spend,
                },
                InputHint::Refund {
                    contract: vec![5; 10].into(),
                },
            ],
            intent: Intent::PayInvoice {
                invoice: "lnbcrt1".to_owned(),
            },
        }),
        Request::SignTransaction(SignRequest {
            federation: FixedBytes([1; 32]),
            transaction: vec![].into(),
            inputs: vec![],
            intent: Intent::PrepareSend { amount_msat: 512 },
        }),
        Request::ReleaseNotes(ReleaseRequest {
            federation: FixedBytes([1; 32]),
            notes: vec![ReleaseNote {
                denomination: 9,
                tweak: FixedBytes([6; 16]),
                signature: FixedBytes([7; 48]),
            }],
            include_invite: true,
            delivery: Delivery::ShowOnDevice,
        }),
        Request::GetResult { request: id(9) },
        Request::GetStatus,
        Request::Cancel { request: id(9) },
    ]
}

fn responses() -> Vec<Response> {
    vec![
        Response::Info(DeviceInfo {
            protocol: PROTOCOL_VERSION,
            key_scheme: KEY_SCHEME.to_owned(),
            federations: vec![FederationSummary {
                federation: FixedBytes([1; 32]),
                name: None,
                guardians: 4,
                modules: vec!["mintv2".to_owned()],
            }],
            busy: false,
        }),
        Response::ViewMaterial(ViewMaterial {
            federation: FixedBytes([1; 32]),
            key_scheme: KEY_SCHEME.to_owned(),
            spend_key: FixedBytes([2; 33]),
            chain_code: FixedBytes([3; 32]),
            view_secret: FixedBytes([4; 32]),
        }),
        Response::OutgoingNonces {
            nonces: vec![FixedBytes([2; 33])],
        },
        Response::Signed(SignedTransaction {
            txid: FixedBytes([8; 32]),
            signatures: vec![FixedBytes([9; 64]), FixedBytes([10; 64])],
            gateway_auth: None,
        }),
        Response::Released(Released {
            amount_msat: 1024,
            notes: 1,
            token: Some("fedimint0123".to_owned()),
        }),
        Response::Status(StatusInfo {
            released: vec![ReleasedNote {
                federation: FixedBytes([1; 32]),
                note: NoteRef {
                    denomination: 9,
                    tweak: FixedBytes([1; 16]),
                },
            }],
            recent: vec![id(1)],
            outflow_today_msat: 5,
            pending: Some(id(2)),
        }),
        Response::Pending,
        Response::Cancelled,
        Response::Rejected(Rejection {
            code: RejectCode::ForeignInput,
            message: "input 0 is not a note of this device".to_owned(),
        }),
    ]
}

#[test]
fn every_request_round_trips() {
    for (n, body) in requests().into_iter().enumerate() {
        let envelope = Envelope::new(id(n as u8), body);
        let bytes = encode(&envelope);
        assert_eq!(decode_request(&bytes).unwrap(), envelope);
    }
}

#[test]
fn every_response_round_trips() {
    for (n, body) in responses().into_iter().enumerate() {
        let envelope = Envelope::new(id(n as u8), body);
        let bytes = encode(&envelope);
        assert_eq!(decode_response(&bytes).unwrap(), envelope);
    }
}

#[test]
fn byte_strings_stay_compact() {
    // A 1,000-byte transaction costs about 1,000 bytes on the wire, not two or three times that.
    let body = Request::SignTransaction(SignRequest {
        federation: FixedBytes([1; 32]),
        transaction: vec![0xab; 1_000].into(),
        inputs: vec![],
        intent: Intent::Reissue,
    });
    let size = encode(&Envelope::new(id(1), body)).len();
    assert!(size < 1_150, "{size} bytes");
}

#[test]
fn fixed_byte_strings_must_have_their_exact_length() {
    #[derive(serde::Serialize)]
    struct Loose {
        v: u16,
        id: ByteBuf,
        body: &'static str,
    }
    let mut bytes = Vec::new();
    ciborium::into_writer(
        &Loose {
            v: PROTOCOL_VERSION,
            id: vec![1; 15].into(),
            body: "GetInfo",
        },
        &mut bytes,
    )
    .unwrap();
    assert!(matches!(
        decode_request(&bytes),
        Err(DecodeError::Malformed { id: None, .. })
    ));
}

#[test]
fn unknown_fields_and_variants_are_refused_under_the_request_id() {
    #[derive(serde::Serialize)]
    struct Extra {
        v: u16,
        id: FixedBytes<16>,
        body: &'static str,
        extra: u8,
    }
    let mut bytes = Vec::new();
    ciborium::into_writer(
        &Extra {
            v: PROTOCOL_VERSION,
            id: id(7),
            body: "GetInfo",
            extra: 1,
        },
        &mut bytes,
    )
    .unwrap();
    let error = decode_request(&bytes).unwrap_err();
    assert_eq!(error.request_id(), Some(id(7)));
    assert_eq!(error.rejection().code, RejectCode::Malformed);

    let mut bytes = Vec::new();
    ciborium::into_writer(
        &Envelope {
            version: PROTOCOL_VERSION,
            id: id(8),
            body: "FormatDevice",
        },
        &mut bytes,
    )
    .unwrap();
    let error = decode_request(&bytes).unwrap_err();
    assert_eq!(error.request_id(), Some(id(8)));
}

#[test]
fn other_versions_are_reported_as_unsupported() {
    let mut envelope = Envelope::new(id(3), Request::GetInfo);
    envelope.version = 2;
    let error = decode_request(&encode(&envelope)).unwrap_err();
    assert_eq!(
        error,
        DecodeError::UnsupportedVersion {
            id: id(3),
            version: 2
        }
    );
    assert_eq!(error.rejection().code, RejectCode::UnsupportedVersion);
}

#[test]
fn garbage_oversized_and_trailing_input_is_refused() {
    assert!(matches!(
        decode_request(b"not cbor"),
        Err(DecodeError::Malformed { id: None, .. })
    ));
    let huge = vec![0u8; MAX_MESSAGE_SIZE + 1];
    assert_eq!(
        decode_request(&huge),
        Err(DecodeError::TooLarge {
            size: MAX_MESSAGE_SIZE + 1
        })
    );
    assert_eq!(
        DecodeError::TooLarge { size: 1 }.rejection().code,
        RejectCode::TooLarge
    );
    let mut bytes = encode(&Envelope::new(id(4), Request::GetStatus));
    bytes.push(0);
    assert!(matches!(
        decode_request(&bytes),
        Err(DecodeError::Malformed { id: Some(_), .. })
    ));
}

#[test]
fn secrets_never_reach_debug_output() {
    let material = ViewMaterial {
        federation: FixedBytes([1; 32]),
        key_scheme: KEY_SCHEME.to_owned(),
        spend_key: FixedBytes([2; 33]),
        chain_code: FixedBytes([0xcc; 32]),
        view_secret: FixedBytes([0xdd; 32]),
    };
    let text = format!("{material:?}");
    assert!(!text.contains("cccc") && !text.contains("dddd"), "{text}");

    let released = Released {
        amount_msat: 1,
        notes: 1,
        token: Some("fedimintsecret".to_owned()),
    };
    assert!(!format!("{released:?}").contains("fedimintsecret"));
}
