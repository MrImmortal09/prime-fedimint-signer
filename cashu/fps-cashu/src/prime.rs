//! Reaching Passport Prime.
//!
//! A Cashu request travels as every other request does: one CBOR [`Envelope`] with a fresh random
//! 16-byte id, carrying `Request::Cashu`, and Prime's answer echoes the id. The app carries the
//! bytes ([`CashuPrimeLink`]), or, while a simulator address is set, [`NetworkLink`] carries them
//! to the Prime app in the KeyOS simulator, framed as fps-wallet's own network link frames them.

use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use bitcoin::secp256k1::rand;
use fps_protocol::{
    CashuLockMaterial, CashuMintDescriptor, CashuRequest, CashuResponse, CashuSignRequest,
    CashuSigned, Envelope, FixedBytes, Request, RequestId, Response, decode_response, encode,
};

use crate::{CashuError, CashuLinkError};

/// How long a request to the simulator may wait for its answer, the review on screen included.
pub const SIMULATOR_TIMEOUT: Duration = Duration::from_secs(600);

/// Carries one request envelope to Prime and brings back its answer envelope. The app implements
/// it; a request that needs approval returns once the person has decided on Prime.
#[cfg_attr(feature = "uniffi", uniffi::export(foreign))]
#[async_trait]
pub trait CashuPrimeLink: Send + Sync {
    /// Delivers one request envelope and returns Prime's answer envelope.
    async fn exchange(&self, request: Vec<u8>) -> Result<Vec<u8>, CashuLinkError>;
}

/// Prime in the KeyOS simulator, over the network link (`fps_transport::net`): one exchange per
/// connection, as `fps_wallet::device::NetworkLink` does it.
#[derive(Debug, Clone)]
pub struct NetworkLink {
    /// The simulator app's host, or `host:port` (port 7865 by default).
    address: String,
    /// How long to wait for an answer, including the time someone takes to approve.
    timeout: Duration,
}

impl NetworkLink {
    pub fn new(address: impl Into<String>, timeout: Duration) -> Self {
        Self {
            address: address.into(),
            timeout,
        }
    }
}

#[async_trait]
impl CashuPrimeLink for NetworkLink {
    async fn exchange(&self, request: Vec<u8>) -> Result<Vec<u8>, CashuLinkError> {
        let (address, timeout) = (self.address.clone(), self.timeout);
        let failed = |message: String| CashuLinkError::Failed { message };
        tokio::task::spawn_blocking(move || {
            let stream = fps_transport::net::connect(&address).map_err(|error| {
                format!("cannot connect to the simulator at {address}: {error}")
            })?;
            fps_transport::net::exchange_on(stream, &request, timeout).map_err(|error| match error
                .kind()
            {
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
                    format!("no answer from the simulator at {address} within {timeout:?}")
                }
                _ => format!("the simulator at {address}: {error}"),
            })
        })
        .await
        .map_err(|error| failed(error.to_string()))?
        .map_err(failed)
    }
}

/// The phone's side of the Cashu requests: the app's link, or the simulator while an address is
/// set.
pub(crate) struct Prime {
    app: Arc<dyn CashuPrimeLink>,
    simulator: Mutex<Option<String>>,
}

impl Prime {
    pub(crate) fn new(app: Arc<dyn CashuPrimeLink>) -> Self {
        Self {
            app,
            simulator: Mutex::new(None),
        }
    }

    pub(crate) fn set_simulator(&self, address: Option<String>) {
        *self
            .simulator
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = address;
    }

    async fn exchange(&self, request: Vec<u8>) -> Result<Vec<u8>, CashuLinkError> {
        let simulator = self
            .simulator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match simulator {
            Some(address) => {
                NetworkLink::new(address, SIMULATOR_TIMEOUT)
                    .exchange(request)
                    .await
            }
            None => self.app.exchange(request).await,
        }
    }

    /// Sends `body` under a fresh request id and returns Prime's Cashu answer. A refusal is
    /// [`CashuError::Prime`] with Prime's message.
    async fn request(&self, body: CashuRequest) -> Result<CashuResponse, CashuError> {
        let id: RequestId = FixedBytes(rand::random());
        let answer = self
            .exchange(encode(&Envelope::new(id, Request::Cashu(body))))
            .await?;
        let envelope = decode_response(&answer)
            .map_err(|error| CashuError::prime(format!("Prime's answer is unreadable: {error}")))?;
        if envelope.id != id {
            return Err(CashuError::prime(format!(
                "Prime's answer is for another request ({})",
                envelope.id.to_hex()
            )));
        }
        match envelope.body {
            Response::Cashu(response) => Ok(response),
            Response::Rejected(rejection) => Err(CashuError::prime(rejection.message)),
            _ => Err(CashuError::prime(
                "Prime answered something a Cashu request does not call for",
            )),
        }
    }

    /// Asks Prime to pin `descriptor` and returns the mint's lock material. Prime asks first.
    pub(crate) async fn add_mint(
        &self,
        descriptor: CashuMintDescriptor,
    ) -> Result<CashuLockMaterial, CashuError> {
        match self.request(CashuRequest::AddMint(descriptor)).await? {
            CashuResponse::MintAdded(material) => Ok(material),
            _ => Err(CashuError::prime(
                "Prime answered adding a mint with something else",
            )),
        }
    }

    /// Asks Prime to review and sign a swap or melt. Prime asks first.
    pub(crate) async fn sign(&self, request: CashuSignRequest) -> Result<CashuSigned, CashuError> {
        match self.request(CashuRequest::Sign(request)).await? {
            CashuResponse::Signed(signed) => Ok(signed),
            _ => Err(CashuError::prime(
                "Prime answered a signing request with something else",
            )),
        }
    }
}
