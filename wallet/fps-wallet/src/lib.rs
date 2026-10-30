//! The phone side of the Passport Prime Fedimint signer.
//!
//! A watch-only fedimint client: the phone receives, keeps the notes and builds every
//! transaction, and Passport Prime holds the spend keys and signs each spend after the person
//! holding it approves on its screen. The phone derives every nonce, blinding key and refund
//! public key from the view material Prime exports (`docs/KEY_SCHEME.md`), so it can receive and
//! recover without Prime, and needs Prime only to spend.
//!
//! - [`device`]: reaching Prime over any transport.
//! - [`DeviceKeys`]: the client's keys as Prime describes them, and the signer that asks Prime.
//! - [`descriptor`]: what Prime needs to know about a federation.
//! - [`send`]: ecash sends through Prime: stage, release, reclaim.
//!
//! A client is built with fedimint's usual builder plus three pieces of `DeviceKeys`: the mint
//! module's key provider (`MintClientInitWithKeys`), the Lightning module's refund keys
//! (`LightningClientInit::refund_keys`), and the transaction signer
//! (`ClientBuilder::with_transaction_signer`).

pub mod descriptor;
pub mod device;
mod keys;
pub mod send;

pub use keys::{DeviceKeys, PairError};
