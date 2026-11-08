//! `deltas [dir]`: the update patches from the latest published releases to this one (release CI,
//! after `package`). For each of this machine's packages in `dir` and each of the `PREVIOUS`
//! newest published releases that shipped the same kind, it compares the files the old package
//! installs with the new one's, file by file (`crates/kchess-delta`), and writes
//! `<package stem>.from-<old version>.delta` beside the package. Nothing here knows which files
//! change: whatever differs is carried, so a release with a new engine has a larger patch.
//!
//! A patch that would save too little is left out; installations without a patch for their
//! version, or whose files do not match it, download the whole package as before.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use kchess_delta::{APPIMAGE, Arch, Asset, Node, Tree, WINDOWS_INSTALLER};

use crate::util::{Result, output, root, run_with, say, version};

/// How many earlier releases each package gets a patch from: an app that missed a few releases
/// still updates by patch.
const PREVIOUS: usize = 3;
/// A patch is published only while it is at most this share of the whole package.
const WORTH: f64 = 0.75;

/// What a patch carries beside the files, by name.
type Extras = Vec<(String, Vec<u8>)>;

/// A package installations update from, by its name: its kind, version and architecture.
fn package(name: &str) -> Option<(Asset, &str, Arch)> {
    Asset::parse(name).filter(|(kind, ..)| kind.patched())
}

fn parse(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.trim_start_matches('v').split('.');
    let mut out = [0; 3];
    for slot in &mut out {
        *slot = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// The `count` newest published releases before `current`, newest first, from GitHub's release
/// list (`gh api …/releases`): `(version, asset names)`.
fn previous(
    current: &str,
    releases: &serde_json::Value,
    count: usize,
) -> Vec<(String, Vec<String>)> {
    let Some(now) = parse(current) else {
        return Vec::new();
    };
    let mut found: Vec<([u64; 3], String, Vec<String>)> = releases
        .as_array()
        .into_iter()
        .flatten()
        .filter(|release| release["draft"] != true && release["prerelease"] != true)
        .filter_map(|release| {
            let tag = release["tag_name"].as_str()?;
            let parsed = parse(tag).filter(|parsed| *parsed < now)?;
            let assets = release["assets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|asset| asset["name"].as_str().map(str::to_string))
                .collect();
            Some((parsed, tag.trim_start_matches('v').to_string(), assets))
        })
        .collect();
    found.sort_by_key(|release| std::cmp::Reverse(release.0));
    found
        .into_iter()
        .take(count)
        .map(|(_, version, assets)| (version, assets))
        .collect()
}

/// The bundle a macOS zip holds, its paths from inside `KChess.app`.
fn zip_tree(bytes: &[u8]) -> Result<Tree> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|cause| cause.to_string())?;
    let mut tree = Tree::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|cause| cause.to_string())?;
        let path = file
            .name()
            .map_err(|cause| cause.to_string())?
            .trim_end_matches('/')
            .to_string();
        // Resource forks `ditto` sequestered: the rebuilt bundle is checked by its signature.
        if path.starts_with("__MACOSX") || file.is_dir() {
            continue;
        }
        let mode = file.unix_mode().unwrap_or(0o644);
        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .map_err(|cause| cause.to_string())?;
        let node = if mode & 0o170_000 == 0o120_000 {
            Node::Link(String::from_utf8_lossy(&data).to_string())
        } else {
            Node::File {
                data,
                executable: mode & 0o111 != 0,
            }
        };
        tree.insert(path, node);
    }
    let name = format!("{}.app", crate::package::NAME);
    let bundle = tree.under(&name);
    if bundle.is_empty() {
        return Err(format!("The zip holds no {name}."));
    }
    Ok(bundle)
}

/// The AppImage as its one file.
fn appimage_tree(bytes: Vec<u8>) -> Tree {
    let mut tree = Tree::new();
    tree.insert(
        APPIMAGE,
        Node::File {
            data: bytes,
            executable: true,
        },
    );
    tree
}

/// Where a Windows installer read in `work` is installed.
fn installed_folder(work: &Path) -> PathBuf {
    // In the system's own separators: `root()` joins `target/package/…` with `/`, which a
    // Windows installer does not take in `/D=`, so it installed somewhere else.
    work.join("installed").components().collect()
}

/// The files the Windows installer at `installer` installs: it is run silently into a folder
/// of its own, so they are exactly what an installation holds. Its uninstaller, which every
/// installer writes for itself, is not one of them.
fn installed_tree(installer: &Path, work: &Path) -> Result<Tree> {
    let folder = installed_folder(work);
    let _ = std::fs::remove_dir_all(&folder);
    forget_installation();
    // `/D=` is the last argument, unquoted, as NSIS reads it.
    run_with(
        Command::new(installer)
            .arg("/S")
            .arg(format!("/D={}", folder.display())),
    )?;
    let nothing = || {
        format!(
            "{} installed nothing in {}.",
            installer.display(),
            folder.display()
        )
    };
    if !folder.is_dir() {
        return Err(nothing());
    }
    let uninstaller = crate::package::windows::uninstaller(crate::package::NAME);
    let tree = Tree::read(&folder, |path| path == uninstaller)
        .map_err(|cause| format!("{}: {cause}", folder.display()))?;
    if tree.is_empty() {
        return Err(nothing());
    }
    Ok(tree)
}

/// Forget the installation an earlier installer recorded (`Software\<GUID>`, per user), so the
/// next one installs where `/D=` says: an installer upgrades a recorded installation in place,
/// as it should on a person's machine, and NSIS takes `/D=` off the command line it reads. Only
/// the machines that make patches run installers (the Windows package job, a disposable runner).
fn forget_installation() {
    if cfg!(windows) {
        let guid = crate::package::GUID;
        let _ = Command::new("reg")
            .args(["delete", &format!("HKCU\\Software\\{guid}"), "/f"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// A package's files, and what its patches carry beside them, read from `path`.
fn files(kind: Asset, path: &Path, work: &Path) -> Result<(Tree, Extras)> {
    let read = || std::fs::read(path).map_err(|cause| format!("{}: {cause}", path.display()));
    match kind {
        Asset::Deb => {
            let (tree, control) = crate::deb::read(&read()?)?;
            Ok((tree, vec![control]))
        }
        Asset::AppImage => Ok((appimage_tree(read()?), Vec::new())),
        Asset::MacZip => Ok((zip_tree(&read()?)?, Vec::new())),
        Asset::WindowsInstaller => Ok((installed_tree(path, work)?, Vec::new())),
        other => Err(format!("No installation updates from a {other:?}.")),
    }
}

/// What a Windows patch carries beside the files: the update installer packaging made.
fn windows_extras(version: &str) -> Result<Extras> {
    let identity = crate::package::Identity::new(version.to_string());
    let path = crate::package::windows::update_installer(&identity);
    let data = std::fs::read(&path).map_err(|cause| {
        format!(
            "{}: {cause} (run `cargo xtask package` first)",
            path.display()
        )
    })?;
    Ok(vec![(WINDOWS_INSTALLER.to_string(), data)])
}

fn report(name: &str, stats: &kchess_delta::Stats, package_size: u64, worth: bool) -> String {
    let mb = |bytes: u64| bytes as f64 / 1e6;
    format!(
        "{name}: {:.1} MB for a {:.1} MB package ({} files copied, {} patched, {} whole){}",
        mb(stats.delta_size),
        mb(package_size),
        stats.copied,
        stats.patched,
        stats.whole,
        if worth { "" } else { "; too large to publish" }
    )
}

/// Write the patch from `old` (release `from`) to `new` (`to`) as `delta`: only if it is worth
/// publishing, unless `only_worth` is false; whether it is worth it.
fn write(
    (from, to): (&str, &str),
    (old, new): (&Tree, &Tree),
    extras: &[(String, Vec<u8>)],
    package_size: u64,
    delta: &Path,
    only_worth: bool,
) -> Result<bool> {
    let mut out = Vec::new();
    let stats = kchess_delta::build(from, to, old, new, extras, &mut out)
        .map_err(|cause| cause.to_string())?;
    let name = delta
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let worth = (stats.delta_size as f64) <= WORTH * package_size as f64;
    say(&report(&name, &stats, package_size, worth));
    if worth || !only_worth {
        std::fs::write(delta, &out).map_err(|cause| cause.to_string())?;
    }
    Ok(worth)
}

/// `deltas --from <old package> <new package> [--stage <out>]`: one patch between two packages
/// on disk, written beside the new one (trying a patch by hand; Windows installers are installed
/// to read them). `--stage` then stages a Windows patch over the old installer's files as `apply`
/// does (the Windows upgrade test).
fn between(old_path: &Path, new_path: &Path, stage: Option<&Path>) -> Result {
    let name = |path: &Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    };
    let (old_name, new_name) = (name(old_path), name(new_path));
    let Some(((kind, to, arch), (_, from, _))) = package(&new_name)
        .zip(package(&old_name))
        .filter(|((kind, _, arch), (old_kind, _, old_arch))| (kind, arch) == (old_kind, old_arch))
    else {
        return Err("Both packages must be of one kind KChess releases.".into());
    };
    if stage.is_some() && kind != Asset::WindowsInstaller {
        return Err("Only a Windows patch is staged beside an installer.".into());
    }
    let work = root().join("target/package/deltas");
    let (old, _) = files(kind, old_path, &work.join(from))?;
    let (new, mut extras) = files(kind, new_path, &work.join(to))?;
    if kind == Asset::WindowsInstaller {
        extras.extend(windows_extras(to)?);
    }
    let size = std::fs::metadata(new_path)
        .map_err(|cause| cause.to_string())?
        .len();
    let delta = new_path.with_file_name(
        kind.patch(to, arch, from)
            .ok_or("A package without a patch name.")?,
    );
    write((from, to), (&old, &new), &extras, size, &delta, false)?;
    match stage {
        Some(out) => apply(&delta, &installed_folder(&work.join(from)), out),
        None => Ok(()),
    }
}

/// `deltas --apply <patch> <installed folder> <out>`: what the Windows updater stages from a
/// patch: the rebuilt files in `<out>/files` and the update installer beside them as
/// `KChess-update.exe` (the Windows upgrade test runs it as an installed KChess does).
fn apply(delta: &Path, installed: &Path, out: &Path) -> Result {
    let text = |cause: kchess_delta::Error| cause.0;
    let mut patch = kchess_delta::Delta::open(delta).map_err(text)?;
    let _ = std::fs::remove_dir_all(out);
    patch
        .rebuild(|path| installed.join(path), &out.join("files"))
        .map_err(text)?;
    let installer = patch.extra(WINDOWS_INSTALLER).map_err(text)?;
    std::fs::write(out.join("KChess-update.exe"), installer).map_err(|cause| cause.to_string())?;
    say(&format!(
        "{} to {} staged in {}.",
        patch.header.from,
        patch.header.to,
        out.display()
    ));
    Ok(())
}

/// Fetches release `from`'s asset `name` into `folder` (`gh release download`).
type Download<'a> = &'a dyn Fn(&str, &str, &Path) -> Result;

/// Write the patches to release `current`'s packages in `dir` from the newest of `releases`
/// (GitHub's release list) that shipped each, fetching their packages into `work` with
/// `download` unless they are already there; how many were worth writing.
fn patches(
    dir: &Path,
    current: &str,
    releases: &serde_json::Value,
    work: &Path,
    download: Download,
) -> Result<usize> {
    let packages: Vec<(Asset, Arch, PathBuf)> = std::fs::read_dir(dir)
        .map_err(|cause| format!("{}: {cause}", dir.display()))?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let (kind, _, arch) = package(&name).filter(|(_, version, _)| *version == current)?;
            Some((kind, arch, entry.path()))
        })
        .collect();
    if packages.is_empty() {
        return Err(format!("{} has no {current} packages.", dir.display()));
    }
    let earlier = previous(current, releases, PREVIOUS);
    let mut written = 0;
    for (kind, arch, path) in &packages {
        let package_size = std::fs::metadata(path)
            .map_err(|cause| cause.to_string())?
            .len();
        let (new, mut extras) = files(*kind, path, &work.join(current))?;
        if *kind == Asset::WindowsInstaller {
            extras.extend(windows_extras(current)?);
        }
        for (from, assets) in &earlier {
            let old_name = kind.name(from, *arch);
            if !assets.contains(&old_name) {
                say(&format!("{from} has no {old_name}: no patch from it."));
                continue;
            }
            let folder = work.join(from);
            std::fs::create_dir_all(&folder).map_err(|cause| cause.to_string())?;
            let old_path = folder.join(&old_name);
            if !old_path.exists() {
                download(from, &old_name, &folder)?;
            }
            let (old, _) = files(*kind, &old_path, &folder)?;
            let delta = dir.join(
                kind.patch(current, *arch, from)
                    .ok_or("A package without a patch name.")?,
            );
            if write(
                (from, current),
                (&old, &new),
                &extras,
                package_size,
                &delta,
                true,
            )? {
                written += 1;
            }
        }
    }
    Ok(written)
}

pub fn run(args: &[String]) -> Result {
    match args {
        [flag, old, new] if flag == "--from" => {
            return between(Path::new(old), Path::new(new), None);
        }
        [flag, old, new, stage, out] if flag == "--from" && stage == "--stage" => {
            return between(Path::new(old), Path::new(new), Some(Path::new(out)));
        }
        [flag, delta, installed, out] if flag == "--apply" => {
            return apply(Path::new(delta), Path::new(installed), Path::new(out));
        }
        _ => {}
    }
    let dir = args
        .first()
        .map_or_else(|| root().join("apps/gpui/dist"), PathBuf::from);
    if !dir.is_dir() {
        return Err(format!("{} is no folder of packages.", dir.display()));
    }
    let releases: serde_json::Value = serde_json::from_str(&output(
        "gh",
        &[
            "api",
            &format!("repos/{}/releases?per_page=30", crate::util::repository()),
        ],
    )?)
    .map_err(|cause| format!("The release list could not be read: {cause}"))?;
    let download = |from: &str, name: &str, folder: &Path| {
        run_with(
            Command::new("gh")
                .args([
                    "release",
                    "download",
                    &format!("v{from}"),
                    "--repo",
                    &crate::util::repository(),
                    "--pattern",
                    name,
                    "--dir",
                ])
                .arg(folder),
        )
    };
    let written = patches(
        &dir,
        &version()?,
        &releases,
        &root().join("target/package/deltas"),
        &download,
    )?;
    say(&format!(
        "{written} update patches written to {}.",
        dir.display()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--apply` stages a Windows patch as the updater does: the files, and the update installer
    /// beside them.
    #[test]
    fn a_windows_patch_is_staged_beside_its_installer() {
        let dir = std::env::temp_dir().join(format!("xtask-apply-{}", std::process::id()));
        let installed = dir.join("installed");
        std::fs::create_dir_all(installed.join("voice")).unwrap();
        std::fs::write(installed.join("KChess.exe"), b"old app").unwrap();
        std::fs::write(installed.join("voice/vosk.dll"), b"voice").unwrap();
        let old = Tree::read(&installed, |_| false).unwrap();
        let mut new = old.clone();
        new.insert(
            "KChess.exe",
            Node::File {
                data: b"new app".to_vec(),
                executable: false,
            },
        );
        let extras = vec![(WINDOWS_INSTALLER.to_string(), b"MZ update".to_vec())];
        let mut bytes = Vec::new();
        kchess_delta::build("1.0.0", "1.0.1", &old, &new, &extras, &mut bytes).unwrap();
        let delta = dir.join("win.delta");
        std::fs::write(&delta, bytes).unwrap();
        let out = dir.join("patch");
        apply(&delta, &installed, &out).unwrap();
        assert_eq!(
            std::fs::read(out.join("files/KChess.exe")).unwrap(),
            b"new app"
        );
        assert_eq!(
            std::fs::read(out.join("files/voice/vosk.dll")).unwrap(),
            b"voice"
        );
        assert_eq!(
            std::fs::read(out.join("KChess-update.exe")).unwrap(),
            b"MZ update"
        );
        std::fs::remove_file(installed.join("voice/vosk.dll")).unwrap();
        assert!(apply(&delta, &installed, &out).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_packages_installations_update_from_get_patches() {
        let kind = |name| package(name).map(|(kind, ..)| kind);
        assert_eq!(kind("KChess-2026.11.5-linux-amd64.deb"), Some(Asset::Deb));
        assert_eq!(
            kind("KChess-2026.11.5-linux-x86_64.AppImage"),
            Some(Asset::AppImage)
        );
        assert_eq!(kind("KChess-2026.11.5-mac-arm64.zip"), Some(Asset::MacZip));
        assert_eq!(
            kind("KChess-2026.11.5-win-x64.exe"),
            Some(Asset::WindowsInstaller)
        );
        for other in [
            "KChess-2026.11.5-win-x64-update.exe",
            "KChess-2026.11.5-mac-arm64.dmg",
            "KChess-2026.11.5-linux-amd64.snap",
            "SHA256SUMS.txt",
        ] {
            assert_eq!(kind(other), None, "{other}");
        }
    }

    /// Patches come from the newest published releases before this one, never a draft, a
    /// prerelease or a later release.
    #[test]
    fn patches_start_from_the_newest_published_releases() {
        let releases = serde_json::json!([
            { "tag_name": "v2026.11.6", "draft": true, "assets": [] },
            { "tag_name": "v2026.11.5", "assets": [{ "name": "a" }] },
            { "tag_name": "v2026.10.9", "assets": [] },
            { "tag_name": "v2026.11.4", "prerelease": true, "assets": [] },
            { "tag_name": "v2026.11.3", "assets": [{ "name": "b" }, { "name": "c" }] },
            { "tag_name": "v2026.11.10", "assets": [] },
            { "tag_name": "not-a-version", "assets": [] },
            { "tag_name": "v2026.11.2", "assets": [] },
            { "tag_name": "v2026.11.6", "assets": [] },
        ]);
        let found = previous("2026.11.6", &releases, 3);
        assert_eq!(
            found,
            vec![
                ("2026.11.5".to_string(), vec!["a".to_string()]),
                (
                    "2026.11.3".to_string(),
                    vec!["b".to_string(), "c".to_string()]
                ),
                ("2026.11.2".to_string(), Vec::new()),
            ]
        );
        assert!(previous("junk", &releases, 3).is_empty());
        assert_eq!(previous("2026.11.6", &releases, 1).len(), 1);
    }

    /// A macOS zip is read as the bundle it holds, links and modes kept, sequestered forks left out.
    /// Unix only: the zip crate writes Unix modes only on Unix, and a release's zip comes from
    /// `ditto` on macOS, so a zip written on Windows has no modes to read.
    #[cfg(unix)]
    #[test]
    fn a_mac_zip_is_read_as_its_bundle() {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = |mode: u32| zip::write::SimpleFileOptions::default().unix_permissions(mode);
        zip.add_directory("KChess.app/", options(0o755)).unwrap();
        zip.add_directory("KChess.app/Contents/", options(0o755))
            .unwrap();
        zip.start_file("KChess.app/Contents/MacOS/kchess", options(0o755))
            .unwrap();
        std::io::Write::write_all(&mut zip, b"app").unwrap();
        zip.start_file("KChess.app/Contents/Info.plist", options(0o644))
            .unwrap();
        std::io::Write::write_all(&mut zip, b"plist").unwrap();
        zip.add_symlink("KChess.app/Contents/Current", "MacOS", options(0o755))
            .unwrap();
        zip.start_file("__MACOSX/KChess.app/._Info.plist", options(0o644))
            .unwrap();
        std::io::Write::write_all(&mut zip, b"fork").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let tree = zip_tree(&bytes).unwrap();
        assert_eq!(tree.len(), 3);
        assert_eq!(
            tree.get("Contents/MacOS/kchess"),
            Some(&Node::File {
                data: b"app".to_vec(),
                executable: true
            })
        );
        assert_eq!(
            tree.get("Contents/Info.plist"),
            Some(&Node::File {
                data: b"plist".to_vec(),
                executable: false
            })
        );
        assert_eq!(
            tree.get("Contents/Current"),
            Some(&Node::Link("MacOS".into()))
        );

        let mut other = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        other.start_file("Other.app/x", options(0o644)).unwrap();
        assert!(zip_tree(&other.finish().unwrap().into_inner()).is_err());
        assert!(zip_tree(b"not a zip").is_err());
    }

    /// A patch is written only when it saves enough of the package.
    #[test]
    fn only_a_patch_worth_it_is_published() {
        let dir = std::env::temp_dir().join(format!("xtask-deltas-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Bytes that compress no more than a program's.
        let noise = |seed: u32| -> Vec<u8> {
            let mut state = seed;
            (0..200_000)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    state as u8
                })
                .collect()
        };
        let old = appimage_tree(noise(1));
        let mut changed = match old.get(APPIMAGE) {
            Some(Node::File { data, .. }) => data.clone(),
            _ => unreachable!(),
        };
        changed[1000..1010].copy_from_slice(b"0123456789");
        let new = appimage_tree(changed);
        let delta = dir.join("small.delta");
        assert!(write(("1.0.0", "1.0.1"), (&old, &new), &[], 200_000, &delta, true).unwrap());
        assert!(kchess_delta::Delta::open(&delta).is_ok());
        let other = appimage_tree(noise(2));
        let large = dir.join("large.delta");
        // The whole file is a little over 200 kB: over three quarters of 250 kB, under all of it.
        assert!(
            !write(
                ("1.0.0", "1.0.1"),
                (&old, &other),
                &[],
                250_000,
                &large,
                true
            )
            .unwrap()
        );
        assert!(!large.exists());
        // Trying a patch by hand writes it whatever its size.
        assert!(
            !write(
                ("1.0.0", "1.0.1"),
                (&old, &other),
                &[],
                200_000,
                &large,
                false
            )
            .unwrap()
        );
        assert!(large.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-deltas-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A stand-in for an NSIS installer: `/S /D=<folder>` installs `app` as KChess.exe, a voice
    /// library and the uninstaller every installer writes for itself.
    #[cfg(unix)]
    fn fake_installer(path: &Path, app: &str) {
        let script = format!(
            "#!/bin/sh\nfor a; do case \"$a\" in /D=*) d=\"${{a#/D=}}\";; esac; done\n\
             mkdir -p \"$d/voice\"\nprintf {app} > \"$d/KChess.exe\"\nprintf v > \"$d/voice/x.dll\"\n\
             printf u > \"$d/Uninstall KChess.exe\"\n"
        );
        std::fs::write(path, script).unwrap();
        crate::package::executable(path).unwrap();
    }

    /// A Windows installer is read as what it installs, run silently into a folder of its own,
    /// without the uninstaller it writes for itself.
    #[cfg(unix)]
    #[test]
    fn a_windows_installer_is_read_as_what_it_installs() {
        let dir = scratch("installed");
        let installer = dir.join("KChess-1.0.0-win-x64.exe");
        fake_installer(&installer, "old");
        let (tree, extras) = files(Asset::WindowsInstaller, &installer, &dir.join("work")).unwrap();
        assert!(extras.is_empty());
        let paths: Vec<&str> = tree.iter().map(|(path, _)| path).collect();
        assert_eq!(paths, ["KChess.exe", "voice/x.dll"]);
        assert!(matches!(tree.get("KChess.exe"), Some(Node::File { data, .. }) if data == b"old"));

        let nothing = dir.join("nothing.exe");
        std::fs::write(
            &nothing,
            "#!/bin/sh\nfor a; do case \"$a\" in /D=*) mkdir -p \"${a#/D=}\";; esac; done\n",
        )
        .unwrap();
        crate::package::executable(&nothing).unwrap();
        assert!(
            installed_tree(&nothing, &dir.join("empty"))
                .unwrap_err()
                .contains("installed nothing")
        );
        // An installer that put its files elsewhere names the folder it was given.
        let elsewhere = dir.join("elsewhere.exe");
        std::fs::write(&elsewhere, "#!/bin/sh\n").unwrap();
        crate::package::executable(&elsewhere).unwrap();
        let refused = installed_tree(&elsewhere, &dir.join("none")).unwrap_err();
        let folder = installed_folder(&dir.join("none"));
        assert!(
            refused.ends_with(&format!("installed nothing in {}.", folder.display())),
            "{refused}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Each kind of package is read as the files it installs.
    #[test]
    fn packages_are_read_by_their_kind() {
        let dir = scratch("kinds");
        let image = dir.join("KChess-1.0.0-linux-x86_64.AppImage");
        std::fs::write(&image, b"an AppImage").unwrap();
        let (tree, extras) = files(Asset::AppImage, &image, &dir).unwrap();
        assert_eq!(
            tree.get(APPIMAGE),
            Some(&Node::File {
                data: b"an AppImage".to_vec(),
                executable: true
            })
        );
        assert!(extras.is_empty());

        let tar = crate::deb::tarball(&[("opt/KChess/kchess".into(), b"app".to_vec(), 0o755)], &[])
            .unwrap();
        let deb = dir.join("KChess-1.0.0-linux-amd64.deb");
        std::fs::write(
            &deb,
            crate::deb::ar(&[
                ("debian-binary", b"2.0\n"),
                ("control.tar.gz", b"ctl"),
                ("data.tar", &tar),
            ]),
        )
        .unwrap();
        let (tree, extras) = files(Asset::Deb, &deb, &dir).unwrap();
        assert!(tree.get("opt/KChess/kchess").is_some());
        assert_eq!(
            extras,
            vec![("control.tar.gz".to_string(), b"ctl".to_vec())]
        );

        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(
            "KChess.app/Contents/Info.plist",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        std::io::Write::write_all(&mut zip, b"plist").unwrap();
        let path = dir.join("KChess-1.0.0-mac-arm64.zip");
        std::fs::write(&path, zip.finish().unwrap().into_inner()).unwrap();
        let (tree, _) = files(Asset::MacZip, &path, &dir).unwrap();
        assert!(tree.get("Contents/Info.plist").is_some());
        assert!(files(Asset::Deb, &dir.join("missing.deb"), &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Windows patch carries the update installer packaging left for its version.
    #[test]
    fn a_windows_patch_carries_the_update_installer() {
        let version = format!("0.0.{}", std::process::id());
        assert!(
            windows_extras(&version)
                .unwrap_err()
                .contains("cargo xtask package")
        );
        let path = crate::package::windows::update_installer(&crate::package::Identity::new(
            version.clone(),
        ));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"MZ").unwrap();
        assert_eq!(
            windows_extras(&version).unwrap(),
            vec![(WINDOWS_INSTALLER.to_string(), b"MZ".to_vec())]
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_patch_is_reported_with_its_sizes() {
        let stats = kchess_delta::Stats {
            copied: 1,
            patched: 2,
            whole: 3,
            files_size: 0,
            delta_size: 2_500_000,
        };
        assert_eq!(
            report("x.delta", &stats, 120_000_000, false),
            "x.delta: 2.5 MB for a 120.0 MB package (1 files copied, 2 patched, 3 whole); too large to publish"
        );
        assert!(report("x.delta", &stats, 120_000_000, true).ends_with("3 whole)"));
    }

    /// `--from` writes the one patch between two packages beside the new one; `--apply` stages it.
    #[cfg(unix)]
    #[test]
    fn patches_are_made_and_applied_by_hand() {
        let dir = scratch("by-hand");
        let old = dir.join("KChess-1.0.0-linux-x86_64.AppImage");
        let new = dir.join("KChess-1.0.1-linux-x86_64.AppImage");
        std::fs::write(&old, vec![3u8; 50_000]).unwrap();
        std::fs::write(&new, vec![4u8; 50_000]).unwrap();
        let args = |list: &[&Path]| -> Vec<String> {
            list.iter()
                .map(|path| path.to_string_lossy().to_string())
                .collect()
        };
        let mut from = vec!["--from".to_string()];
        from.extend(args(&[&old, &new]));
        run(&from).unwrap();
        let delta = dir.join("KChess-1.0.1-linux-x86_64.from-1.0.0.delta");
        assert!(kchess_delta::Delta::open(&delta).is_ok());
        let deb = dir.join("KChess-1.0.0-linux-amd64.deb");
        std::fs::write(&deb, b"").unwrap();
        let mut mixed = vec!["--from".to_string()];
        mixed.extend(args(&[&deb, &new]));
        assert!(run(&mixed).unwrap_err().contains("one kind"));
        let mut staged = from.clone();
        staged.extend(["--stage".to_string(), "out".to_string()]);
        assert!(run(&staged).unwrap_err().contains("Only a Windows patch"));

        // A Windows patch carries its update installer, which packaging leaves for the version,
        // and is staged over the files the old installer installed.
        let version = format!("1.{}.1", std::process::id());
        let (win_old, win_new) = (
            dir.join("KChess-1.0.0-win-x64.exe"),
            dir.join(format!("KChess-{version}-win-x64.exe")),
        );
        fake_installer(&win_old, "old");
        fake_installer(&win_new, "new");
        let update = crate::package::windows::update_installer(&crate::package::Identity::new(
            version.clone(),
        ));
        std::fs::create_dir_all(update.parent().unwrap()).unwrap();
        std::fs::write(&update, b"MZ update").unwrap();
        let out = dir.join("staged");
        let mut windows = vec!["--from".to_string()];
        windows.extend(args(&[&win_old, &win_new]));
        windows.push("--stage".to_string());
        windows.extend(args(&[&out]));
        run(&windows).unwrap();
        let _ = std::fs::remove_file(&update);
        assert!(
            dir.join(format!("KChess-{version}-win-x64.from-1.0.0.delta"))
                .exists()
        );
        assert_eq!(std::fs::read(out.join("files/KChess.exe")).unwrap(), b"new");
        assert_eq!(
            std::fs::read(out.join("KChess-update.exe")).unwrap(),
            b"MZ update"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Only `--from` with two packages (and `--stage <out>`) and `--apply` with three paths are
    /// the by-hand forms;
    /// anything else names the folder of a release's packages.
    #[test]
    fn other_arguments_name_the_packages_folder() {
        let missing = |args: &[&str]| {
            let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
            run(&args).unwrap_err()
        };
        assert!(missing(&["/no/such/dist", "a", "b"]).contains("is no folder of packages"));
        assert!(missing(&["/no/such/dist", "a", "b", "c"]).contains("is no folder of packages"));
        // `--stage` only after `--from` and two packages.
        for args in [
            ["--from", "a", "b", "--into", "c"],
            ["/no/such/dist", "a", "b", "--stage", "c"],
        ] {
            assert!(
                missing(&args).contains("is no folder of packages"),
                "{args:?}"
            );
        }
    }

    /// In release CI, each package gets a patch from each earlier release that shipped one, its
    /// package fetched once.
    #[test]
    fn the_release_writes_a_patch_from_each_earlier_release() {
        let dir = scratch("release");
        let work = dir.join("work");
        let dist = dir.join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        assert!(
            patches(
                &dist,
                "1.0.2",
                &serde_json::json!([]),
                &work,
                &|_, _, _| Ok(())
            )
            .is_err()
        );
        let program: Vec<u8> = (0..100_000u32).map(|n| (n * 7 % 251) as u8).collect();
        let mut next = program.clone();
        next[500] ^= 1;
        std::fs::write(dist.join("KChess-1.0.2-linux-x86_64.AppImage"), &next).unwrap();
        std::fs::write(dist.join("SHA256SUMS.txt"), b"").unwrap();
        let releases = serde_json::json!([
            { "tag_name": "v1.0.1", "assets": [{ "name": "KChess-1.0.1-linux-x86_64.AppImage" }] },
            { "tag_name": "v1.0.0", "assets": [{ "name": "KChess-1.0.0-mac-arm64.zip" }] },
        ]);
        let fetched = std::cell::RefCell::new(Vec::new());
        let download = |from: &str, name: &str, folder: &Path| -> Result {
            fetched.borrow_mut().push(format!("{from} {name}"));
            std::fs::write(folder.join(name), &program).map_err(|cause| cause.to_string())
        };
        assert_eq!(
            patches(&dist, "1.0.2", &releases, &work, &download).unwrap(),
            1
        );
        assert!(
            dist.join("KChess-1.0.2-linux-x86_64.from-1.0.1.delta")
                .exists()
        );
        assert!(
            !dist
                .join("KChess-1.0.2-linux-x86_64.from-1.0.0.delta")
                .exists()
        );
        assert_eq!(
            *fetched.borrow(),
            ["1.0.1 KChess-1.0.1-linux-x86_64.AppImage"]
        );
        // A package already fetched is not fetched again.
        assert_eq!(
            patches(&dist, "1.0.2", &releases, &work, &download).unwrap(),
            1
        );
        assert_eq!(fetched.borrow().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
