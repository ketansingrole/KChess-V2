//! Small updates: the next release's files rebuilt from this installation's own, with the update
//! patch the release publishes for this version (`cargo xtask deltas`, `crates/kchess-delta`).
//!
//! What a patch makes is staged exactly as the whole package's download is, so it installs the
//! same way: the new `.app` on macOS, the new AppImage, a `.deb` of the new files for the package
//! manager, and on Windows the release's update installer (the installer without its files),
//! which writes the files beside it as the full installer writes its own.
//!
//! Every file the patch starts from and every file it makes is checked by its SHA-256, and the
//! patch itself is checked against the release's signed checksums like any download. Anything
//! that does not match (a file someone deleted or changed, a damaged patch) fails here, and the
//! updater downloads the whole package instead.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use kchess_delta::{APPIMAGE, Delta, Entry, WINDOWS_INSTALLER};

use super::Package;

/// Where a Windows patch's installer is staged; the files go in `files` beside it, where it
/// reads them (`$EXEDIR\files`).
pub const UPDATE_EXE: &str = "KChess-update.exe";

/// The release asset of the patch from `current`'s files to `package` (the release asset this
/// installation updates from).
pub fn asset_name(package: &str, current: &str) -> Option<String> {
    kchess_delta::name(package, current)
}

/// A patch to apply: the release `version` it makes, from `current`'s files.
pub struct Patch<'a> {
    pub delta: &'a Path,
    pub package: Package,
    /// The installation (`Install::path`).
    pub install: &'a Path,
    /// Where a `.deb`'s paths start: `/`.
    pub root: &'a Path,
    pub current: &'a str,
    pub version: &'a str,
    /// The `.deb`'s name, as the release names it.
    pub deb: &'a str,
}

/// Rebuild the release `patch` makes into `work`, from the installed files; what to install,
/// staged as the whole package would be.
pub fn rebuild(patch: &Patch, work: &Path) -> Result<PathBuf, String> {
    let Patch {
        delta,
        package,
        install,
        root,
        current,
        version,
        deb,
    } = *patch;
    let text = |cause: kchess_delta::Error| cause.0;
    let mut delta = Delta::open(delta).map_err(text)?;
    if delta.header.from != current || delta.header.to != version {
        return Err(format!(
            "The update patch is from {} to {}, not {current} to {version}",
            delta.header.from, delta.header.to
        ));
    }
    let out = work.join("patched");
    let _ = std::fs::remove_dir_all(&out);
    match package {
        Package::MacZip => {
            let bundle = out.join("KChess.app");
            delta
                .rebuild(|path| install.join(path), &bundle)
                .map_err(text)?;
            Ok(bundle)
        }
        Package::AppImage => {
            delta
                .rebuild(|_| install.to_path_buf(), &out)
                .map_err(text)?;
            Ok(out.join(APPIMAGE))
        }
        Package::WindowsInstaller => {
            delta
                .rebuild(|path| install.join(path), &out.join("files"))
                .map_err(text)?;
            let installer = out.join(UPDATE_EXE);
            std::fs::write(&installer, delta.extra(WINDOWS_INSTALLER).map_err(text)?)
                .map_err(|cause| cause.to_string())?;
            Ok(installer)
        }
        Package::Deb => {
            let files = out.join("root");
            delta
                .rebuild(|path| root.join(path), &files)
                .map_err(text)?;
            let control = delta
                .header
                .extras
                .iter()
                .find(|extra| extra.name.starts_with("control.tar"))
                .map(|extra| extra.name.clone())
                .ok_or("The update patch has no package control")?;
            let control_data = delta.extra(&control).map_err(text)?;
            let package = out.join(deb);
            write_deb(
                &delta.header.entries,
                &files,
                &control,
                &control_data,
                &package,
            )?;
            Ok(package)
        }
    }
}

/// A `.deb` of the rebuilt `files`, with the release's control archive: an uncompressed
/// `data.tar` (dpkg reads it as it reads any), its folders listed before what is in them.
fn write_deb(
    entries: &[Entry],
    files: &Path,
    control: &str,
    control_data: &[u8],
    out: &Path,
) -> Result<(), String> {
    let fail = |cause: std::io::Error| cause.to_string();
    let data_path = out.with_extension("data.tar");
    let mut tar = tar::Builder::new(std::io::BufWriter::new(
        std::fs::File::create(&data_path).map_err(fail)?,
    ));
    let header = |path: &str, size: u64, mode: u32, kind: tar::EntryType| {
        let mut header = tar::Header::new_gnu();
        header.set_path(path)?;
        header.set_size(size);
        header.set_mode(mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_entry_type(kind);
        header.set_cksum();
        Ok::<_, std::io::Error>(header)
    };
    let mut folders = std::collections::BTreeSet::new();
    for entry in entries {
        let mut parent = Path::new(entry.path()).parent();
        while let Some(folder) = parent.filter(|folder| !folder.as_os_str().is_empty()) {
            folders.insert(folder.to_string_lossy().replace('\\', "/"));
            parent = folder.parent();
        }
        if let Entry::Dir { path } = entry {
            folders.insert(path.clone());
        }
    }
    for folder in &folders {
        let header =
            header(&format!("./{folder}/"), 0, 0o755, tar::EntryType::Directory).map_err(fail)?;
        tar.append(&header, std::io::empty()).map_err(fail)?;
    }
    for entry in entries {
        match entry {
            Entry::File {
                path,
                executable,
                size,
                ..
            } => {
                let mode = if *executable { 0o755 } else { 0o644 };
                let header = header(&format!("./{path}"), *size, mode, tar::EntryType::Regular)
                    .map_err(fail)?;
                let file = std::fs::File::open(files.join(path)).map_err(fail)?;
                tar.append(&header, file).map_err(fail)?;
            }
            Entry::Link { path, target } => {
                let mut header = header(&format!("./{path}"), 0, 0o777, tar::EntryType::Symlink)
                    .map_err(fail)?;
                header.set_link_name(target).map_err(fail)?;
                header.set_cksum();
                tar.append(&header, std::io::empty()).map_err(fail)?;
            }
            Entry::Dir { .. } => {}
        }
    }
    tar.into_inner()
        .and_then(|mut file| file.flush())
        .map_err(fail)?;
    let data_size = std::fs::metadata(&data_path).map_err(fail)?.len();
    let mut deb = std::io::BufWriter::new(std::fs::File::create(out).map_err(fail)?);
    deb.write_all(b"!<arch>\n").map_err(fail)?;
    let mut member = |name: &str, size: u64, data: &mut dyn std::io::Read| {
        write!(
            deb,
            "{name:<16}{:<12}{:<6}{:<6}{:<8}{size:<10}`\n",
            0, 0, 0, "100644"
        )?;
        std::io::copy(data, &mut deb)?;
        if size % 2 == 1 {
            deb.write_all(b"\n")?;
        }
        Ok::<_, std::io::Error>(())
    };
    member("debian-binary", 4, &mut b"2.0\n".as_slice()).map_err(fail)?;
    member(control, control_data.len() as u64, &mut &*control_data).map_err(fail)?;
    let mut data = std::fs::File::open(&data_path).map_err(fail)?;
    member("data.tar", data_size, &mut data).map_err(fail)?;
    deb.flush().map_err(fail)?;
    drop(data);
    let _ = std::fs::remove_file(&data_path);
    Ok(())
}
