//! The phone wallet's core, for the Android app.
//!
//! A watch-only fedimint client whose spend keys Passport Prime holds (see `fps-wallet`), exported
//! to Kotlin with UniFFI. The app implements [`PrimeLink`], which carries each request to Prime and
//! brings back the answer: as animated QR codes, with [`request_frames`] and [`AnswerScanner`], or
//! over the development relay to the simulator. Everything else happens here: joining a federation
//! and pairing it with Prime, receiving, paying, withdrawing, and sending ecash.

uniffi::setup_scaffolding!();

mod error;
mod link;
mod qr;
mod store;
mod wallet;

pub use error::{LinkError, WalletError};
pub use link::PrimeLink;
pub use qr::{AnswerScanner, Scan, request_frames};
pub use wallet::{
    Activity, EcashSend, FederationSummary, LightningInvoice, Payment, PendingSend, Wallet,
};
