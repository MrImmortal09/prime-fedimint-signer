//! The one secret the wallet's proofs carry: a NUT-11 P2PK lock to a single key with `SIG_ALL`,
//! in exactly the form CDK writes it.
//!
//! ```text
//! ["P2PK",{"nonce":"<64 lowercase hex>","data":"<ser_P(P_e) as 66 lowercase hex>","tags":[["sigflag","SIG_ALL"]]}]
//! ```
//!
//! 192 characters. No `locktime` (an expired lock without a `refund` tag is spendable by anyone),
//! no other keys. Prime checks the template byte for byte and parses nothing else, and secrets are
//! never re-serialised: the mint signed `hash_to_curve` of these exact bytes.

use bitcoin::secp256k1::PublicKey;

use crate::hex;

/// Length of every template secret, in bytes.
pub const TEMPLATE_LEN: usize = 192;

const PREFIX: &str = r#"["P2PK",{"nonce":""#;
const INFIX: &str = r#"","data":""#;
const SUFFIX: &str = r#"","tags":[["sigflag","SIG_ALL"]]}]"#;

/// What a template secret holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateSecret {
    pub nonce: [u8; 32],
    /// The `data` key every signature must come from.
    pub lock_key: PublicKey,
}

/// The template secret with this nonce, locked to `lock_key`.
pub fn build(nonce: &[u8; 32], lock_key: &PublicKey) -> String {
    let mut out = String::with_capacity(TEMPLATE_LEN);
    out.push_str(PREFIX);
    hex::push(&mut out, nonce);
    out.push_str(INFIX);
    hex::push(&mut out, &lock_key.serialize());
    out.push_str(SUFFIX);
    out
}

/// Reads a template secret: exactly the prefix, 64 lowercase hex characters, the infix, a
/// compressed point as 66 lowercase hex characters, and the suffix. Anything else, including
/// other JSON spellings of the same lock, is `None`.
pub fn parse(secret: &str) -> Option<TemplateSecret> {
    if secret.len() != TEMPLATE_LEN {
        return None;
    }
    let rest = secret.strip_prefix(PREFIX)?;
    let (nonce, rest) = rest.split_at_checked(64)?;
    let rest = rest.strip_prefix(INFIX)?;
    let (data, rest) = rest.split_at_checked(66)?;
    if rest != SUFFIX {
        return None;
    }
    let nonce = hex::decode_lower::<32>(nonce)?;
    let lock_key = PublicKey::from_slice(&hex::decode_lower::<33>(data)?).ok()?;
    Some(TemplateSecret { nonce, lock_key })
}
