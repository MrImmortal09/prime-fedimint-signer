//! How the wallet reaches Prime: the app implements [`PrimeLink`].

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use fps_wallet::device::DeviceLink;

use crate::LinkError;

/// Carries requests to Prime and brings back its answers. Over QR codes the app shows the request
/// with [`crate::request_frames`], the person scans it with Prime, approves, and the app reads
/// Prime's answer back with an [`crate::AnswerScanner`].
///
/// The wallet calls this whenever a spend needs Prime, including from background work such as
/// claiming the refund of a failed payment, so the app must be ready to show the exchange at any
/// time.
#[uniffi::export(foreign)]
#[async_trait]
pub trait PrimeLink: Send + Sync {
    /// Delivers one request envelope and returns Prime's answer envelope.
    async fn exchange(&self, request: Vec<u8>) -> Result<Vec<u8>, LinkError>;
}

/// The app's link, as `fps-wallet` sees it.
pub(crate) struct ForeignLink(pub Arc<dyn PrimeLink>);

impl fmt::Debug for ForeignLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ForeignLink")
    }
}

#[async_trait]
impl DeviceLink for ForeignLink {
    async fn call(&self, request: Vec<u8>) -> Result<Vec<u8>, String> {
        self.0
            .exchange(request)
            .await
            .map_err(|error| error.to_string())
    }
}
