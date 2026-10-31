//! The NUT-11 `SIG_ALL` message, and the check that its outputs can be read only one way.
//!
//! For a swap the message is `secret_0 || C_0 || … || secret_n || C_n || amount_0 || B_0 || … ||
//! amount_m || B_m`, and a melt appends its quote id. Secrets are verbatim, `C` and `B_` are
//! compressed points in lowercase hex, amounts are decimal (`cashu` 0.18.3, `nut03.rs` and
//! `nut05.rs`). The signature is BIP-340 over the SHA-256 of the message's UTF-8.
//!
//! Nothing separates an amount from the `B_` before or after it, so one message can stand for two
//! output lists: a phone that grinds blinding factors can find a second list that signs the same,
//! even with power-of-two amounts, and post that one instead. Prime therefore refuses a message
//! unless its output part splits in exactly one way into (amount, key) pairs whose keys are points:
//! every mint parses each `B_` as a point before it signs, so no other list can be posted. The
//! input part needs no such check: every input of a `SIG_ALL` spend has a JSON secret, which ends at
//! its own closing bracket. Nor does the boundary with a melt's quote id: the mint issues quote ids
//! at random, so the phone cannot get one that reads as outputs, and Prime refuses a quote id that
//! begins like an output ([`starts_with_output`]), which could otherwise hide an extra blank.

use std::ops::Range;

use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::PublicKey;
use fps_protocol::RejectCode;
use fps_signer_core::error::{SignerError, SignerResult};

use crate::hex;

/// Most digits an amount in a reading may have: every `u64` has at most 20, and mints sign only
/// amounts their keysets have.
pub const MAX_AMOUNT_DIGITS: usize = 20;

/// A compressed key in hex: `02` or `03` and 64 more digits.
const COMPRESSED_HEX_LEN: usize = 66;
/// An uncompressed or hybrid key in hex: `04`, `06` or `07` and 128 more digits. CDK re-encodes
/// every `B_` compressed before it builds the message, but Nutshell hashes the `B_` string as the
/// wallet posted it, and libsecp256k1 parses these forms too, so a reading may use them. Such a
/// window is a point only if its last 64 digits are the y coordinate of its first 64, which no
/// phone can arrange by grinding blinding factors.
const FULL_HEX_LEN: usize = 130;

/// The message of one spend, with the position of its output part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigAllMessage {
    text: String,
    outputs: Range<usize>,
}

impl SigAllMessage {
    /// Builds the message from `(secret, C)` per input and `(amount, B_)` per output, in request
    /// order, and the quote id for a melt.
    pub fn build(
        inputs: &[(&str, &[u8; 33])],
        outputs: &[(u64, &[u8; 33])],
        quote: Option<&str>,
    ) -> Self {
        let mut text = String::new();
        for (secret, c) in inputs {
            text.push_str(secret);
            hex::push(&mut text, *c);
        }
        let start = text.len();
        for (amount, b) in outputs {
            text.push_str(&amount.to_string());
            hex::push(&mut text, *b);
        }
        let end = text.len();
        if let Some(quote) = quote {
            text.push_str(quote);
        }
        Self {
            text,
            outputs: start..end,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn into_string(self) -> String {
        self.text
    }

    /// `amount_0 || B_0 || … || amount_m || B_m`.
    pub fn output_part(&self) -> &str {
        self.text.get(self.outputs.clone()).unwrap_or_default()
    }

    /// SHA-256 of the message, which the signature covers.
    pub fn hash(&self) -> [u8; 32] {
        message_hash(&self.text)
    }
}

/// SHA-256 of a message's UTF-8.
pub fn message_hash(message: &str) -> [u8; 32] {
    sha256::Hash::hash(message.as_bytes()).to_byte_array()
}

/// In how many ways `part` splits into pairs of a canonical decimal amount (`0`, or up to
/// [`MAX_AMOUNT_DIGITS`] digits without a leading zero) followed by a key in lowercase hex that
/// parses as a point: `02` or `03` and 64 digits, or `04`, `06` or `07` and 128 digits. Counts up
/// to 2, which means two or more. The empty string has one reading, the empty list.
///
/// Only points count because a mint parses every `B_` as a point before it signs (CDK as it reads
/// the request, Nutshell before it signs), so a reading with any other key can never be posted.
/// A window is parsed only where the rest of `part` reads on from it, and at most once.
pub fn output_readings(part: &str) -> u8 {
    let mut points = Points::new(part);
    readings(part.as_bytes(), &mut |start, end| {
        points.is_point(start, end)
    })
}

/// The readings of `part` in which `is_key(start, end)` accepts every key, capped at 2.
fn readings(part: &[u8], is_key: &mut dyn FnMut(usize, usize) -> bool) -> u8 {
    let len = part.len();
    // hex_run[i] and digit_run[i]: how many lowercase hex digits, and decimal digits, start at i.
    let mut hex_run = vec![0usize; len + 1];
    let mut digit_run = vec![0usize; len + 1];
    // ways[i]: readings of part[i..], capped at 2.
    let mut ways = vec![0u8; len + 1];
    if let Some(last) = ways.last_mut() {
        *last = 1;
    }
    for i in (0..len).rev() {
        let byte = part.get(i).copied().unwrap_or_default();
        let next_hex = hex_run.get(i + 1).copied().unwrap_or_default();
        let next_digit = digit_run.get(i + 1).copied().unwrap_or_default();
        if let Some(run) = hex_run.get_mut(i) {
            *run = if hex::is_lower(byte) { next_hex + 1 } else { 0 };
        }
        if let Some(run) = digit_run.get_mut(i) {
            *run = if byte.is_ascii_digit() {
                next_digit + 1
            } else {
                0
            };
        }
        let digits = match byte {
            b'0' => 1,
            b'1'..=b'9' => digit_run
                .get(i)
                .copied()
                .unwrap_or_default()
                .min(MAX_AMOUNT_DIGITS),
            _ => 0,
        };
        let mut count = 0u8;
        for amount_end in (i + 1)..=(i + digits) {
            for key_end in key_ends(part, &hex_run, amount_end) {
                let rest = ways.get(key_end).copied().unwrap_or_default();
                if rest > 0 && is_key(amount_end, key_end) {
                    count = count.saturating_add(rest).min(2);
                }
            }
        }
        if let Some(slot) = ways.get_mut(i) {
            *slot = count;
        }
    }
    ways.first().copied().unwrap_or_default()
}

/// Which key windows of a string are points, each parsed once. Where a window starts fixes its
/// length, because its prefix does (`02` or `03`, or `04`, `06` or `07`).
struct Points<'a> {
    text: &'a str,
    /// Per start: whether the window there is a point, once parsed.
    known: Vec<Option<bool>>,
}

impl<'a> Points<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            known: vec![None; text.len()],
        }
    }

    /// Whether `text[start..end]`, a window [`key_ends`] found, parses as a point.
    fn is_point(&mut self, start: usize, end: usize) -> bool {
        let window = self.text.get(start..end).unwrap_or_default();
        let Some(slot) = self.known.get_mut(start) else {
            return false;
        };
        *slot.get_or_insert_with(|| parses_as_point(window))
    }
}

/// Whether a key in lowercase hex, compressed or uncompressed, is a point, as libsecp256k1 (which
/// CDK and Nutshell both use) parses it.
fn parses_as_point(hex: &str) -> bool {
    let parsed = match hex.len() {
        COMPRESSED_HEX_LEN => hex::decode_lower::<33>(hex).map(|key| PublicKey::from_slice(&key)),
        FULL_HEX_LEN => hex::decode_lower::<65>(hex).map(|key| PublicKey::from_slice(&key)),
        _ => None,
    };
    parsed.is_some_and(|point| point.is_ok())
}

/// Where a key starting at `start` can end, from its prefix and the hex after it alone: after a
/// compressed key, or after an uncompressed one.
fn key_ends(part: &[u8], hex_run: &[usize], start: usize) -> impl Iterator<Item = usize> {
    let run = hex_run.get(start).copied().unwrap_or_default();
    let prefix = (part.get(start).copied(), part.get(start + 1).copied());
    let compressed = matches!(prefix, (Some(b'0'), Some(b'2' | b'3'))) && run >= COMPRESSED_HEX_LEN;
    let full = matches!(prefix, (Some(b'0'), Some(b'4' | b'6' | b'7'))) && run >= FULL_HEX_LEN;
    [
        compressed.then_some(start + COMPRESSED_HEX_LEN),
        full.then_some(start + FULL_HEX_LEN),
    ]
    .into_iter()
    .flatten()
}

/// Refuses an output part with more than one reading. `part` is the output part of a message Prime
/// built, whose own keys are points. An honest phone that meets this derives the outputs again at
/// new counters.
pub fn check_one_reading(part: &str) -> SignerResult<()> {
    // A part that reads one way before any key is parsed reads one way after: that way is Prime's
    // own, whose keys are points. Only a part that could read two ways needs its windows parsed.
    let count = match readings(part.as_bytes(), &mut |_, _| true) {
        1 => 1,
        _ => output_readings(part),
    };
    match count {
        1 => Ok(()),
        0 => Err(SignerError::new(
            RejectCode::Internal,
            "the outputs of the message do not parse",
        )),
        _ => Err(SignerError::new(
            RejectCode::InvalidTransaction,
            "the outputs can be read in more than one way, so the signature would also cover another output list; derive them again at new counters",
        )),
    }
}

/// Whether `text` begins with an (amount, key) pair, keys judged by their form alone. A quote id
/// that does could hide an extra output in a melt's message. Mint quote ids are UUIDs or about 40
/// random characters, which never hold 66 hex digits in a row, so no point needs parsing here.
pub fn starts_with_output(text: &str) -> bool {
    let text = text.as_bytes();
    let mut hex_run = vec![0usize; text.len() + 1];
    for i in (0..text.len()).rev() {
        let next = hex_run.get(i + 1).copied().unwrap_or_default();
        let byte = text.get(i).copied().unwrap_or_default();
        if let Some(run) = hex_run.get_mut(i) {
            *run = if hex::is_lower(byte) { next + 1 } else { 0 };
        }
    }
    let digits = match text.first() {
        Some(b'0') => 1,
        Some(b'1'..=b'9') => text
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count()
            .min(MAX_AMOUNT_DIGITS),
        _ => 0,
    };
    (1..=digits).any(|amount_end| key_ends(text, &hex_run, amount_end).next().is_some())
}
