//! What the device persists between sessions.

use fps_policy::{PolicyConfig, PolicyState};
use fps_protocol::{ByteBuf, FederationDescriptor, FederationId, NoteRef, RequestId};
use fps_signer_core::fedimint::OutPoint;
use fps_signer_core::Ledger;
use serde::{Deserialize, Serialize};

/// Format version of [`State`].
pub const STATE_VERSION: u16 = 1;

/// How many final responses are kept for replay and `GetResult`.
pub const MAX_STORED_RESULTS: usize = 32;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u16,
    pub federations: Vec<FederationDescriptor>,
    /// Outgoing-branch notes whose keys were given away in a token.
    pub released: Vec<FederationNote>,
    /// Outgoing-branch notes staged by an approved "prepare send", not yet released.
    pub prepared: Vec<FederationNote>,
    /// LNv2 contracts this device funded, by outpoint, so refunds can be checked against them.
    pub contracts: Vec<FundedContract>,
    /// Final responses of recent requests, oldest first.
    pub results: Vec<StoredResult>,
    pub policy: PolicyConfig,
    pub policy_state: PolicyState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FederationNote {
    pub federation: FederationId,
    pub note: NoteRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FundedContract {
    pub federation: FederationId,
    pub txid: fps_protocol::FixedBytes<32>,
    pub out_idx: u64,
    /// Consensus encoding of the LNv2 `OutgoingContract`.
    pub contract: ByteBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredResult {
    pub request: RequestId,
    /// The encoded response envelope.
    pub response: ByteBuf,
}

impl State {
    pub fn new() -> Self {
        Self {
            version: STATE_VERSION,
            ..Self::default()
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        ciborium::into_writer(self, &mut out).expect("serialising to a Vec cannot fail");
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let state: Self = ciborium::from_reader(bytes).map_err(|error| error.to_string())?;
        if state.version != STATE_VERSION {
            return Err(format!(
                "stored state is version {}, this build reads version {STATE_VERSION}",
                state.version
            ));
        }
        Ok(state)
    }

    pub fn stored_result(&self, request: &RequestId) -> Option<&[u8]> {
        self.results
            .iter()
            .find(|stored| stored.request == *request)
            .map(|stored| stored.response.as_slice())
    }

    pub fn store_result(&mut self, request: RequestId, response: Vec<u8>) {
        self.results.retain(|stored| stored.request != request);
        self.results.push(StoredResult {
            request,
            response: response.into(),
        });
        if self.results.len() > MAX_STORED_RESULTS {
            let excess = self.results.len() - MAX_STORED_RESULTS;
            self.results.drain(..excess);
        }
    }

    pub fn is_prepared(&self, federation: &FederationId, note: &NoteRef) -> bool {
        self.prepared
            .iter()
            .any(|p| p.federation == *federation && p.note == *note)
    }
}

impl Ledger for State {
    fn is_released(&self, federation: &[u8; 32], denomination: u8, tweak: &[u8; 16]) -> bool {
        self.released.iter().any(|released| {
            released.federation.0 == *federation
                && released.note.denomination == denomination
                && released.note.tweak.0 == *tweak
        })
    }

    fn approved_contract(&self, federation: &[u8; 32], outpoint: &OutPoint) -> Option<Vec<u8>> {
        self.contracts
            .iter()
            .find(|funded| {
                funded.federation.0 == *federation
                    && funded.txid.0 == outpoint.txid
                    && funded.out_idx == outpoint.out_idx
            })
            .map(|funded| funded.contract.0.clone())
    }
}
