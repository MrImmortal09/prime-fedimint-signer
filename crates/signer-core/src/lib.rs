//! The Passport Prime Fedimint signer core.
//!
//! Decodes fedimint 0.13 transactions for the default module shape (mint v2, LNv2, wallet v2) from
//! their consensus bytes, decides whether they are the device's to sign, builds what the user
//! reviews, signs, and releases ecash tokens. No network, no fedimint dependency: it builds for
//! `armv7a-unknown-xous-elf`, and golden vectors from the real fedimint crates check its encodings.
//!
//! - [`keys`]: the `fps-v1` key scheme (docs/KEY_SCHEME.md).
//! - [`federation`]: validating an imported federation descriptor.
//! - [`analysis`]: what a transaction does, and whether to refuse it.
//! - [`sign`]: BIP-340 signatures over the transaction id.
//! - [`release`]: ecash tokens from outgoing-branch notes.

pub mod analysis;
pub mod encoding;
pub mod error;
pub mod federation;
pub mod fedimint;
pub mod invoice;
pub mod keys;
pub mod release;
pub mod review;
pub mod sign;
pub mod tbs;

pub use analysis::{analyze, Analysis, AnalysisOptions, EmptyLedger, Ledger};
pub use error::{SignerError, SignerResult};
pub use federation::Federation;
pub use keys::{FederationKeys, ViewKeys};
pub use release::{plan_release, ReleasePlan};
pub use review::Review;
pub use sign::sign_transaction;
