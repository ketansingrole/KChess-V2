//! Where a profile's core is found (`docs/shared-core.md`, "Where it lives"): its socket, the
//! `core.json` that names it, the `core.token` a client proves itself with, and the per-release
//! folder the service binary runs from.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::protocol::hex;

/// The file in the profile that names the running core.
pub const INFO_FILE: &str = "core.json";
/// The file in the profile with the token clients send in `hello`.
pub const TOKEN_FILE: &str = "core.token";

/// What `core.json` says about the running core.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoreInfo {
    pub pid: u32,
    /// The socket path (Unix) or pipe name (Windows).
    pub socket: String,
    pub release: String,
    pub protocol: String,
    /// Milliseconds since the epoch.
    pub started_at: u64,
}

/// The profile's real path, which names its socket: two spellings of one folder share a core.
pub fn canonical(profile: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(profile)
}

/// A short, stable name for a profile.
fn profile_hash(profile: &Path) -> String {
    let mut hash = Sha256::new();
    hash.update(profile.to_string_lossy().as_bytes());
    hex(&hash.finalize()[..8])
}

/// `dir`, made if needed as a `0700` folder only this user may enter; refused when it belongs to
/// someone else or is not a folder.
#[cfg(unix)]
fn private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(dir)?;
    if !metadata.is_dir() || metadata.uid() != rustix::process::getuid().as_raw() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} is not a folder of this user", dir.display()),
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// The socket of `profile`'s core, in this user's private `kchess-<uid>` folder under the
/// user's runtime directory (else the temporary one). Unix socket paths are limited to about
/// 104 bytes, so a long runtime directory falls back to `/tmp`.
#[cfg(unix)]
pub fn socket_path(profile: &Path) -> std::io::Result<PathBuf> {
    let uid = rustix::process::getuid().as_raw();
    let name = format!("core-{}.sock", profile_hash(profile));
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .or_else(|| std::env::var_os("TMPDIR"))
        .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    let mut dir = base.join(format!("kchess-{uid}"));
    if dir.join(&name).as_os_str().len() >= 100 {
        dir = PathBuf::from("/tmp").join(format!("kchess-{uid}"));
    }
    private_dir(&dir)?;
    Ok(dir.join(name))
}

/// The pipe of `profile`'s core, per user.
#[cfg(windows)]
pub fn socket_path(profile: &Path) -> std::io::Result<PathBuf> {
    let user = std::env::var("USERNAME").unwrap_or_default();
    let mut hash = Sha256::new();
    hash.update(user.as_bytes());
    hash.update(profile.to_string_lossy().as_bytes());
    Ok(PathBuf::from(format!(
        r"\\.\pipe\kchess-core-{}",
        hex(&hash.finalize()[..8])
    )))
}

/// Writes `contents` to `path` readable by this user only, replacing it whole.
pub fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension("tmp");
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        std::io::Write::write_all(&mut file, contents)?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)
}

/// A new token for `core.token`: 32 random bytes, as hex.
pub fn new_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(hex(&bytes))
}

/// The token clients of `profile`'s core send.
pub fn read_token(profile: &Path) -> std::io::Result<String> {
    Ok(std::fs::read_to_string(profile.join(TOKEN_FILE))?
        .trim()
        .to_string())
}

/// What `core.json` says, when there is one.
pub fn read_info(profile: &Path) -> Option<CoreInfo> {
    let text = std::fs::read_to_string(profile.join(INFO_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether two tokens are equal, in time that does not depend on where they differ.
pub fn same_token(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    given.len() == expected.len()
        && given
            .iter()
            .zip(expected)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

/// The user's KChess runtime folder: the service binary of each release runs from here, outside
/// any app and any profile (`docs/shared-core.md`, "Where the service runs from").
pub fn runtime_dir() -> Option<PathBuf> {
    // Tests and development builds keep their copies elsewhere.
    if let Some(dir) = std::env::var_os("KCHESS_RUNTIME_DIR") {
        return Some(PathBuf::from(dir).join("core"));
    }
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/KChess Runtime"));
    #[cfg(windows)]
    let base =
        std::env::var_os("LOCALAPPDATA").map(|local| PathBuf::from(local).join("KChess Runtime"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .map(|data| data.join("kchess-runtime"));
    base.map(|base| base.join("core"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_compare_whole() {
        let token = new_token().expect("random bytes");
        assert_eq!(token.len(), 64);
        assert!(same_token(&token, &token));
        assert!(!same_token(&token, &token[..63]));
        assert!(!same_token("a".repeat(64).as_str(), &"b".repeat(64)));
        assert_ne!(token, new_token().expect("random bytes"));
    }

    #[cfg(unix)]
    #[test]
    fn a_profile_has_one_short_socket_in_a_private_folder() {
        use std::os::unix::fs::PermissionsExt;
        let profile = tempfile::tempdir().expect("profile");
        let path = socket_path(profile.path()).expect("socket path");
        assert_eq!(path, socket_path(profile.path()).expect("again"));
        assert!(path.as_os_str().len() < 104);
        let dir = path.parent().expect("folder");
        let mode = std::fs::metadata(dir).expect("folder").permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        let other = tempfile::tempdir().expect("another profile");
        assert_ne!(path, socket_path(other.path()).expect("socket path"));
    }

    #[cfg(unix)]
    #[test]
    fn private_files_are_this_users_only() {
        use std::os::unix::fs::PermissionsExt;
        let profile = tempfile::tempdir().expect("profile");
        let token = profile.path().join(TOKEN_FILE);
        write_private(&token, b"secret\n").expect("written");
        let mode = std::fs::metadata(&token)
            .expect("token")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(read_token(profile.path()).expect("token"), "secret");
    }
}
