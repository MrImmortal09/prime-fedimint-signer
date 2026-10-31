//! What can go wrong, as the app sees it.

/// Why a Cashu wallet call failed. Flat for the app: each case carries the message to show.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error), uniffi(flat_error))]
pub enum CashuError {
    /// A mint URL, token, invoice or amount the wallet cannot use, or a mint it has not added.
    #[error("{message}")]
    Invalid { message: String },
    /// Prime refused, or the exchange with it failed or was cancelled.
    #[error("{message}")]
    Prime { message: String },
    /// The mint refused, or could not be reached.
    #[error("{message}")]
    Mint { message: String },
    /// The phone's storage, or anything else, failed.
    #[error("{message}")]
    Failed { message: String },
}

impl CashuError {
    pub(crate) fn invalid(message: impl std::fmt::Display) -> Self {
        Self::Invalid {
            message: message.to_string(),
        }
    }

    pub(crate) fn prime(message: impl std::fmt::Display) -> Self {
        Self::Prime {
            message: message.to_string(),
        }
    }

    pub(crate) fn mint(message: impl std::fmt::Display) -> Self {
        Self::Mint {
            message: message.to_string(),
        }
    }

    pub(crate) fn failed(message: impl std::fmt::Display) -> Self {
        Self::Failed {
            message: message.to_string(),
        }
    }
}

/// Why an exchange with Prime failed, as the app's link reports it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
pub enum CashuLinkError {
    /// The person stopped the exchange.
    #[error("the exchange with Prime was cancelled")]
    Cancelled,
    /// The transport failed.
    #[error("{message}")]
    Failed { message: String },
}

#[cfg(feature = "uniffi")]
impl From<uniffi::UnexpectedUniFFICallbackError> for CashuLinkError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed {
            message: error.reason,
        }
    }
}

impl From<CashuLinkError> for CashuError {
    fn from(error: CashuLinkError) -> Self {
        Self::prime(error)
    }
}
