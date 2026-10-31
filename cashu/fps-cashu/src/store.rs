//! What the wallet keeps on the phone: one JSON file, `cashu-wallet.json`, in the wallet's
//! directory, written whole under a temporary name (`.part`) and renamed into place.
//!
//! It holds the mints (with the keysets Prime pinned and the lock material `P`, `v` Prime gave for
//! them), the proofs, the next output counter per keyset, unfinished top-ups, sends (with what
//! taking them back needs) and the activity list. Counters are advanced and saved before a request
//! that uses them leaves the phone, so no output is ever derived twice.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::CashuError;

/// The file's name in the wallet's directory.
pub const FILE: &str = "cashu-wallet.json";
/// The file's format.
pub const VERSION: u32 = 1;
/// Most entries the activity list keeps.
const MAX_ACTIVITY: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Data {
    pub version: u32,
    pub mints: Vec<StoredMint>,
    pub proofs: Vec<StoredProof>,
    pub quotes: Vec<StoredQuote>,
    pub sends: Vec<StoredSend>,
    /// Oldest first.
    pub activity: Vec<StoredActivity>,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            version: VERSION,
            mints: Vec::new(),
            proofs: Vec::new(),
            quotes: Vec::new(),
            sends: Vec::new(),
            activity: Vec::new(),
        }
    }
}

/// A mint Prime pinned, with its lock material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredMint {
    /// Canonical URL.
    pub url: String,
    pub name: Option<String>,
    /// `P`, compressed, in hex.
    pub lock_pubkey: String,
    /// `v`, in hex.
    pub view_key: String,
    pub keysets: Vec<StoredKeyset>,
    /// The next output counter of epoch 0, per keyset id in hex.
    pub counters: BTreeMap<String, u64>,
}

/// A keyset as Prime pinned it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredKeyset {
    /// The id, in hex.
    pub id: String,
    pub input_fee_ppk: u64,
    pub final_expiry: Option<u64>,
    pub active: bool,
    /// The key per amount, compressed, in hex.
    pub keys: BTreeMap<u64, String>,
}

/// A NUT-12 proof, in hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredDleq {
    pub e: String,
    pub s: String,
    pub r: String,
}

/// A proof the wallet holds, locked by the template to `P_epoch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredProof {
    pub mint_url: String,
    pub epoch: u32,
    /// The keyset id, in hex.
    pub keyset: String,
    /// Its position in the `fps-cashu-v1` tree of that keyset and epoch.
    pub counter: Option<u64>,
    pub amount: u64,
    /// The secret, verbatim.
    pub secret: String,
    /// `C`, compressed, in hex.
    pub c: String,
    pub dleq: Option<StoredDleq>,
}

/// A top-up whose invoice may not be paid yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredQuote {
    pub id: String,
    pub mint_url: String,
    pub amount_sat: u64,
    pub invoice: String,
    pub created_unix: u64,
}

/// One proof given away in a send: a plain secret, so the wallet can take it back with a swap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SentProof {
    pub keyset: String,
    pub amount: u64,
    pub secret: String,
    /// The blinding factor, in hex.
    pub r: String,
    /// `C`, compressed, in hex.
    pub c: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SendState {
    /// Nobody is known to have redeemed it.
    Pending,
    TakenBack,
    /// Someone redeemed it.
    Redeemed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredSend {
    pub id: String,
    pub mint_url: String,
    pub amount_sat: u64,
    pub fee_sat: u64,
    pub token: String,
    pub proofs: Vec<SentProof>,
    pub created_unix: u64,
    pub state: SendState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredActivity {
    pub kind: String,
    pub amount_sat: u64,
    pub mint_url: String,
    pub unix_time: u64,
    pub detail: String,
}

impl Data {
    pub fn mint(&self, url: &str) -> Option<&StoredMint> {
        self.mints.iter().find(|mint| mint.url == url)
    }

    pub fn mint_mut(&mut self, url: &str) -> Option<&mut StoredMint> {
        self.mints.iter_mut().find(|mint| mint.url == url)
    }

    pub fn balance(&self, url: Option<&str>) -> u64 {
        self.proofs
            .iter()
            .filter(|proof| url.is_none_or(|url| proof.mint_url == url))
            .map(|proof| proof.amount)
            .sum()
    }

    pub fn record(&mut self, activity: StoredActivity) {
        self.activity.push(activity);
        if self.activity.len() > MAX_ACTIVITY {
            let excess = self.activity.len() - MAX_ACTIVITY;
            self.activity.drain(..excess);
        }
    }
}

fn storage(error: impl std::fmt::Display) -> CashuError {
    CashuError::failed(format!("the Cashu wallet's storage: {error}"))
}

/// The file, and the wallet's state as last written.
pub(crate) struct Store {
    path: PathBuf,
    data: Mutex<Data>,
}

impl Store {
    /// Opens the wallet kept in `dir`, creating the directory; a missing file is an empty wallet.
    pub fn open(dir: &Path) -> Result<Self, CashuError> {
        fs::create_dir_all(dir).map_err(storage)?;
        let path = dir.join(FILE);
        let data = match fs::read(&path) {
            Ok(bytes) => {
                let data: Data = serde_json::from_slice(&bytes).map_err(storage)?;
                if data.version != VERSION {
                    return Err(storage(format!(
                        "{FILE} has version {}, and this wallet reads version {VERSION}",
                        data.version
                    )));
                }
                data
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Data::default(),
            Err(error) => return Err(storage(error)),
        };
        Ok(Self {
            path,
            data: Mutex::new(data),
        })
    }

    /// Reads the state.
    pub fn read<T>(&self, read: impl FnOnce(&Data) -> T) -> T {
        read(&self.data.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Changes the state and writes it. If `change` fails or the write does, nothing changes.
    pub fn update<T>(
        &self,
        change: impl FnOnce(&mut Data) -> Result<T, CashuError>,
    ) -> Result<T, CashuError> {
        let mut data = self.data.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = data.clone();
        let result = change(&mut next)?;
        write_atomically(&self.path, &next)?;
        *data = next;
        Ok(result)
    }
}

fn write_atomically(path: &Path, data: &Data) -> Result<(), CashuError> {
    let bytes = serde_json::to_vec_pretty(data).map_err(storage)?;
    let staging = path.with_extension("part");
    fs::write(&staging, bytes).map_err(storage)?;
    fs::rename(&staging, path).map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Data {
        let mut data = Data::default();
        data.mints.push(StoredMint {
            url: "https://testnut.cashu.space".to_owned(),
            name: Some("Testnut mint".to_owned()),
            lock_pubkey: format!("02{}", "11".repeat(32)),
            view_key: "22".repeat(32),
            keysets: vec![StoredKeyset {
                id: format!("01{}", "33".repeat(32)),
                input_fee_ppk: 100,
                final_expiry: None,
                active: true,
                keys: BTreeMap::from([
                    (1, format!("03{}", "44".repeat(32))),
                    (2, format!("02{}", "55".repeat(32))),
                ]),
            }],
            counters: BTreeMap::from([(format!("01{}", "33".repeat(32)), 7)]),
        });
        data.proofs.push(StoredProof {
            mint_url: "https://testnut.cashu.space".to_owned(),
            epoch: 0,
            keyset: format!("01{}", "33".repeat(32)),
            counter: Some(6),
            amount: 2,
            secret: r#"["P2PK",{"nonce":"00","data":"02","tags":[["sigflag","SIG_ALL"]]}]"#
                .to_owned(),
            c: format!("02{}", "66".repeat(32)),
            dleq: Some(StoredDleq {
                e: "77".repeat(32),
                s: "88".repeat(32),
                r: "99".repeat(32),
            }),
        });
        data.quotes.push(StoredQuote {
            id: "quote".to_owned(),
            mint_url: "https://testnut.cashu.space".to_owned(),
            amount_sat: 256,
            invoice: "lnbc2560n1".to_owned(),
            created_unix: 1_800_000_000,
        });
        data.sends.push(StoredSend {
            id: "ab".repeat(16),
            mint_url: "https://testnut.cashu.space".to_owned(),
            amount_sat: 64,
            fee_sat: 1,
            token: "cashuBo2F0".to_owned(),
            proofs: vec![SentProof {
                keyset: format!("01{}", "33".repeat(32)),
                amount: 64,
                secret: "aa".repeat(32),
                r: "bb".repeat(32),
                c: format!("03{}", "cc".repeat(32)),
            }],
            created_unix: 1_800_000_001,
            state: SendState::Pending,
        });
        data.record(StoredActivity {
            kind: "send".to_owned(),
            amount_sat: 64,
            mint_url: "https://testnut.cashu.space".to_owned(),
            unix_time: 1_800_000_001,
            detail: "fee 1 sat".to_owned(),
        });
        data
    }

    #[test]
    fn the_store_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.read(Data::clone), Data::default());

        let data = sample();
        store
            .update(|state| {
                *state = data.clone();
                Ok(())
            })
            .unwrap();
        assert!(!dir.path().join("cashu-wallet.part").exists());

        let reopened = Store::open(dir.path()).unwrap();
        assert_eq!(reopened.read(Data::clone), data);
        assert_eq!(reopened.read(|state| state.balance(None)), 2);

        // A change that fails leaves the state and the file as they were.
        let refused = reopened.update(|state| {
            state.proofs.clear();
            Err::<(), _>(CashuError::invalid("no"))
        });
        assert!(refused.is_err());
        assert_eq!(reopened.read(Data::clone), data);
        assert_eq!(Store::open(dir.path()).unwrap().read(Data::clone), data);
    }
}
