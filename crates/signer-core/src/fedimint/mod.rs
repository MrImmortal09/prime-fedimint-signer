//! Decoders and encoders for the fedimint 0.13 types the signer handles: the unsigned transaction,
//! the default module shape (`mintv2`, `lnv2`, `walletv2`), guardian endpoints, base32 and mint v2
//! ecash tokens.
//!
//! Each type mirrors a fedimint type field for field; the doc comment names it. Golden vectors
//! produced with the real fedimint crates (see `devtools/`) check every encoding byte for byte.

pub mod base32;
pub mod ecash;
pub mod endpoints;
pub mod lnv2;
pub mod mintv2;
pub mod primitives;
pub mod transaction;
pub mod walletv2;

pub use primitives::{DynItem, OutPoint};
pub use transaction::UnsignedTransaction;

/// fedimint's FeeConsensus rule, shared by mint v2, LNv2 and wallet v2:
/// `base + amount * ppm / 1_000_000`, saturating.
pub fn module_fee_msat(base_msat: u64, ppm: u64, amount_msat: u64) -> u64 {
    (amount_msat.saturating_mul(ppm) / 1_000_000).saturating_add(base_msat)
}
