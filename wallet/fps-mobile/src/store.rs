//! What the wallet keeps on the phone, next to fedimint's own database.
//!
//! | File | Contents |
//! |---|---|
//! | `client.redb` | fedimint's client database |
//! | `phone-secret` | The client's own root secret, 64 random bytes in hex. Receives use it (Lightning claims, deposit addresses); funds stop depending on it once they land |
//! | `prime.cbor` | The view material Prime exported when it paired with the federation |
//! | `sends.json` | Ecash sends, kept until they are reclaimed or forgotten |
//!
//! Every file is written under a temporary name and renamed into place.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use fedimint_core::secp256k1::rand;
use fps_protocol::ViewMaterial;
use serde::{Deserialize, Serialize};

use crate::WalletError;

const DATABASE: &str = "client.redb";
const PHONE_SECRET: &str = "phone-secret";
const PAIRING: &str = "prime.cbor";
const SENDS: &str = "sends.json";

/// An ecash send, from staging on Prime until it is reclaimed or forgotten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredSend {
    /// The staging operation, in hex.
    pub id: String,
    pub amount_msat: u64,
    pub created_unix: u64,
    /// The staged notes, consensus-encoded, in hex.
    pub notes: String,
    /// Prime released the notes as a token.
    pub released: bool,
    /// The token, when Prime gave it to the phone.
    pub token: Option<String>,
}

pub(crate) struct Store {
    dir: PathBuf,
}

fn storage(error: impl std::fmt::Display) -> WalletError {
    WalletError::failed(format!("the wallet's storage: {error}"))
}

/// Writes `bytes` under a temporary name and renames them into place.
fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let staging = path.with_extension("part");
    fs::write(&staging, bytes)?;
    fs::rename(&staging, path)
}

/// `path`'s contents, or `None` if it does not exist.
fn read_if_present(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

impl Store {
    pub fn new(dir: PathBuf) -> Result<Self, WalletError> {
        fs::create_dir_all(&dir).map_err(storage)?;
        Ok(Self { dir })
    }

    pub fn database(&self) -> PathBuf {
        self.dir.join(DATABASE)
    }

    /// Removes the database of a join that did not finish.
    pub fn remove_database(&self) -> Result<(), WalletError> {
        match fs::remove_file(self.database()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(storage(error)),
        }
    }

    /// The client's root secret, made on first use.
    pub fn phone_secret(&self) -> Result<[u8; 64], WalletError> {
        let path = self.dir.join(PHONE_SECRET);
        if let Some(bytes) = read_if_present(&path).map_err(storage)? {
            let text = String::from_utf8(bytes).map_err(storage)?;
            let mut secret = [0; 64];
            hex::decode_to_slice(text.trim(), &mut secret).map_err(storage)?;
            return Ok(secret);
        }
        let mut secret = [0; 64];
        secret[..32].copy_from_slice(&rand::random::<[u8; 32]>());
        secret[32..].copy_from_slice(&rand::random::<[u8; 32]>());
        write_atomically(&path, hex::encode(secret).as_bytes()).map_err(storage)?;
        Ok(secret)
    }

    /// The view material of the federation the wallet paired with Prime, if any.
    pub fn pairing(&self) -> Result<Option<ViewMaterial>, WalletError> {
        read_if_present(&self.dir.join(PAIRING))
            .map_err(storage)?
            .map(|bytes| ciborium::from_reader(bytes.as_slice()).map_err(storage))
            .transpose()
    }

    pub fn save_pairing(&self, material: &ViewMaterial) -> Result<(), WalletError> {
        let mut bytes = Vec::new();
        ciborium::into_writer(material, &mut bytes).map_err(storage)?;
        write_atomically(&self.dir.join(PAIRING), &bytes).map_err(storage)
    }

    pub fn forget_pairing(&self) -> Result<(), WalletError> {
        match fs::remove_file(self.dir.join(PAIRING)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(storage(error)),
        }
    }

    pub fn sends(&self) -> Result<Vec<StoredSend>, WalletError> {
        match read_if_present(&self.dir.join(SENDS)).map_err(storage)? {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(storage),
            None => Ok(Vec::new()),
        }
    }

    fn save_sends(&self, sends: &[StoredSend]) -> Result<(), WalletError> {
        let bytes = serde_json::to_vec_pretty(sends).map_err(storage)?;
        write_atomically(&self.dir.join(SENDS), &bytes).map_err(storage)
    }

    /// Adds `send`, or replaces the send with the same id.
    pub fn put_send(&self, send: StoredSend) -> Result<(), WalletError> {
        let mut sends = self.sends()?;
        sends.retain(|stored| stored.id != send.id);
        sends.push(send);
        self.save_sends(&sends)
    }

    pub fn send(&self, id: &str) -> Result<Option<StoredSend>, WalletError> {
        Ok(self.sends()?.into_iter().find(|send| send.id == id))
    }

    pub fn remove_send(&self, id: &str) -> Result<(), WalletError> {
        let mut sends = self.sends()?;
        sends.retain(|send| send.id != id);
        self.save_sends(&sends)
    }
}
