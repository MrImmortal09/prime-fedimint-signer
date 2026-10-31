//! What the Cashu service persists: `fps-cashu.cbor`, a file of its own beside the Fedimint signer's
//! `fps-state.cbor`, with its own version. Cashu never writes the signer's file, and a Cashu file
//! this build cannot read turns Cashu off without touching Fedimint.

use fps_cashu_core::PinnedMint;
use fps_policy::PolicyState;
use fps_protocol::{ByteBuf, CashuKey, CashuKeyset, CashuMintDescriptor, FixedBytes, RequestId};
use serde::{Deserialize, Serialize};

/// Format version of [`CashuState`], separate from the signer's.
pub const CASHU_STATE_VERSION: u16 = 1;

/// How many final Cashu answers are kept for replay and `GetResult`: as many as the Fedimint signer
/// keeps of its own.
pub const MAX_CASHU_RESULTS: usize = 32;

/// Most mints Prime pins.
pub const MAX_MINTS: usize = 16;

/// Most keys Prime pins across all its mints: sixteen mints of 128 keys. It bounds the size of
/// `fps-cashu.cbor`, which every Cashu decision writes whole, and the time to check it when it is
/// read. Without it the per-mint limits of `fps-cashu-core` would allow 16,384 keys.
pub const MAX_PINNED_KEYS: usize = 2_048;

/// The Cashu service's state. Decoding is strict: unknown fields are errors, and the version must
/// be this build's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuState {
    pub version: u16,
    /// Every pinned mint as one descriptor of everything pinned for it, the keysets it no longer
    /// lists included (inactive): what `validate_mint` returned when it was last approved. Each is
    /// validated again when the file is read, and must come out exactly as stored.
    pub mints: Vec<CashuMintDescriptor>,
    /// Final answers of recent Cashu requests, oldest first.
    pub results: Vec<CashuResult>,
    /// Cashu's own count of the day's outflow. The Fedimint signer keeps its count in its own file.
    pub outflow: CashuOutflow,
}

/// A stored final answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuResult {
    pub request: RequestId,
    /// The encoded response envelope.
    pub response: ByteBuf,
}

/// What approved Cashu spends let leave, counted as `fps_policy::PolicyState` counts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashuOutflow {
    /// UTC day of `outflow_msat`, or `None` if it was counted without a clock.
    pub day: Option<u64>,
    pub outflow_msat: u64,
}

impl CashuOutflow {
    /// The count as fps-policy keeps one.
    pub fn counter(&self) -> PolicyState {
        PolicyState {
            day: self.day,
            outflow_msat: self.outflow_msat,
        }
    }

    /// Adds an approved spend's outflow, by `PolicyState::record`.
    pub fn record(&mut self, today: Option<u64>, outflow_msat: u64) {
        let mut counter = self.counter();
        counter.record(today, outflow_msat);
        self.day = counter.day;
        self.outflow_msat = counter.outflow_msat;
    }
}

/// Just the version, read from a file whose other fields this build may not know.
#[derive(Deserialize)]
struct VersionOnly {
    version: u16,
}

impl Default for CashuState {
    fn default() -> Self {
        Self::new()
    }
}

impl CashuState {
    pub fn new() -> Self {
        Self {
            version: CASHU_STATE_VERSION,
            mints: Vec::new(),
            results: Vec::new(),
            outflow: CashuOutflow::default(),
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        ciborium::into_writer(self, &mut out)
            .map_err(|error| format!("cannot encode the Cashu state: {error}"))?;
        Ok(out)
    }

    /// Reads a state file: one CBOR item and nothing after it, of this version, within the limits.
    /// The mints are checked again by the service as it loads them.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut reader = bytes;
        let state: Self = match ciborium::from_reader(&mut reader) {
            Ok(state) => state,
            Err(error) => {
                // Another version may have other fields: name the version rather than a field.
                if let Ok(other) = ciborium::from_reader::<VersionOnly, _>(bytes) {
                    if other.version != CASHU_STATE_VERSION {
                        return Err(version_mismatch(other.version));
                    }
                }
                return Err(format!("the Cashu state does not decode: {error}"));
            }
        };
        if !reader.is_empty() {
            return Err(format!(
                "the Cashu state has {} bytes after its end",
                reader.len()
            ));
        }
        if state.version != CASHU_STATE_VERSION {
            return Err(version_mismatch(state.version));
        }
        if state.mints.len() > MAX_MINTS {
            return Err(format!(
                "the Cashu state holds {} mints, more than {MAX_MINTS}",
                state.mints.len()
            ));
        }
        if state.results.len() > MAX_CASHU_RESULTS {
            return Err(format!(
                "the Cashu state holds {} answers, more than {MAX_CASHU_RESULTS}",
                state.results.len()
            ));
        }
        let keys = state.pinned_keys();
        if keys > MAX_PINNED_KEYS {
            return Err(format!(
                "the Cashu state pins {keys} keys, more than {MAX_PINNED_KEYS}"
            ));
        }
        Ok(state)
    }

    /// How many keys the stored mints pin, over all their keysets.
    pub fn pinned_keys(&self) -> usize {
        self.mints
            .iter()
            .flat_map(|mint| &mint.keysets)
            .fold(0, |keys, keyset| keys.saturating_add(keyset.keys.len()))
    }

    pub fn stored_result(&self, request: &RequestId) -> Option<&[u8]> {
        self.results
            .iter()
            .find(|stored| stored.request == *request)
            .map(|stored| stored.response.as_slice())
    }

    /// Stores `response` as the final answer to `request`, dropping the oldest beyond the limit.
    pub fn store_result(&mut self, request: RequestId, response: Vec<u8>) {
        self.results.retain(|stored| stored.request != request);
        self.results.push(CashuResult {
            request,
            response: response.into(),
        });
        let excess = self.results.len().saturating_sub(MAX_CASHU_RESULTS);
        self.results.drain(..excess);
    }

    /// Pins `mint`, in place of what was pinned for it before.
    pub fn pin(&mut self, mint: &PinnedMint) {
        let descriptor = pinned_descriptor(mint);
        match self
            .mints
            .iter_mut()
            .find(|stored| stored.url == descriptor.url)
        {
            Some(stored) => *stored = descriptor,
            None => self.mints.push(descriptor),
        }
    }
}

fn version_mismatch(version: u16) -> String {
    format!("the Cashu state is version {version}, this build reads version {CASHU_STATE_VERSION}")
}

/// Everything pinned for a mint, as the descriptor that pins exactly that: every keyset with its
/// keys in increasing order of amount, its fee, final expiry and `active` flag.
pub fn pinned_descriptor(mint: &PinnedMint) -> CashuMintDescriptor {
    CashuMintDescriptor {
        url: mint.url.clone(),
        unit: mint.unit.clone(),
        keysets: mint
            .keysets
            .iter()
            .map(|keyset| CashuKeyset {
                id: ByteBuf(keyset.id.clone()),
                // A keyset is pinned only in the mint's unit.
                unit: mint.unit.clone(),
                input_fee_ppk: keyset.input_fee_ppk,
                final_expiry: keyset.final_expiry,
                active: keyset.active,
                keys: keyset
                    .keys
                    .iter()
                    .map(|(amount, key)| CashuKey {
                        amount: *amount,
                        pubkey: FixedBytes(key.serialize()),
                    })
                    .collect(),
            })
            .collect(),
    }
}
