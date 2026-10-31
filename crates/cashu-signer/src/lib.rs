//! The Cashu backend on Passport Prime, served beside the Fedimint signer.
//!
//! The app opens a [`Device`] where it opened a [`Signer`]. A `Device` derefs to its `Signer`, so
//! every call the app makes besides [`Device::handle`], [`Device::pending`], [`Device::approve`]
//! and [`Device::reject`] reaches the signer unchanged. Those four route by request
//! (`docs/plan/12_CASHU.md`, "Routing on Prime"):
//!
//! - `Request::Cashu` goes to the Cashu service. `Info` is always answered; `AddMint` and `Sign`
//!   get `Busy` while any review waits, Fedimint's or Cashu's, unless they have a stored answer.
//! - Every other message, undecodable bytes included, goes to `Signer::handle` unchanged, with one
//!   exception: while a Cashu review waits, `ImportFederation`, `ExportViewMaterial`,
//!   `SignTransaction` and `ReleaseNotes` get the answer the signer gives while its own review
//!   waits, byte for byte: their stored answer, or `Busy`.
//! - `GetResult` and `Cancel` go to the signer first, and to the Cashu service only when the signer
//!   has nothing for that request. If neither has, the signer's answer stands.
//! - `GetInfo` and `GetStatus` are the signer's: `busy` and `pending` describe Fedimint only.
//! - Approving and rejecting reach whichever service owns the waiting review.
//!
//! So one review waits at a time across both backends, and with Cashu idle every answer and every
//! write of `fps-state.cbor` is byte for byte what the bare `Signer` makes.
//!
//! The Cashu service keeps its own state in `fps-cashu.cbor`, through a second [`Storage`]: the
//! mints it pinned, its own ring of stored answers, and its own count of the day's outflow. It reads
//! the spending policy through [`Signer::policy`] at each decision and never writes the signer's
//! state. [`Device::open`] opens the signer and only that: the Cashu state is read on the first
//! request that needs it (a Cashu request, or a `GetResult` the signer has nothing for), so opening
//! the app costs Fedimint nothing whatever Cashu holds. A Cashu state that cannot be read turns
//! Cashu off and is left as it is; [`Device::open`] fails only as [`Signer::open`] does.
//!
//! Like `fps-signer`, this crate does not log: it holds lock material, openings and signatures,
//! and the app logs what it shows.

#![deny(
    unsafe_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod service;
mod state;

use std::ops::{Deref, DerefMut};

use fps_protocol::{
    decode_request, decode_response, CashuMintSummary, RejectCode, Rejection, Request, RequestId,
    Response,
};
use fps_signer::{Approval, Approved, Outcome, Platform, ServiceError, Signer, Storage};
use zeroize::Zeroizing;

pub use state::{
    pinned_descriptor, CashuOutflow, CashuResult, CashuState, CASHU_STATE_VERSION,
    MAX_CASHU_RESULTS, MAX_MINTS, MAX_PINNED_KEYS,
};

use service::{busy, envelope, CashuService, Fedimint};

/// The Fedimint signer with the Cashu service beside it. `S` stores `fps-state.cbor`, `C` stores
/// `fps-cashu.cbor`; each service has its own `P`.
pub struct Device<S: Storage, P: Platform, C: Storage> {
    signer: Signer<S, P>,
    cashu: CashuService<C, P>,
}

impl<S: Storage, P: Platform, C: Storage> Deref for Device<S, P, C> {
    type Target = Signer<S, P>;

    fn deref(&self) -> &Signer<S, P> {
        &self.signer
    }
}

impl<S: Storage, P: Platform, C: Storage> DerefMut for Device<S, P, C> {
    fn deref_mut(&mut self) -> &mut Signer<S, P> {
        &mut self.signer
    }
}

/// Whether the signer answered that it has nothing for the request: `Rejected(NotFound)`, which it
/// never stores, so it is never a replayed answer.
fn found_nothing(outcome: &Outcome) -> bool {
    let Outcome::Reply(bytes) = outcome else {
        return false;
    };
    matches!(
        decode_response(bytes).map(|envelope| envelope.body),
        Ok(Response::Rejected(Rejection {
            code: RejectCode::NotFound,
            ..
        }))
    )
}

impl<S: Storage, P: Platform, C: Storage> Device<S, P, C> {
    /// Opens the signer over its stored state, as [`Signer::open`] does and with its error, and the
    /// Cashu service beside it over `cashu_storage`, which is read on first use. The Cashu service
    /// draws its randomness and reads the time from `cashu_platform`.
    pub fn open(
        app_seed: [u8; 32],
        storage: S,
        platform: P,
        cashu_storage: C,
        cashu_platform: P,
    ) -> Result<Self, ServiceError> {
        let app_seed = Zeroizing::new(app_seed);
        let signer = Signer::open(*app_seed, storage, platform)?;
        let cashu = CashuService::open(&app_seed, cashu_storage, cashu_platform);
        Ok(Self { signer, cashu })
    }

    /// Handles one message from the phone.
    pub fn handle(&mut self, bytes: &[u8]) -> Outcome {
        let Ok(message) = decode_request(bytes) else {
            return self.signer.handle(bytes);
        };
        let id = message.id;
        match message.body {
            Request::Cashu(request) => {
                let fedimint = Fedimint {
                    waiting: self.signer.pending().map(|approval| approval.request),
                    policy: self.signer.policy(),
                    outflow: &self.signer.state().policy_state,
                };
                self.cashu.handle(id, request, &fedimint)
            }
            Request::GetResult { request } => {
                let outcome = self.signer.handle(bytes);
                if found_nothing(&outcome) {
                    if let Some(answer) = self.cashu.result(id, &request) {
                        return Outcome::Reply(answer);
                    }
                }
                outcome
            }
            Request::Cancel { request } => {
                let outcome = self.signer.handle(bytes);
                if found_nothing(&outcome) {
                    if let Some(answer) = self.cashu.cancel(id, &request) {
                        return Outcome::Reply(answer);
                    }
                }
                outcome
            }
            // What the signer answers these while its own review waits: a stored answer, else Busy.
            Request::ImportFederation(_)
            | Request::ExportViewMaterial { .. }
            | Request::SignTransaction(_)
            | Request::ReleaseNotes(_) => match self.cashu.pending() {
                Some(waiting) if self.signer.state().stored_result(&id).is_none() => {
                    Outcome::Reply(envelope(id, Response::Rejected(busy(&waiting.request))))
                }
                _ => self.signer.handle(bytes),
            },
            Request::GetInfo | Request::GetStatus | Request::DeriveOutgoingNonces { .. } => {
                self.signer.handle(bytes)
            }
        }
    }

    /// The request waiting for approval, Fedimint's or Cashu's; at most one waits.
    pub fn pending(&self) -> Option<&Approval> {
        self.signer.pending().or_else(|| self.cashu.pending())
    }

    /// Approves the waiting request, in whichever service it waits. Fails without side effects if
    /// `request` is not the one waiting, or if the PIN is required and was not verified.
    pub fn approve(
        &mut self,
        request: &RequestId,
        pin_verified: bool,
    ) -> Result<Approved, ServiceError> {
        if self.cashu.is_waiting(request) {
            self.cashu.approve(request, pin_verified)
        } else {
            self.signer.approve(request, pin_verified)
        }
    }

    /// Rejects the waiting request, in whichever service it waits.
    pub fn reject(&mut self, request: &RequestId) -> Result<Vec<u8>, ServiceError> {
        if self.cashu.is_waiting(request) {
            self.cashu.reject(request)
        } else {
            self.signer.reject(request)
        }
    }

    /// The mints the Cashu service has pinned, as its `Info` lists them. Reads the Cashu state if
    /// no request has yet.
    pub fn cashu_mints(&mut self) -> Vec<CashuMintSummary> {
        self.cashu.mints()
    }

    /// Why Cashu is off on this device, or `None` while it works. Reads the Cashu state if no
    /// request has yet.
    pub fn cashu_off(&mut self) -> Option<&str> {
        self.cashu.off()
    }
}
