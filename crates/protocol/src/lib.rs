//! Wire protocol between the phone wallet and the Passport Prime Fedimint signer.
//!
//! Every message is one CBOR-encoded [`Envelope`]: a protocol version, a request id chosen by the
//! phone, and a body. The same bytes travel over every transport (animated UR, the simulator file
//! bridge, QuantumLink, NFC), so the device has one parser. The full specification is
//! `docs/PROTOCOL.md`.
//!
//! Decoding is strict: unknown fields and variants are errors, byte strings must have their exact
//! length, and messages over [`MAX_MESSAGE_SIZE`] are refused before parsing.

mod bytes;
mod cashu;
mod messages;

pub use bytes::{ByteBuf, FixedBytes};
pub use cashu::*;
pub use messages::*;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};

/// Protocol version carried in every envelope.
pub const PROTOCOL_VERSION: u16 = 1;

/// Name of the key derivation scheme the device implements. See docs/KEY_SCHEME.md.
pub const KEY_SCHEME: &str = "fps-v1";

/// Largest message either side accepts, in bytes. A federation import with every mint v2
/// denomination key is about 5 KiB; a signing request with 64 inputs and outputs is under 16 KiB.
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024;

/// UR type for phone-to-device messages on the animated-QR transport.
pub const UR_TYPE_REQUEST: &str = "fps-request";
/// UR type for device-to-phone messages on the animated-QR transport.
pub const UR_TYPE_RESPONSE: &str = "fps-response";

/// One protocol message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope<T> {
    #[serde(rename = "v")]
    pub version: u16,
    pub id: RequestId,
    pub body: T,
}

pub type RequestEnvelope = Envelope<Request>;
pub type ResponseEnvelope = Envelope<Response>;

impl<T> Envelope<T> {
    pub fn new(id: RequestId, body: T) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            body,
        }
    }
}

/// Why a message could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    TooLarge {
        size: usize,
    },
    /// The bytes are not a well-formed envelope. Carries the id when the header was readable, so the
    /// receiver can still address a rejection to it.
    Malformed {
        id: Option<RequestId>,
        reason: String,
    },
    UnsupportedVersion {
        id: RequestId,
        version: u16,
    },
}

impl DecodeError {
    /// The request id, when it could be read.
    pub fn request_id(&self) -> Option<RequestId> {
        match self {
            Self::TooLarge { .. } => None,
            Self::Malformed { id, .. } => *id,
            Self::UnsupportedVersion { id, .. } => Some(*id),
        }
    }

    /// The rejection a device should answer with.
    pub fn rejection(&self) -> Rejection {
        let (code, message) = match self {
            Self::TooLarge { size } => (
                RejectCode::TooLarge,
                format!("message is {size} bytes, the limit is {MAX_MESSAGE_SIZE}"),
            ),
            Self::Malformed { reason, .. } => (RejectCode::Malformed, reason.clone()),
            Self::UnsupportedVersion { version, .. } => (
                RejectCode::UnsupportedVersion,
                format!("protocol version {version} is not supported, expected {PROTOCOL_VERSION}"),
            ),
        };
        Rejection { code, message }
    }
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.rejection().message)
    }
}

impl std::error::Error for DecodeError {}

/// Serialises an envelope to CBOR.
pub fn encode<T: Serialize>(envelope: &Envelope<T>) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(envelope, &mut out).expect("serialising to a Vec cannot fail");
    out
}

/// Parses a request envelope.
pub fn decode_request(bytes: &[u8]) -> Result<RequestEnvelope, DecodeError> {
    decode(bytes)
}

/// Parses a response envelope.
pub fn decode_response(bytes: &[u8]) -> Result<ResponseEnvelope, DecodeError> {
    decode(bytes)
}

#[derive(Deserialize)]
struct Header {
    #[serde(rename = "v")]
    version: u16,
    id: RequestId,
    #[allow(dead_code)]
    body: IgnoredAny,
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<Envelope<T>, DecodeError> {
    if bytes.len() > MAX_MESSAGE_SIZE {
        return Err(DecodeError::TooLarge { size: bytes.len() });
    }
    // Read the header first, so a malformed body can still be answered under its request id and a
    // future version is reported as such rather than as garbage.
    let header: Header = ciborium::from_reader(bytes).map_err(|error| DecodeError::Malformed {
        id: None,
        reason: format!("not a protocol envelope: {error}"),
    })?;
    if header.version != PROTOCOL_VERSION {
        return Err(DecodeError::UnsupportedVersion {
            id: header.id,
            version: header.version,
        });
    }
    let mut reader = bytes;
    let envelope: Envelope<T> =
        ciborium::from_reader(&mut reader).map_err(|error| DecodeError::Malformed {
            id: Some(header.id),
            reason: format!("invalid message body: {error}"),
        })?;
    if !reader.is_empty() {
        return Err(DecodeError::Malformed {
            id: Some(header.id),
            reason: format!("{} trailing bytes after the message", reader.len()),
        });
    }
    Ok(envelope)
}
