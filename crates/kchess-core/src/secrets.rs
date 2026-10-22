//! The profile's credentials. Lichess tokens are sealed with AES-256-GCM under a key that the OS
//! credential store keeps (Keychain on macOS, Credential Manager on Windows, Secret Service on
//! Linux), so every KChess app that opens a profile, Electron or GPUI, opens the same tokens.
//!
//! Sealed text starts with `kcs1:`. Text without it was sealed by an earlier release's host
//! (`CorePlatform.secrets`, Electron's `safeStorage`): that host still opens it, and the core
//! reseals it once (`Core::reseal_tokens`).
//!
//! The key is named after the profile's `credentials.id`, a random id kept in the profile, so two
//! profiles on one machine never share a key and a copied profile does not find one. Under the
//! test switch (`KCHESS_STORE_DEBUG=1`) the key is a file in the profile instead, so automated
//! runs never touch the user's credential store.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{CoreError, Result};

/// What every sealed credential starts with.
pub const SEALED: &str = "kcs1:";
/// The credential store's service name for every KChess key.
const SERVICE: &str = "KChess";
/// The profile file that names its key.
const ID_FILE: &str = "credentials.id";
/// The key file used instead of the credential store under the test switch.
const TEST_KEY_FILE: &str = "credentials.test-key";
const NONCE: usize = 12;
const TAG: usize = 16;

const UNAVAILABLE: &str =
    "OS credential storage is unavailable. Unlock the system keychain and retry.";

/// Whether `text` was sealed by `Credentials`, rather than by an earlier release's host.
pub fn is_sealed(text: &str) -> bool {
    text.starts_with(SEALED)
}

/// Where a profile's key is kept.
pub trait KeyStore: Send + Sync {
    /// Whether the store can be used at all. Never prompts.
    fn available(&self) -> bool;
    /// The stored key, or `None` only when the store answers that there is none. A locked, denied
    /// or failing store is an error: replacing the key then would make every saved token unreadable.
    fn read(&self) -> Result<Option<String>>;
    /// Store the key, replacing any other.
    fn write(&self, key: &str) -> Result<()>;
}

/// A profile's credential sealing.
pub struct Credentials {
    keys: Box<dyn KeyStore>,
    key: Mutex<Option<[u8; 32]>>,
}

impl Credentials {
    /// The credentials of the profile at `data_dir`: the OS credential store, or a key file in
    /// the profile under the test switch.
    pub fn for_profile(data_dir: &Path) -> Credentials {
        if crate::store::debug::enabled() {
            return Credentials::new(Box::new(FileKeys(data_dir.join(TEST_KEY_FILE))));
        }
        Credentials::new(Box::new(SystemKeys {
            data_dir: data_dir.to_path_buf(),
        }))
    }

    pub fn new(keys: Box<dyn KeyStore>) -> Credentials {
        Credentials {
            keys,
            key: Mutex::new(None),
        }
    }

    /// `SecretStore.available()`: whether tokens can be sealed here.
    pub fn available(&self) -> bool {
        self.keys.available()
    }

    /// Seal a token; creates the profile's key on first use.
    pub fn seal(&self, plain: &str) -> Result<String> {
        if !self.available() {
            return Err(CoreError::new("OS credential encryption is unavailable."));
        }
        let key = self.key(true)?;
        let mut nonce = [0u8; NONCE];
        getrandom::fill(&mut nonce).map_err(|e| CoreError::wrap("credential nonce", e))?;
        let sealed = cipher(&key)
            .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
            .map_err(|_| CoreError::new("The credential could not be encrypted."))?;
        let mut data = nonce.to_vec();
        data.extend_from_slice(&sealed);
        Ok(format!("{SEALED}{}", STANDARD.encode(data)))
    }

    /// Open a token `seal` made; never creates a key.
    pub fn open(&self, sealed: &str) -> Result<String> {
        let invalid = || CoreError::new("Invalid encrypted credential.");
        let data = sealed
            .strip_prefix(SEALED)
            .and_then(|body| STANDARD.decode(body).ok())
            .filter(|data| data.len() >= NONCE + TAG)
            .ok_or_else(invalid)?;
        if !self.available() {
            return Err(CoreError::new("OS credential encryption is unavailable."));
        }
        let key = self.key(false)?;
        let plain = cipher(&key)
            .decrypt(Nonce::from_slice(&data[..NONCE]), &data[NONCE..])
            .map_err(|_| {
                CoreError::new("A saved credential could not be read. Reconnect your account.")
            })?;
        String::from_utf8(plain).map_err(|_| invalid())
    }

    fn key(&self, create: bool) -> Result<[u8; 32]> {
        let mut cached = self
            .key
            .lock()
            .map_err(|_| CoreError::new("The credential store is unavailable."))?;
        if let Some(key) = *cached {
            return Ok(key);
        }
        let stored = match self.keys.read()? {
            Some(stored) => stored,
            None if !create => {
                return Err(CoreError::new(
                    "The KChess credential key is unavailable. Reconnect your account.",
                ));
            }
            None => {
                let mut bytes = [0u8; 32];
                getrandom::fill(&mut bytes).map_err(|e| CoreError::wrap("credential key", e))?;
                let created = hex(&bytes);
                self.keys.write(&created)?;
                // A store that dropped the key silently would leave the token unreadable.
                if self.keys.read()?.as_deref() != Some(created.as_str()) {
                    return Err(CoreError::new(
                        "The OS credential key could not be saved. Retry login.",
                    ));
                }
                created
            }
        };
        let key = unhex(&stored).ok_or_else(|| CoreError::new("Invalid KChess credential key."))?;
        *cached = Some(key);
        Ok(key)
    }
}

fn cipher(key: &[u8; 32]) -> Aes256Gcm {
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    if text.len() != 64 || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(key)
}

/// Write `text` to a file only its owner can read.
fn write_private(path: &Path, text: &str) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| CoreError::wrap("credential file", e))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| CoreError::wrap("credential file", e))
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text.trim().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::wrap("credential file", e)),
    }
}

/// The key in the OS credential store, under the profile's id.
struct SystemKeys {
    data_dir: PathBuf,
}

impl SystemKeys {
    /// The store account naming this profile's key; `create` writes a new id when there is none.
    fn account(&self, create: bool) -> Result<Option<String>> {
        let path = self.data_dir.join(ID_FILE);
        let id = match read_optional(&path)? {
            Some(id) => id,
            None if !create => return Ok(None),
            None => {
                let mut bytes = [0u8; 16];
                getrandom::fill(&mut bytes).map_err(|e| CoreError::wrap("credential id", e))?;
                let id = hex(&bytes);
                write_private(&path, &id)?;
                id
            }
        };
        if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(CoreError::new("Invalid KChess credential key."));
        }
        Ok(Some(format!("credentials-{id}")))
    }

    fn entry(account: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, account).map_err(|_| CoreError::new(UNAVAILABLE))
    }
}

impl KeyStore for SystemKeys {
    fn available(&self) -> bool {
        keyring::Entry::store_status().is_ok()
    }

    fn read(&self) -> Result<Option<String>> {
        let Some(account) = self.account(false)? else {
            return Ok(None);
        };
        match Self::entry(&account)?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            // The store's error can carry platform detail, never the key: still, report the kind only.
            Err(_) => Err(CoreError::new(UNAVAILABLE)),
        }
    }

    fn write(&self, key: &str) -> Result<()> {
        let account = self
            .account(true)?
            .ok_or_else(|| CoreError::new(UNAVAILABLE))?;
        Self::entry(&account)?
            .set_password(key)
            .map_err(|_| CoreError::new(UNAVAILABLE))
    }
}

/// The key in a profile file, for automated runs only (`KCHESS_STORE_DEBUG=1`).
struct FileKeys(PathBuf);

impl KeyStore for FileKeys {
    fn available(&self) -> bool {
        true
    }

    fn read(&self) -> Result<Option<String>> {
        read_optional(&self.0)
    }

    fn write(&self, key: &str) -> Result<()> {
        write_private(&self.0, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A credential store in memory; `fail` makes it answer like a locked keychain.
    #[derive(Default)]
    struct Memory {
        key: Mutex<Option<String>>,
        fail: std::sync::atomic::AtomicBool,
        drop_writes: bool,
    }

    impl KeyStore for Arc<Memory> {
        fn available(&self) -> bool {
            true
        }
        fn read(&self) -> Result<Option<String>> {
            if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(CoreError::new(UNAVAILABLE));
            }
            Ok(self.key.lock().unwrap().clone())
        }
        fn write(&self, key: &str) -> Result<()> {
            if !self.drop_writes {
                *self.key.lock().unwrap() = Some(key.to_string());
            }
            Ok(())
        }
    }

    #[test]
    fn a_sealed_token_opens_with_the_same_key_in_another_app() {
        let memory = Arc::new(Memory::default());
        let sealed = Credentials::new(Box::new(Arc::clone(&memory)))
            .seal("lip_token")
            .unwrap();
        assert!(is_sealed(&sealed));
        assert!(!sealed.contains("lip_token"));
        // A second app over the same profile reads the key from the store, not from memory.
        let other = Credentials::new(Box::new(Arc::clone(&memory)));
        assert_eq!(other.open(&sealed).unwrap(), "lip_token");
    }

    #[test]
    fn sealing_twice_gives_different_text() {
        let credentials = Credentials::new(Box::new(Arc::new(Memory::default())));
        assert_ne!(
            credentials.seal("t").unwrap(),
            credentials.seal("t").unwrap()
        );
    }

    #[test]
    fn opening_never_creates_a_key() {
        let memory = Arc::new(Memory::default());
        let sealed = Credentials::new(Box::new(Arc::new(Memory::default())))
            .seal("lip_token")
            .unwrap();
        let error = Credentials::new(Box::new(Arc::clone(&memory)))
            .open(&sealed)
            .unwrap_err();
        assert_eq!(
            error.message,
            "The KChess credential key is unavailable. Reconnect your account."
        );
        assert!(memory.key.lock().unwrap().is_none());
    }

    #[test]
    fn a_locked_store_is_an_error_and_keeps_the_key() {
        let memory = Arc::new(Memory::default());
        *memory.key.lock().unwrap() = Some("ab".repeat(32));
        memory.fail.store(true, std::sync::atomic::Ordering::SeqCst);
        let error = Credentials::new(Box::new(Arc::clone(&memory)))
            .seal("t")
            .unwrap_err();
        assert_eq!(error.message, UNAVAILABLE);
        memory
            .fail
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            memory.key.lock().unwrap().as_deref(),
            Some("ab".repeat(32).as_str())
        );
    }

    #[test]
    fn a_store_that_drops_the_key_fails_the_login() {
        let memory = Arc::new(Memory {
            drop_writes: true,
            ..Memory::default()
        });
        let error = Credentials::new(Box::new(memory)).seal("t").unwrap_err();
        assert_eq!(
            error.message,
            "The OS credential key could not be saved. Retry login."
        );
    }

    #[test]
    fn tampered_or_foreign_text_is_refused() {
        let credentials = Credentials::new(Box::new(Arc::new(Memory::default())));
        let sealed = credentials.seal("lip_token").unwrap();
        let mut bytes = STANDARD.decode(&sealed[SEALED.len()..]).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let tampered = format!("{SEALED}{}", STANDARD.encode(bytes));
        assert_eq!(
            credentials.open(&tampered).unwrap_err().message,
            "A saved credential could not be read. Reconnect your account."
        );
        assert_eq!(
            credentials.open("djEwAAAA").unwrap_err().message,
            "Invalid encrypted credential."
        );
        assert_eq!(
            credentials.open("kcs1:AAAA").unwrap_err().message,
            "Invalid encrypted credential."
        );
    }

    #[test]
    fn an_invalid_stored_key_is_reported() {
        let memory = Arc::new(Memory::default());
        *memory.key.lock().unwrap() = Some("not hex".into());
        let error = Credentials::new(Box::new(memory)).seal("t").unwrap_err();
        assert_eq!(error.message, "Invalid KChess credential key.");
    }

    #[test]
    fn the_test_key_file_is_private_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(TEST_KEY_FILE);
        let sealed = Credentials::new(Box::new(FileKeys(path.clone())))
            .seal("lip_token")
            .unwrap();
        let reopened = Credentials::new(Box::new(FileKeys(path.clone())));
        assert_eq!(reopened.open(&sealed).unwrap(), "lip_token");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_profile_id_names_the_key_and_is_reused() {
        let dir = tempfile::tempdir().unwrap();
        let keys = SystemKeys {
            data_dir: dir.path().to_path_buf(),
        };
        assert_eq!(keys.account(false).unwrap(), None);
        let created = keys.account(true).unwrap().unwrap();
        assert!(created.starts_with("credentials-"));
        assert_eq!(keys.account(false).unwrap(), Some(created));
        std::fs::write(dir.path().join(ID_FILE), "../escape").unwrap();
        assert_eq!(
            keys.account(false).unwrap_err().message,
            "Invalid KChess credential key."
        );
    }
}
