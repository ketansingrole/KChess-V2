//! The disk as the core and its frontends use it alike: what a folder takes, as the core measures
//! the profile and the app its own install, and the files only their owner may read.

use std::path::Path;

/// Replace `path` whole with `contents`, readable by its owner alone (0600 on Unix): written
/// beside it under a random name of its own, so no leftover or other writer lends it a mode, and
/// renamed over it, so a crash leaves the old file or the new one, never part of either.
pub fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    std::io::Write::write_all(&mut file, contents)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// The bytes of every file under `path` (or of `path` itself, a file), links not followed, so a
/// link out of the folder is never counted. What cannot be read counts as nothing. It reads the
/// file system: call it off the main thread and the async workers.
pub fn folder_bytes(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let mut total = 0u64;
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                total = total.saturating_add(entry.metadata().map_or(0, |meta| meta.len()));
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::folder_bytes;

    /// Files in nested folders count; a missing path is nothing; a file is its own size.
    #[test]
    fn a_folder_is_the_sum_of_its_files() {
        let dir = tempfile::tempdir().expect("a folder");
        std::fs::write(dir.path().join("a"), [0u8; 10]).expect("a file");
        std::fs::create_dir_all(dir.path().join("b/c")).expect("folders");
        std::fs::write(dir.path().join("b/c/d"), [0u8; 5]).expect("a nested file");
        assert_eq!(folder_bytes(dir.path()), 15);
        assert_eq!(folder_bytes(&dir.path().join("a")), 10);
        assert_eq!(folder_bytes(&dir.path().join("missing")), 0);
    }

    /// A link out of the folder is not followed, so what it points to is not counted twice.
    #[cfg(unix)]
    #[test]
    fn a_link_is_not_followed() {
        let outside = tempfile::tempdir().expect("a folder");
        std::fs::write(outside.path().join("big"), [0u8; 100]).expect("a file");
        let dir = tempfile::tempdir().expect("a folder");
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).expect("a link");
        assert_eq!(folder_bytes(dir.path()), 0);
    }
}
