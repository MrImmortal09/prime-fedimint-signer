//! Lowercase hex, the only form Cashu messages and secrets use.

/// Appends the lowercase hex of `bytes` to `out`.
pub(crate) fn push(out: &mut String, bytes: &[u8]) {
    for byte in bytes {
        out.push(nibble(byte >> 4));
        out.push(nibble(byte & 0x0f));
    }
}

/// The lowercase hex digit of a value below 16.
fn nibble(value: u8) -> char {
    char::from(if value < 10 {
        b'0' + value
    } else {
        b'a' + (value - 10)
    })
}

/// Lowercase hex of `bytes`.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    push(&mut out, bytes);
    out
}

/// Decodes exactly `2 * N` lowercase hex characters. Upper case is refused: the secrets and
/// messages Prime checks are compared byte for byte.
pub(crate) fn decode_lower<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.as_bytes();
    if text.len() != 2 * N {
        return None;
    }
    let mut out = [0u8; N];
    for (slot, pair) in out.iter_mut().zip(text.chunks_exact(2)) {
        let [high, low] = pair else { return None };
        *slot = (digit(*high)? << 4) | digit(*low)?;
    }
    Some(out)
}

/// Whether `byte` is a lowercase hex digit.
pub(crate) fn is_lower(byte: u8) -> bool {
    digit(byte).is_some()
}

fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
