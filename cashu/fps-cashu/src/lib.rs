//! The phone's Cashu wallet: watch-only, with Passport Prime as its signer.
//!
//! Every proof is locked to a key only Prime holds (`fps-cashu-v1`, `docs/KEY_SCHEME_CASHU.md`),
//! with a NUT-11 `SIG_ALL` lock, so every spend is a swap or melt Prime checks, shows and signs.
//! Receiving (top-ups, tokens, taking a send back) needs no Prime. The design is
//! `docs/plan/12_CASHU.md`, "The phone".
//!
//! With the `uniffi` feature the API is exported under the UniFFI namespace `fps_cashu`, every
//! name prefixed `Cashu` so nothing collides with fps-mobile's names in the same Swift module.
//! fps-core links it beside fps-mobile into the one library the iOS app links.

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!("fps_cashu");

mod error;
mod mint;
mod prime;
mod store;
mod tls;
mod wallet;

pub use error::{CashuError, CashuLinkError};
pub use mint::canonical_url;
pub use prime::{CashuPrimeLink, NetworkLink, SIMULATOR_TIMEOUT};
pub use wallet::{CashuActivity, CashuMint, CashuPayment, CashuSend, CashuTopUp, CashuWallet};

/// Installs aws-lc-rs, the TLS provider fedimint uses, as rustls's process-wide default when none
/// is set. The app calls it at launch, before either wallet opens.
#[cfg_attr(feature = "uniffi", uniffi::export)]
pub fn cashu_install_tls() {
    tls::install_tls();
}
