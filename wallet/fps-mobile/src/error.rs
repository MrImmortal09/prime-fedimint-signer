//! What can go wrong, as the app sees it.

use fps_wallet::PairError;
use fps_wallet::device::DeviceError;
use fps_wallet::send::SendError;

/// Why a wallet call failed. Flat for the app: each exception carries the message to show.
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum WalletError {
    /// The wallet has not joined a federation yet.
    #[error("join a federation first")]
    NotJoined,
    /// The wallet already belongs to a federation.
    #[error("this wallet already belongs to a federation")]
    AlreadyJoined,
    /// An invite code, token, invoice, address or amount the wallet cannot use.
    #[error("{reason}")]
    Invalid { reason: String },
    /// Prime refused, or the exchange with it failed or was cancelled.
    #[error("{reason}")]
    Prime { reason: String },
    /// The federation, the network or the phone's storage failed.
    #[error("{reason}")]
    Failed { reason: String },
}

impl WalletError {
    pub(crate) fn invalid(message: impl std::fmt::Display) -> Self {
        Self::Invalid {
            reason: message.to_string(),
        }
    }

    pub(crate) fn failed(message: impl std::fmt::Display) -> Self {
        Self::Failed {
            reason: message.to_string(),
        }
    }
}

impl From<DeviceError> for WalletError {
    fn from(error: DeviceError) -> Self {
        Self::Prime {
            reason: error.to_string(),
        }
    }
}

impl From<PairError> for WalletError {
    fn from(error: PairError) -> Self {
        match error {
            PairError::Device(error) => error.into(),
            other => Self::failed(other),
        }
    }
}

impl From<SendError> for WalletError {
    fn from(error: SendError) -> Self {
        match error {
            SendError::Amount(reason) => Self::Invalid { reason },
            SendError::Device(error) => error.into(),
            // A spend Prime declined surfaces from the mint module as a failed transaction.
            SendError::Mint(error) => Self::Prime {
                reason: error.to_string(),
            },
        }
    }
}

/// Why an exchange with Prime failed, as the app reports it.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum LinkError {
    /// The person stopped the exchange.
    #[error("the exchange with Prime was cancelled")]
    Cancelled,
    /// The transport failed.
    #[error("{reason}")]
    Failed { reason: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for LinkError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed {
            reason: error.reason,
        }
    }
}
