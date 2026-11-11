//! What a release and the installed app must agree on: the repository releases come from, the
//! release versions ([`Version`]), the release notes ([`notes`]), the packages' names
//! ([`Asset`]), the names an installation holds ([`installed`]), and update patches.
//!
//! Update patches are the program files of a new release, rebuilt from the files of the one that
//! is installed.
//!
//! A release's packages hold the same program files in different wrappers (a bundle in a zip, a
//! folder in an installer, a tree in a `.deb`, one AppImage). A [`Delta`] describes the new
//! release's files, each as one of:
//!
//! - **copy**: an installed file with this content, unchanged (wherever it lives now);
//! - **patch**: a bsdiff patch from the installed file at the same place;
//! - **whole**: the file itself, compressed, for one that is new or changed beyond patching.
//!
//! Nothing is assumed about which files change: the release tooling compares every file by its
//! SHA-256 ([`build`]), so a release that replaces the engine or the voice library carries it,
//! and one that does not leaves it out. Every file's digest, and the digest of each installed
//! file it starts from, is in the delta: a base that is missing or was changed fails
//! [`Delta::rebuild`], and the updater then downloads the whole package as it always has.
//!
//! The delta itself is a release asset listed in the release's signed `SHA256SUMS.txt`, so its
//! digests are as trustworthy as the package's own.
//!
//! The format: `MAGIC`, the header's length as a little-endian `u64`, the header (JSON), then
//! the blobs it refers to by offset.

#![cfg_attr(not(test), deny(clippy::indexing_slicing))]

mod assets;
pub mod installed;
pub mod notes;
mod tree;
mod version;

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use assets::{Arch, Asset, PREFIX};
pub use tree::{Node, Tree};
pub use version::Version;

macro_rules! owner {
    () => {
        "ketansingrole"
    };
}
macro_rules! project {
    () => {
        "KChess-V2"
    };
}
/// The GitHub repository releases are published from, as `owner/name`.
pub const REPOSITORY: &str = concat!(owner!(), "/", project!());
/// The repository's page, its issue tracker and the project's website.
pub const SOURCE: &str = concat!("https://github.com/", owner!(), "/", project!());
pub const ISSUES: &str = concat!("https://github.com/", owner!(), "/", project!(), "/issues");
pub const WEBSITE: &str = concat!("https://", owner!(), ".github.io/", project!(), "/");

pub const MAGIC: &[u8; 13] = b"KCHESS-DELTA\n";
pub const FORMAT: u32 = 1;
/// The longest header a delta may have: a release's file list is a few kilobytes.
const MAX_HEADER: u64 = 16 << 20;

/// The installed AppImage's one file, as its deltas name it: an AppImage is patched whole.
pub const APPIMAGE: &str = "KChess.AppImage";
/// The update installer a Windows delta carries (the installer without its files, which writes
/// the files beside it).
pub const WINDOWS_INSTALLER: &str = "installer.exe";
/// Where a Windows patch's installer is staged; the files go in `files` beside it, where it
/// reads them (`$EXEDIR\files`).
pub const UPDATE_EXE: &str = "KChess-update.exe";

/// Stage a Windows patch over the `installed` folder as the updater installs it: the rebuilt
/// files in `out/files` and the update installer beside them as [`UPDATE_EXE`]; the installer's
/// path.
pub fn stage_windows(delta: &mut Delta, installed: &Path, out: &Path) -> Result<PathBuf> {
    delta.rebuild(|path| installed.join(path), &out.join("files"))?;
    let installer = out.join(UPDATE_EXE);
    std::fs::write(&installer, delta.extra(WINDOWS_INSTALLER)?)?;
    Ok(installer)
}

/// The release asset of the delta that makes `package` (a release asset,
/// `KChess-2026.11.5-linux-amd64.deb`) from release `from`'s installed files:
/// `KChess-2026.11.5-linux-amd64.from-2026.11.4.delta` ([`Asset::patch`]).
pub fn name(package: &str, from: &str) -> Option<String> {
    let (asset, version, arch) = Asset::parse(package)?;
    asset.patch(version, arch, from)
}

/// Why a delta could not be written, read or used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(cause: std::io::Error) -> Self {
        Error(cause.to_string())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(Error(message.into()))
}

/// The SHA-256 of `bytes`, in lowercase hex.
pub fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// The SHA-256 of the file at `path`, read a piece at a time.
pub fn file_digest(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        let Some(chunk) = buffer.get(..read).filter(|chunk| !chunk.is_empty()) else {
            break;
        };
        hasher.update(chunk);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// An installed file a new one starts from: where it is, and what it must hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    pub path: String,
    pub sha256: String,
}

/// Where a new file's bytes come from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Copy(Base),
    Patch { base: Base, blob: usize },
    Whole { blob: usize },
}

/// One entry of the new release's files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Entry {
    File {
        path: String,
        executable: bool,
        size: u64,
        sha256: String,
        source: Source,
    },
    Link {
        path: String,
        target: String,
    },
    Dir {
        path: String,
    },
}

impl Entry {
    pub fn path(&self) -> &str {
        match self {
            Entry::File { path, .. } | Entry::Link { path, .. } | Entry::Dir { path } => path,
        }
    }
}

/// A file the installer needs beside the program files (a `.deb`'s control archive, Windows'
/// installer without its payload).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extra {
    pub name: String,
    pub sha256: String,
    pub blob: usize,
}

/// Where a blob lies in the delta's data, after the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blob {
    pub offset: u64,
    pub length: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub format: u32,
    /// The release whose installed files this delta starts from, and the one it makes.
    pub from: String,
    pub to: String,
    pub entries: Vec<Entry>,
    #[serde(default)]
    pub extras: Vec<Extra>,
    pub blobs: Vec<Blob>,
}

/// What a delta holds, as the release tooling reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub copied: usize,
    pub patched: usize,
    pub whole: usize,
    /// The bytes of the new release's files, and of the delta that makes them.
    pub files_size: u64,
    pub delta_size: u64,
}

/// The bsdiff patch that makes `new` from `old`.
fn patch(old: &[u8], new: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    qbsdiff::Bsdiff::new(old, new)
        .compression_level(9)
        .compare(std::io::Cursor::new(&mut out))?;
    Ok(out)
}

/// `patch` applied to `old`.
fn apply(old: &[u8], patch: &[u8]) -> Result<Vec<u8>> {
    let patcher = qbsdiff::Bspatch::new(patch)?;
    let mut new = Vec::with_capacity(usize::try_from(patcher.hint_target_size()).unwrap_or(0));
    patcher.apply(old, std::io::Cursor::new(&mut new))?;
    Ok(new)
}

/// Write the delta that makes `new` from `old` (the release `from`'s files, as installed) to
/// `out`, with `extras` beside it. Each file is copied when `old` has one with its content,
/// patched from the file at its own path when the patch is smaller than the file whole, and
/// whole otherwise.
pub fn build(
    from: &str,
    to: &str,
    old: &Tree,
    new: &Tree,
    extras: &[(String, Vec<u8>)],
    out: &mut impl Write,
) -> Result<Stats> {
    let mut by_digest: HashMap<String, &str> = HashMap::new();
    for (path, node) in old.iter() {
        if let Node::File { data, .. } = node {
            by_digest.entry(digest(data)).or_insert(path);
        }
    }
    let mut blobs: Vec<Vec<u8>> = Vec::new();
    let mut entries = Vec::new();
    let mut stats = Stats::default();
    for (path, node) in new.iter() {
        let path = path.to_string();
        let entry = match node {
            Node::Dir => Entry::Dir { path },
            Node::Link(target) => Entry::Link {
                path,
                target: target.clone(),
            },
            Node::File { data, executable } => {
                let sha256 = digest(data);
                stats.files_size += data.len() as u64;
                let same_place = old.get(&path).and_then(|node| match node {
                    Node::File { data, .. } => Some(data.as_slice()),
                    _ => None,
                });
                let source = if let Some(base) = by_digest.get(&sha256) {
                    stats.copied += 1;
                    Source::Copy(Base {
                        path: (*base).to_string(),
                        sha256: sha256.clone(),
                    })
                } else {
                    let whole = patch(&[], data)?;
                    let patched = same_place
                        .map(|base| Ok::<_, Error>((base, patch(base, data)?)))
                        .transpose()?
                        // A patch no smaller than the file whole costs a base for nothing.
                        .filter(|(_, patched)| patched.len() < whole.len());
                    match patched {
                        Some((base, patched)) => {
                            stats.patched += 1;
                            blobs.push(patched);
                            Source::Patch {
                                base: Base {
                                    path: path.clone(),
                                    sha256: digest(base),
                                },
                                blob: blobs.len() - 1,
                            }
                        }
                        None => {
                            stats.whole += 1;
                            blobs.push(whole);
                            Source::Whole {
                                blob: blobs.len() - 1,
                            }
                        }
                    }
                };
                Entry::File {
                    path,
                    executable: *executable,
                    size: data.len() as u64,
                    sha256,
                    source,
                }
            }
        };
        entries.push(entry);
    }
    let mut listed = Vec::new();
    for (name, data) in extras {
        blobs.push(patch(&[], data)?);
        listed.push(Extra {
            name: name.clone(),
            sha256: digest(data),
            blob: blobs.len() - 1,
        });
    }
    let mut offset = 0;
    let header = Header {
        format: FORMAT,
        from: from.to_string(),
        to: to.to_string(),
        entries,
        extras: listed,
        blobs: blobs
            .iter()
            .map(|blob| {
                let placed = Blob {
                    offset,
                    length: blob.len() as u64,
                };
                offset += blob.len() as u64;
                placed
            })
            .collect(),
    };
    let header = serde_json::to_vec(&header).map_err(|cause| Error(cause.to_string()))?;
    out.write_all(MAGIC)?;
    out.write_all(&(header.len() as u64).to_le_bytes())?;
    out.write_all(&header)?;
    for blob in &blobs {
        out.write_all(blob)?;
    }
    stats.delta_size = (MAGIC.len() + 8 + header.len()) as u64 + offset;
    Ok(stats)
}

/// Whether `path` (a delta's, `/`-separated) stays inside the folder it is rebuilt into.
fn contained(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// A delta on disk, its header read.
pub struct Delta {
    file: File,
    data: u64,
    pub header: Header,
}

impl Delta {
    /// Open the delta at `path` and read its header.
    pub fn open(path: &Path) -> Result<Delta> {
        let mut file = File::open(path)?;
        let mut magic = [0u8; MAGIC.len()];
        file.read_exact(&mut magic)
            .map_err(|_| Error("The update patch is not one KChess reads".into()))?;
        if &magic != MAGIC {
            return fail("The update patch is not one KChess reads");
        }
        let mut length = [0u8; 8];
        file.read_exact(&mut length)?;
        let length = u64::from_le_bytes(length);
        if length > MAX_HEADER {
            return fail("The update patch's header is too long");
        }
        let mut header = vec![0u8; usize::try_from(length).unwrap_or(0)];
        file.read_exact(&mut header)?;
        let header: Header = serde_json::from_slice(&header)
            .map_err(|cause| Error(format!("The update patch could not be read ({cause})")))?;
        if header.format != FORMAT {
            return fail(format!(
                "The update patch is format {}, not {FORMAT}",
                header.format
            ));
        }
        if let Some(entry) = header.entries.iter().find(|entry| !contained(entry.path())) {
            return fail(format!(
                "The update patch names a file outside the app: {}",
                entry.path()
            ));
        }
        let size = file.metadata()?.len();
        let data = MAGIC.len() as u64 + 8 + length;
        if header.blobs.iter().any(|blob| {
            blob.offset
                .checked_add(blob.length)
                .is_none_or(|end| data + end > size)
        }) {
            return fail("The update patch is cut short");
        }
        Ok(Delta { file, data, header })
    }

    fn blob(&mut self, index: usize) -> Result<Vec<u8>> {
        let blob = *self
            .header
            .blobs
            .get(index)
            .ok_or_else(|| Error(format!("The update patch has no part {index}")))?;
        self.file.seek(SeekFrom::Start(self.data + blob.offset))?;
        let mut bytes = vec![0u8; usize::try_from(blob.length).unwrap_or(0)];
        self.file.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    /// The extra file `name`, checked against its digest.
    pub fn extra(&mut self, name: &str) -> Result<Vec<u8>> {
        let extra = self
            .header
            .extras
            .iter()
            .find(|extra| extra.name == name)
            .cloned()
            .ok_or_else(|| Error(format!("The update patch has no {name}")))?;
        let bytes = apply(&[], &self.blob(extra.blob)?)?;
        if digest(&bytes) != extra.sha256 {
            return fail(format!(
                "The update patch's {name} did not match its checksum"
            ));
        }
        Ok(bytes)
    }

    /// The installed files this delta needs, each with the digest it must have.
    pub fn bases(&self) -> Vec<&Base> {
        self.header
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::File {
                    source: Source::Copy(base) | Source::Patch { base, .. },
                    ..
                } => Some(base),
                _ => None,
            })
            .collect()
    }

    /// Whether every installed file this delta starts from is where `base` finds it, with the
    /// content the delta expects; the first that is not, otherwise.
    pub fn check_bases(&self, base: impl Fn(&str) -> PathBuf) -> Result<()> {
        let mut checked: HashMap<&str, ()> = HashMap::new();
        for needed in self.bases() {
            if checked.insert(&needed.path, ()).is_some() {
                continue;
            }
            let path = base(&needed.path);
            match file_digest(&path) {
                Ok(found) if found == needed.sha256 => {}
                Ok(_) => return fail(format!("{} was changed", needed.path)),
                Err(_) => return fail(format!("{} is missing", needed.path)),
            }
        }
        Ok(())
    }

    /// Rebuild the new release's files into the empty folder `out`, taking installed files from
    /// where `base` says each is. Every file is checked against its digest as it is written.
    pub fn rebuild(&mut self, base: impl Fn(&str) -> PathBuf, out: &Path) -> Result<()> {
        self.check_bases(&base)?;
        std::fs::create_dir_all(out)?;
        let entries = self.header.entries.clone();
        for entry in &entries {
            let target = out.join(entry.path());
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            match entry {
                Entry::Dir { .. } => std::fs::create_dir_all(&target)?,
                Entry::Link { target: link, .. } => symlink(link, &target)?,
                Entry::File {
                    path,
                    executable,
                    sha256,
                    source,
                    ..
                } => {
                    let bytes = match source {
                        Source::Copy(from) => std::fs::read(base(&from.path))?,
                        Source::Patch { base: from, blob } => {
                            let old = std::fs::read(base(&from.path))?;
                            if digest(&old) != from.sha256 {
                                return fail(format!("{} was changed", from.path));
                            }
                            apply(&old, &self.blob(*blob)?)?
                        }
                        Source::Whole { blob } => apply(&[], &self.blob(*blob)?)?,
                    };
                    if digest(&bytes) != *sha256 {
                        return fail(format!("{path} did not match its checksum"));
                    }
                    std::fs::write(&target, &bytes)?;
                    set_executable(&target, *executable)?;
                }
            }
        }
        Ok(())
    }
}

/// A link at `at` to `target`, where the system has them.
fn symlink(target: &str, at: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        Ok(std::os::unix::fs::symlink(target, at)?)
    }
    #[cfg(not(unix))]
    {
        let _ = target;
        fail(format!(
            "{} is a link, which this system cannot make",
            at.display()
        ))
    }
}

/// Give `path` the mode its entry names, where files have one.
fn set_executable(path: &Path, executable: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if executable { 0o755 } else { 0o644 };
        Ok(std::fs::set_permissions(
            path,
            std::fs::Permissions::from_mode(mode),
        )?)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, executable);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
