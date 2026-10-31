//! `Request::Cashu` and `Response::Cashu` in envelopes: every Cashu body round trips, the new
//! variants are tagged by name inside the Fedimint enums, their bytes are pinned, decoding stays
//! strict, and openings and view keys stay out of Debug output.

mod common;

use ciborium::Value;
use common::mint::{self, Fixture};
use fps_protocol::*;

/// The request id of the pinned envelopes, as in `crates/protocol/tests/compat_v1.rs`.
const ID: RequestId = FixedBytes([
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
]);

fn requests(fixture: &Fixture) -> Vec<CashuRequest> {
    vec![
        CashuRequest::Info,
        CashuRequest::AddMint(fixture.descriptor.clone()),
        CashuRequest::Sign(mint::send(fixture)),
        CashuRequest::Sign(mint::melt(fixture)),
    ]
}

fn responses(fixture: &Fixture) -> Vec<CashuResponse> {
    vec![
        CashuResponse::Info(CashuInfo {
            scheme: CASHU_KEY_SCHEME.to_owned(),
            mints: vec![fixture.mint.summary()],
        }),
        CashuResponse::MintAdded(fixture.keys.lock_material()),
        CashuResponse::Signed(CashuSigned {
            message_hash: FixedBytes([0x11; 32]),
            signature: FixedBytes([0x22; 64]),
        }),
    ]
}

/// The envelope's body as CBOR.
fn body(bytes: &[u8]) -> Value {
    let envelope: Value = ciborium::from_reader(bytes).unwrap();
    envelope
        .as_map()
        .unwrap()
        .iter()
        .find(|(key, _)| key.as_text() == Some("body"))
        .map(|(_, body)| body.clone())
        .unwrap()
}

/// The single entry of a one-entry map: an externally tagged variant with data.
fn entry(value: &Value) -> (&str, &Value) {
    let map = value.as_map().expect("a variant with data");
    assert_eq!(map.len(), 1);
    (map[0].0.as_text().unwrap(), &map[0].1)
}

#[test]
fn every_cashu_body_round_trips_in_an_envelope() {
    let fixture = Fixture::new();
    for request in requests(&fixture) {
        let envelope = Envelope::new(ID, Request::Cashu(request));
        let bytes = encode(&envelope);
        let decoded = decode_request(&bytes).unwrap();
        assert_eq!(decoded, envelope);
        assert!(encode(&decoded) == bytes);
    }
    for response in responses(&fixture) {
        let envelope = Envelope::new(ID, Response::Cashu(response));
        let bytes = encode(&envelope);
        let decoded = decode_response(&bytes).unwrap();
        assert_eq!(decoded, envelope);
        assert!(encode(&decoded) == bytes);
    }
}

#[test]
fn the_cashu_variants_are_tagged_by_name() {
    let fixture = Fixture::new();
    let tags: Vec<String> = requests(&fixture)
        .into_iter()
        .map(|request| {
            let body = body(&encode(&Envelope::new(ID, Request::Cashu(request))));
            let (outer, inner) = entry(&body);
            assert_eq!(outer, "Cashu");
            match inner {
                Value::Text(unit) => unit.clone(),
                other => entry(other).0.to_owned(),
            }
        })
        .collect();
    assert_eq!(tags, ["Info", "AddMint", "Sign", "Sign"]);

    let tags: Vec<String> = responses(&fixture)
        .into_iter()
        .map(|response| {
            let body = body(&encode(&Envelope::new(ID, Response::Cashu(response))));
            let (outer, inner) = entry(&body);
            assert_eq!(outer, "Cashu");
            entry(inner).0.to_owned()
        })
        .collect();
    assert_eq!(tags, ["Info", "MintAdded", "Signed"]);
}

#[test]
fn the_cashu_envelopes_have_pinned_bytes() {
    // {"v": 1, "id": h'00112233…', "body": {"Cashu": "Info"}}
    assert_eq!(
        hex::encode(encode(&Envelope::new(
            ID,
            Request::Cashu(CashuRequest::Info)
        ))),
        concat!(
            "a3617601626964",
            "5000112233445566778899aabbccddeeff",
            "64626f6479",
            "a1",
            "654361736875",
            "64496e666f",
        )
    );
    // {"v": 1, "id": h'00112233…', "body": {"Cashu": {"Signed": {"message_hash": h'11…',
    // "signature": h'22…'}}}}
    let signed = Response::Cashu(CashuResponse::Signed(CashuSigned {
        message_hash: FixedBytes([0x11; 32]),
        signature: FixedBytes([0x22; 64]),
    }));
    assert_eq!(
        hex::encode(encode(&Envelope::new(ID, signed))),
        [
            "a3617601626964",
            "5000112233445566778899aabbccddeeff",
            "64626f6479a1654361736875a1665369676e6564a2",
            "6c6d6573736167655f68617368",
            "5820",
            &"11".repeat(32),
            "697369676e6174757265",
            "5840",
            &"22".repeat(64),
        ]
        .concat()
    );
}

#[test]
fn decoding_stays_strict() {
    // An unknown Cashu request, and a known one with a field too many, are malformed under the id.
    let unknown = Value::Map(vec![
        (Value::Text("v".into()), Value::Integer(1.into())),
        (Value::Text("id".into()), Value::Bytes(ID.0.to_vec())),
        (
            Value::Text("body".into()),
            Value::Map(vec![(
                Value::Text("Cashu".into()),
                Value::Text("Restore".into()),
            )]),
        ),
    ]);
    let mut bytes = Vec::new();
    ciborium::into_writer(&unknown, &mut bytes).unwrap();
    let error = decode_request(&bytes).unwrap_err();
    assert_eq!(error.request_id(), Some(ID));
    assert_eq!(error.rejection().code, RejectCode::Malformed);
    assert!(
        error.to_string().contains("unknown variant `Restore`"),
        "{error}"
    );

    let fixture = Fixture::new();
    let mut extra: Value = ciborium::from_reader(
        encode(&Envelope::new(
            ID,
            Request::Cashu(CashuRequest::Sign(mint::send(&fixture))),
        ))
        .as_slice(),
    )
    .unwrap();
    let Value::Map(fields) = &mut extra else {
        panic!("an envelope is a map");
    };
    let (_, body) = fields
        .iter_mut()
        .find(|(key, _)| key.as_text() == Some("body"))
        .unwrap();
    let Value::Map(cashu) = body else { panic!() };
    let Value::Map(sign) = &mut cashu[0].1 else {
        panic!()
    };
    let Value::Map(request) = &mut sign[0].1 else {
        panic!()
    };
    request.push((Value::Text("note".into()), Value::Integer(1.into())));
    let mut bytes = Vec::new();
    ciborium::into_writer(&extra, &mut bytes).unwrap();
    let error = decode_request(&bytes).unwrap_err();
    assert_eq!(error.request_id(), Some(ID));
    assert_eq!(error.rejection().code, RejectCode::Malformed);
    assert!(
        error.to_string().contains("unknown field `note`"),
        "{error}"
    );
}

#[test]
fn openings_and_view_keys_stay_out_of_debug_output() {
    let fixture = Fixture::new();
    let spend = mint::send(&fixture);
    let leaving = spend
        .outputs
        .iter()
        .find(|output| output.opening.hint.is_none())
        .unwrap();
    let shown = format!("{:?}", Request::Cashu(CashuRequest::Sign(spend.clone())));
    assert!(!shown.contains(&leaving.opening.secret));
    assert!(!shown.contains(&hex::encode(leaving.opening.r.0)));
    assert!(!shown.contains(&format!("{:?}", leaving.opening.r)));

    let material = fixture.keys.lock_material();
    let shown = format!(
        "{:?}",
        Response::Cashu(CashuResponse::MintAdded(material.clone()))
    );
    assert!(!shown.contains(&hex::encode(material.view_key.0)));
}
