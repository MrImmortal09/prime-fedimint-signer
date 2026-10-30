//! How the wallet reaches Prime: the app implements [`PrimeLink`].

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use fps_wallet::device::{DeviceLink, NetworkLink};

use crate::LinkError;

/// How long a request to the simulator may wait for an answer, review included.
const SIMULATOR_TIMEOUT: Duration = Duration::from_secs(600);

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

/// Where the wallet's requests go: to the app's own link, or to the app in the KeyOS simulator
/// over the network while an address is set.
pub(crate) struct RoutedLink {
    app: ForeignLink,
    simulator: Mutex<Option<String>>,
}

impl RoutedLink {
    pub fn new(app: Arc<dyn PrimeLink>) -> Self {
        Self {
            app: ForeignLink(app),
            simulator: Mutex::new(None),
        }
    }

    pub fn set_simulator(&self, address: Option<String>) {
        *self
            .simulator
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = address;
    }
}

impl fmt::Debug for RoutedLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RoutedLink")
    }
}

#[async_trait]
impl DeviceLink for RoutedLink {
    async fn call(&self, request: Vec<u8>) -> Result<Vec<u8>, String> {
        let simulator = self
            .simulator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match simulator {
            Some(address) => {
                NetworkLink::new(address, SIMULATOR_TIMEOUT)
                    .call(request)
                    .await
            }
            None => self.app.call(request).await,
        }
    }
}
