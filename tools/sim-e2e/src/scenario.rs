//! The canned requests: the golden scenario that devtools builds with the real fedimint crates for
//! the app seed `5e…5e`.

use fps_protocol::{
    FederationDescriptor, FederationId, FixedBytes, NoteRef, ReleaseNote, SignRequest,
};
use fps_signer_core::fedimint::OutPoint;
use fps_signer_core::Ledger;
use serde_json::Value;

const VECTORS: &str =
    include_str!("../../../crates/signer-core/tests/vectors/fedimint-0.13.0-beta.1.json");

pub struct Scenario {
    pub app_seed: [u8; 32],
    pub descriptor: FederationDescriptor,
    pub cases: Vec<Case>,
}

/// One transaction of the scenario.
pub struct Case {
    pub name: String,
    pub request: SignRequest,
    pub txid: [u8; 32],
    /// Outgoing notes that must already be released for the case to be signable.
    pub released: Vec<NoteRef>,
    /// Lightning contracts the device must remember funding.
    pub funded: Vec<(OutPoint, Vec<u8>)>,
    /// The outgoing notes the case stages or reclaims, with their mint signatures.
    pub issued: Vec<ReleaseNote>,
}

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().expect("a hex string")).expect("valid hex")
}

fn array<const N: usize>(value: &Value) -> [u8; N] {
    bytes(value).try_into().expect("the right length")
}

fn cbor<T: serde::de::DeserializeOwned>(value: &Value) -> T {
    ciborium::from_reader(bytes(value).as_slice()).expect("valid CBOR")
}

fn list(value: &Value) -> &[Value] {
    value.as_array().expect("an array")
}

impl Scenario {
    pub fn load() -> Self {
        let vectors: Value = serde_json::from_str(VECTORS).expect("valid vectors");
        let scenario = &vectors["scenario"];
        let cases = list(&scenario["transactions"])
            .iter()
            .map(|case| Case {
                name: case["name"].as_str().expect("a name").to_owned(),
                request: cbor(&case["request"]),
                txid: array(&case["txid"]),
                released: list(&case["released"])
                    .iter()
                    .map(|note| NoteRef {
                        denomination: note["denomination"].as_u64().expect("a number") as u8,
                        tweak: FixedBytes(array(&note["tweak"])),
                    })
                    .collect(),
                funded: list(&case["funded"])
                    .iter()
                    .map(|funded| {
                        let outpoint = OutPoint {
                            txid: array(&funded["txid"]),
                            out_idx: funded["out_idx"].as_u64().expect("a number"),
                        };
                        (outpoint, bytes(&funded["contract"]))
                    })
                    .collect(),
                issued: list(&case["issued"])
                    .iter()
                    .map(|note| ReleaseNote {
                        denomination: note["denomination"].as_u64().expect("a number") as u8,
                        tweak: FixedBytes(array(&note["tweak"])),
                        signature: FixedBytes(array(&note["signature"])),
                    })
                    .collect(),
            })
            .collect();
        Self {
            app_seed: array(&scenario["app_seed"]),
            descriptor: cbor(&scenario["descriptor"]),
            cases,
        }
    }

    pub fn federation(&self) -> FederationId {
        self.descriptor.federation
    }

    pub fn case(&self, name: &str) -> &Case {
        self.cases
            .iter()
            .find(|case| case.name == name)
            .unwrap_or_else(|| panic!("no scenario case {name}"))
    }

    /// Everything the scenario says the device remembers, for checking signatures independently.
    pub fn ledger(&self) -> ScenarioLedger {
        ScenarioLedger {
            federation: self.federation().0,
            released: self
                .cases
                .iter()
                .flat_map(|case| case.released.iter().copied())
                .collect(),
            funded: self
                .cases
                .iter()
                .flat_map(|case| case.funded.iter().cloned())
                .collect(),
        }
    }
}

pub struct ScenarioLedger {
    federation: [u8; 32],
    released: Vec<NoteRef>,
    funded: Vec<(OutPoint, Vec<u8>)>,
}

impl Ledger for ScenarioLedger {
    fn is_released(&self, federation: &[u8; 32], denomination: u8, tweak: &[u8; 16]) -> bool {
        *federation == self.federation
            && self
                .released
                .iter()
                .any(|note| note.denomination == denomination && note.tweak.0 == *tweak)
    }

    fn approved_contract(&self, federation: &[u8; 32], outpoint: &OutPoint) -> Option<Vec<u8>> {
        if *federation != self.federation {
            return None;
        }
        self.funded
            .iter()
            .find(|(funded, _)| funded == outpoint)
            .map(|(_, contract)| contract.clone())
    }
}
