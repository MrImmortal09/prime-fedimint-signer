//! The NUT-11 message and its one-reading check: the spec's own vectors, and the dynamic program
//! against an independent brute force.

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{schnorr, Message, PublicKey, Secp256k1, SecretKey};
use fps_cashu_core::message::{
    check_one_reading, message_hash, output_readings, starts_with_output, SigAllMessage,
};
use fps_cashu_core::secret;
use fps_protocol::RejectCode;

fn point(text: &str) -> [u8; 33] {
    hex::decode(text).unwrap().try_into().unwrap()
}

/// Checks a BIP-340 signature over SHA-256 of the message, by the `data` key of the secret.
fn signed_by_data_key(secret_text: &str, message: &str, signature: &str) -> bool {
    let lock = secret::parse(secret_text).unwrap().lock_key;
    let signature = schnorr::Signature::from_slice(&hex::decode(signature).unwrap()).unwrap();
    Secp256k1::verification_only()
        .verify_schnorr(
            &signature,
            &Message::from_digest(message_hash(message)),
            &lock.x_only_public_key().0,
        )
        .is_ok()
}

#[test]
fn the_nut_11_swap_vector() {
    // tests/11-test.md: a SIG_ALL swap with one input and one output.
    let secret = r#"["P2PK",{"nonce":"c7f280eb55c1e8564e03db06973e94bc9b666d9e1ca42ad278408fe625950303","data":"030d8acedfe072c9fa449a1efe0817157403fbec460d8e79f957966056e5dd76c1","tags":[["sigflag","SIG_ALL"]]}]"#;
    let c = point("02c97ee3d1db41cf0a3ddb601724be8711a032950811bf326f8219c50c4808d3cd");
    let b = point("038ec853d65ae1b79b5cdbc2774150b2cb288d6d26e12958a16fb33c32d9a86c39");
    let message = SigAllMessage::build(&[(secret, &c)], &[(2, &b)], None);
    assert_eq!(
        message.as_str(),
        format!("{secret}02c97ee3d1db41cf0a3ddb601724be8711a032950811bf326f8219c50c4808d3cd2038ec853d65ae1b79b5cdbc2774150b2cb288d6d26e12958a16fb33c32d9a86c39")
    );
    assert_eq!(
        hex::encode(message.hash()),
        "de7f9e3ca0fcc5ed3258fcf83dbf1be7fa78a5ed6da7bf2aa60d61e9dc6eb09a"
    );
    assert_eq!(
        message.output_part(),
        "2038ec853d65ae1b79b5cdbc2774150b2cb288d6d26e12958a16fb33c32d9a86c39"
    );
    assert!(signed_by_data_key(
        secret,
        message.as_str(),
        "ce017ca25b1b97df2f72e4b49f69ac26a240ce14b3690a8fe619d41ccc42d3c1282e073f85acd36dc50011638906f35b56615f24e4d03e8effe8257f6a808538"
    ));
    check_one_reading(message.output_part()).unwrap();
}

#[test]
fn the_nut_11_melt_vector() {
    // tests/11-test.md: a SIG_ALL melt with one blank output and a quote id.
    let secret = r#"["P2PK",{"nonce":"bbf9edf441d17097e39f5095a3313ba24d3055ab8a32f758ff41c10d45c4f3de","data":"029116d32e7da635c8feeb9f1f4559eb3d9b42d400f9d22a64834d89cde0eb6835","tags":[["sigflag","SIG_ALL"]]}]"#;
    let c = point("02a9d461ff36448469dccf828fa143833ae71c689886ac51b62c8d61ddaa10028b");
    let b = point("038ec853d65ae1b79b5cdbc2774150b2cb288d6d26e12958a16fb33c32d9a86c39");
    let quote = "cF8911fzT88aEi1d-6boZZkq5lYxbUSVs-HbJxK0";
    let message = SigAllMessage::build(&[(secret, &c)], &[(0, &b)], Some(quote));
    assert_eq!(
        hex::encode(message.hash()),
        "9efa1067cc7dc870f4074f695115829c3cd817a6866c3b84e9814adf3c3cf262"
    );
    assert!(message.as_str().ends_with(quote));
    // The quote is not part of the outputs the check reads.
    assert_eq!(message.output_part(), format!("0{}", hex::encode(b)));
    assert!(signed_by_data_key(
        secret,
        message.as_str(),
        "478224fbe715e34f78cb33451db6fcf8ab948afb8bd04ff1a952c92e562ac0f7c1cb5e61809410635be0aa94d0448f7f7959bd5762cc3802b0a00ff58b2da747"
    ));
    assert_eq!(
        message.hash(),
        sha256::Hash::hash(message.as_str().as_bytes()).to_byte_array()
    );
}

/// Readings by plain recursion, capped at 2: an independent statement of the grammar, in which
/// `is_key` decides which windows of 66 or 130 characters are keys.
fn brute_readings(part: &[u8], is_key: &dyn Fn(&[u8]) -> bool) -> u8 {
    if part.is_empty() {
        return 1;
    }
    let mut count = 0u8;
    let digits = part.iter().take_while(|b| b.is_ascii_digit()).count();
    let amounts: Vec<usize> = match part.first() {
        Some(b'0') => vec![1],
        Some(b'1'..=b'9') => (1..=digits.min(20)).collect(),
        _ => vec![],
    };
    for amount in amounts {
        let rest = &part[amount..];
        for len in [66, 130] {
            if rest.len() >= len && is_key(&rest[..len]) {
                count = (count + brute_readings(&rest[len..], is_key)).min(2);
            }
        }
    }
    count
}

fn lower_hex(key: &[u8]) -> bool {
    key.iter()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

/// A key a mint takes: lowercase hex of a point, compressed or uncompressed.
fn point_key(key: &[u8]) -> bool {
    lower_hex(key) && hex::decode(key).is_ok_and(|bytes| PublicKey::from_slice(&bytes).is_ok())
}

/// A compressed point only, as CDK reads `B_`.
fn compressed_point_key(key: &[u8]) -> bool {
    key.len() == 66 && point_key(key)
}

/// A key by its form alone: `02` or `03` and 64 hex digits, or `04`, `06` or `07` and 128.
fn formed_key(key: &[u8]) -> bool {
    let prefixes: &[&[u8]] = match key.len() {
        66 => &[b"02", b"03"],
        130 => &[b"04", b"06", b"07"],
        _ => &[],
    };
    lower_hex(key) && prefixes.iter().any(|prefix| key.starts_with(prefix))
}

/// `k·G` for a small `k`.
fn point_of(k: u64) -> PublicKey {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&k.to_be_bytes());
    SecretKey::from_slice(&bytes)
        .unwrap()
        .public_key(&Secp256k1::new())
}

/// The compressed hex of the first point `k·G`, `k` from 1 up, that `accept` takes.
fn find_point(accept: impl Fn(&str) -> bool) -> String {
    let mut current = point_of(1);
    loop {
        let text = hex::encode(current.serialize());
        if accept(&text) {
            return text;
        }
        current = current.combine(&point_of(1)).unwrap();
    }
}

fn is_point_hex(text: &str) -> bool {
    point_key(text.as_bytes())
}

/// A small deterministic generator, so the test needs no RNG crate.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// Hex heavy in digits and in the characters that start keys, so readings interfere often.
    fn hex(&mut self, len: usize) -> String {
        const BIASED: &[u8] = b"0123456789abcdef0123456702";
        (0..len)
            .map(|_| char::from(BIASED[self.below(BIASED.len() as u64) as usize]))
            .collect()
    }

    /// A random point.
    fn point(&mut self) -> PublicKey {
        loop {
            let bytes: Vec<u8> = (0..4).flat_map(|_| self.next().to_be_bytes()).collect();
            if let Ok(secret) = SecretKey::from_slice(&bytes) {
                return secret.public_key(&Secp256k1::new());
            }
        }
    }
}

#[test]
fn the_dynamic_program_counts_what_plain_recursion_counts() {
    let mut rng = XorShift(0x5eed_cafe_f00d_1234);
    let amounts = [
        "0",
        "1",
        "2",
        "4",
        "8",
        "16",
        "1024",
        "2048",
        "102",
        "1021",
        "2199023255552",
    ];
    let mut seen = [0usize; 3];
    for _ in 0..6_000 {
        let pairs = 1 + rng.below(4) as usize;
        let mut part = String::new();
        for _ in 0..pairs {
            part.push_str(amounts[rng.below(amounts.len() as u64) as usize]);
            match rng.below(16) {
                // A real point, uncompressed or hybrid (`06` or `07` by the parity of y).
                0 => {
                    let mut full = rng.point().serialize_uncompressed();
                    if rng.below(2) == 0 {
                        full[0] = 0x06 | (full[64] & 1);
                    }
                    part.push_str(&hex::encode(full));
                }
                // A long key's form that is no point.
                1 => {
                    part.push_str(["04", "06", "07"][rng.below(3) as usize]);
                    part.push_str(&rng.hex(128));
                }
                // A real compressed point.
                2..=4 => part.push_str(&hex::encode(rng.point().serialize())),
                // A compressed key's form, a point about half the time.
                _ => {
                    part.push_str(["02", "03"][rng.below(2) as usize]);
                    part.push_str(&rng.hex(64));
                }
            }
        }
        // Sometimes cut or extend it, to cover strings with no reading.
        match rng.below(10) {
            0 => {
                part.pop();
            }
            1 => part.push('7'),
            _ => {}
        }
        let expected = brute_readings(part.as_bytes(), &point_key);
        assert_eq!(output_readings(&part), expected, "{part}");
        seen[usize::from(expected)] += 1;
    }
    // The generator reaches every outcome.
    assert!(seen.iter().all(|count| *count > 50), "{seen:?}");
}

#[test]
fn a_shifted_digit_gives_a_second_reading_when_its_key_is_a_point() {
    // 1,024 then a key ending in three digits: `1 | 024…` and `<digits>2 | …` read the same when
    // `024…` is a point too, as half of all keys of that form are.
    let shifts = |key: &str| key[63..].bytes().all(|b| b.is_ascii_digit()) && &key[63..64] != "0";
    let shifted = |key: &str| format!("024{}", &key[..63]);
    let key1 = find_point(|key| shifts(key) && is_point_hex(&shifted(key)));
    let key2 = hex::encode(point_of(2).serialize());
    let part = format!("1024{key1}2{key2}");
    assert_eq!(output_readings(&part), 2);
    assert_eq!(brute_readings(part.as_bytes(), &point_key), 2);
    let error = check_one_reading(&part).unwrap_err();
    assert_eq!(error.code, RejectCode::InvalidTransaction);

    // When `024…` is no point, no mint takes the second list, and the part reads one way, though
    // its form alone reads two.
    let key3 = find_point(|key| shifts(key) && !is_point_hex(&shifted(key)));
    let part = format!("1024{key3}2{key2}");
    assert_eq!(brute_readings(part.as_bytes(), &formed_key), 2);
    assert_eq!(output_readings(&part), 1);
    check_one_reading(&part).unwrap();

    // A key ending in a letter, or a leading zero in the shifted amount, leaves one reading.
    let letter = find_point(|key| key.ends_with(['a', 'b', 'c', 'd', 'e', 'f']));
    assert_eq!(output_readings(&format!("1024{letter}2{key2}")), 1);
    let zero = find_point(|key| {
        &key[63..64] == "0"
            && key[64..].bytes().all(|b| b.is_ascii_digit())
            && is_point_hex(&shifted(key))
    });
    assert_eq!(output_readings(&format!("1024{zero}2{key2}")), 1);

    // The empty list has one reading; nothing parseable has none, nor has a key that is no point.
    assert_eq!(output_readings(""), 1);
    assert_eq!(output_readings("zz"), 0);
    assert_eq!(output_readings(&format!("1{}", "f".repeat(66))), 0);
    assert_eq!(output_readings(&format!("102{}", "f".repeat(64))), 0);
    assert_eq!(
        check_one_reading("zz").unwrap_err().code,
        RejectCode::Internal
    );
}

#[test]
fn uncompressed_keys_count_as_readings() {
    // Nutshell signs the B_ string as posted, so a 130-character key that is a point is a reading
    // too. Build one, `04 X Y`, across three compressed keys:
    //
    //   1 | 02 04 X[..62] | X[62..] | Y t | 8 | K3      as compressed keys
    //   102 | 04 X Y | t8 | K3                          with the uncompressed one
    //
    // which needs `02 04 X[..62]` and `Y t` to be points, Y to begin `02` or `03`, and the two
    // digits ending X and t to be amounts.
    let mut current = point_of(7);
    let (x, y) = loop {
        let full = hex::encode(current.serialize_uncompressed());
        let (x, y) = (full[2..66].to_owned(), full[66..].to_owned());
        let amount = x[62..].bytes().all(|b| b.is_ascii_digit()) && &x[62..63] != "0";
        if (y.starts_with("02") || y.starts_with("03"))
            && amount
            && is_point_hex(&format!("0204{}", &x[..62]))
        {
            break (x, y);
        }
        current = current.combine(&point_of(1)).unwrap();
    };
    let t = (10..100)
        .map(|t| t.to_string())
        .find(|t| is_point_hex(&format!("{y}{t}")))
        .unwrap();
    let key3 = hex::encode(point_of(3).serialize());
    let part = format!("10204{}{}{y}{t}8{key3}", &x[..62], &x[62..]);
    assert_eq!(brute_readings(part.as_bytes(), &compressed_point_key), 1);
    assert_eq!(brute_readings(part.as_bytes(), &point_key), 2);
    assert_eq!(output_readings(&part), 2);
}

#[test]
fn quote_ids_that_begin_like_an_output_are_recognised() {
    let key = format!("02{}", "1f".repeat(32));
    assert!(starts_with_output(&format!("0{key}")));
    assert!(starts_with_output(&format!("1024{key}rest")));
    assert!(starts_with_output(&format!("704{}", "e".repeat(128))));
    for quote in [
        "",
        "0192f4c8-1d2b-7c5a-8e3f-2b7c9d0a1e5f",
        "cF8911fzT88aEi1d-6boZZkq5lYxbUSVs-HbJxK0",
        "1",
        &format!("01{key}"),
        &format!("0{}", &key[..65]),
    ] {
        assert!(!starts_with_output(quote), "{quote}");
    }
}

/// How often an honest transcript reads two ways, as `docs/PHASES.md` reports it: a random total
/// below 2^bits split into powers of two, each with a random `B_`, in increasing and in decreasing
/// order of amount. A measurement, not a check:
///
///   cargo test --release -p fps-cashu-core --test message -- --ignored --nocapture
#[test]
#[ignore]
fn honest_refusal_rates() {
    const TRIALS: u64 = 100_000;
    let mut rng = XorShift(0x7261_7465_7321);
    for bits in [10u32, 20, 30] {
        let mut refused = [0u64; 2];
        // Refused with a 1,024 output that is not the last: `1 | 024…` is a reading.
        let mut after_1024 = [0u64; 2];
        let mut syntactic = [0u64; 2];
        for _ in 0..TRIALS {
            let total = 1 + rng.below((1 << bits) - 1);
            let amounts: Vec<u64> = (0..64)
                .map(|bit| 1u64 << bit)
                .filter(|amount| total & amount != 0)
                .collect();
            let decreasing: Vec<u64> = amounts.iter().rev().copied().collect();
            for (order, amounts) in [amounts, decreasing].iter().enumerate() {
                let part: String = amounts
                    .iter()
                    .map(|amount| format!("{amount}{}", hex::encode(rng.point().serialize())))
                    .collect();
                if check_one_reading(&part).is_err() {
                    refused[order] += 1;
                    let at = amounts.iter().position(|amount| *amount == 1024);
                    if at.is_some_and(|at| at + 1 < amounts.len()) {
                        after_1024[order] += 1;
                    }
                }
                if brute_readings(part.as_bytes(), &formed_key) != 1 {
                    syntactic[order] += 1;
                }
            }
        }
        let percent = |count: u64| 100.0 * count as f64 / TRIALS as f64;
        println!(
            "totals below 2^{bits}: refused {:.1}% increasing, {:.1}% decreasing, {:.1}% and {:.1}% with a 1,024 output not last; by form alone {:.1}% and {:.1}%",
            percent(refused[0]),
            percent(refused[1]),
            percent(after_1024[0]),
            percent(after_1024[1]),
            percent(syntactic[0]),
            percent(syntactic[1]),
        );
    }
}
