//! Animated QR codes.
//!
//! A message travels as one Uniform Resource (BCR-2020-005) whose payload is the protocol envelope
//! itself, which is already CBOR. The sender shows the pure fountain parts in a loop; the receiver
//! collects them in any order, starting anywhere. Parts are upper case so QR codes can use the
//! alphanumeric mode, which packs about a fifth more characters into a code of the same size.
//!
//! On Passport Prime the system scanner reassembles multi-part codes before the app sees them, so
//! the app only checks the result with [`scanned_message`]. [`PartDecoder`] is for the other end.

use foundation_ur::bytewords::{self, Style};
use foundation_ur::fountain::part::Part;
use foundation_ur::{Decoder, Encoder, UR};
use fps_protocol::MAX_MESSAGE_SIZE;
pub use fps_protocol::{UR_TYPE_REQUEST, UR_TYPE_RESPONSE};

/// Characters per QR code shown on Prime's 480 px screen: small codes scan quickly from it. The
/// phone chooses its own size with [`encode_with`].
pub const MAX_PART_CHARS: usize = 300;

/// The sequence number fragments are sized for. Only the pure parts are shown, and even the
/// largest message has fewer than this many.
const MAX_SEQUENCE: usize = 1_000;

/// Why a code could not be read as a protocol message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrError {
    /// Not a Uniform Resource, or a damaged one.
    Malformed(String),
    /// A Uniform Resource of another type.
    WrongType { expected: String, found: String },
    /// A part that does not belong with the parts already collected.
    Inconsistent(String),
    /// The message is larger than the protocol allows.
    TooLarge { size: usize },
}

impl std::fmt::Display for UrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(reason) => write!(f, "not a readable code: {reason}"),
            Self::WrongType { expected, found } => {
                write!(f, "this is a {found} code, not {expected}")
            }
            Self::Inconsistent(reason) => {
                write!(f, "this part belongs to another message: {reason}")
            }
            Self::TooLarge { size } => write!(
                f,
                "the message is {size} bytes, the limit is {MAX_MESSAGE_SIZE}"
            ),
        }
    }
}

impl std::error::Error for UrError {}

/// The parts that carry `message` as `ur_type`, sized for Prime's screen, to show one after
/// another in a loop.
pub fn encode(ur_type: &str, message: &[u8]) -> Vec<String> {
    encode_with(ur_type, message, MAX_PART_CHARS)
}

/// The parts that carry `message` as `ur_type`, each at most `max_part_chars` characters, to show
/// one after another in a loop. A phone's screen can show denser codes than Prime's, and so fewer
/// of them.
pub fn encode_with(ur_type: &str, message: &[u8], max_part_chars: usize) -> Vec<String> {
    if message.is_empty() {
        // The fountain encoder needs at least one byte; an empty message is one single-part code.
        return vec![foundation_ur::to_string(ur_type, message).to_ascii_uppercase()];
    }
    let fragment_len = foundation_ur::max_fragment_len(ur_type, MAX_SEQUENCE, max_part_chars);
    let mut encoder = Encoder::new();
    encoder.start(ur_type, message, fragment_len);
    (0..encoder.sequence_count())
        .map(|_| encoder.next_part().to_string().to_ascii_uppercase())
        .collect()
}

/// The message in a code the KeyOS scanner reassembled, if the code is of type `expected`.
pub fn scanned_message(expected: &str, ur_type: &str, data: Vec<u8>) -> Result<Vec<u8>, UrError> {
    if !ur_type.eq_ignore_ascii_case(expected) {
        return Err(UrError::WrongType {
            expected: expected.to_owned(),
            found: ur_type.to_owned(),
        });
    }
    check_size(data.len())?;
    Ok(data)
}

fn check_size(size: usize) -> Result<(), UrError> {
    if size > MAX_MESSAGE_SIZE {
        return Err(UrError::TooLarge { size });
    }
    Ok(())
}

/// Collects the parts of one message of an expected type.
pub struct PartDecoder {
    expected: String,
    decoder: Decoder,
}

impl PartDecoder {
    pub fn new(expected: &str) -> Self {
        Self {
            expected: expected.to_owned(),
            decoder: Decoder::default(),
        }
    }

    /// Takes one scanned part, in either case. Returns the message once every fragment has
    /// arrived; parts after that are ignored by starting over with [`PartDecoder::new`].
    pub fn receive(&mut self, part: &str) -> Result<Option<Vec<u8>>, UrError> {
        let text = part.trim().to_ascii_lowercase();
        let ur = UR::parse(&text).map_err(|error| UrError::Malformed(format!("{error:?}")))?;
        if ur.as_type() != self.expected {
            return Err(UrError::WrongType {
                expected: self.expected.clone(),
                found: ur.as_type().to_owned(),
            });
        }
        let encoded = ur
            .as_bytewords()
            .ok_or_else(|| UrError::Malformed("no payload".to_owned()))?;
        let payload = bytewords::decode(encoded, Style::Minimal)
            .map_err(|error| UrError::Malformed(format!("{error:?}")))?;
        if ur.is_single_part() {
            check_size(payload.len())?;
            return Ok(Some(payload));
        }

        // Read the part header first: the reassembly buffer is sized from the length it claims.
        let header: Part<'_> = minicbor::decode(&payload)
            .map_err(|error| UrError::Malformed(format!("part header: {error}")))?;
        check_size(header.message_length)?;
        self.decoder
            .receive(UR::MultiPartDeserialized {
                ur_type: ur.as_type(),
                fragment: header,
            })
            .map_err(|error| UrError::Inconsistent(format!("{error:?}")))?;
        if !self.decoder.is_complete() {
            return Ok(None);
        }
        let message = self
            .decoder
            .message()
            .map_err(|error| UrError::Inconsistent(format!("{error:?}")))?
            .ok_or_else(|| UrError::Inconsistent("complete but empty".to_owned()))?;
        Ok(Some(message.to_vec()))
    }

    /// Roughly how much of the message has arrived, from 0 to 1.
    pub fn progress(&self) -> f64 {
        self.decoder.estimated_percent_complete()
    }
}
