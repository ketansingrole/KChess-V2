//! What the Electron app left in a KChess profile, for the Rust apps that replace it.
//!
//! The Electron renderer kept documents and view preferences in Chromium's `localStorage`
//! (`local_storage`), and Electron's `safeStorage` sealed Lichess logins before the core sealed
//! them itself (`safe_storage`). Every Electron release used the `kchess://app` origin and the
//! app name `KChess`. Nothing here writes to the profile.

pub mod local_storage;
pub mod safe_storage;

/// The Electron app's name: its Keychain item, Secret Service label and `Local State` belong to it.
pub const APP_NAME: &str = "KChess";

/// What went wrong, without key material or token text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn error(text: impl Into<String>) -> Error {
    Error(text.into())
}
