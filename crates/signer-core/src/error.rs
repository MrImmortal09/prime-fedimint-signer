//! The signer's error: a stable protocol code plus a message for people.

use std::fmt;

use fps_protocol::{RejectCode, Rejection};

use crate::encoding::DecodeError;
use crate::keys::KeyError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerError {
    pub code: RejectCode,
    pub message: String,
}

pub type SignerResult<T> = Result<T, SignerError>;

impl SignerError {
    pub fn new(code: RejectCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn to_rejection(&self) -> Rejection {
        Rejection {
            code: self.code,
            message: self.message.clone(),
        }
    }

    /// Bytes of a fedimint transaction or contract did not decode.
    pub fn invalid_transaction(error: DecodeError) -> Self {
        let code = match error {
            DecodeError::UnknownVariant { .. } => RejectCode::Unsupported,
            DecodeError::TooMany { .. } => RejectCode::TooLarge,
            _ => RejectCode::InvalidTransaction,
        };
        Self::new(code, error.to_string())
    }

    /// Bytes of a federation descriptor did not decode.
    pub fn invalid_descriptor(error: DecodeError) -> Self {
        Self::new(RejectCode::InvalidDescriptor, error.to_string())
    }
}

impl From<KeyError> for SignerError {
    fn from(error: KeyError) -> Self {
        Self::new(RejectCode::Internal, error.to_string())
    }
}

impl fmt::Display for SignerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for SignerError {}
