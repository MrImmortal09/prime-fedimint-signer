//! The phone wallet's Rust core for the iOS app: `fps-mobile`, the Fedimint wallet, and
//! `fps-cashu`, the Cashu wallet, compiled into the one static library the app links, each with its
//! own UniFFI namespace (`docs/plan/12_CASHU.md`, "The phone"). Two Rust static libraries cannot be
//! linked into one app: each carries its own copy of std and of every shared dependency.
//!
//! `scripts/ios-core-cashu.sh` builds this crate with `--crate-type staticlib` and fps-cashu's
//! `uniffi` feature, then generates both namespaces' Swift bindings from the library in library
//! mode. fps-mobile is linked unchanged, so its bindings stay byte-identical.
//!
//! The re-exports are what put both crates in the library (UniFFI's multi-crate "megazord"): a
//! dependency that nothing names is not linked at all, and with it would go its exported scaffolding
//! functions and the metadata the bindings generator reads.

pub use fps_cashu;
pub use fps_mobile;
