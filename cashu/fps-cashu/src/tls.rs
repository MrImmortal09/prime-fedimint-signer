//! The TLS provider for connections to mints.
//!
//! rustls takes its cryptography from one process-wide default provider. In the app the Cashu and
//! Fedimint wallets share the process: CDK installs ring when it creates an HTTP client and finds
//! none, fedimint installs aws-lc-rs on its first websocket. Whichever ran first would decide for
//! both, so a Cashu client made first would move fedimint's connections onto ring. So the app
//! calls [`install_tls`] when it starts, and [`CashuWallet::open`](crate::CashuWallet::open) calls
//! it again: it installs aws-lc-rs, the provider fedimint would install, when none is set.

use rustls::crypto::CryptoProvider;

/// Installs aws-lc-rs as rustls's process-wide default provider, unless one is installed already.
/// Later calls change nothing.
pub fn install_tls() {
    if CryptoProvider::get_default().is_none() {
        // Another thread may have installed one since; then this one is refused, which is fine.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }
}
