//! The Cashu wire types of fps-protocol (`crates/protocol/src/cashu.rs`): every request and
//! response body round trips through CBOR, decoding is as strict as for the Fedimint messages, and
//! secrets stay out of Debug output. They are tested here so that fps-protocol, which the Fedimint
//! signer and wallet share, gains nothing but the module itself.
//!
//! `Request::Cashu` and `Response::Cashu` come with the next change, so these bodies are encoded
//! on their own here.

use ciborium::Value;
use fps_protocol::*;

fn to_cbor<T: serde::Serialize>(value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(value, &mut out).unwrap();
    out
}

fn from_cbor<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    let mut reader = bytes;
    let value = ciborium::from_reader(&mut reader).map_err(|error| error.to_string())?;
    if !reader.is_empty() {
        return Err("trailing bytes".to_owned());
    }
    Ok(value)
}

fn round_trip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let bytes = to_cbor(value);
    assert_eq!(&from_cbor::<T>(&bytes).unwrap(), value);
}

fn keyset(id: Vec<u8>, final_expiry: Option<u64>) -> CashuKeyset {
    CashuKeyset {
        id: id.into(),
        unit: "sat".to_owned(),
        input_fee_ppk: 100,
        final_expiry,
        active: true,
        keys: vec![
            CashuKey {
                amount: 1,
                pubkey: FixedBytes([2; 33]),
            },
            CashuKey {
                amount: 1 << 63,
                pubkey: FixedBytes([3; 33]),
            },
        ],
    }
}

fn descriptor() -> CashuMintDescriptor {
    CashuMintDescriptor {
        url: "https://mint.example".to_owned(),
        unit: "sat".to_owned(),
        keysets: vec![
            keyset(vec![0; 8], None),
            keyset(vec![1; 33], Some(1_900_000_000)),
        ],
    }
}

fn input(dleq: Option<CashuDleq>) -> CashuInput {
    CashuInput {
        amount: 8,
        keyset: vec![1; 33].into(),
        secret: r#"["P2PK",{"nonce":"00","data":"02","tags":[["sigflag","SIG_ALL"]]}]"#.to_owned(),
        c: FixedBytes([2; 33]),
        dleq,
    }
}

fn output(amount: u64, hint: Option<CashuOutputHint>) -> CashuOutput {
    CashuOutput {
        amount,
        keyset: vec![0; 8].into(),
        b: FixedBytes([3; 33]),
        opening: CashuOpening {
            secret: "secret".to_owned(),
            r: FixedBytes([4; 32]),
            hint,
        },
    }
}

fn requests() -> Vec<CashuRequest> {
    let dleq = CashuDleq {
        e: FixedBytes([5; 32]),
        s: FixedBytes([6; 32]),
        r: FixedBytes([7; 32]),
    };
    let hint = CashuOutputHint {
        epoch: u32::MAX,
        keyset: vec![0; 8].into(),
        counter: u64::MAX,
    };
    vec![
        CashuRequest::Info,
        CashuRequest::AddMint(descriptor()),
        CashuRequest::Sign(CashuSignRequest {
            mint: FixedBytes([1; 32]),
            epoch: 3,
            inputs: vec![input(None), input(Some(dleq.clone()))],
            outputs: vec![output(8, Some(hint)), output(4, None)],
            kind: CashuSpendKind::Swap,
        }),
        CashuRequest::Sign(CashuSignRequest {
            mint: FixedBytes([1; 32]),
            epoch: 0,
            inputs: vec![input(Some(dleq))],
            outputs: vec![],
            kind: CashuSpendKind::Melt {
                quote: "0192f4c8-1d2b-7c5a-8e3f-2b7c9d0a1e5f".to_owned(),
                invoice: "lnbc1".to_owned(),
                amount: 101,
                fee_reserve: 2,
            },
        }),
    ]
}

fn responses() -> Vec<CashuResponse> {
    vec![
        CashuResponse::Info(CashuInfo {
            scheme: CASHU_KEY_SCHEME.to_owned(),
            mints: vec![CashuMintSummary {
                mint: FixedBytes([1; 32]),
                url: "https://mint.example".to_owned(),
                unit: "sat".to_owned(),
                keysets: 2,
            }],
        }),
        CashuResponse::MintAdded(CashuLockMaterial {
            mint: FixedBytes([1; 32]),
            scheme: CASHU_KEY_SCHEME.to_owned(),
            lock_pubkey: FixedBytes([2; 33]),
            view_key: FixedBytes([3; 32]),
        }),
        CashuResponse::Signed(CashuSigned {
            message_hash: FixedBytes([4; 32]),
            signature: FixedBytes([5; 64]),
        }),
    ]
}

#[test]
fn the_scheme_name_is_its_own() {
    assert_eq!(CASHU_KEY_SCHEME, "fps-cashu-v1");
    assert_ne!(CASHU_KEY_SCHEME, KEY_SCHEME);
}

#[test]
fn every_cashu_request_and_response_round_trips() {
    for request in requests() {
        round_trip(&request);
    }
    for response in responses() {
        round_trip(&response);
    }
}

#[test]
fn variants_and_fields_are_tagged_by_name() {
    // As for the Fedimint messages: a unit variant is its name, a data variant a one-entry map.
    let info: Value = ciborium::from_reader(to_cbor(&CashuRequest::Info).as_slice()).unwrap();
    assert_eq!(info, Value::Text("Info".to_owned()));
    let swap: Value = ciborium::from_reader(to_cbor(&CashuSpendKind::Swap).as_slice()).unwrap();
    assert_eq!(swap, Value::Text("Swap".to_owned()));
    let sign: Value = ciborium::from_reader(to_cbor(&requests()[2]).as_slice()).unwrap();
    let map = sign.as_map().unwrap();
    assert_eq!(map.len(), 1);
    assert_eq!(map[0].0, Value::Text("Sign".to_owned()));
    let fields: Vec<String> = map[0]
        .1
        .as_map()
        .unwrap()
        .iter()
        .map(|(key, _)| key.as_text().unwrap().to_owned())
        .collect();
    assert_eq!(fields, ["mint", "epoch", "inputs", "outputs", "kind"]);
}

#[test]
fn byte_strings_stay_compact_and_keep_their_lengths() {
    // A 64-input, 64-output swap fits the 64 KiB message limit with room to spare.
    let request = CashuRequest::Sign(CashuSignRequest {
        mint: FixedBytes([1; 32]),
        epoch: 0,
        inputs: vec![
            CashuInput {
                secret: "x".repeat(192),
                ..input(None)
            };
            64
        ],
        outputs: vec![
            CashuOutput {
                opening: CashuOpening {
                    secret: "x".repeat(192),
                    r: FixedBytes([4; 32]),
                    hint: Some(CashuOutputHint {
                        epoch: 0,
                        keyset: vec![1; 33].into(),
                        counter: 1_000_000,
                    }),
                },
                ..output(1 << 20, None)
            };
            64
        ],
        kind: CashuSpendKind::Swap,
    });
    let size = to_cbor(&request).len();
    assert!(size < 48 * 1024, "{size} bytes");

    // A 33-byte key where 32 belong is refused, like every fixed-length byte string.
    #[derive(serde::Serialize)]
    struct Loose {
        message_hash: ByteBuf,
        signature: FixedBytes<64>,
    }
    let bytes = to_cbor(&Loose {
        message_hash: vec![1; 33].into(),
        signature: FixedBytes([2; 64]),
    });
    assert!(from_cbor::<CashuSigned>(&bytes).is_err());
}

#[test]
fn unknown_fields_and_variants_are_refused() {
    #[derive(serde::Serialize)]
    struct Extra {
        message_hash: FixedBytes<32>,
        signature: FixedBytes<64>,
        extra: u8,
    }
    let bytes = to_cbor(&Extra {
        message_hash: FixedBytes([1; 32]),
        signature: FixedBytes([2; 64]),
        extra: 1,
    });
    assert!(from_cbor::<CashuSigned>(&bytes).is_err());
    assert!(from_cbor::<CashuRequest>(&to_cbor(&"Mint")).is_err());
    assert!(from_cbor::<CashuSpendKind>(&to_cbor(&"Burn")).is_err());

    // A melt missing a field, or carrying one more, does not decode.
    type Fields = Vec<(Value, Value)>;
    let melt_with = |edit: &dyn Fn(&mut Fields)| -> Vec<u8> {
        let mut melt: Value = ciborium::from_reader(to_cbor(&requests()[3]).as_slice()).unwrap();
        let sign = &mut melt.as_map_mut().unwrap()[0].1;
        let kind = sign
            .as_map_mut()
            .unwrap()
            .iter_mut()
            .find(|(key, _)| key.as_text() == Some("kind"))
            .unwrap();
        edit(kind.1.as_map_mut().unwrap()[0].1.as_map_mut().unwrap());
        to_cbor(&melt)
    };
    assert!(from_cbor::<CashuRequest>(&melt_with(&|_| {})).is_ok());
    let extra = melt_with(&|fields| {
        fields.push((Value::Text("prefer_async".to_owned()), Value::Bool(true)))
    });
    assert!(from_cbor::<CashuRequest>(&extra).is_err());
    assert!(from_cbor::<CashuRequest>(&melt_with(&|fields| fields.truncate(3))).is_err());
}

/// Every copy of `value` with one entry added to one of its maps, at any depth.
fn with_extra_field(value: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    match value {
        Value::Map(entries) => {
            let mut extended = entries.clone();
            extended.push((Value::Text("zz".to_owned()), Value::Bool(true)));
            out.push(Value::Map(extended));
            for (index, (_, child)) in entries.iter().enumerate() {
                for variant in with_extra_field(child) {
                    let mut copy = entries.clone();
                    copy[index].1 = variant;
                    out.push(Value::Map(copy));
                }
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                for variant in with_extra_field(child) {
                    let mut copy = items.clone();
                    copy[index] = variant;
                    out.push(Value::Array(copy));
                }
            }
        }
        _ => {}
    }
    out
}

#[test]
fn every_body_refuses_an_unknown_field_at_any_depth() {
    // Each map of every request and response, from the envelope of a variant down to a hint, a
    // DLEQ proof or a key, with one field it does not know.
    let mut checked = 0;
    for request in requests() {
        let value: Value = ciborium::from_reader(to_cbor(&request).as_slice()).unwrap();
        for variant in with_extra_field(&value) {
            assert!(
                from_cbor::<CashuRequest>(&to_cbor(&variant)).is_err(),
                "{variant:?}"
            );
            checked += 1;
        }
    }
    for response in responses() {
        let value: Value = ciborium::from_reader(to_cbor(&response).as_slice()).unwrap();
        for variant in with_extra_field(&value) {
            assert!(
                from_cbor::<CashuResponse>(&to_cbor(&variant)).is_err(),
                "{variant:?}"
            );
            checked += 1;
        }
    }
    assert!(checked > 30, "{checked}");
}

#[test]
fn optional_fields_may_be_absent() {
    // `final_expiry`, `dleq` and `hint` decode as absent when the phone leaves them out.
    #[derive(serde::Serialize)]
    struct ShortOpening {
        secret: String,
        r: FixedBytes<32>,
    }
    let bytes = to_cbor(&ShortOpening {
        secret: "s".to_owned(),
        r: FixedBytes([1; 32]),
    });
    let opening: CashuOpening = from_cbor(&bytes).unwrap();
    assert_eq!(opening.hint, None);
}

#[test]
fn secrets_never_reach_debug_output() {
    let material = CashuLockMaterial {
        mint: FixedBytes([1; 32]),
        scheme: CASHU_KEY_SCHEME.to_owned(),
        lock_pubkey: FixedBytes([2; 33]),
        view_key: FixedBytes([0xdd; 32]),
    };
    let text = format!("{material:?}");
    assert!(!text.contains("dddd"), "{text}");
    assert!(text.contains("0202"), "{text}");

    // The opening of a leaving output is the recipient's ecash once the mint signs it.
    let opening = CashuOpening {
        secret: "recipient-secret".to_owned(),
        r: FixedBytes([0xcc; 32]),
        hint: None,
    };
    let text = format!("{:?}", output(1, None).opening) + &format!("{opening:?}");
    assert!(
        !text.contains("secret") && !text.contains("cccc") && !text.contains("0404"),
        "{text}"
    );
    let request = format!("{:?}", requests()[2]);
    assert!(!request.contains("0404040404"), "{request}");
}
