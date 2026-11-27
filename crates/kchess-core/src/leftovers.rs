//! What the Electron app (2026.11.4 and before) left in the profile it shared with KChess:
//! Chromium's caches and web storage, which nothing has read since KChess became a native app.
//! The one-time import (`store::import`) reads `kchess-data.json` and the earlier database,
//! neither of them here. Only these exact names are measured or removed: the profile also holds
//! KChess's own files, among them `databases`, the name Chromium gives its web databases too.

use std::path::Path;

/// Chromium's files and folders in an Electron app's profile.
pub const NAMES: &[&str] = &[
    "blob_storage",
    "Cache",
    "Code Cache",
    "Cookies",
    "Cookies-journal",
    "Crashpad",
    "DawnCache",
    "DawnGraphiteCache",
    "DawnWebGPUCache",
    "declarative_performance_observer.db",
    "declarative_performance_observer.db-journal",
    "DevToolsActivePort",
    "Dictionaries",
    "DIPS",
    "DIPS-journal",
    "DIPS-wal",
    "first_party_sets.db",
    "first_party_sets.db-journal",
    "GPUCache",
    "GPUPersistentCache",
    "IndexedDB",
    "Local State",
    "Local Storage",
    "Network",
    "Network Persistent State",
    "Preferences",
    "QuotaManager",
    "QuotaManager-journal",
    "Service Worker",
    "Session Storage",
    "Shared Dictionary",
    "shared_proto_db",
    "SharedStorage",
    "SharedStorage-wal",
    "TransportSecurity",
    "Trust Tokens",
    "Trust Tokens-journal",
    "VideoDecodeStats",
    "WebStorage",
];

/// The bytes they take in `profile`. It reads the file system: call it off the async workers.
pub fn bytes(profile: &Path) -> u64 {
    NAMES.iter().fold(0u64, |total, name| {
        total.saturating_add(kchess_host::disk::folder_bytes(&profile.join(name)))
    })
}

/// Remove them from `profile`, links not followed. Every one is tried; the first failure is
/// returned, and what could be removed is gone. It writes the file system: call it off the
/// async workers.
pub fn remove(profile: &Path) -> std::io::Result<()> {
    let mut first = None;
    for name in NAMES {
        let path = profile.join(name);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let removed = if meta.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(cause) = removed {
            first.get_or_insert(cause);
        }
    }
    first.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use std::fs;

    /// Chromium's leftovers are counted and removed; KChess's own files, its `databases` folder
    /// among them, are neither.
    #[test]
    fn only_chromiums_leftovers_are_counted_and_removed() {
        let dir = tempfile::tempdir().expect("a profile");
        let profile = dir.path();
        fs::create_dir_all(profile.join("IndexedDB/app.leveldb")).expect("IndexedDB");
        fs::write(profile.join("IndexedDB/app.leveldb/000003.log"), [0; 300]).expect("a log");
        fs::write(profile.join("Cookies"), [0; 20]).expect("cookies");
        fs::create_dir_all(profile.join("databases")).expect("databases");
        fs::write(profile.join("databases/own.kdb"), [0; 1000]).expect("own games");
        fs::write(profile.join("kchess.db"), [0; 500]).expect("the library");
        assert_eq!(super::bytes(profile), 320);

        super::remove(profile).expect("removed");
        assert_eq!(super::bytes(profile), 0);
        assert!(!profile.join("IndexedDB").exists() && !profile.join("Cookies").exists());
        assert!(profile.join("databases/own.kdb").exists() && profile.join("kchess.db").exists());
        // Nothing left: removing again is nothing to do.
        super::remove(profile).expect("nothing to remove");
    }
}
