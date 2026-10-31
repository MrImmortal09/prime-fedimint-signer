//! What the signer service needs from Passport Prime: the app seed, randomness, a clock and
//! durable storage.

use fps_signer::{Platform, Storage};

security::use_api!();

/// The signer state, in the app's encrypted `AppData` directory.
const STATE_FILE: &str = "fps-state.cbor";

/// KeyOS's seed for this app, derived from the master seed and the app id: unique to the app, and
/// recovered with the device backup. The first read shows the system's consent prompt, so call it
/// on the main thread with the event loop running.
fn system_app_seed() -> Result<[u8; 32], String> {
    Security::default()
        .app_seed()
        .map(|seed| *seed.as_bytes())
        .map_err(|error| format!("the app seed is unavailable: {error:?}"))
}

/// The 32-byte seed every key derives from.
#[cfg(keyos)]
pub fn app_seed() -> Result<[u8; 32], String> {
    system_app_seed()
}

/// In the simulator a test can pin the seed in the bridge directory, so scenarios built for a
/// known seed replay end to end. Otherwise the simulator's own `GetAppSeed`, as on the device.
#[cfg(not(keyos))]
pub fn app_seed() -> Result<[u8; 32], String> {
    match crate::bridge::pinned_app_seed()? {
        Some(seed) => {
            log::warn!("simulator: using the app seed pinned in the bridge directory");
            Ok(seed)
        }
        None => system_app_seed(),
    }
}

/// Randomness and time.
pub struct DevicePlatform;

impl Platform for DevicePlatform {
    fn random_32(&mut self) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        // On the device the SDK's getrandom reads the hardware TRNG server.
        getrandom::getrandom(&mut bytes).expect("the TRNG is always available");
        bytes
    }

    /// The device has no trusted clock wired for apps yet, so it reports none: invoice expiry is
    /// not checked and the daily limit never resets, which only makes it stricter. The simulator
    /// uses the host clock.
    fn now_unix(&self) -> Option<u64> {
        #[cfg(keyos)]
        {
            None
        }
        #[cfg(not(keyos))]
        {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|elapsed| elapsed.as_secs())
        }
    }
}

/// The signer state as one durable file in `AppData`.
pub struct AppDataStorage;

impl AppDataStorage {
    /// Forgets the stored state, including generations a crash left behind under the names
    /// `durable_file_write` documents. Tests on the simulator use it to start clean.
    #[cfg(not(keyos))]
    pub fn wipe(&self) -> Result<(), String> {
        let files = crate::FileSystem::default();
        for name in [
            STATE_FILE.to_owned(),
            format!("{STATE_FILE}.tmp"),
            format!("{STATE_FILE}.new"),
        ] {
            match files.remove(name, fs::Location::AppData) {
                Ok(()) | Err(fs::Error::FileNotFound) => {}
                Err(error) => return Err(format!("cannot remove the signer state: {error:?}")),
            }
        }
        Ok(())
    }
}

impl Storage for AppDataStorage {
    fn load(&mut self) -> Result<Option<Vec<u8>>, String> {
        match crate::FileSystem::default().durable_file_read(STATE_FILE, fs::Location::AppData) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(fs::Error::FileNotFound) => Ok(None),
            Err(error) => Err(format!("reading the signer state failed: {error:?}")),
        }
    }

    fn store(&mut self, bytes: &[u8]) -> Result<(), String> {
        crate::FileSystem::default()
            .durable_file_write(STATE_FILE, fs::Location::AppData, bytes)
            .map_err(|error| format!("saving the signer state failed: {error:?}"))
    }
}

/// The Cashu state, in a file of its own beside the signer state: Cashu never writes the signer's
/// file, and a Cashu file the app cannot read turns Cashu off and leaves Fedimint working.
const CASHU_FILE: &str = "fps-cashu.cbor";

/// The Cashu service's state as one durable file in `AppData`, written as the signer state is.
pub struct CashuStorage;

impl CashuStorage {
    /// Forgets the stored Cashu state, including generations a crash left behind under the names
    /// `durable_file_write` documents. Tests on the simulator use it to start clean.
    #[cfg(not(keyos))]
    pub fn wipe(&self) -> Result<(), String> {
        let files = crate::FileSystem::default();
        for name in [
            CASHU_FILE.to_owned(),
            format!("{CASHU_FILE}.tmp"),
            format!("{CASHU_FILE}.new"),
        ] {
            match files.remove(name, fs::Location::AppData) {
                Ok(()) | Err(fs::Error::FileNotFound) => {}
                Err(error) => return Err(format!("cannot remove the Cashu state: {error:?}")),
            }
        }
        Ok(())
    }
}

impl Storage for CashuStorage {
    fn load(&mut self) -> Result<Option<Vec<u8>>, String> {
        match crate::FileSystem::default().durable_file_read(CASHU_FILE, fs::Location::AppData) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(fs::Error::FileNotFound) => Ok(None),
            Err(error) => Err(format!("reading the Cashu state failed: {error:?}")),
        }
    }

    fn store(&mut self, bytes: &[u8]) -> Result<(), String> {
        crate::FileSystem::default()
            .durable_file_write(CASHU_FILE, fs::Location::AppData, bytes)
            .map_err(|error| format!("saving the Cashu state failed: {error:?}"))
    }
}
