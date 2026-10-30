//! The simulator file bridge.
//!
//! In the hosted KeyOS simulator an app is a native process, so it can share a host directory with
//! programs outside the simulator. The bridge is that directory:
//!
//! | Path | Written by | Contents |
//! |---|---|---|
//! | `inbox/*.cbor` | requester | One request envelope per file, taken in name order |
//! | `outbox/<request id in hex>.cbor` | app | The response envelope |
//! | `app-seed.hex` | test | Pins the app seed, so scenarios built for a known seed replay |
//! | `reset` | test | Present at app start: the app forgets its stored state |
//!
//! Both sides write a file under a temporary name and rename it into place, so neither ever reads
//! half a message. The directory is `$FPS_SIM_DIR`, or `~/.fps-signer-sim`. Without a pinned
//! seed the app reads its seed from KeyOS, as on the device.
//!
//! None of this exists on hardware: the device app compiles the bridge only for the simulator.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fps_protocol::RequestId;

/// Environment variable naming the bridge directory.
pub const ENV_DIR: &str = "FPS_SIM_DIR";
/// Directory name under `$HOME` when [`ENV_DIR`] is unset.
pub const DEFAULT_DIR_NAME: &str = ".fps-signer-sim";

const MESSAGE_EXTENSION: &str = "cbor";
/// A file being written. Readers ignore it.
const STAGING_EXTENSION: &str = "part";
/// A request the app has claimed and is reading.
const CLAIMED_EXTENSION: &str = "taken";

/// How often [`Bridge::wait_response`] looks for the answer.
const POLL: Duration = Duration::from_millis(50);

/// The bridge directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bridge {
    root: PathBuf,
}

impl Bridge {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `$FPS_SIM_DIR`, or `~/.fps-signer-sim`.
    pub fn from_env() -> Self {
        if let Some(dir) = std::env::var_os(ENV_DIR) {
            return Self::new(dir);
        }
        let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
        Self::new(PathBuf::from(home).join(DEFAULT_DIR_NAME))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn inbox(&self) -> PathBuf {
        self.root.join("inbox")
    }

    pub fn outbox(&self) -> PathBuf {
        self.root.join("outbox")
    }

    pub fn seed_file(&self) -> PathBuf {
        self.root.join("app-seed.hex")
    }

    pub fn reset_file(&self) -> PathBuf {
        self.root.join("reset")
    }

    /// Creates the inbox and the outbox.
    pub fn create(&self) -> io::Result<()> {
        fs::create_dir_all(self.inbox())?;
        fs::create_dir_all(self.outbox())
    }

    // The app's side.

    /// Takes the oldest request out of the inbox. The file is claimed by renaming it before it is
    /// read, so a request that cannot be removed afterwards is still never handled twice.
    pub fn take_request(&self) -> io::Result<Option<Vec<u8>>> {
        let Some(path) = oldest_message(&self.inbox())? else {
            return Ok(None);
        };
        let claimed = path.with_extension(CLAIMED_EXTENSION);
        fs::rename(&path, &claimed)?;
        let bytes = fs::read(&claimed);
        let _ = fs::remove_file(&claimed);
        bytes.map(Some)
    }

    /// Writes a response where its requester looks for it, and returns that path. A response
    /// whose envelope cannot be read is filed as `unaddressed.cbor`.
    pub fn write_response(&self, response: &[u8]) -> io::Result<PathBuf> {
        let name = match fps_protocol::decode_response(response) {
            Ok(envelope) => envelope.id.to_hex(),
            Err(_) => "unaddressed".to_owned(),
        };
        let target = self.outbox().join(format!("{name}.{MESSAGE_EXTENSION}"));
        write_atomic(&target, response)?;
        Ok(target)
    }

    /// The seed pinned in `app-seed.hex`, if there is one.
    pub fn pinned_app_seed(&self) -> Result<Option<[u8; 32]>, String> {
        let path = self.seed_file();
        match fs::read_to_string(&path) {
            Ok(text) => parse_seed(&text)
                .map(Some)
                .map_err(|reason| format!("{}: {reason}", path.display())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("cannot read {}: {error}", path.display())),
        }
    }

    /// Whether a test asked for a clean start. Asking is consumed.
    pub fn take_reset(&self) -> bool {
        fs::remove_file(self.reset_file()).is_ok()
    }

    // The requester's side.

    /// Queues a request envelope and returns its inbox file.
    pub fn submit(&self, request: &[u8]) -> io::Result<PathBuf> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = self
            .inbox()
            .join(format!("{nanos:024}-{sequence:08}.{MESSAGE_EXTENSION}"));
        write_atomic(&path, request)?;
        Ok(path)
    }

    /// Takes the response to `request` if it has arrived.
    pub fn take_response(&self, request: &RequestId) -> io::Result<Option<Vec<u8>>> {
        let path = self
            .outbox()
            .join(format!("{}.{MESSAGE_EXTENSION}", request.to_hex()));
        match fs::read(&path) {
            Ok(bytes) => {
                fs::remove_file(&path)?;
                Ok(Some(bytes))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Waits up to `timeout` for the response to `request`.
    pub fn wait_response(
        &self,
        request: &RequestId,
        timeout: Duration,
    ) -> io::Result<Option<Vec<u8>>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(bytes) = self.take_response(request)? {
                return Ok(Some(bytes));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(POLL);
        }
    }

    /// Asks the app to forget its stored state the next time it starts.
    pub fn request_reset(&self) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        fs::write(self.reset_file(), b"")
    }

    /// Pins the simulator app seed, readable by the current user only. Takes effect when the app
    /// next starts.
    pub fn pin_app_seed(&self, seed: &[u8; 32]) -> io::Result<()> {
        write_private(&self.seed_file(), hex::encode(seed).as_bytes())
    }

    /// Removes everything queued in either direction.
    pub fn clear_queues(&self) -> io::Result<()> {
        for dir in [self.inbox(), self.outbox()] {
            match fs::read_dir(&dir) {
                Ok(entries) => {
                    for entry in entries {
                        let path = entry?.path();
                        if path.is_file() {
                            fs::remove_file(path)?;
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

fn parse_seed(text: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(text.trim()).map_err(|error| format!("not hex: {error}"))?;
    bytes
        .try_into()
        .map_err(|bytes: Vec<u8>| format!("{} bytes, expected 32", bytes.len()))
}

/// The complete message file that sorts first, if any.
fn oldest_message(dir: &Path) -> io::Result<Option<PathBuf>> {
    let mut oldest: Option<PathBuf> = None;
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let is_message = path
            .extension()
            .is_some_and(|extension| extension == MESSAGE_EXTENSION);
        if is_message && path.is_file() && oldest.as_ref().is_none_or(|first| path < *first) {
            oldest = Some(path);
        }
    }
    Ok(oldest)
}

/// Writes `bytes` under a staging name and renames it to `path`.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let staged = path.with_extension(STAGING_EXTENSION);
    fs::write(&staged, bytes)?;
    fs::rename(&staged, path)
}

/// Replaces `path` with a file only the current user can read.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(bytes)
}
