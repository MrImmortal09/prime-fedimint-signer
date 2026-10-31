//! The Cashu service: Cashu requests, their approvals, and `fps-cashu.cbor`.
//!
//! It works like the Fedimint signer: a request is checked, its review waits for the user, and the
//! final answer is written durably, with the answers kept for replay, before it leaves. Its state is
//! its own ([`CashuState`]); what it needs from the Fedimint signer (whether a review waits, the
//! spending policy, Fedimint's count of the day's outflow) it is handed for each request, read-only.
//!
//! The state is read on the first request that needs it, not when the service opens, so opening the
//! device does no Cashu work before the Fedimint signer answers.

use fps_cashu_core::keys::mint_id;
use fps_cashu_core::{
    analyze, review_add_mint, sign_spend, validate_mint, CashuAnalysis, MintKeys, PinnedMint,
};
use fps_policy::{day_of, Decision, PolicyConfig, PolicyState, Routing, Spend};
use fps_protocol::{
    encode, CashuInfo, CashuMintDescriptor, CashuMintSummary, CashuRequest, CashuResponse,
    CashuSignRequest, CashuSigned, CashuSpendKind, Envelope, FixedBytes, RejectCode, Rejection,
    RequestId, Response, CASHU_KEY_SCHEME,
};
use fps_signer::{Approval, ApprovalKind, Approved, Outcome, Platform, ServiceError, Storage};
use fps_signer_core::error::SignerError;
use fps_signer_core::invoice::parse_invoice;
use zeroize::Zeroizing;

use crate::state::{pinned_descriptor, CashuState, MAX_MINTS, MAX_PINNED_KEYS};

/// What the Cashu service reads from the Fedimint signer for one request.
pub(crate) struct Fedimint<'a> {
    /// The Fedimint request waiting for approval, if one is.
    pub waiting: Option<RequestId>,
    /// The spending policy, from `Signer::policy`.
    pub policy: &'a PolicyConfig,
    /// Fedimint's count of the day's outflow, from the signer's state.
    pub outflow: &'a PolicyState,
}

/// A mint Prime has pinned, with its keys.
struct Mint {
    pinned: PinnedMint,
    keys: MintKeys,
}

/// The service once its state is read.
struct Ready {
    state: CashuState,
    /// In the order of `state.mints`.
    mints: Vec<Mint>,
}

impl Ready {
    fn mint(&self, id: &[u8; 32]) -> Option<&Mint> {
        self.mints.iter().find(|mint| mint.pinned.id == *id)
    }
}

enum Status {
    /// Not read yet: no request has needed it since the service opened.
    Unread,
    Ready(Ready),
    /// The state could not be read, for this reason. Cashu stays off and the file is left as it is.
    Off(String),
}

enum Action {
    AddMint(Box<PinnedMint>),
    Sign {
        mint: [u8; 32],
        analysis: Box<CashuAnalysis>,
    },
}

struct Pending {
    action: Action,
    approval: Approval,
}

enum Step {
    /// Answer, without storing it for replay.
    Reply(Response),
    Approve(Pending),
}

enum ExecuteError {
    /// Answered with this refusal; the request no longer waits.
    Rejected(SignerError),
    /// Nothing was committed; the request keeps waiting.
    Service(ServiceError),
}

impl From<ServiceError> for ExecuteError {
    fn from(error: ServiceError) -> Self {
        Self::Service(error)
    }
}

pub(crate) struct CashuService<C: Storage, P: Platform> {
    app_seed: Zeroizing<[u8; 32]>,
    storage: C,
    platform: P,
    status: Status,
    pending: Option<Pending>,
}

fn error(code: RejectCode, message: impl Into<String>) -> SignerError {
    SignerError::new(code, message)
}

/// `Busy`, worded as the Fedimint signer words it, so the phone sees one kind of answer whichever
/// backend's review waits.
pub(crate) fn busy(waiting: &RequestId) -> Rejection {
    Rejection {
        code: RejectCode::Busy,
        message: format!(
            "request {} is waiting for approval on the device",
            waiting.to_hex()
        ),
    }
}

pub(crate) fn envelope(id: RequestId, response: Response) -> Vec<u8> {
    encode(&Envelope::new(id, response))
}

fn cashu(response: CashuResponse) -> Response {
    Response::Cashu(response)
}

/// Reads the state, checking every stored mint again as `AddMint` checked it.
fn load(app_seed: &[u8; 32], bytes: &[u8]) -> Result<Ready, String> {
    let state = CashuState::decode(bytes)?;
    let mut mints: Vec<Mint> = Vec::with_capacity(state.mints.len());
    for (index, stored) in state.mints.iter().enumerate() {
        let pinned = validate_mint(stored, None)
            .map_err(|error| format!("stored mint {index} is not valid: {}", error.message))?;
        if pinned_descriptor(&pinned) != *stored {
            return Err(format!(
                "stored mint {index} is not in the form this build pins"
            ));
        }
        if mints.iter().any(|mint| mint.pinned.id == pinned.id) {
            return Err(format!("stored mint {index} is stored twice"));
        }
        let keys = MintKeys::derive(app_seed, &pinned.url)
            .map_err(|error| format!("stored mint {index} has no keys: {error}"))?;
        mints.push(Mint { pinned, keys });
    }
    Ok(Ready { state, mints })
}

/// How many keys a pinned mint holds, over all its keysets.
fn pinned_keys(mint: &PinnedMint) -> usize {
    mint.keysets
        .iter()
        .fold(0, |keys, keyset| keys.saturating_add(keyset.keys.len()))
}

/// Writes `next` durably, then makes it the state.
fn commit<C: Storage>(
    storage: &mut C,
    ready: &mut Ready,
    next: CashuState,
) -> Result<(), ServiceError> {
    let bytes = next.encode().map_err(ServiceError::Storage)?;
    storage.store(&bytes).map_err(ServiceError::Storage)?;
    ready.state = next;
    Ok(())
}

impl<C: Storage, P: Platform> CashuService<C, P> {
    /// Opens the service over `storage` without reading it: the state is read on first use. Never
    /// fails.
    pub(crate) fn open(app_seed: &[u8; 32], storage: C, platform: P) -> Self {
        Self {
            app_seed: Zeroizing::new(*app_seed),
            storage,
            platform,
            status: Status::Unread,
            pending: None,
        }
    }

    /// Reads the stored state, or starts fresh state if there is none, the first time it is
    /// needed. A state that cannot be read turns Cashu off until the app opens again.
    fn read(&mut self) {
        if !matches!(self.status, Status::Unread) {
            return;
        }
        self.status = match self.storage.load() {
            Ok(None) => Status::Ready(Ready {
                state: CashuState::new(),
                mints: Vec::new(),
            }),
            Ok(Some(bytes)) => match load(&self.app_seed, &bytes) {
                Ok(ready) => Status::Ready(ready),
                Err(reason) => Status::Off(reason),
            },
            Err(error) => Status::Off(format!("reading the Cashu state failed: {error}")),
        };
    }

    /// Why Cashu is off, or `None` while it works. Reads the state if no request has yet.
    pub(crate) fn off(&mut self) -> Option<&str> {
        self.read();
        match &self.status {
            Status::Off(reason) => Some(reason),
            Status::Unread | Status::Ready(_) => None,
        }
    }

    /// The pinned mints. Reads the state if no request has yet.
    pub(crate) fn mints(&mut self) -> Vec<CashuMintSummary> {
        self.read();
        self.listed()
    }

    /// The pinned mints as `Info` lists them: none while Cashu is off.
    fn listed(&self) -> Vec<CashuMintSummary> {
        match &self.status {
            Status::Ready(ready) => ready
                .mints
                .iter()
                .map(|mint| mint.pinned.summary())
                .collect(),
            Status::Unread | Status::Off(_) => Vec::new(),
        }
    }

    /// The Cashu request waiting for approval, if any.
    pub(crate) fn pending(&self) -> Option<&Approval> {
        self.pending.as_ref().map(|pending| &pending.approval)
    }

    pub(crate) fn is_waiting(&self, request: &RequestId) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.approval.request == *request)
    }

    fn stored_result(&self, request: &RequestId) -> Option<&[u8]> {
        match &self.status {
            Status::Ready(ready) => ready.state.stored_result(request),
            Status::Unread | Status::Off(_) => None,
        }
    }

    fn ready(&self) -> Result<&Ready, SignerError> {
        match &self.status {
            Status::Ready(ready) => Ok(ready),
            Status::Off(reason) => Err(off_error(reason)),
            // Every request reads the state before it is dispatched.
            Status::Unread => Err(error(
                RejectCode::Internal,
                "the Cashu state has not been read",
            )),
        }
    }

    fn today(&self) -> Option<u64> {
        self.platform.now_unix().map(day_of)
    }

    /// Handles one Cashu request.
    pub(crate) fn handle(
        &mut self,
        id: RequestId,
        request: CashuRequest,
        fedimint: &Fedimint,
    ) -> Outcome {
        self.read();
        // A request seen before gets its stored answer again, never a second signature.
        if let Some(stored) = self.stored_result(&id) {
            return Outcome::Reply(stored.to_vec());
        }
        match self.dispatch(id, request, fedimint) {
            Ok(Step::Reply(response)) => Outcome::Reply(envelope(id, response)),
            Ok(Step::Approve(pending)) => {
                let approval = pending.approval.clone();
                self.pending = Some(pending);
                Outcome::NeedsApproval(approval)
            }
            Err(refusal) => {
                Outcome::Reply(envelope(id, Response::Rejected(refusal.to_rejection())))
            }
        }
    }

    fn dispatch(
        &self,
        id: RequestId,
        request: CashuRequest,
        fedimint: &Fedimint,
    ) -> Result<Step, SignerError> {
        match request {
            CashuRequest::Info => Ok(Step::Reply(cashu(CashuResponse::Info(CashuInfo {
                scheme: CASHU_KEY_SCHEME.to_owned(),
                mints: self.listed(),
            })))),
            CashuRequest::AddMint(descriptor) => self.add_mint(id, &descriptor, fedimint),
            CashuRequest::Sign(request) => self.sign(id, &request, fedimint),
        }
    }

    /// One review at a time, across both backends.
    fn ensure_idle(&self, fedimint: &Fedimint) -> Result<(), SignerError> {
        let waiting = fedimint
            .waiting
            .or_else(|| self.pending().map(|approval| approval.request));
        match waiting {
            Some(waiting) => {
                let busy = busy(&waiting);
                Err(error(busy.code, busy.message))
            }
            None => Ok(()),
        }
    }

    fn add_mint(
        &self,
        id: RequestId,
        descriptor: &CashuMintDescriptor,
        fedimint: &Fedimint,
    ) -> Result<Step, SignerError> {
        let ready = self.ready()?;
        self.ensure_idle(fedimint)?;
        // What is pinned for this mint already, so an update can only add to it.
        let existing = ready.mint(&mint_id(&descriptor.url));
        let pinned = validate_mint(descriptor, existing.map(|mint| &mint.pinned))?;
        if existing.is_none() && ready.mints.len() >= MAX_MINTS {
            return Err(error(
                RejectCode::TooLarge,
                format!("this device holds at most {MAX_MINTS} mints"),
            ));
        }
        // An update replaces what is pinned for its mint, so that mint counts as updated.
        let keys = ready
            .mints
            .iter()
            .filter(|mint| mint.pinned.id != pinned.id)
            .fold(pinned_keys(&pinned), |keys, mint| {
                keys.saturating_add(pinned_keys(&mint.pinned))
            });
        if keys > MAX_PINNED_KEYS {
            return Err(error(
                RejectCode::TooLarge,
                format!(
                    "this device pins at most {MAX_PINNED_KEYS} keys over all its mints, and this would make {keys}"
                ),
            ));
        }
        let review = review_add_mint(&pinned, existing.map(|mint| &mint.pinned));
        Ok(Step::Approve(Pending {
            approval: Approval {
                request: id,
                kind: ApprovalKind::Cashu,
                review,
                require_pin: false,
            },
            action: Action::AddMint(Box::new(pinned)),
        }))
    }

    fn sign(
        &self,
        id: RequestId,
        request: &CashuSignRequest,
        fedimint: &Fedimint,
    ) -> Result<Step, SignerError> {
        let ready = self.ready()?;
        self.ensure_idle(fedimint)?;
        let mint = ready.mint(&request.mint.0).ok_or_else(|| {
            error(
                RejectCode::UnknownFederation,
                "this mint is not added on the device",
            )
        })?;
        let analysis = analyze(request, &mint.pinned, &mint.keys)?;
        let require_pin = self.decide(ready, request, &analysis, fedimint)?;
        Ok(Step::Approve(Pending {
            approval: Approval {
                request: id,
                kind: ApprovalKind::Cashu,
                review: analysis.review.clone(),
                require_pin,
            },
            action: Action::Sign {
                mint: mint.pinned.id,
                analysis: Box::new(analysis),
            },
        }))
    }

    /// The spending policy, by fps-policy's own evaluation: the same per-request limit, PIN
    /// threshold and daily limit as Fedimint. The day's count is Cashu's own plus Fedimint's, so
    /// a Cashu spend cannot take the two together over the daily limit. A melt's fee reserve is
    /// held to the routing fee limit that bounds a Lightning gateway's fee.
    fn decide(
        &self,
        ready: &Ready,
        request: &CashuSignRequest,
        analysis: &CashuAnalysis,
        fedimint: &Fedimint,
    ) -> Result<bool, SignerError> {
        let today = self.today();
        let counted = PolicyState {
            day: today,
            outflow_msat: ready
                .state
                .outflow
                .counter()
                .outflow_on(today)
                .saturating_add(fedimint.outflow.outflow_on(today)),
        };
        let routing = melt_routing(request)?;
        let spend = Spend {
            outflow_msat: analysis.outflow_msat,
            routing,
        };
        match fedimint.policy.evaluate(&counted, today, &spend) {
            Decision::Allow { require_pin } => Ok(require_pin),
            Decision::Deny { reason } => {
                // fps-policy words the routing limit for a Lightning gateway; say what it held here.
                let reason = match routing {
                    Some(routing)
                        if routing.fee_msat
                            > fedimint.policy.routing_fee_limit_msat(routing.invoice_msat) =>
                    {
                        format!(
                            "the mint's fee reserve of {} msat is above the routing fee limit of {} msat",
                            routing.fee_msat,
                            fedimint.policy.routing_fee_limit_msat(routing.invoice_msat)
                        )
                    }
                    _ => reason,
                };
                Err(error(RejectCode::PolicyLimit, reason))
            }
        }
    }

    /// Approves the waiting Cashu request. Fails without side effects if `request` is not the one
    /// waiting, or if the PIN is required and was not verified.
    pub(crate) fn approve(
        &mut self,
        request: &RequestId,
        pin_verified: bool,
    ) -> Result<Approved, ServiceError> {
        let waiting = self
            .pending
            .as_ref()
            .filter(|pending| pending.approval.request == *request)
            .ok_or(ServiceError::NoSuchRequest)?;
        if waiting.approval.require_pin && !pin_verified {
            return Err(ServiceError::PinRequired);
        }
        let Some(pending) = self.pending.take() else {
            return Err(ServiceError::NoSuchRequest);
        };
        let id = pending.approval.request;
        let result = match &pending.action {
            Action::AddMint(pinned) => self.execute_add_mint(id, pinned),
            Action::Sign { mint, analysis } => self.execute_sign(id, mint, analysis),
        };
        match result {
            Ok(reply) => Ok(Approved {
                reply,
                show_token: None,
            }),
            // A failure to sign is answered, not retried.
            Err(ExecuteError::Rejected(refusal)) => Ok(Approved {
                reply: envelope(id, Response::Rejected(refusal.to_rejection())),
                show_token: None,
            }),
            // Nothing was committed: the request stays waiting so the user can try again.
            Err(ExecuteError::Service(failure)) => {
                self.pending = Some(pending);
                Err(failure)
            }
        }
    }

    /// Pins the mint and answers with its lock material. The answer is not stored: sending the
    /// request again asks again, as an export of Fedimint view material does.
    fn execute_add_mint(
        &mut self,
        id: RequestId,
        pinned: &PinnedMint,
    ) -> Result<Vec<u8>, ExecuteError> {
        let Status::Ready(ready) = &mut self.status else {
            return Err(ExecuteError::Rejected(turned_off()));
        };
        let keys = MintKeys::derive(&self.app_seed, &pinned.url)
            .map_err(|failure| ExecuteError::Rejected(failure.into()))?;
        let reply = envelope(id, cashu(CashuResponse::MintAdded(keys.lock_material())));
        let mut next = ready.state.clone();
        next.pin(pinned);
        commit(&mut self.storage, ready, next)?;
        let mint = Mint {
            pinned: pinned.clone(),
            keys,
        };
        match ready
            .mints
            .iter_mut()
            .find(|known| known.pinned.id == pinned.id)
        {
            Some(known) => *known = mint,
            None => ready.mints.push(mint),
        }
        Ok(reply)
    }

    /// Signs, then stores the answer and counts the outflow, durably, before the answer leaves.
    fn execute_sign(
        &mut self,
        id: RequestId,
        mint: &[u8; 32],
        analysis: &CashuAnalysis,
    ) -> Result<Vec<u8>, ExecuteError> {
        let today = self.today();
        let Status::Ready(ready) = &mut self.status else {
            return Err(ExecuteError::Rejected(turned_off()));
        };
        let keys = &ready
            .mint(mint)
            .ok_or_else(|| {
                ExecuteError::Rejected(error(
                    RejectCode::UnknownFederation,
                    "the mint is no longer added",
                ))
            })?
            .keys;
        let signature = sign_spend(keys, analysis, self.platform.random_32())
            .map_err(ExecuteError::Rejected)?;
        let reply = envelope(
            id,
            cashu(CashuResponse::Signed(CashuSigned {
                message_hash: FixedBytes(analysis.message_hash),
                signature: FixedBytes(signature),
            })),
        );
        let mut next = ready.state.clone();
        next.outflow.record(today, analysis.outflow_msat);
        next.store_result(id, reply.clone());
        commit(&mut self.storage, ready, next)?;
        Ok(reply)
    }

    /// Rejects the waiting Cashu request, storing the refusal for replay. The review ends even if
    /// the write fails, as the signer's does: the app stops waiting for a review once the user
    /// rejects it, whatever the result, and the phone, finding no answer, asks again.
    pub(crate) fn reject(&mut self, request: &RequestId) -> Result<Vec<u8>, ServiceError> {
        if !self.is_waiting(request) {
            return Err(ServiceError::NoSuchRequest);
        }
        self.pending = None;
        let reply = envelope(
            *request,
            Response::Rejected(Rejection {
                code: RejectCode::UserRejected,
                message: "rejected on the device".to_owned(),
            }),
        );
        self.store(*request, reply.clone())?;
        Ok(reply)
    }

    /// `GetResult` for a request the Fedimint signer has nothing for: its stored Cashu answer, or
    /// `Pending` under `id` while it waits; `None` if Cashu has nothing either. Reads the state if
    /// no request has yet, since the request may be one answered before the app opened.
    pub(crate) fn result(&mut self, id: RequestId, request: &RequestId) -> Option<Vec<u8>> {
        self.read();
        if let Some(stored) = self.stored_result(request) {
            return Some(stored.to_vec());
        }
        self.is_waiting(request)
            .then(|| envelope(id, Response::Pending))
    }

    /// `Cancel` for a request the Fedimint signer has nothing for: abandons the waiting Cashu
    /// request and stores `Cancelled` as its answer, then answers `Cancelled` under `id`. `None` if
    /// no Cashu request with that id waits, which is always so before the state is read. If the
    /// write fails the request keeps waiting, and the app with it, so the user can still decide.
    pub(crate) fn cancel(&mut self, id: RequestId, request: &RequestId) -> Option<Vec<u8>> {
        if !self.is_waiting(request) {
            return None;
        }
        let answer = match self.store(*request, envelope(*request, Response::Cancelled)) {
            Ok(()) => {
                self.pending = None;
                Response::Cancelled
            }
            Err(failure) => Response::Rejected(Rejection {
                code: RejectCode::Internal,
                message: failure.to_string(),
            }),
        };
        Some(envelope(id, answer))
    }

    /// Stores a final answer, durably. Only a waiting request has one, and a request waits only
    /// once the state is read.
    fn store(&mut self, request: RequestId, reply: Vec<u8>) -> Result<(), ServiceError> {
        let Status::Ready(ready) = &mut self.status else {
            return Err(ServiceError::Storage("Cashu is off".to_owned()));
        };
        let mut next = ready.state.clone();
        next.store_result(request, reply);
        commit(&mut self.storage, ready, next)
    }
}

/// A melt's invoice amount and fee reserve, for the policy's routing fee limit.
fn melt_routing(request: &CashuSignRequest) -> Result<Option<Routing>, SignerError> {
    let CashuSpendKind::Melt {
        invoice,
        fee_reserve,
        ..
    } = &request.kind
    else {
        return Ok(None);
    };
    // The analysis has parsed the invoice and required an amount.
    let invoice_msat = parse_invoice(invoice)?.amount_msat.unwrap_or_default();
    let fee_msat = fee_reserve
        .checked_mul(1000)
        .ok_or_else(|| error(RejectCode::InvalidTransaction, "amounts overflow"))?;
    Ok(Some(Routing {
        invoice_msat,
        fee_msat,
    }))
}

/// For the paths only a waiting request reaches, which Cashu never has while it is off.
fn turned_off() -> SignerError {
    error(RejectCode::Internal, "Cashu is off on this device")
}

fn off_error(reason: &str) -> SignerError {
    error(
        RejectCode::Internal,
        format!(
            "Cashu is off on this device because its state could not be read ({reason}); Fedimint is not affected"
        ),
    )
}
