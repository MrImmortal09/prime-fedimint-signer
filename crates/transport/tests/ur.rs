//! Animated QR framing: what the device shows must reassemble into the same bytes on the other
//! side, whatever order the camera catches the parts in, and nothing oversized gets through.

use fps_protocol::{
    decode_request, encode, Envelope, FixedBytes, Request, MAX_MESSAGE_SIZE, UR_TYPE_REQUEST,
    UR_TYPE_RESPONSE,
};
use fps_transport::ur::{
    encode as encode_parts, scanned_message, PartDecoder, UrError, MAX_PART_CHARS,
};

/// Deterministic bytes that do not compress into a pattern.
fn message(len: usize) -> Vec<u8> {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

fn reassemble<'a>(ur_type: &str, parts: impl IntoIterator<Item = &'a String>) -> Vec<u8> {
    let mut decoder = PartDecoder::new(ur_type);
    for part in parts {
        if let Some(message) = decoder.receive(part).expect("a valid part") {
            return message;
        }
    }
    panic!("the parts did not complete the message");
}

#[test]
fn a_short_message_is_one_upper_case_part() {
    let bytes = message(40);
    let parts = encode_parts(UR_TYPE_RESPONSE, &bytes);
    assert_eq!(parts.len(), 1);
    assert!(parts[0].starts_with("UR:FPS-RESPONSE/1-1/"));
    assert_eq!(parts[0], parts[0].to_ascii_uppercase());
    assert_eq!(reassemble(UR_TYPE_RESPONSE, &parts), bytes);
}

#[test]
fn a_long_message_reassembles_from_any_order() {
    let bytes = message(3_000);
    let parts = encode_parts(UR_TYPE_RESPONSE, &bytes);
    assert!(parts.len() > 10, "{} parts", parts.len());
    for part in &parts {
        assert!(part.len() <= MAX_PART_CHARS, "{} characters", part.len());
    }
    assert_eq!(reassemble(UR_TYPE_RESPONSE, &parts), bytes);
    assert_eq!(reassemble(UR_TYPE_RESPONSE, parts.iter().rev()), bytes);
    // A camera that joins mid-loop sees the tail first, and repeats.
    let mid = parts.len() / 2;
    let looped: Vec<&String> = parts[mid..].iter().chain(&parts[..]).collect();
    assert_eq!(reassemble(UR_TYPE_RESPONSE, looped), bytes);
}

#[test]
fn lower_case_parts_read_the_same() {
    let bytes = message(700);
    let lower: Vec<String> = encode_parts(UR_TYPE_REQUEST, &bytes)
        .iter()
        .map(|part| part.to_ascii_lowercase())
        .collect();
    assert_eq!(reassemble(UR_TYPE_REQUEST, &lower), bytes);
}

#[test]
fn a_protocol_envelope_survives_the_round_trip() {
    let envelope = Envelope::new(FixedBytes([9; 16]), Request::GetStatus);
    let bytes = encode(&envelope);
    let parts = encode_parts(UR_TYPE_REQUEST, &bytes);
    let decoded = decode_request(&reassemble(UR_TYPE_REQUEST, &parts)).unwrap();
    assert_eq!(decoded, envelope);
}

#[test]
fn an_empty_message_is_a_single_part_code() {
    let parts = encode_parts(UR_TYPE_RESPONSE, &[]);
    assert_eq!(parts.len(), 1);
    assert_eq!(reassemble(UR_TYPE_RESPONSE, &parts), Vec::<u8>::new());
}

#[test]
fn a_code_of_another_type_is_refused() {
    let parts = encode_parts(UR_TYPE_RESPONSE, &message(50));
    let mut decoder = PartDecoder::new(UR_TYPE_REQUEST);
    assert!(matches!(
        decoder.receive(&parts[0]),
        Err(UrError::WrongType { .. })
    ));
    assert!(matches!(
        decoder.receive("UR:CRYPTO-PSBT/1-1/LPADADCSAEAE"),
        Err(UrError::WrongType { .. })
    ));
}

#[test]
fn garbage_is_malformed() {
    let mut decoder = PartDecoder::new(UR_TYPE_RESPONSE);
    for text in [
        "",
        "hello",
        "ur:",
        "ur:fps-response",
        "ur:fps-response/1-2/notbytewords",
    ] {
        assert!(
            matches!(decoder.receive(text), Err(UrError::Malformed(_))),
            "{text:?}"
        );
    }
}

#[test]
fn parts_of_two_messages_do_not_mix() {
    let first = encode_parts(UR_TYPE_RESPONSE, &message(1_000));
    let second = encode_parts(UR_TYPE_RESPONSE, &message(1_200));
    let mut decoder = PartDecoder::new(UR_TYPE_RESPONSE);
    assert_eq!(decoder.receive(&first[0]).unwrap(), None);
    assert!(matches!(
        decoder.receive(&second[1]),
        Err(UrError::Inconsistent(_))
    ));
}

#[test]
fn an_oversized_message_is_refused_from_its_first_part() {
    let parts = encode_parts(UR_TYPE_RESPONSE, &message(MAX_MESSAGE_SIZE + 1));
    let mut decoder = PartDecoder::new(UR_TYPE_RESPONSE);
    assert_eq!(
        decoder.receive(&parts[0]),
        Err(UrError::TooLarge {
            size: MAX_MESSAGE_SIZE + 1
        })
    );
}

#[test]
fn the_largest_message_still_fits() {
    let bytes = message(MAX_MESSAGE_SIZE);
    let parts = encode_parts(UR_TYPE_REQUEST, &bytes);
    assert_eq!(reassemble(UR_TYPE_REQUEST, &parts), bytes);
}

#[test]
fn a_scanned_request_is_checked() {
    let bytes = message(64);
    assert_eq!(
        scanned_message(UR_TYPE_REQUEST, "fps-request", bytes.clone()),
        Ok(bytes.clone())
    );
    assert_eq!(
        scanned_message(UR_TYPE_REQUEST, "FPS-REQUEST", bytes.clone()),
        Ok(bytes.clone())
    );
    assert!(matches!(
        scanned_message(UR_TYPE_REQUEST, "crypto-psbt", bytes),
        Err(UrError::WrongType { .. })
    ));
    assert_eq!(
        scanned_message(
            UR_TYPE_REQUEST,
            "fps-request",
            vec![0; MAX_MESSAGE_SIZE + 1]
        ),
        Err(UrError::TooLarge {
            size: MAX_MESSAGE_SIZE + 1
        })
    );
}

#[test]
fn denser_parts_make_fewer_codes() {
    let bytes = message(5_000);
    let small = encode_parts(UR_TYPE_REQUEST, &bytes);
    let large = fps_transport::ur::encode_with(UR_TYPE_REQUEST, &bytes, 700);
    assert!(
        large.len() * 2 < small.len(),
        "{} parts at 700 characters, {} at {MAX_PART_CHARS}",
        large.len(),
        small.len()
    );
    for part in &large {
        assert!(part.len() <= 700, "{} characters", part.len());
    }
    assert_eq!(reassemble(UR_TYPE_REQUEST, &large), bytes);
    assert_eq!(reassemble(UR_TYPE_REQUEST, large.iter().rev()), bytes);
}

#[test]
fn response_recovery_frames_replace_a_missed_original_fragment() {
    let bytes = message(3_000);
    let originals = encode_parts(UR_TYPE_RESPONSE, &bytes);
    let recovery = fps_transport::ur::encode_redundant(UR_TYPE_RESPONSE, &bytes);
    assert_eq!(recovery.len(), originals.len() * 3);
    for missed in 0..originals.len() {
        let received: Vec<_> = recovery
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != missed)
            .map(|(_, part)| part)
            .collect();
        assert_eq!(
            reassemble(UR_TYPE_RESPONSE, received),
            bytes,
            "camera missed original fragment {missed}"
        );
    }
}
