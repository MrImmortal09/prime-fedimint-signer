//! Compatibility pins for the Fedimint golden vectors, frozen against the Fedimint-only code (`main`
//! at `3455d17`) before the Cashu backend was added.
//!
//! `vectors/fedimint-0.13.0-beta.1.json` is what the real fedimint crates produce, and
//! `vectors/key-scheme-fps-v1.json` freezes the `fps-v1` keys the paired phone derives too. Both are
//! pinned by their SHA-256. A diff in this file means the Fedimint vectors changed, so the signer
//! core is now checked against something else. Cashu keeps its own vectors elsewhere.
//!
//! After a deliberate regeneration (`cargo run --release --manifest-path devtools/Cargo.toml --bin
//! golden`), take the new hashes from `sha256sum crates/signer-core/tests/vectors/*.json` and
//! rewrite the state fixture (`write_fixture` in `crates/signer/tests/compat_state.rs`) in the same
//! commit.

use bitcoin::hashes::{sha256, Hash};

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(sha256::Hash::hash(bytes).to_byte_array())
}

#[test]
fn the_fedimint_vectors_are_unchanged() {
    assert_eq!(
        sha256_hex(include_bytes!("vectors/fedimint-0.13.0-beta.1.json")),
        "87230fe724e8185901dc066337ca7372b9a4c4fdbf6901a6bdad6f2bcdb241f1",
        "the fedimint 0.13.0-beta.1 golden vectors changed"
    );
}

#[test]
fn the_key_scheme_vectors_are_unchanged() {
    assert_eq!(
        sha256_hex(include_bytes!("vectors/key-scheme-fps-v1.json")),
        "9a387e97bcc1d4b4e4378d211e5f8cf9c40e85ca61e4106f64f24cc75f1ad076",
        "the fps-v1 key scheme vectors changed"
    );
}
