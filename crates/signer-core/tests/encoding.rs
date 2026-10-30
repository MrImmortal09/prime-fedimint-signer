//! The consensus encoding primitives: canonical integers, bounded lengths, exact variants.

use fps_signer_core::encoding::{Decode, DecodeError, Encode, Reader, Writer};
use fps_signer_core::fedimint::mintv2::MintOutput;
use fps_signer_core::fedimint::{DynItem, UnsignedTransaction};

fn decode_bigsize(bytes: &[u8]) -> Result<u64, DecodeError> {
    let mut reader = Reader::new(bytes);
    let value = reader.read_bigsize("value")?;
    reader.finish("value")?;
    Ok(value)
}

#[test]
fn bigsize_uses_the_shortest_form_at_every_boundary() {
    for (value, expected) in [
        (0u64, vec![0x00]),
        (0xfc, vec![0xfc]),
        (0xfd, vec![0xfd, 0x00, 0xfd]),
        (0xffff, vec![0xfd, 0xff, 0xff]),
        (0x1_0000, vec![0xfe, 0x00, 0x01, 0x00, 0x00]),
        (0xffff_ffff, vec![0xfe, 0xff, 0xff, 0xff, 0xff]),
        (0x1_0000_0000, vec![0xff, 0, 0, 0, 1, 0, 0, 0, 0]),
    ] {
        let mut writer = Writer::new();
        writer.write_bigsize(value);
        assert_eq!(writer.as_bytes(), expected.as_slice(), "{value:#x}");
        assert_eq!(decode_bigsize(&expected).unwrap(), value);
    }
}

#[test]
fn bigsize_refuses_longer_forms_than_needed() {
    for bytes in [
        vec![0xfd, 0x00, 0xfc],
        vec![0xfd, 0x00, 0x00],
        vec![0xfe, 0x00, 0x00, 0xff, 0xff],
        vec![0xff, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff],
    ] {
        assert_eq!(
            decode_bigsize(&bytes),
            Err(DecodeError::NonCanonicalBigSize),
            "{bytes:02x?}"
        );
    }
}

#[test]
fn truncated_input_is_reported_not_panicked_on() {
    for bytes in [
        vec![],
        vec![0xfd, 0x01],
        vec![0xfe, 0, 1],
        vec![0xff, 0, 0, 0, 1],
    ] {
        assert!(matches!(
            decode_bigsize(&bytes),
            Err(DecodeError::UnexpectedEnd { .. })
        ));
    }
}

#[test]
fn narrower_integers_check_their_range() {
    let mut writer = Writer::new();
    writer.write_bigsize(0x1_0000);
    let mut reader = Reader::new(writer.as_bytes());
    assert!(matches!(
        reader.read_u16("id"),
        Err(DecodeError::OutOfRange {
            value: 0x1_0000,
            ..
        })
    ));
}

#[test]
fn collection_lengths_are_bounded_before_allocating() {
    // A count above the limit.
    let mut reader = Reader::new(&[0x05, 1, 2, 3, 4, 5]);
    assert!(matches!(
        reader.read_len("items", 1, 4),
        Err(DecodeError::TooMany { count: 5, .. })
    ));
    // A count that the remaining bytes cannot possibly hold.
    let mut writer = Writer::new();
    writer.write_bigsize(1_000_000);
    let mut reader = Reader::new(writer.as_bytes());
    assert!(matches!(
        reader.read_len("items", 2, u64::MAX),
        Err(DecodeError::UnexpectedEnd { .. })
    ));
    // A byte vector longer than the input.
    let mut reader = Reader::new(&[0x10, 1, 2]);
    assert!(matches!(
        reader.read_byte_vec("bytes"),
        Err(DecodeError::UnexpectedEnd { .. })
    ));
}

#[test]
fn strings_must_be_utf8_and_bools_zero_or_one() {
    let mut reader = Reader::new(&[0x02, 0xc3, 0x28]);
    assert!(matches!(
        reader.read_string("name"),
        Err(DecodeError::Invalid { .. })
    ));
    let mut reader = Reader::new(&[0x02]);
    assert!(matches!(
        reader.read_bool("flag"),
        Err(DecodeError::OutOfRange { value: 2, .. })
    ));
    let mut writer = Writer::new();
    writer.write_string("Guardian ✓");
    let mut reader = Reader::new(writer.as_bytes());
    assert_eq!(reader.read_string("name").unwrap(), "Guardian ✓");
}

#[test]
fn variants_must_consume_exactly_their_bytes() {
    let output = MintOutput {
        denomination: 9,
        nonce: bls12_381::G1Affine::generator(),
        tweak: [4; 16],
    };
    let mut encoded = output.to_bytes();
    assert_eq!(
        MintOutput::decode_exact(&encoded, "output").unwrap(),
        output
    );

    // Grow the variant's length prefix by one and add a byte inside it.
    encoded[1] += 1;
    encoded.push(0);
    assert!(matches!(
        MintOutput::decode_exact(&encoded, "output"),
        Err(DecodeError::TrailingBytes { .. })
    ));
}

#[test]
fn transactions_round_trip_and_hash_their_own_bytes() {
    let tx = UnsignedTransaction {
        inputs: vec![DynItem {
            instance: 0,
            payload: vec![1, 2, 3],
        }],
        outputs: vec![
            DynItem {
                instance: 1,
                payload: vec![],
            },
            DynItem {
                instance: 300,
                payload: vec![9; 300],
            },
        ],
        nonce: [7; 8],
    };
    let bytes = tx.to_bytes();
    assert_eq!(UnsignedTransaction::decode_canonical(&bytes).unwrap(), tx);
    use bitcoin::hashes::{sha256, Hash};
    assert_eq!(tx.txid(), sha256::Hash::hash(&bytes).to_byte_array());

    let mut longer = bytes.clone();
    longer.push(0);
    assert!(UnsignedTransaction::decode_canonical(&longer).is_err());
}
