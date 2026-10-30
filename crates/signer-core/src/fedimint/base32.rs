//! fedimint's prefixed base32 (`fedimint-core::base32`): the lowercase RFC 4648 base32hex alphabet
//! with bits packed least significant first and no padding, behind a `fedimint` prefix.

/// `FEDIMINT_PREFIX`.
pub const FEDIMINT_PREFIX: &str = "fedimint";

const ALPHABET: &[u8; 32] = b"0123456789abcdefghijklmnopqrstuv";

pub fn encode(input: &[u8]) -> String {
    let mut output = String::with_capacity(input.len() * 8 / 5 + 1);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for byte in input {
        buffer |= u32::from(*byte) << bits;
        bits += 8;
        while bits >= 5 {
            output.push(ALPHABET[(buffer & 0b1_1111) as usize] as char);
            buffer >>= 5;
            bits -= 5;
        }
    }
    if bits > 0 {
        output.push(ALPHABET[(buffer & 0b1_1111) as usize] as char);
    }
    output
}

/// Decodes base32hex as fedimint does. Leftover bits after the last full byte are dropped.
pub fn decode(input: &str) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len() * 5 / 8 + 1);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for ch in input.bytes() {
        let value = ALPHABET.iter().position(|c| *c == ch)? as u32;
        buffer |= value << bits;
        bits += 5;
        while bits >= 8 {
            output.push((buffer & 0xff) as u8);
            buffer >>= 8;
            bits -= 8;
        }
    }
    Some(output)
}

/// `encode_prefixed_bytes(FEDIMINT_PREFIX, bytes)`.
pub fn encode_prefixed(bytes: &[u8]) -> String {
    format!("{FEDIMINT_PREFIX}{}", encode(bytes))
}

/// `decode_prefixed_bytes(FEDIMINT_PREFIX, s)`: case-insensitive, prefix required.
pub fn decode_prefixed(input: &str) -> Option<Vec<u8>> {
    let lower = input.to_ascii_lowercase();
    decode(lower.strip_prefix(FEDIMINT_PREFIX)?)
}
