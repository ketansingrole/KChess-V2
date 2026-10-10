//! Helpers the verified downloads share (`engine::managed` and `voice`): member paths kept inside
//! the target folder, bounded writes, and SHA-256 digests. Links, devices and absolute or `..`
//! paths are the callers' to refuse; these helpers only make the safe cases easy to get right.

use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

/// A member name as a path inside the archive root. Absolute paths, `..`, drive prefixes and
/// backslashes (a separator on Windows) are refused (`None`); `.` components are dropped.
pub fn safe_relative(name: &str) -> Option<PathBuf> {
    if name.contains('\\') {
        return None;
    }
    let mut relative = PathBuf::new();
    for component in Path::new(name).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(relative)
}

/// `S_IFLNK` in the Unix mode bits of an archive entry marks a symbolic link.
pub fn is_symlink_mode(mode: u32) -> bool {
    mode & 0o170000 == 0o120000
}

/// The outcome of `write_member`.
#[derive(Debug, PartialEq, Eq)]
pub enum Member {
    Written,
    /// The member expands beyond the remaining budget; nothing more is kept from it.
    OverBudget,
}

/// Write one regular file under `root`, creating its parent folders. Its bytes are charged to
/// `budget`, the bytes the archive may still expand to; a member larger than that is not kept.
pub fn write_member(
    root: &Path,
    relative: &Path,
    reader: &mut impl Read,
    budget: &mut u64,
) -> io::Result<Member> {
    let target = root.join(relative);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(&target)?;
    let copied = io::copy(&mut reader.by_ref().take(*budget + 1), &mut file)?;
    if copied > *budget {
        return Ok(Member::OverBudget);
    }
    *budget -= copied;
    Ok(Member::Written)
}

/// Lowercase hexadecimal of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The lowercase SHA-256 of a file, read in chunks.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}
