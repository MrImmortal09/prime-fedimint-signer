//! The Passport Prime Cashu signer core.
//!
//! Checks and signs Cashu spends of a wallet whose proofs are all locked, with `SIG_ALL`, to keys
//! only Prime holds: NUT-03 swaps and NUT-05 melts, at a mint whose keysets Prime pinned. No network
//! and no CDK: it builds for `armv7a-unknown-xous-elf`, and golden vectors and cross-checks made with
//! the `cashu` 0.18.3 crate (`devtools-cashu/`) check every primitive. The design is
//! `docs/plan/12_CASHU.md`.
//!
//! - [`keys`]: the `fps-cashu-v1` key scheme (docs/KEY_SCHEME_CASHU.md), both Prime's half and the
//!   phone's.
//! - [`url`]: canonical mint URLs, as CDK renders them.
//! - [`secret`]: the one P2PK secret template the wallet's proofs carry.
//! - [`dhke`]: NUT-00 `hash_to_curve` and blinded messages.
//! - [`message`]: the NUT-11 `SIG_ALL` message, and the check that its outputs read one way only.
//! - [`dleq`]: NUT-12 DLEQ proofs.
//! - [`keyset`]: NUT-02 keyset ids, and validating a mint before pinning it.
//! - [`analysis`]: what a swap or melt does, whether to refuse it, and the reviews.
//! - [`sign`]: the BIP-340 signature over the message.

pub mod analysis;
pub mod dhke;
pub mod dleq;
mod hex;
pub mod keys;
pub mod keyset;
pub mod message;
pub mod secret;
pub mod sign;
pub mod url;

use fps_protocol::RejectCode;
use fps_signer_core::error::SignerError;

pub use analysis::{analyze, review_add_mint, CashuAnalysis, SpendKind};
pub use keys::{KeyError, MintKeys, PhoneMintKeys};
pub use keyset::{validate_mint, PinnedKeyset, PinnedMint};
pub use sign::sign_spend;

impl From<KeyError> for SignerError {
    fn from(error: KeyError) -> Self {
        SignerError::new(RejectCode::Internal, error.to_string())
    }
}
