//! Linux: an AppImage (made with appimagetool, pinned by its digest) and a `.deb` written here.
//!
//! The `.deb` keeps the Electron app's: package `kchess`, the app in `/opt/KChess`, the
//! `/usr/bin/kchess` link through update-alternatives, its desktop entry and icon, so installing
//! it (or Electron's updater) upgrades the Electron package in place and dpkg removes its files.

#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Built, Identity, copy_program, copy_voice};
use crate::util::{Result, root, run_with, say, sha256_file};

const APPIMAGETOOL: &str =
    "https://github.com/AppImage/appimagetool/releases/download/1.9.0/appimagetool-x86_64.AppImage";
const APPIMAGETOOL_SHA256: &str =
    "46fdd785094c7f6e545b61afcfb0f3d98d8eab243f644b4b17698c01d06083d1";

/// What the app needs from the system (GPUI on X11 or Wayland with Vulkan, ALSA for sound).
const DEPENDS: &str = "libc6 (>= 2.35), libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, \
libvulkan1, libfontconfig1, libfreetype6, libx11-6, libx11-xcb1, libxcb1, libasound2t64 | libasound2, \
libdbus-1-3";

pub fn desktop_entry(identity: &Identity, exec: &str) -> String {
    format!(
        "[Desktop Entry]\nName={}\nExec={exec} %U\nTerminal=false\nType=Application\nIcon={}\nStartupWMClass={}\nComment={}\nCategories=Game;BoardGame;\n",
        identity.name,
        identity.linux_exe,
        identity.linux_exe,
        super::SYNOPSIS
    )
}

/// The app folder: the launcher, Stockfish, the core service, the command line and voice.
fn stage_folder(identity: &Identity, built: &Built, folder: &Path) -> Result {
    copy_program(&built.app, &folder.join(identity.linux_exe))?;
    copy_program(&built.stockfish, &folder.join("stockfish"))?;
    copy_program(&built.service, &folder.join("kchess-service"))?;
    copy_program(&built.cli, &folder.join("kchess-cli"))?;
    copy_voice(built, folder)?;
    std::fs::copy(&built.licenses, folder.join("THIRD_PARTY_LICENSES.txt"))
        .map_err(|cause| cause.to_string())?;
    Ok(())
}

fn appimage(identity: &Identity, built: &Built, stage: &Path, out: &Path) -> Result<PathBuf> {
    let tool = root().join("target/package/appimagetool-1.9.0");
    if !tool.exists() || sha256_file(&tool)? != APPIMAGETOOL_SHA256 {
        say("Downloading appimagetool…");
        crate::util::download(APPIMAGETOOL, &tool)?;
        if sha256_file(&tool)? != APPIMAGETOOL_SHA256 {
            let _ = std::fs::remove_file(&tool);
            return Err("appimagetool's checksum does not match.".into());
        }
        super::executable(&tool)?;
    }
    let dir = stage.join("AppDir");
    stage_folder(identity, built, &dir)?;
    let icon = format!("{}.png", identity.linux_exe);
    std::fs::copy(root().join("apps/gpui/build/icon.png"), dir.join(&icon))
        .map_err(|cause| cause.to_string())?;
    std::os::unix::fs::symlink(&icon, dir.join(".DirIcon")).map_err(|cause| cause.to_string())?;
    std::fs::write(
        dir.join(format!("{}.desktop", identity.linux_exe)),
        desktop_entry(identity, identity.linux_exe),
    )
    .map_err(|cause| cause.to_string())?;
    let app_run = dir.join("AppRun");
    std::fs::write(
        &app_run,
        format!(
            "#!/bin/sh\nHERE=\"$(dirname \"$(readlink -f \"$0\")\")\"\nexec \"$HERE/{}\" \"$@\"\n",
            identity.linux_exe
        ),
    )
    .map_err(|cause| cause.to_string())?;
    super::executable(&app_run)?;
    let image = out.join(format!(
        "{}-{}-linux-x86_64.AppImage",
        identity.prefix(),
        identity.version
    ));
    say("Making the AppImage…");
    run_with(
        Command::new(&tool)
            .arg("--no-appstream")
            .arg(&dir)
            .arg(&image)
            .env("ARCH", "x86_64")
            // Runners have no FUSE: the tool runs from its own extracted files.
            .env("APPIMAGE_EXTRACT_AND_RUN", "1"),
    )?;
    Ok(image)
}

/// A tar.gz of `entries` (path in the archive → contents and mode), with root ownership.
fn tarball(entries: &[(String, Vec<u8>, u32)], dirs: &[String]) -> Result<Vec<u8>> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    let mut archive = tar::Builder::new(encoder);
    let header = |path: &str, size: u64, mode: u32, kind: tar::EntryType| -> Result<tar::Header> {
        let mut header = tar::Header::new_gnu();
        header.set_path(path).map_err(|cause| cause.to_string())?;
        header.set_size(size);
        header.set_mode(mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_entry_type(kind);
        header.set_cksum();
        Ok(header)
    };
    for dir in dirs {
        let header = header(&format!("./{dir}/"), 0, 0o755, tar::EntryType::Directory)?;
        archive
            .append(&header, std::io::empty())
            .map_err(|cause| cause.to_string())?;
    }
    for (path, data, mode) in entries {
        let header = header(
            &format!("./{path}"),
            data.len() as u64,
            *mode,
            tar::EntryType::Regular,
        )?;
        archive
            .append(&header, data.as_slice())
            .map_err(|cause| cause.to_string())?;
    }
    archive
        .into_inner()
        .and_then(|encoder| encoder.finish())
        .map_err(|cause| cause.to_string())
}

/// An `ar` archive, as dpkg reads it.
fn ar(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = b"!<arch>\n".to_vec();
    for (name, data) in members {
        let header = format!(
            "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            name,
            0,
            0,
            0,
            "100644",
            data.len()
        );
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(b'\n');
        }
    }
    out
}

pub fn control(identity: &Identity, installed_kb: u64) -> String {
    format!(
        "Package: {}\nVersion: {}\nArchitecture: amd64\nMaintainer: {}\nInstalled-Size: {installed_kb}\nDepends: {DEPENDS}\nSection: games\nPriority: optional\nHomepage: {}\nDescription: {}\n {}\nReplaces: kchess-electron\nConflicts: kchess-electron\n",
        identity.package,
        identity.version,
        super::AUTHOR,
        super::HOMEPAGE,
        super::SYNOPSIS,
        super::DESCRIPTION
    )
}

fn deb(identity: &Identity, built: &Built, stage: &Path, out: &Path) -> Result<PathBuf> {
    let folder = stage.join("deb").join(identity.name);
    stage_folder(identity, built, &folder)?;
    let opt = format!("opt/{}", identity.name);
    let mut entries = Vec::new();
    let mut dirs = vec!["opt".to_string(), opt.clone(), format!("{opt}/voice")];
    let mut size = 0u64;
    for path in walk(&folder) {
        let relative = path
            .strip_prefix(&folder)
            .map_err(|cause| cause.to_string())?;
        let data = std::fs::read(&path).map_err(|cause| cause.to_string())?;
        size += data.len() as u64;
        let name = relative.to_string_lossy().replace('\\', "/");
        let mode = if name.ends_with(".txt") { 0o644 } else { 0o755 };
        entries.push((format!("{opt}/{name}"), data, mode));
    }
    let exec = format!("/opt/{}/{}", identity.name, identity.linux_exe);
    for dir in [
        "usr",
        "usr/share",
        "usr/share/applications",
        "usr/share/icons",
        "usr/share/icons/hicolor",
        "usr/share/icons/hicolor/512x512",
        "usr/share/icons/hicolor/512x512/apps",
    ] {
        dirs.push(dir.to_string());
    }
    entries.push((
        format!("usr/share/applications/{}.desktop", identity.linux_exe),
        desktop_entry(identity, &format!("\"{exec}\"")).into_bytes(),
        0o644,
    ));
    entries.push((
        format!(
            "usr/share/icons/hicolor/512x512/apps/{}.png",
            identity.linux_exe
        ),
        std::fs::read(root().join("apps/gpui/build/icon.png"))
            .map_err(|cause| cause.to_string())?,
        0o644,
    ));
    let cli = format!("/opt/{}/kchess-cli", identity.name);
    let cli_link = "kchess-cli";
    let postinst = format!(
        "#!/bin/sh\nset -e\nif type update-alternatives >/dev/null 2>&1; then\n  update-alternatives --install '/usr/bin/{exe}' '{exe}' '{exec}' 100 || ln -sf '{exec}' '/usr/bin/{exe}'\n  update-alternatives --install '/usr/bin/{cli_link}' '{cli_link}' '{cli}' 100 || ln -sf '{cli}' '/usr/bin/{cli_link}'\nelse\n  ln -sf '{exec}' '/usr/bin/{exe}'\n  ln -sf '{cli}' '/usr/bin/{cli_link}'\nfi\nif type update-desktop-database >/dev/null 2>&1; then update-desktop-database -q /usr/share/applications || true; fi\n",
        exe = identity.linux_exe
    );
    let postrm = format!(
        "#!/bin/sh\nset -e\nif [ \"$1\" = remove ] || [ \"$1\" = purge ]; then\n  if type update-alternatives >/dev/null 2>&1; then\n    update-alternatives --remove '{exe}' '{exec}' || true\n    update-alternatives --remove '{cli_link}' '{cli}' || true\n  fi\n  rm -f '/usr/bin/{exe}' '/usr/bin/{cli_link}'\nfi\n",
        exe = identity.linux_exe
    );
    let data = tarball(&entries, &dirs)?;
    let control = tarball(
        &[
            (
                "control".into(),
                control(identity, size / 1024).into_bytes(),
                0o644,
            ),
            ("postinst".into(), postinst.into_bytes(), 0o755),
            ("postrm".into(), postrm.into_bytes(), 0o755),
        ],
        &[],
    )?;
    let package = out.join(format!(
        "{}-{}-linux-amd64.deb",
        identity.prefix(),
        identity.version
    ));
    say("Making the .deb…");
    let mut file = std::fs::File::create(&package).map_err(|cause| cause.to_string())?;
    file.write_all(&ar(&[
        ("debian-binary", b"2.0\n"),
        ("control.tar.gz", &control),
        ("data.tar.gz", &data),
    ]))
    .map_err(|cause| cause.to_string())?;
    Ok(package)
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path))
        } else {
            files.push(path)
        }
    }
    files.sort();
    files
}

pub fn package(
    identity: &Identity,
    built: &Built,
    stage: &Path,
    out: &Path,
) -> Result<Vec<PathBuf>> {
    if std::env::consts::ARCH != "x86_64" {
        return Err("Linux packages are x64 only.".into());
    }
    let image = appimage(identity, built, stage, out)?;
    let deb = deb(identity, built, stage, out)?;
    Ok(vec![image, deb])
}

/// The Snap's recipe: the staged app folder dumped into the Snap, with the GNOME extension for the
/// desktop libraries and Mesa's Vulkan drivers for GPUI. Its name, plugs and confinement are the
/// Electron Snap's, and its profile (`$SNAP_USER_DATA/.config/KChess`) is the same folder, so the
/// store's update replaces the Electron Snap with this app and keeps everything.
pub fn snapcraft_yaml(version: &str) -> String {
    format!(
        r#"name: kchess
title: KChess
version: '{version}'
summary: Play, study and improve at chess with Stockfish and Lichess
description: |
  {}
base: core24
grade: stable
confinement: strict
license: GPL-3.0-or-later
website: https://github.com/ketansingrole/KChess-V2#readme
source-code: https://github.com/ketansingrole/KChess-V2
issues: https://github.com/ketansingrole/KChess-V2/issues
contact: https://github.com/ketansingrole/KChess-V2/issues
platforms:
  amd64:

apps:
  kchess:
    command: app/kchess
    extensions: [gnome]
    desktop: app/kchess.desktop
    plugs:
      - desktop
      - desktop-legacy
      - home
      - x11
      - wayland
      - network
      - network-bind
      - opengl
      - audio-playback
      - audio-record
      - password-manager-service

parts:
  kchess:
    plugin: dump
    source: app
    organize:
      '*': app/
    stage-packages:
      - libvulkan1
      - mesa-vulkan-drivers
      - libxkbcommon-x11-0
      - libx11-xcb1
      - libasound2t64
"#,
        super::DESCRIPTION
    )
}

fn snap(identity: &Identity, built: &Built, stage: &Path, out: &Path) -> Result<PathBuf> {
    let project = stage.join("snap");
    let app = project.join("app");
    stage_folder(identity, built, &app)?;
    std::fs::copy(
        root().join("apps/gpui/build/icon.png"),
        app.join("kchess.png"),
    )
    .map_err(|cause| cause.to_string())?;
    let entry =
        desktop_entry(identity, "kchess").replace("Icon=kchess", "Icon=${SNAP}/app/kchess.png");
    std::fs::write(app.join("kchess.desktop"), entry).map_err(|cause| cause.to_string())?;
    std::fs::create_dir_all(project.join("snap")).map_err(|cause| cause.to_string())?;
    std::fs::write(
        project.join("snap/snapcraft.yaml"),
        snapcraft_yaml(&identity.version),
    )
    .map_err(|cause| cause.to_string())?;
    let package = out.join(format!("KChess-{}-linux-amd64.snap", identity.version));
    say("Making the Snap…");
    // snapcraft writes only inside its project folder; the package is moved out afterwards.
    let packed = project.join("kchess.snap");
    let mut snapcraft = Command::new("snapcraft");
    snapcraft
        .args(["pack", "--output", "kchess.snap"])
        .current_dir(&project);
    // CI builds in LXD (KCHESS_SNAP_LXD=1); elsewhere snapcraft chooses.
    if std::env::var("KCHESS_SNAP_LXD").as_deref() == Ok("1") {
        snapcraft.arg("--use-lxd");
    }
    let status = snapcraft
        .status()
        .map_err(|cause| format!("Could not run snapcraft: {cause}"))?;
    if !status.success() {
        return Err(format!("snapcraft failed ({status})."));
    }
    std::fs::rename(&packed, &package)
        .or_else(|_| std::fs::copy(&packed, &package).map(|_| ()))
        .map_err(|cause| format!("Cannot move the Snap to {}: {cause}", package.display()))?;
    Ok(package)
}

pub fn package_snap(
    identity: &Identity,
    built: &Built,
    stage: &Path,
    out: &Path,
) -> Result<Vec<PathBuf>> {
    snap(identity, built, stage, out).map(|snap| vec![snap])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snap_keeps_the_electron_snaps_name_and_plugs() {
        let text = snapcraft_yaml("2026.10.16");
        assert!(text.starts_with("name: kchess\n"));
        assert!(text.contains("version: '2026.10.16'"));
        assert!(text.contains("confinement: strict"));
        assert!(text.contains("- password-manager-service"));
        assert!(text.contains("- audio-record"));
    }

    #[test]
    fn ar_members_are_padded_to_even_lengths() {
        let archive = ar(&[("debian-binary", b"2.0\n"), ("odd", b"abc")]);
        assert!(archive.starts_with(
            b"!<arch>\ndebian-binary   0           0     0     100644  4         `\n2.0\n"
        ));
        assert!(archive.ends_with(b"abc\n"));
    }

    #[test]
    fn the_final_deb_replaces_the_electron_package() {
        let text = control(&Identity::new("2026.10.16".into()), 100);
        assert!(text.starts_with("Package: kchess\nVersion: 2026.10.16\nArchitecture: amd64\n"));
        assert!(text.contains("Replaces: kchess-electron\n"));
    }
}
