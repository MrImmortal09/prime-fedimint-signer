//! Shared by the tests of `Device`: a deterministic platform, storage the test can watch, the Fedimint
//! golden scenario (`crates/signer-core/tests/vectors`), and a Cashu test mint (the helpers of
//! `crates/cashu-core/tests/common`, used as they are).

#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use fps_cashu_signer::{CashuState, Device};
use fps_policy::PolicyConfig;
use fps_protocol::*;
use fps_signer::{Approval, Outcome, Platform, ServiceError, Signer, Storage};
use serde_json::Value;

/// The Cashu test mint of `fps-cashu-core`'s tests: its keysets, proofs with valid signatures and
/// DLEQ proofs, change, leaving outputs, invoices, and the standard send and melt.
#[path = "../../../cashu-core/tests/common/mod.rs"]
pub mod mint;

/// The state file the Fedimint signer wrote at the end of its golden scenario, frozen by
/// `crates/signer/tests/compat_state.rs`.
pub const FROZEN_STATE: &[u8] = include_bytes!("../../../signer/tests/compat/fps-state-v1.cbor");

/// The clock of the frozen state: 2026-01-01T00:00:00Z, UTC day 20454.
pub const NOW: u64 = 1_767_225_600;
pub const DAY: u64 = 86_400;
/// What the frozen state counted as today's Fedimint outflow, in msat.
pub const FROZEN_OUTFLOW_MSAT: u64 = 552_067_136;

/// Randomness from a counter, so every run signs the same bytes, and a fixed clock.
pub struct TestPlatform {
    pub counter: u8,
    pub now: Option<u64>,
}

impl TestPlatform {
    pub fn at(now: u64) -> Self {
        Self {
            counter: 0,
            now: Some(now),
        }
    }
}

impl Platform for TestPlatform {
    fn random_32(&mut self) -> [u8; 32] {
        self.counter = self.counter.wrapping_add(1);
        [self.counter; 32]
    }

    fn now_unix(&self) -> Option<u64> {
        self.now
    }
}

#[derive(Default)]
struct Held {
    bytes: Option<Vec<u8>>,
    /// Every write, in order.
    writes: Vec<Vec<u8>>,
    /// How many times it was read.
    reads: usize,
    fail_writes: bool,
    fail_reads: bool,
}

/// Storage the test keeps a handle on: what it holds, every read and write, and switches to fail.
#[derive(Clone, Default)]
pub struct Watched(Rc<RefCell<Held>>);

impl Watched {
    pub fn holding(bytes: &[u8]) -> Self {
        let watched = Self::default();
        watched.0.borrow_mut().bytes = Some(bytes.to_vec());
        watched
    }

    pub fn bytes(&self) -> Option<Vec<u8>> {
        self.0.borrow().bytes.clone()
    }

    pub fn writes(&self) -> Vec<Vec<u8>> {
        self.0.borrow().writes.clone()
    }

    pub fn write_count(&self) -> usize {
        self.0.borrow().writes.len()
    }

    pub fn read_count(&self) -> usize {
        self.0.borrow().reads
    }

    pub fn fail_writes(&self, fail: bool) {
        self.0.borrow_mut().fail_writes = fail;
    }

    pub fn fail_reads(&self, fail: bool) {
        self.0.borrow_mut().fail_reads = fail;
    }

    /// The Cashu state it holds.
    pub fn cashu_state(&self) -> CashuState {
        CashuState::decode(&self.bytes().expect("the Cashu state was written")).unwrap()
    }
}

impl Storage for Watched {
    fn load(&mut self) -> Result<Option<Vec<u8>>, String> {
        let mut held = self.0.borrow_mut();
        held.reads += 1;
        if held.fail_reads {
            return Err("flash read failed".to_owned());
        }
        Ok(held.bytes.clone())
    }

    fn store(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut held = self.0.borrow_mut();
        if held.fail_writes {
            return Err("flash write failed".to_owned());
        }
        held.bytes = Some(bytes.to_vec());
        held.writes.push(bytes.to_vec());
        Ok(())
    }
}

pub type TestDevice = Device<Watched, TestPlatform, Watched>;
pub type TestSigner = Signer<Watched, TestPlatform>;

/// A device over these storages, with the clock at `now`.
pub fn open_device(
    app_seed: [u8; 32],
    fedimint: &Watched,
    cashu: &Watched,
    now: u64,
) -> TestDevice {
    Device::open(
        app_seed,
        fedimint.clone(),
        TestPlatform::at(now),
        cashu.clone(),
        TestPlatform::at(now),
    )
    .unwrap()
}

pub fn open_signer(app_seed: [u8; 32], fedimint: &Watched, now: u64) -> TestSigner {
    Signer::open(app_seed, fedimint.clone(), TestPlatform::at(now)).unwrap()
}

/// A device for the Cashu test mint, whose proofs are locked to keys of its app seed, over fresh
/// storages.
pub fn cashu_device() -> (TestDevice, Watched, Watched) {
    let (fedimint, cashu) = (Watched::default(), Watched::default());
    let device = open_device(mint::APP_SEED, &fedimint, &cashu, NOW);
    (device, fedimint, cashu)
}

pub fn id(byte: u8) -> RequestId {
    FixedBytes([byte; 16])
}

pub fn message(request: RequestId, body: Request) -> Vec<u8> {
    encode(&Envelope::new(request, body))
}

pub fn send(device: &mut TestDevice, request: RequestId, body: Request) -> Outcome {
    device.handle(&message(request, body))
}

pub fn send_signer(signer: &mut TestSigner, request: RequestId, body: Request) -> Outcome {
    signer.handle(&message(request, body))
}

pub fn info() -> Request {
    Request::Cashu(CashuRequest::Info)
}

pub fn add_mint(descriptor: &CashuMintDescriptor) -> Request {
    Request::Cashu(CashuRequest::AddMint(descriptor.clone()))
}

pub fn sign(request: CashuSignRequest) -> Request {
    Request::Cashu(CashuRequest::Sign(request))
}

/// The bytes of an immediate reply.
pub fn reply_bytes(outcome: Outcome) -> Vec<u8> {
    match outcome {
        Outcome::Reply(bytes) => bytes,
        Outcome::NeedsApproval(approval) => {
            panic!("unexpected approval: {}", approval.review.title)
        }
        Outcome::Released { .. } => panic!("unexpected release"),
    }
}

/// Expects an immediate reply and returns it.
pub fn reply(outcome: Outcome) -> ResponseEnvelope {
    decode_response(&reply_bytes(outcome)).unwrap()
}

pub fn rejection(outcome: Outcome) -> Rejection {
    match reply(outcome).body {
        Response::Rejected(rejection) => rejection,
        other => panic!("expected a rejection, got {other:?}"),
    }
}

pub fn approval(outcome: Outcome) -> Approval {
    match outcome {
        Outcome::NeedsApproval(approval) => approval,
        Outcome::Reply(bytes) => panic!(
            "expected an approval, got {:?}",
            decode_response(&bytes).unwrap().body
        ),
        Outcome::Released { .. } => panic!("expected an approval, got a release"),
    }
}

pub fn cashu_answer(envelope: ResponseEnvelope) -> CashuResponse {
    match envelope.body {
        Response::Cashu(answer) => answer,
        other => panic!("expected a Cashu answer, got {other:?}"),
    }
}

pub fn cashu_info(device: &mut TestDevice, request: RequestId) -> CashuInfo {
    match cashu_answer(reply(send(device, request, info()))) {
        CashuResponse::Info(info) => info,
        other => panic!("expected Cashu info, got {other:?}"),
    }
}

/// Sends a request that needs approval, approves it with the PIN, and returns the answer.
pub fn approved(device: &mut TestDevice, request: RequestId, body: Request) -> ResponseEnvelope {
    let waiting = approval(send(device, request, body));
    assert_eq!(waiting.request, request);
    decode_response(&device.approve(&request, true).unwrap().reply).unwrap()
}

/// Approves an `AddMint` and returns the lock material.
pub fn added(
    device: &mut TestDevice,
    request: RequestId,
    descriptor: &CashuMintDescriptor,
) -> CashuLockMaterial {
    match cashu_answer(approved(device, request, add_mint(descriptor))) {
        CashuResponse::MintAdded(material) => material,
        other => panic!("expected lock material, got {other:?}"),
    }
}

/// Approves a `Sign` and returns the signature.
pub fn signed(device: &mut TestDevice, request: RequestId, spend: CashuSignRequest) -> CashuSigned {
    match cashu_answer(approved(device, request, sign(spend))) {
        CashuResponse::Signed(signed) => signed,
        other => panic!("expected a signature, got {other:?}"),
    }
}

/// The Fedimint golden scenario: what the real fedimint crates built, for one app seed.
pub struct Golden {
    pub vectors: Value,
    pub app_seed: [u8; 32],
    pub descriptor: FederationDescriptor,
}

fn hex_bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap()).unwrap()
}

impl Golden {
    pub fn load() -> Self {
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../signer-core/tests/vectors/fedimint-0.13.0-beta.1.json"
        ))
        .unwrap();
        let scenario = &vectors["scenario"];
        let app_seed = hex_bytes(&scenario["app_seed"]).try_into().unwrap();
        let descriptor =
            ciborium::from_reader(hex_bytes(&scenario["descriptor"]).as_slice()).unwrap();
        Self {
            vectors,
            app_seed,
            descriptor,
        }
    }

    pub fn federation(&self) -> FederationId {
        self.descriptor.federation
    }

    fn case(&self, name: &str) -> &Value {
        self.vectors["scenario"]["transactions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == name)
            .unwrap_or_else(|| panic!("no golden case {name}"))
    }

    pub fn request(&self, name: &str) -> SignRequest {
        ciborium::from_reader(hex_bytes(&self.case(name)["request"]).as_slice()).unwrap()
    }

    pub fn sign(&self, name: &str) -> Request {
        Request::SignTransaction(self.request(name))
    }

    /// The mint signatures the scenario federation issued for a case's outgoing notes.
    pub fn issued(&self, name: &str) -> Vec<ReleaseNote> {
        self.case(name)["issued"]
            .as_array()
            .unwrap()
            .iter()
            .map(|note| ReleaseNote {
                denomination: note["denomination"].as_u64().unwrap() as u8,
                tweak: FixedBytes(hex_bytes(&note["tweak"]).try_into().unwrap()),
                signature: FixedBytes(hex_bytes(&note["signature"]).try_into().unwrap()),
            })
            .collect()
    }

    pub fn release(&self, case: &str, delivery: Delivery) -> Request {
        Request::ReleaseNotes(ReleaseRequest {
            federation: self.federation(),
            notes: self.issued(case),
            include_invite: true,
            delivery,
        })
    }
}

/// The policy of the frozen state: every setting differs from its default.
pub fn compat_policy() -> PolicyConfig {
    PolicyConfig {
        max_outflow_per_request_msat: Some(1_000_000_000),
        max_outflow_per_day_msat: Some(600_000_000),
        pin_above_msat: Some(10_000_000),
        max_routing_fee_base_msat: 200_000,
        max_routing_fee_ppm: 20_000,
        verify_note_signatures: false,
        verify_release_signatures: false,
        release_prepared_without_review: false,
        max_inputs: 32,
        max_outputs: 48,
    }
}

/// What `Device` and the bare `Signer` have in common, so one driver runs both.
pub trait Prime {
    fn handle(&mut self, bytes: &[u8]) -> Outcome;
    fn approve(
        &mut self,
        request: &RequestId,
        pin_verified: bool,
    ) -> Result<fps_signer::Approved, ServiceError>;
    fn reject(&mut self, request: &RequestId) -> Result<Vec<u8>, ServiceError>;
    fn pending(&self) -> Option<Approval>;
    fn set_policy(&mut self, policy: PolicyConfig) -> Result<(), ServiceError>;
    fn forget_federation(&mut self, federation: &FederationId) -> Result<(), ServiceError>;
    fn federations(&self) -> Vec<FederationSummary>;
}

impl Prime for TestSigner {
    fn handle(&mut self, bytes: &[u8]) -> Outcome {
        Signer::handle(self, bytes)
    }

    fn approve(
        &mut self,
        request: &RequestId,
        pin_verified: bool,
    ) -> Result<fps_signer::Approved, ServiceError> {
        Signer::approve(self, request, pin_verified)
    }

    fn reject(&mut self, request: &RequestId) -> Result<Vec<u8>, ServiceError> {
        Signer::reject(self, request)
    }

    fn pending(&self) -> Option<Approval> {
        Signer::pending(self).cloned()
    }

    fn set_policy(&mut self, policy: PolicyConfig) -> Result<(), ServiceError> {
        Signer::set_policy(self, policy)
    }

    fn forget_federation(&mut self, federation: &FederationId) -> Result<(), ServiceError> {
        Signer::forget_federation(self, federation)
    }

    fn federations(&self) -> Vec<FederationSummary> {
        Signer::federations(self)
    }
}

impl Prime for TestDevice {
    fn handle(&mut self, bytes: &[u8]) -> Outcome {
        Device::handle(self, bytes)
    }

    fn approve(
        &mut self,
        request: &RequestId,
        pin_verified: bool,
    ) -> Result<fps_signer::Approved, ServiceError> {
        Device::approve(self, request, pin_verified)
    }

    fn reject(&mut self, request: &RequestId) -> Result<Vec<u8>, ServiceError> {
        Device::reject(self, request)
    }

    fn pending(&self) -> Option<Approval> {
        Device::pending(self).cloned()
    }

    // The app reaches these through `Deref`, as here.
    fn set_policy(&mut self, policy: PolicyConfig) -> Result<(), ServiceError> {
        (**self).set_policy(policy)
    }

    fn forget_federation(&mut self, federation: &FederationId) -> Result<(), ServiceError> {
        (**self).forget_federation(federation)
    }

    fn federations(&self) -> Vec<FederationSummary> {
        (**self).federations()
    }
}

/// A Cashu state with something in every field: the test mint pinned, a signed send, a rejected
/// melt and a cancelled send stored, and the send's outflow counted.
///
/// Its answers sit under ids that Fedimint traffic in these tests uses too, so a Fedimint answer
/// taken from Cashu's ring shows as a difference: 2 and 3 (the signed send and the rejected melt),
/// whose Fedimint answers `differential.rs` stores and asks for with `GetResult`, so a lookup in
/// Cashu first would show as well, and 41 (the cancelled send), which it sends while a Fedimint
/// review waits and which `isolation.rs` does not use for Fedimint. The `AddMint` (0xc0) and the
/// `Cancel` (0xc4) leave no answer under their own ids. None is 99, the request `GetResult` and
/// `Cancel` name when neither backend should have it.
pub fn populated_cashu_state() -> Vec<u8> {
    let fixture = mint::Fixture::new();
    let (mut device, fedimint, cashu) = cashu_device();
    added(&mut device, id(0xc0), &fixture.descriptor);
    signed(&mut device, id(2), mint::send(&fixture));
    approval(send(&mut device, id(3), sign(mint::melt(&fixture))));
    device.reject(&id(3)).unwrap();
    approval(send(&mut device, id(41), sign(mint::send(&fixture))));
    assert_eq!(
        reply(send(
            &mut device,
            id(0xc4),
            Request::Cancel { request: id(41) }
        ))
        .body,
        Response::Cancelled
    );
    assert_eq!(fedimint.write_count(), 0);
    let state = cashu.cashu_state();
    assert_eq!(state.mints.len(), 1);
    assert_eq!(state.results.len(), 3);
    assert_eq!(state.outflow.outflow_msat, 9_000);
    cashu.bytes().unwrap()
}
