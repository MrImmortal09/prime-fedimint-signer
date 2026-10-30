//! Reaching Passport Prime.

use std::fmt::Debug;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use fedimint_core::secp256k1::rand;
use fps_protocol::{
    Envelope, FixedBytes, RejectCode, Request, RequestId, Response, decode_request,
    decode_response, encode,
};
use fps_transport::bridge::Bridge;

/// One way to reach Prime: the simulator file bridge, animated QR codes, Bluetooth.
#[async_trait]
pub trait DeviceLink: Debug + Send + Sync + 'static {
    /// Delivers one request envelope and returns Prime's final answer. A request that needs
    /// approval waits for the person at the device.
    async fn call(&self, request: Vec<u8>) -> Result<Vec<u8>, String>;
}

/// Why a request to Prime failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeviceError {
    /// The link failed.
    #[error("the device could not be reached: {0}")]
    Link(String),
    /// The answer is not a protocol message for this request.
    #[error("the device's answer is malformed: {0}")]
    Malformed(String),
    /// Prime refused.
    #[error("the device refused ({code:?}): {message}")]
    Rejected { code: RejectCode, message: String },
    /// Prime answered something the request does not call for.
    #[error("unexpected answer from the device: {0}")]
    Unexpected(String),
}

/// Passport Prime, through a link.
#[derive(Debug, Clone)]
pub struct Device {
    link: Arc<dyn DeviceLink>,
}

impl Device {
    pub fn new(link: Arc<dyn DeviceLink>) -> Self {
        Self { link }
    }

    /// Sends `body` under a fresh request id and returns Prime's answer; a rejection is an error.
    pub async fn request(&self, body: Request) -> Result<Response, DeviceError> {
        let id: RequestId = FixedBytes(rand::random());
        let answer = self
            .link
            .call(encode(&Envelope::new(id, body)))
            .await
            .map_err(DeviceError::Link)?;
        let envelope =
            decode_response(&answer).map_err(|error| DeviceError::Malformed(error.to_string()))?;
        if envelope.id != id {
            return Err(DeviceError::Malformed(format!(
                "the answer is addressed to {}",
                envelope.id.to_hex()
            )));
        }
        match envelope.body {
            Response::Rejected(rejection) => Err(DeviceError::Rejected {
                code: rejection.code,
                message: rejection.message,
            }),
            body => Ok(body),
        }
    }
}

/// Prime in the KeyOS simulator, through the file bridge (`fps_transport::bridge`).
#[derive(Debug, Clone)]
pub struct BridgeLink {
    bridge: Bridge,
    /// How long to wait for an answer, including the time someone takes to approve.
    timeout: Duration,
}

impl BridgeLink {
    pub fn new(bridge: Bridge, timeout: Duration) -> Self {
        Self { bridge, timeout }
    }
}

#[async_trait]
impl DeviceLink for BridgeLink {
    async fn call(&self, request: Vec<u8>) -> Result<Vec<u8>, String> {
        let id = decode_request(&request)
            .map_err(|error| error.to_string())?
            .id;
        let bridge = self.bridge.clone();
        let timeout = self.timeout;
        tokio::task::spawn_blocking(move || {
            bridge
                .submit(&request)
                .map_err(|error| format!("cannot queue the request: {error}"))?;
            bridge
                .wait_response(&id, timeout)
                .map_err(|error| format!("cannot read the answer: {error}"))?
                .ok_or_else(|| format!("no answer within {timeout:?}"))
        })
        .await
        .map_err(|error| error.to_string())?
    }
}

/// Prime in the KeyOS simulator, through the network link (`fps_transport::net`): a stand-in for
/// QuantumLink while there is no device.
#[derive(Debug, Clone)]
pub struct NetworkLink {
    /// The simulator app's host, or `host:port`.
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
impl DeviceLink for NetworkLink {
    async fn call(&self, request: Vec<u8>) -> Result<Vec<u8>, String> {
        let (address, timeout) = (self.address.clone(), self.timeout);
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
        .map_err(|error| error.to_string())?
    }
}
