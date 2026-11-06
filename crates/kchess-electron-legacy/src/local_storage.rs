//! Chromium's `localStorage` for the app's origin, from `<profile>/Local Storage/leveldb`.
//!
//! Each entry's key is `_` + origin + `\0` + the name, and the name and the value each start with
//! a format byte: `1` for Latin-1 text, `0` for UTF-16LE. The database is copied first, so the
//! profile is never locked, compacted or changed.

use std::collections::BTreeMap;
use std::path::Path;

use crate::{Result, error};

/// The origin every Electron release loaded its renderer from.
pub const ORIGIN: &str = "kchess://app";

/// The `localStorage` of `kchess://app` in `profile`, by name; empty when there is none.
pub fn read(profile: &Path) -> Result<BTreeMap<String, String>> {
    let source = profile.join("Local Storage").join("leveldb");
    if !source.join("CURRENT").is_file() {
        return Ok(BTreeMap::new());
    }
    let copy =
        tempfile::tempdir().map_err(|cause| error(format!("No temporary folder: {cause}")))?;
    let entries = std::fs::read_dir(&source)
        .map_err(|cause| error(format!("Unreadable localStorage: {cause}")))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        // Another process may hold the original's lock; the copy needs none of its own yet.
        if name == "LOCK" || !entry.path().is_file() {
            continue;
        }
        std::fs::copy(entry.path(), copy.path().join(&name))
            .map_err(|cause| error(format!("Unreadable localStorage: {cause}")))?;
    }
    let options = rusty_leveldb::Options {
        create_if_missing: false,
        ..rusty_leveldb::Options::default()
    };
    let mut db = rusty_leveldb::DB::open(copy.path(), options)
        .map_err(|cause| error(format!("Unreadable localStorage: {cause}")))?;
    let mut prefix = format!("_{ORIGIN}").into_bytes();
    prefix.push(0);
    let mut found = BTreeMap::new();
    let mut iter = db
        .new_iter()
        .map_err(|cause| error(format!("Unreadable localStorage: {cause}")))?;
    use rusty_leveldb::LdbIterator;
    while let Some((key, value)) = iter.next() {
        let Some(name) = key.strip_prefix(prefix.as_slice()) else {
            continue;
        };
        if let (Some(name), Some(value)) = (text(name), text(&value)) {
            found.insert(name, value);
        }
    }
    drop(iter);
    drop(db);
    Ok(found)
}

/// A name or value as Chromium stores it: a format byte, then Latin-1 or UTF-16LE text.
fn text(bytes: &[u8]) -> Option<String> {
    let (format, rest) = bytes.split_first()?;
    match format {
        1 => Some(rest.iter().map(|&b| char::from(b)).collect()),
        0 if rest.len() % 2 == 0 => {
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .collect();
            String::from_utf16(&units).ok()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_text_formats() {
        assert_eq!(text(b"\x01caf\xe9").as_deref(), Some("café"));
        assert_eq!(text(&[0, 0x5e, 0x26]).as_deref(), Some("♞"));
        assert_eq!(text(&[0, 1]), None, "an odd UTF-16 length");
        assert_eq!(text(&[2, b'a']), None, "an unknown format");
        assert_eq!(text(&[]), None);
    }

    fn key(origin: &str, name: &str) -> Vec<u8> {
        [format!("_{origin}\0").as_bytes(), &[1], name.as_bytes()].concat()
    }

    /// The app's entries are read from a copy of a database another process still holds open,
    /// skipping other origins and anything in the folder that is not a file.
    #[test]
    fn reads_the_apps_entries_from_a_held_database() {
        let profile = tempfile::tempdir().unwrap();
        let folder = profile.path().join("Local Storage").join("leveldb");
        std::fs::create_dir_all(&folder).unwrap();
        let mut db = rusty_leveldb::DB::open(&folder, rusty_leveldb::Options::default()).unwrap();
        db.put(&key(ORIGIN, "settings"), b"\x01{\"a\":1}").unwrap();
        db.put(&key("https://example.com", "other"), b"\x01no")
            .unwrap();
        db.put(b"VERSION", b"1").unwrap();
        db.flush().unwrap();
        std::fs::create_dir(folder.join("not-a-file")).unwrap();

        let found = read(profile.path()).unwrap();
        assert_eq!(
            found.into_iter().collect::<Vec<_>>(),
            [("settings".to_string(), "{\"a\":1}".to_string())]
        );
        drop(db);
    }

    #[test]
    fn a_profile_without_local_storage_has_none() {
        let profile = tempfile::tempdir().unwrap();
        assert!(read(profile.path()).unwrap().is_empty());
    }
}
