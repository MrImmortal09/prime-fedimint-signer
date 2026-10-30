//! The signer service: one object that turns request bytes from any transport into response bytes,
//! with the user's approval in between when a request needs it.
//!
//! The device app owns a [`Signer`], feeds it every message that arrives ([`Signer::handle`]),
//! shows the [`Approval`] it asks for, and calls [`Signer::approve`] or [`Signer::reject`] when the
//! user decides. The service keeps the device's ledger (released notes, staged notes, funded
//! Lightning contracts), the spending policy, and the last responses for replay, and persists all
//! of it through [`Storage`] before any response that depends on it leaves the device.
//!
//! Requests that need approval: importing a federation, exporting view material, signing, and
//! releasing ecash (unless the notes were staged by an approved "prepare send" and the policy allows
//! releasing them without asking again). One request waits for approval at a time.

mod state;

use fps_policy::{day_of, Decision, PolicyConfig, Routing, Spend};
use fps_protocol::{
    decode_request, encode, Branch, Delivery, DeviceInfo, Envelope, FederationDescriptor,
    FederationId, FederationSummary, FixedBytes, Intent, RejectCode, Rejection, Released,
    ReleasedNote, Request, RequestId, Response, SignedTransaction, StatusInfo, ViewMaterial,
    KEY_SCHEME, PROTOCOL_VERSION,
};
use fps_signer_core::analysis::{InputKind, OutputSummary};
use fps_signer_core::encoding::Encode;
use fps_signer_core::federation::short_hex;
use fps_signer_core::{
    analyze, plan_release, sign_transaction, Analysis, AnalysisOptions, Federation, FederationKeys,
    ReleasePlan, Review, SignerError,
};
pub use state::{FederationNote, FundedContract, State, StoredResult, MAX_STORED_RESULTS};
use zeroize::Zeroizing;

/// Most outgoing nonces derived per request.
pub const MAX_NONCES_PER_REQUEST: usize = 64;
/// Fee overpayment tolerated for rounding; see `AnalysisOptions::max_overpay_msat`.
pub const MAX_OVERPAY_MSAT: u64 = 1_000;

/// Where the service keeps its state. On the device this is a file in the app's encrypted
/// `AppData`; `store` must be durable when it returns.
pub trait Storage {
    fn load(&mut self) -> Result<Option<Vec<u8>>, String>;
    fn store(&mut self, bytes: &[u8]) -> Result<(), String>;
}

/// Storage in memory, for tests and the host.
#[derive(Debug, Default, Clone)]
pub struct MemoryStorage {
    pub bytes: Option<Vec<u8>>,
}

impl Storage for MemoryStorage {
    fn load(&mut self) -> Result<Option<Vec<u8>>, String> {
        Ok(self.bytes.clone())
    }

    fn store(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.bytes = Some(bytes.to_vec());
        Ok(())
    }
}

/// What the service needs from the device.
pub trait Platform {
    /// 32 bytes from the hardware RNG.
    fn random_32(&mut self) -> [u8; 32];
    /// Unix time, if the device has a trustworthy clock.
    fn now_unix(&self) -> Option<u64>;
}

/// What a waiting request asks the user to approve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub request: RequestId,
    pub kind: ApprovalKind,
    pub review: Review,
    /// The device must re-verify the PIN before approving.
    pub require_pin: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalKind {
    ImportFederation,
    ExportViewMaterial,
    SignTransaction,
    ReleaseNotes,
}

/// The result of handling one message.
pub enum Outcome {
    /// Send these bytes back.
    Reply(Vec<u8>),
    /// Show the review and call [`Signer::approve`] or [`Signer::reject`].
    NeedsApproval(Approval),
    /// A release of notes staged by an approved "prepare send" went through without a second
    /// review. Send `reply`; if `show_token` is set, display it as a QR code.
    Released {
        reply: Vec<u8>,
        show_token: Option<Zeroizing<String>>,
    },
}

/// An approved request's result.
pub struct Approved {
    pub reply: Vec<u8>,
    /// For a release delivered on the device: the token to display.
    pub show_token: Option<Zeroizing<String>>,
}

impl std::fmt::Debug for Approved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is the money; keep it out of logs.
        f.debug_struct("Approved")
            .field("reply_bytes", &self.reply.len())
            .field(
                "show_token",
                &self.show_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    /// No request with that id is waiting for approval.
    NoSuchRequest,
    /// This approval needs the PIN re-verified first.
    PinRequired,
    Storage(String),
    /// The stored state could not be read.
    Corrupt(String),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchRequest => f.write_str("no request with that id is waiting for approval"),
            Self::PinRequired => f.write_str("this approval needs the PIN"),
            Self::Storage(error) => write!(f, "storage failed: {error}"),
            Self::Corrupt(error) => write!(f, "stored state is unreadable: {error}"),
        }
    }
}

impl std::error::Error for ServiceError {}

struct Loaded {
    federation: Federation,
    keys: FederationKeys,
}

enum Action {
    Import(Box<FederationDescriptor>),
    ExportView(FederationId),
    Sign(Box<Analysis>),
    Release(Box<ReleasePlan>),
}

struct Pending {
    action: Action,
    approval: Approval,
}

enum Step {
    /// Answer, without storing it for replay.
    Reply(Response),
    /// Answer with these exact bytes.
    Raw(Vec<u8>),
    /// Final answer: store for replay before sending.
    Final(Response),
    Approve(Pending),
    AutoRelease(Box<ReleasePlan>),
}

pub struct Signer<S: Storage, P: Platform> {
    app_seed: Zeroizing<[u8; 32]>,
    storage: S,
    platform: P,
    state: State,
    loaded: Vec<Loaded>,
    pending: Option<Pending>,
}

const ZERO_ID: RequestId = FixedBytes([0; 16]);

fn error(code: RejectCode, message: impl Into<String>) -> SignerError {
    SignerError::new(code, message)
}

impl<S: Storage, P: Platform> Signer<S, P> {
    /// Opens the service over stored state, or fresh state if there is none.
    pub fn open(app_seed: [u8; 32], mut storage: S, platform: P) -> Result<Self, ServiceError> {
        let state = match storage.load().map_err(ServiceError::Storage)? {
            Some(bytes) => State::decode(&bytes).map_err(ServiceError::Corrupt)?,
            None => State::new(),
        };
        let mut signer = Self {
            app_seed: Zeroizing::new(app_seed),
            storage,
            platform,
            state: State::new(),
            loaded: Vec::new(),
            pending: None,
        };
        signer.loaded = signer.load_federations(&state)?;
        signer.state = state;
        Ok(signer)
    }

    fn load_federations(&self, state: &State) -> Result<Vec<Loaded>, ServiceError> {
        state
            .federations
            .iter()
            .map(|descriptor| {
                let federation = Federation::from_descriptor(descriptor)
                    .map_err(|e| ServiceError::Corrupt(e.to_string()))?;
                let keys = FederationKeys::derive(&self.app_seed, &federation.id)
                    .map_err(|e| ServiceError::Corrupt(e.to_string()))?;
                Ok(Loaded { federation, keys })
            })
            .collect()
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn policy(&self) -> &PolicyConfig {
        &self.state.policy
    }

    pub fn set_policy(&mut self, policy: PolicyConfig) -> Result<(), ServiceError> {
        let mut next = self.state.clone();
        next.policy = policy;
        self.commit(next)
    }

    pub fn federations(&self) -> Vec<FederationSummary> {
        self.loaded.iter().map(|l| l.federation.summary()).collect()
    }

    /// The request waiting for approval, if any.
    pub fn pending(&self) -> Option<&Approval> {
        self.pending.as_ref().map(|p| &p.approval)
    }

    fn today(&self) -> Option<u64> {
        self.platform.now_unix().map(day_of)
    }

    fn find(&self, federation: &FederationId) -> Result<&Loaded, SignerError> {
        self.loaded
            .iter()
            .find(|l| l.federation.id == federation.0)
            .ok_or_else(|| {
                error(
                    RejectCode::UnknownFederation,
                    "this federation is not imported",
                )
            })
    }

    fn commit(&mut self, next: State) -> Result<(), ServiceError> {
        self.storage
            .store(&next.encode())
            .map_err(ServiceError::Storage)?;
        self.state = next;
        Ok(())
    }

    fn envelope(id: RequestId, response: Response) -> Vec<u8> {
        encode(&Envelope::new(id, response))
    }

    /// Handles one message from the phone.
    pub fn handle(&mut self, bytes: &[u8]) -> Outcome {
        let envelope = match decode_request(bytes) {
            Ok(envelope) => envelope,
            Err(decode_error) => {
                let id = decode_error.request_id().unwrap_or(ZERO_ID);
                return Outcome::Reply(Self::envelope(
                    id,
                    Response::Rejected(decode_error.rejection()),
                ));
            }
        };
        let id = envelope.id;
        // A request seen before gets its stored answer again, never a second signature.
        if let Some(stored) = self.state.stored_result(&id) {
            return Outcome::Reply(stored.to_vec());
        }
        let step = match self.dispatch(id, envelope.body) {
            Ok(step) => step,
            Err(rejection) => {
                return Outcome::Reply(Self::envelope(
                    id,
                    Response::Rejected(rejection.to_rejection()),
                ))
            }
        };
        match step {
            Step::Reply(response) => Outcome::Reply(Self::envelope(id, response)),
            Step::Raw(bytes) => Outcome::Reply(bytes),
            Step::Final(response) => match self.finish(id, response, |_| Ok(())) {
                Ok(reply) => Outcome::Reply(reply),
                Err(failure) => Outcome::Reply(Self::storage_failure(id, failure)),
            },
            Step::Approve(pending) => {
                let approval = pending.approval.clone();
                self.pending = Some(pending);
                Outcome::NeedsApproval(approval)
            }
            Step::AutoRelease(plan) => match self.execute_release(id, &plan) {
                Ok(approved) => Outcome::Released {
                    reply: approved.reply,
                    show_token: approved.show_token,
                },
                Err(ExecuteError::Rejected(rejection)) => Outcome::Reply(Self::envelope(
                    id,
                    Response::Rejected(rejection.to_rejection()),
                )),
                Err(ExecuteError::Service(failure)) => {
                    Outcome::Reply(Self::storage_failure(id, failure))
                }
            },
        }
    }

    fn storage_failure(id: RequestId, failure: ServiceError) -> Vec<u8> {
        Self::envelope(
            id,
            Response::Rejected(Rejection {
                code: RejectCode::Internal,
                message: failure.to_string(),
            }),
        )
    }

    fn ensure_idle(&self) -> Result<(), SignerError> {
        match &self.pending {
            Some(pending) => Err(error(
                RejectCode::Busy,
                format!(
                    "request {} is waiting for approval on the device",
                    pending.approval.request.to_hex()
                ),
            )),
            None => Ok(()),
        }
    }

    fn dispatch(&mut self, id: RequestId, body: Request) -> Result<Step, SignerError> {
        match body {
            Request::GetInfo => Ok(Step::Reply(Response::Info(DeviceInfo {
                protocol: PROTOCOL_VERSION,
                key_scheme: KEY_SCHEME.to_owned(),
                federations: self.federations(),
                busy: self.pending.is_some(),
            }))),
            Request::GetStatus => Ok(Step::Reply(Response::Status(self.status()))),
            Request::GetResult { request } => {
                if let Some(stored) = self.state.stored_result(&request) {
                    Ok(Step::Raw(stored.to_vec()))
                } else if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.approval.request == request)
                {
                    Ok(Step::Reply(Response::Pending))
                } else {
                    Err(error(
                        RejectCode::NotFound,
                        "no result stored for that request",
                    ))
                }
            }
            Request::Cancel { request } => {
                if !self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.approval.request == request)
                {
                    return Err(error(RejectCode::NotFound, "that request is not waiting"));
                }
                self.pending = None;
                let mut next = self.state.clone();
                next.store_result(request, Self::envelope(request, Response::Cancelled));
                self.commit(next)
                    .map_err(|e| error(RejectCode::Internal, e.to_string()))?;
                Ok(Step::Reply(Response::Cancelled))
            }
            Request::ImportFederation(descriptor) => {
                self.ensure_idle()?;
                let federation = Federation::from_descriptor(&descriptor)?;
                if self.state.federations.contains(&descriptor) {
                    return Ok(Step::Final(Response::FederationImported(
                        federation.summary(),
                    )));
                }
                let replacing = self.loaded.iter().any(|l| l.federation.id == federation.id);
                Ok(Step::Approve(Pending {
                    approval: Approval {
                        request: id,
                        kind: ApprovalKind::ImportFederation,
                        review: import_review(&federation, replacing),
                        require_pin: false,
                    },
                    action: Action::Import(descriptor.into()),
                }))
            }
            Request::ExportViewMaterial { federation } => {
                self.ensure_idle()?;
                let loaded = self.find(&federation)?;
                let review = Review::new("Connect wallet app", 0)
                    .line("Federation", loaded.federation.display_name())
                    .warn("The app will see every note you hold in this federation, but cannot spend any of them.");
                Ok(Step::Approve(Pending {
                    approval: Approval {
                        request: id,
                        kind: ApprovalKind::ExportViewMaterial,
                        review,
                        require_pin: false,
                    },
                    action: Action::ExportView(federation),
                }))
            }
            Request::DeriveOutgoingNonces { federation, notes } => {
                let loaded = self.find(&federation)?;
                if notes.len() > MAX_NONCES_PER_REQUEST {
                    return Err(error(
                        RejectCode::TooLarge,
                        format!("at most {MAX_NONCES_PER_REQUEST} nonces per request"),
                    ));
                }
                let nonces = notes
                    .iter()
                    .map(|note| {
                        loaded
                            .keys
                            .note_nonce(Branch::Outgoing, note.denomination, &note.tweak.0)
                            .map(|nonce| FixedBytes(nonce.serialize()))
                            .map_err(SignerError::from)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Step::Reply(Response::OutgoingNonces { nonces }))
            }
            Request::SignTransaction(request) => {
                self.ensure_idle()?;
                let loaded = self.find(&request.federation)?;
                let policy = &self.state.policy;
                let options = AnalysisOptions {
                    verify_note_signatures: policy.verify_note_signatures,
                    max_inputs: policy.max_inputs,
                    max_outputs: policy.max_outputs,
                    max_overpay_msat: MAX_OVERPAY_MSAT,
                    now_unix: self.platform.now_unix(),
                };
                let analysis = analyze(
                    &loaded.federation,
                    &loaded.keys,
                    &request,
                    &self.state,
                    &options,
                )?;
                let spend = Spend {
                    outflow_msat: analysis.outflow_msat(),
                    routing: analysis.lightning.as_ref().map(|payment| Routing {
                        invoice_msat: payment.invoice.amount_msat.unwrap_or(0),
                        fee_msat: payment.routing_fee_msat,
                    }),
                };
                let require_pin = self.decide(&spend)?;
                let mut review = analysis.review.clone();
                if matches!(analysis.intent, Intent::PrepareSend { .. }) {
                    review = review.warn(if policy.release_prepared_without_review {
                        "When your phone sends this ecash, the device will not ask again."
                    } else {
                        "You will confirm again on this device when your phone sends it."
                    });
                }
                Ok(Step::Approve(Pending {
                    approval: Approval {
                        request: id,
                        kind: ApprovalKind::SignTransaction,
                        review,
                        require_pin,
                    },
                    action: Action::Sign(analysis.into()),
                }))
            }
            Request::ReleaseNotes(request) => {
                self.ensure_idle()?;
                let loaded = self.find(&request.federation)?;
                let plan = plan_release(
                    &loaded.federation,
                    &loaded.keys,
                    &request,
                    &self.state,
                    self.state.policy.verify_release_signatures,
                )?;
                let require_pin = self.decide(&Spend {
                    outflow_msat: plan.amount_msat,
                    routing: None,
                })?;
                let prepared = plan
                    .notes
                    .iter()
                    .all(|note| self.state.is_prepared(&request.federation, note));
                if prepared && self.state.policy.release_prepared_without_review && !require_pin {
                    return Ok(Step::AutoRelease(plan.into()));
                }
                Ok(Step::Approve(Pending {
                    approval: Approval {
                        request: id,
                        kind: ApprovalKind::ReleaseNotes,
                        review: plan.review.clone(),
                        require_pin,
                    },
                    action: Action::Release(plan.into()),
                }))
            }
        }
    }

    fn decide(&self, spend: &Spend) -> Result<bool, SignerError> {
        match self
            .state
            .policy
            .evaluate(&self.state.policy_state, self.today(), spend)
        {
            Decision::Allow { require_pin } => Ok(require_pin),
            Decision::Deny { reason } => Err(error(RejectCode::PolicyLimit, reason)),
        }
    }

    fn status(&self) -> StatusInfo {
        StatusInfo {
            released: self
                .state
                .released
                .iter()
                .map(|r| ReleasedNote {
                    federation: r.federation,
                    note: r.note,
                })
                .collect(),
            recent: self.state.results.iter().map(|r| r.request).collect(),
            outflow_today_msat: self.state.policy_state.outflow_on(self.today()),
            pending: self.pending.as_ref().map(|p| p.approval.request),
        }
    }

    /// Stores `response` as the final answer to `id`, together with the state changes `update`
    /// makes, durably, then returns the reply bytes.
    fn finish(
        &mut self,
        id: RequestId,
        response: Response,
        update: impl FnOnce(&mut State) -> Result<(), ServiceError>,
    ) -> Result<Vec<u8>, ServiceError> {
        let reply = Self::envelope(id, response);
        let mut next = self.state.clone();
        update(&mut next)?;
        next.store_result(id, reply.clone());
        self.commit(next)?;
        Ok(reply)
    }

    /// Approves the waiting request. Fails without side effects if `request` is not the one
    /// waiting, or if the PIN is required and was not verified.
    pub fn approve(
        &mut self,
        request: &RequestId,
        pin_verified: bool,
    ) -> Result<Approved, ServiceError> {
        let waiting = self
            .pending
            .as_ref()
            .filter(|p| p.approval.request == *request)
            .ok_or(ServiceError::NoSuchRequest)?;
        if waiting.approval.require_pin && !pin_verified {
            return Err(ServiceError::PinRequired);
        }
        let pending = self.pending.take().expect("checked above");
        let id = pending.approval.request;
        let result = match &pending.action {
            Action::Import(descriptor) => self.execute_import(id, descriptor),
            Action::ExportView(federation) => self.execute_export(id, federation),
            Action::Sign(analysis) => self.execute_sign(id, analysis),
            Action::Release(plan) => self.execute_release(id, plan),
        };
        match result {
            Ok(approved) => Ok(approved),
            // A signing failure is answered, not retried.
            Err(ExecuteError::Rejected(rejection)) => Ok(Approved {
                reply: Self::envelope(id, Response::Rejected(rejection.to_rejection())),
                show_token: None,
            }),
            // Nothing was committed: the request stays waiting so the user can try again.
            Err(ExecuteError::Service(service)) => {
                self.pending = Some(pending);
                Err(service)
            }
        }
    }

    /// Rejects the waiting request.
    pub fn reject(&mut self, request: &RequestId) -> Result<Vec<u8>, ServiceError> {
        if !self
            .pending
            .as_ref()
            .is_some_and(|p| p.approval.request == *request)
        {
            return Err(ServiceError::NoSuchRequest);
        }
        self.pending = None;
        let response = Response::Rejected(Rejection {
            code: RejectCode::UserRejected,
            message: "rejected on the device".to_owned(),
        });
        self.finish(*request, response, |_| Ok(()))
    }

    fn execute_import(
        &mut self,
        id: RequestId,
        descriptor: &FederationDescriptor,
    ) -> Result<Approved, ExecuteError> {
        let federation = Federation::from_descriptor(descriptor).map_err(ExecuteError::Rejected)?;
        let keys = FederationKeys::derive(&self.app_seed, &federation.id)
            .map_err(|e| ExecuteError::Rejected(e.into()))?;
        let summary = federation.summary();
        let reply = self.finish(id, Response::FederationImported(summary), |next| {
            next.federations.retain(|d| d.federation.0 != federation.id);
            next.federations.push(descriptor.clone());
            Ok(())
        })?;
        self.loaded.retain(|l| l.federation.id != federation.id);
        self.loaded.push(Loaded { federation, keys });
        Ok(Approved {
            reply,
            show_token: None,
        })
    }

    fn execute_export(
        &mut self,
        id: RequestId,
        federation: &FederationId,
    ) -> Result<Approved, ExecuteError> {
        let loaded = self.find(federation).map_err(ExecuteError::Rejected)?;
        let view = loaded.keys.view();
        let material = ViewMaterial {
            federation: *federation,
            key_scheme: KEY_SCHEME.to_owned(),
            spend_key: FixedBytes(view.spend_key().serialize()),
            chain_code: FixedBytes(view.chain_code()),
            view_secret: FixedBytes(view.view_secret()),
        };
        // View material is not stored for replay: a replayed export asks again.
        let reply = Self::envelope(id, Response::ViewMaterial(material));
        Ok(Approved {
            reply,
            show_token: None,
        })
    }

    fn execute_sign(
        &mut self,
        id: RequestId,
        analysis: &Analysis,
    ) -> Result<Approved, ExecuteError> {
        let federation = FederationId::from(analysis.federation);
        let index = self
            .loaded
            .iter()
            .position(|l| l.federation.id == analysis.federation)
            .ok_or_else(|| {
                ExecuteError::Rejected(error(
                    RejectCode::UnknownFederation,
                    "the federation was removed",
                ))
            })?;
        let platform = &mut self.platform;
        let signatures = sign_transaction(&self.loaded[index].keys, analysis, &mut || {
            platform.random_32()
        })
        .map_err(ExecuteError::Rejected)?;
        let today = self.today();
        let response = Response::Signed(SignedTransaction {
            txid: FixedBytes(analysis.txid),
            signatures: signatures.into_iter().map(FixedBytes).collect(),
        });
        let reply = self.finish(id, response, |next| {
            next.policy_state.record(today, analysis.outflow_msat());
            // Notes this transaction spends are no longer waiting to be released.
            for input in &analysis.inputs {
                if let InputKind::Note {
                    branch: Branch::Outgoing,
                    denomination,
                    tweak,
                    ..
                } = input.kind
                {
                    next.prepared.retain(|p| {
                        !(p.federation == federation
                            && p.note.denomination == denomination
                            && p.note.tweak.0 == tweak)
                    });
                }
            }
            if matches!(analysis.intent, Intent::PrepareSend { .. }) {
                for output in &analysis.outputs {
                    if let OutputSummary::Staged {
                        denomination,
                        tweak,
                        ..
                    } = output
                    {
                        next.prepared.push(FederationNote {
                            federation,
                            note: fps_protocol::NoteRef {
                                denomination: *denomination,
                                tweak: FixedBytes(*tweak),
                            },
                        });
                    }
                }
            }
            if let (Some(payment), Some(outpoint)) =
                (&analysis.lightning, analysis.lightning_outpoint())
            {
                next.contracts.push(FundedContract {
                    federation,
                    txid: FixedBytes(outpoint.txid),
                    out_idx: outpoint.out_idx,
                    contract: payment.contract.to_bytes().into(),
                });
            }
            Ok(())
        })?;
        Ok(Approved {
            reply,
            show_token: None,
        })
    }

    fn execute_release(
        &mut self,
        id: RequestId,
        plan: &ReleasePlan,
    ) -> Result<Approved, ExecuteError> {
        let federation = FederationId::from(plan.token.federation);
        let today = self.today();
        let token = Zeroizing::new(plan.token.to_token_string());
        let (returned, shown) = match plan.delivery {
            Delivery::ReturnToPhone => (Some(token.to_string()), None),
            Delivery::ShowOnDevice => (None, Some(token)),
        };
        let response = Response::Released(Released {
            amount_msat: plan.amount_msat,
            notes: plan.notes.len() as u32,
            token: returned,
        });
        // The release is recorded durably before the token can leave the device.
        let reply = self.finish(id, response, |next| {
            next.policy_state.record(today, plan.amount_msat);
            for note in &plan.notes {
                next.prepared
                    .retain(|p| !(p.federation == federation && p.note == *note));
                next.released.push(FederationNote {
                    federation,
                    note: *note,
                });
            }
            Ok(())
        })?;
        Ok(Approved {
            reply,
            show_token: shown,
        })
    }
}

enum ExecuteError {
    Rejected(SignerError),
    Service(ServiceError),
}

impl From<ServiceError> for ExecuteError {
    fn from(error: ServiceError) -> Self {
        Self::Service(error)
    }
}

fn import_review(federation: &Federation, replacing: bool) -> Review {
    const SHOWN_GUARDIANS: usize = 4;
    let title = if replacing {
        "Update federation"
    } else {
        "Add federation"
    };
    let mut review = Review::new(title, 0)
        .line("Name", federation.display_name())
        .line("Federation id", short_hex(&federation.id))
        .line("Guardians", federation.guardians.len().to_string());
    for guardian in federation.guardians.iter().take(SHOWN_GUARDIANS) {
        review = review.line(
            format!("Guardian {}", guardian.peer),
            format!("{}, {}", guardian.name, guardian.url),
        );
    }
    if federation.guardians.len() > SHOWN_GUARDIANS {
        review = review.line(
            "More guardians",
            (federation.guardians.len() - SHOWN_GUARDIANS).to_string(),
        );
    }
    let modules: Vec<&str> = federation
        .modules
        .values()
        .map(|m| m.kind.as_str())
        .collect();
    review = review
        .line("Modules", modules.join(", "))
        .warn("Only the guardian list is checked against the federation id. Check this is the federation you meant to add.");
    if replacing {
        review = review.warn("This replaces the stored settings for this federation.");
    }
    review
}
