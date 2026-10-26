//! The per-release copy of the service binary (`docs/shared-core.md`, "Where the service runs
//! from"). The service never runs from inside an app: Windows could not replace a running
//! executable during an update, an AppImage's files vanish when the app quits, and deleting an
//! app would pull the core out from under the other one.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::discovery;
use crate::protocol::{self, hex};

fn digest(path: &Path) -> std::io::Result<String> {
    let mut hash = Sha256::new();
    hash.update(std::fs::read(path)?);
    Ok(hex(&hash.finalize()))
}

/// This release's copy of `bundled` in the runtime folder, made (or remade, if it differs) from
/// the app's own binary. The copy is written under a temporary name and renamed into place, so
/// two apps copying at once never leave a broken file.
pub fn install(bundled: &Path) -> std::io::Result<PathBuf> {
    let dir = discovery::runtime_dir()
        .ok_or_else(|| std::io::Error::other("no runtime folder"))?
        .join(protocol::release());
    install_into(bundled, &dir)
}

pub(crate) fn install_into(bundled: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    let name = bundled
        .file_name()
        .ok_or_else(|| std::io::Error::other("the service binary has no name"))?;
    let target = dir.join(name);
    let wanted = digest(bundled)?;
    if digest(&target).is_ok_and(|found| found == wanted) {
        return Ok(target);
    }
    std::fs::create_dir_all(dir)?;
    let temporary = dir.join(format!(
        ".{}-{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    std::fs::copy(bundled, &temporary)?;
    if digest(&temporary)? != wanted {
        let _ = std::fs::remove_file(&temporary);
        return Err(std::io::Error::other("the copied service binary differs"));
    }
    std::fs::rename(&temporary, &target)?;
    Ok(target)
}

/// Removes other releases' copies. A running service's file can be deleted on macOS and Linux
/// (it keeps running); on Windows the delete fails and the folder stays until the next start.
pub fn remove_other_releases() {
    let Some(root) = discovery::runtime_dir() else {
        return;
    };
    let ours = protocol::release();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy() != ours {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_is_copied_once_and_replaced_when_it_differs() {
        let source = tempfile::tempdir().expect("source");
        let runtime = tempfile::tempdir().expect("runtime");
        let bundled = source.path().join("kchess-service");
        std::fs::write(&bundled, b"first build").expect("bundled");
        let copy = install_into(&bundled, runtime.path()).expect("copied");
        assert_eq!(std::fs::read(&copy).expect("copy"), b"first build");
        let modified = std::fs::metadata(&copy)
            .expect("copy")
            .modified()
            .expect("time");
        // The same binary is reused as it is.
        assert_eq!(
            install_into(&bundled, runtime.path()).expect("reused"),
            copy
        );
        assert_eq!(
            std::fs::metadata(&copy)
                .expect("copy")
                .modified()
                .expect("time"),
            modified
        );
        // A different binary under the same release replaces it.
        std::fs::write(&bundled, b"second build").expect("bundled");
        install_into(&bundled, runtime.path()).expect("replaced");
        assert_eq!(std::fs::read(&copy).expect("copy"), b"second build");
        let leftovers: Vec<_> = std::fs::read_dir(runtime.path())
            .expect("runtime")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "no temporary files are left");
    }
}
