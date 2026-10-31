//! Canonical mint URLs. A mint's keys derive from its URL, so the same mint must always be spelled
//! the same way: the form CDK's `MintUrl` renders, which is what the phone sends.
//!
//! CDK (`cashu` 0.18.3, `mint_url.rs`) removes every trailing `/`, lower-cases the scheme and the
//! host, and keeps the path as it is. [`canonicalize`] does the same on the phone; Prime refuses any
//! URL that is not already in that form ([`is_canonical`]), and a few more Prime cannot show
//! honestly ([`check_mint_url`]).

use fps_protocol::RejectCode;
use fps_signer_core::error::{SignerError, SignerResult};

/// Longest mint URL Prime accepts, in bytes.
pub const MAX_URL_LEN: usize = 256;

/// CDK's `MintUrl` form of `url`, or `None` where CDK refuses it (an empty URL, or one without
/// `://`). Everything after a second `://`, which CDK drops, is dropped here too.
pub fn canonicalize(url: &str) -> Option<String> {
    if url.is_empty() {
        return None;
    }
    let url = url.trim_end_matches('/');
    let mut parts = url.split("://");
    let scheme = parts.next()?.to_lowercase();
    let mut segments = parts.next()?.split('/');
    let host = segments.next()?.to_lowercase();
    let path = segments.collect::<Vec<&str>>().join("/");
    let mut out = format!("{scheme}://{host}");
    if !path.is_empty() {
        out.push('/');
        out.push_str(&path);
    }
    Some(out)
}

/// Whether CDK would render `url` unchanged.
pub fn is_canonical(url: &str) -> bool {
    canonicalize(url).as_deref() == Some(url)
}

/// The URL rules for pinning a mint: at most [`MAX_URL_LEN`] bytes of printable ASCII, `https` or
/// `http`, a host, no user info, query or fragment, and canonical. What the review shows is then
/// exactly the mint the keys belong to.
pub fn check_mint_url(url: &str) -> SignerResult<()> {
    let invalid = |reason: &str| {
        Err(SignerError::new(
            RejectCode::InvalidDescriptor,
            format!("the mint URL {reason}"),
        ))
    };
    if url.len() > MAX_URL_LEN {
        return invalid(&format!("is longer than {MAX_URL_LEN} bytes"));
    }
    if !url.bytes().all(|byte| byte.is_ascii_graphic()) {
        return invalid("has spaces, control or non-ASCII characters");
    }
    if url.contains(['@', '?', '#']) {
        return invalid("has user info, a query or a fragment");
    }
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return invalid("is not https or http");
    };
    if rest.split('/').next().is_none_or(str::is_empty) {
        return invalid("has no host");
    }
    if !is_canonical(url) {
        return invalid("is not in canonical form (lower-case scheme and host, no trailing /)");
    }
    Ok(())
}
