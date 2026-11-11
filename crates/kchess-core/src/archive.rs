//! Unpacking the verified downloads (`engine::managed` and `voice`) and their SHA-256 digests.
//! Only regular files and folders are written, each strictly inside the target folder and within
//! a size budget; links, devices and absolute or `..` paths fail the whole extraction.

use std::fs;
use std::io::{self, Read, Seek};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

/// Where and how much an archive may unpack.
pub struct Unpack<'a> {
    pub root: &'a Path,
    /// The bytes the files may take in all.
    pub budget: u64,
    /// When given, every file must be under this folder (`name/`); folders outside it are
    /// skipped.
    pub prefix: Option<&'a str>,
    /// Stops the extraction at the next member.
    pub cancel: Option<&'a CancellationToken>,
}

/// Why an archive was not unpacked.
#[derive(Debug)]
pub enum Refusal {
    /// The archive cannot be read.
    Unreadable(String),
    Link,
    UnsafePath,
    /// A device, a FIFO or another entry that is neither a file nor a folder.
    Unsupported,
    /// A file outside the required prefix.
    Outside,
    TooLarge,
    Cancelled,
    /// Writing a member failed.
    Io(io::Error),
}

/// Unpacks a zip; the number of files written.
pub fn unpack_zip(reader: impl Read + Seek, to: &Unpack) -> Result<usize, Refusal> {
    let unreadable = |cause: zip::result::ZipError| Refusal::Unreadable(cause.to_string());
    let mut archive = zip::ZipArchive::new(reader).map_err(unreadable)?;
    let mut writer = Writer::new(to);
    for index in 0..archive.len() {
        writer.check_cancelled()?;
        let mut entry = archive.by_index(index).map_err(unreadable)?;
        if entry.unix_mode().is_some_and(is_symlink_mode) {
            return Err(Refusal::Link);
        }
        let name = entry.name().map_err(unreadable)?.into_owned();
        if entry.is_dir() {
            writer.folder(&name)?;
        } else {
            writer.file(&name, &mut entry)?;
        }
    }
    Ok(writer.files)
}

/// Unpacks a tar stream; the number of files written.
pub fn unpack_tar(reader: impl Read, to: &Unpack) -> Result<usize, Refusal> {
    let unreadable = |cause: io::Error| Refusal::Unreadable(cause.to_string());
    let mut archive = tar::Archive::new(reader);
    let mut writer = Writer::new(to);
    for entry in archive.entries().map_err(unreadable)? {
        writer.check_cancelled()?;
        let mut entry = entry.map_err(unreadable)?;
        let name = entry
            .path()
            .map_err(unreadable)?
            .to_string_lossy()
            .into_owned();
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            return Err(Refusal::Link);
        }
        if kind.is_dir() {
            writer.folder(&name)?;
        } else if kind.is_file() {
            writer.file(&name, &mut entry)?;
        } else {
            return Err(Refusal::Unsupported);
        }
    }
    Ok(writer.files)
}

struct Writer<'a> {
    to: &'a Unpack<'a>,
    budget: u64,
    files: usize,
}

impl<'a> Writer<'a> {
    fn new(to: &'a Unpack<'a>) -> Writer<'a> {
        Writer {
            to,
            budget: to.budget,
            files: 0,
        }
    }

    fn check_cancelled(&self) -> Result<(), Refusal> {
        match self.to.cancel {
            Some(cancel) if cancel.is_cancelled() => Err(Refusal::Cancelled),
            _ => Ok(()),
        }
    }

    fn inside(&self, name: &str) -> bool {
        self.to.prefix.is_none_or(|prefix| name.starts_with(prefix))
    }

    fn folder(&self, name: &str) -> Result<(), Refusal> {
        let relative = safe_relative(name).ok_or(Refusal::UnsafePath)?;
        if !self.inside(name) {
            return Ok(());
        }
        fs::create_dir_all(self.to.root.join(relative)).map_err(Refusal::Io)
    }

    fn file(&mut self, name: &str, reader: &mut impl Read) -> Result<(), Refusal> {
        let relative = safe_relative(name).ok_or(Refusal::UnsafePath)?;
        if relative.as_os_str().is_empty() {
            return Err(Refusal::UnsafePath);
        }
        if !self.inside(name) {
            return Err(Refusal::Outside);
        }
        match write_member(self.to.root, &relative, reader, &mut self.budget)
            .map_err(Refusal::Io)?
        {
            Member::Written => {
                self.files += 1;
                Ok(())
            }
            Member::OverBudget => Err(Refusal::TooLarge),
        }
    }
}

/// A member name as a path inside the archive root. Absolute paths, `..`, drive prefixes and
/// backslashes (a separator on Windows) are refused (`None`); `.` components are dropped.
pub fn safe_relative(name: &str) -> Option<PathBuf> {
    // A backslash is a separator on Windows, and a colon there names a drive or an NTFS
    // alternate data stream (`kchess.exe:hidden`).
    if name.contains('\\') || (cfg!(windows) && name.contains(':')) {
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
fn is_symlink_mode(mode: u32) -> bool {
    mode & 0o170000 == 0o120000
}

/// The outcome of `write_member`.
#[derive(Debug, PartialEq, Eq)]
enum Member {
    Written,
    /// The member expands beyond the remaining budget; nothing more is kept from it.
    OverBudget,
}

/// Write one regular file under `root`, creating its parent folders. Its bytes are charged to
/// `budget`, the bytes the archive may still expand to; a member larger than that is not kept.
fn write_member(
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
    Ok(crate::ids::hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write as _};

    #[test]
    fn safe_relative_keeps_only_normal_components() {
        assert_eq!(safe_relative("./a/b").unwrap(), Path::new("a/b"));
        assert!(safe_relative("../a").is_none());
        assert!(safe_relative("a/../../b").is_none());
        assert!(safe_relative("/etc/passwd").is_none());
        assert!(safe_relative("a\\..\\b").is_none());
    }

    fn zip(files: &[(&str, &[u8])], link: Option<&str>) -> Cursor<Vec<u8>> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        if let Some(name) = link {
            writer.add_symlink(name, "/bin/sh", options).unwrap();
        }
        Cursor::new(writer.finish().unwrap().into_inner())
    }

    fn unpack(
        archive: Cursor<Vec<u8>>,
        prefix: Option<&str>,
        budget: u64,
    ) -> Result<usize, Refusal> {
        let root = tempfile::tempdir().unwrap();
        let to = Unpack {
            root: root.path(),
            budget,
            prefix,
            cancel: None,
        };
        unpack_zip(archive, &to)
    }

    #[test]
    fn refuses_links_unsafe_paths_files_outside_the_prefix_and_overruns() {
        assert!(matches!(
            unpack(zip(&[], Some("model/link")), None, 100),
            Err(Refusal::Link)
        ));
        assert!(matches!(
            unpack(zip(&[("../escape", b"x")], None), None, 100),
            Err(Refusal::UnsafePath)
        ));
        assert!(matches!(
            unpack(zip(&[("other/file", b"x")], None), Some("model/"), 100),
            Err(Refusal::Outside)
        ));
        assert!(matches!(
            unpack(zip(&[("model/big", &[0; 10])], None), None, 9),
            Err(Refusal::TooLarge)
        ));
        assert!(matches!(
            unpack(
                zip(&[("model/a", b"x"), ("model/b/c", b"y")], None),
                Some("model/"),
                100
            ),
            Ok(2)
        ));
    }

    #[test]
    fn a_cancelled_extraction_stops() {
        let root = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let to = Unpack {
            root: root.path(),
            budget: 100,
            prefix: None,
            cancel: Some(&cancel),
        };
        assert!(matches!(
            unpack_zip(zip(&[("a", b"x")], None), &to),
            Err(Refusal::Cancelled)
        ));
        assert!(!root.path().join("a").exists());
    }
}
