//! Linux: an AppImage (made with appimagetool, pinned by its digest) and a `.deb` written here.
//!
//! The `.deb` keeps the Electron app's: package `kchess`, the app in `/opt/KChess`, the
//! `/usr/bin/kchess` link through update-alternatives, its desktop entry and icon, so installing
//! it (or Electron's updater) upgrades the Electron package in place and dpkg removes its files.

#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::path::{Path, PathBuf};
use std::process::Command;

use kchess_delta::Asset;

use super::{Built, Identity, copy_program, copy_voice};
use crate::deb::{ar, gzip, tarball, xz};
use crate::util::{Result, download, pinned, root, run_with, say};

// The pins below change only in a reviewed pull request, which CI packages and tests. The
// weekly Pins workflow (`cargo xtask pins --check`) opens an issue when either has a newer release.
pub const APPIMAGETOOL: &str =
    "https://github.com/AppImage/appimagetool/releases/download/1.9.0/appimagetool-x86_64.AppImage";
pub const APPIMAGETOOL_SHA256: &str =
    "46fdd785094c7f6e545b61afcfb0f3d98d8eab243f644b4b17698c01d06083d1";
/// The runtime appimagetool puts at the front of every AppImage: the launcher that mounts the
/// app and starts it. Without `--runtime-file`, appimagetool would fetch the moving `continuous`
/// release on every build, unverified.
pub const APPIMAGE_RUNTIME: &str =
    "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64";
pub const APPIMAGE_RUNTIME_SHA256: &str =
    "2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d";

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

/// The app folder: the launcher, Stockfish and Fairy-Stockfish, the core service, the command
/// line and voice.
fn stage_folder(identity: &Identity, built: &Built, folder: &Path) -> Result {
    copy_program(&built.app, &folder.join(identity.linux_exe))?;
    copy_program(&built.stockfish, &folder.join("stockfish"))?;
    copy_program(&built.fairy, &folder.join("fairy-stockfish"))?;
    copy_program(&built.service, &folder.join("kchess-service"))?;
    copy_program(&built.cli, &folder.join("kchess-cli"))?;
    copy_voice(built, folder)?;
    std::fs::copy(&built.licenses, folder.join("THIRD_PARTY_LICENSES.txt"))
        .map_err(|cause| cause.to_string())?;
    Ok(())
}

fn appimage(identity: &Identity, built: &Built, stage: &Path, out: &Path) -> Result<PathBuf> {
    let tools = AppImageTools {
        cache: root().join("target/package"),
        appimagetool: (APPIMAGETOOL, APPIMAGETOOL_SHA256),
        runtime: (APPIMAGE_RUNTIME, APPIMAGE_RUNTIME_SHA256),
        fetch: download,
    };
    appimage_with(identity, built, stage, out, &tools)
}

/// Where the AppImage step keeps its pinned tools, each as (URL, SHA-256), and how it fetches them.
struct AppImageTools<F> {
    cache: PathBuf,
    appimagetool: (&'static str, &'static str),
    runtime: (&'static str, &'static str),
    fetch: F,
}

fn appimage_with(
    identity: &Identity,
    built: &Built,
    stage: &Path,
    out: &Path,
    tools: &AppImageTools<impl Fn(&str, &Path) -> Result>,
) -> Result<PathBuf> {
    let tool = tools.cache.join("appimagetool-x86_64.AppImage");
    let (url, sha256) = tools.appimagetool;
    pinned("appimagetool", url, sha256, &tool, &tools.fetch)?;
    super::executable(&tool)?;
    let runtime = tools.cache.join("appimage-runtime-x86_64");
    let (url, sha256) = tools.runtime;
    pinned("the AppImage runtime", url, sha256, &runtime, &tools.fetch)?;
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
    let image = out.join(identity.asset(Asset::AppImage));
    say("Making the AppImage…");
    run_with(&mut appimagetool(&tool, &runtime, &dir, &image))?;
    Ok(image)
}

/// appimagetool making `image` from `dir` around the pinned `runtime`, so it downloads nothing.
fn appimagetool(tool: &Path, runtime: &Path, dir: &Path, image: &Path) -> Command {
    let mut command = Command::new(tool);
    command
        .arg("--no-appstream")
        .arg("--runtime-file")
        .arg(runtime)
        .arg(dir)
        .arg(image)
        .env("ARCH", "x86_64")
        // Runners have no FUSE: the tool runs from its own extracted files.
        .env("APPIMAGE_EXTRACT_AND_RUN", "1");
    command
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
    let data = xz(&tarball(&entries, &dirs)?)?;
    let control = gzip(&tarball(
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
    )?)?;
    let package = out.join(identity.asset(Asset::Deb));
    say("Making the .deb…");
    std::fs::write(
        &package,
        ar(&[
            ("debian-binary", b"2.0\n"),
            ("control.tar.gz", &control),
            ("data.tar.xz", &data),
        ]),
    )
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
website: {homepage}
source-code: https://github.com/{repository}
issues: https://github.com/{repository}/issues
contact: https://github.com/{repository}/issues
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
        super::DESCRIPTION,
        homepage = super::HOMEPAGE,
        repository = crate::util::REPOSITORY,
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
    let package = out.join(identity.asset(Asset::Snap));
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

    fn digest(bytes: &[u8]) -> &'static str {
        kchess_delta::digest(bytes).leak()
    }

    /// Stand-ins for what packaging builds, in `dir`: each program holds its own name.
    fn built(dir: &Path) -> Built {
        let voice = dir.join("voice");
        std::fs::create_dir_all(&voice).unwrap();
        std::fs::write(voice.join("libvosk.so"), b"vosk").unwrap();
        std::fs::write(voice.join("NOTICE.txt"), b"notice").unwrap();
        let file = |name: &str| {
            let path = dir.join(name);
            std::fs::write(&path, name).unwrap();
            path
        };
        Built {
            app: file("app"),
            service: file("service"),
            cli: file("cli"),
            stockfish: file("stockfish"),
            fairy: file("fairy"),
            voice,
            licenses: file("licenses"),
        }
    }

    /// The AppImage step fetches both pinned tools into its cache and runs appimagetool on the
    /// staged app with the pinned runtime, from its own extracted files (runners have no FUSE):
    /// a stand-in appimagetool records its environment and arguments and makes the "image" from
    /// the runtime it was handed.
    #[test]
    fn the_appimage_is_made_around_the_pinned_runtime() {
        let scratch = tempfile::tempdir().unwrap();
        let scratch = scratch.path();
        let built = built(&scratch.join("built"));
        let tool = b"#!/bin/sh\nprintf '%s\\n' \"$ARCH\" \"$APPIMAGE_EXTRACT_AND_RUN\" \"$@\" > \"$5.args\"\ncp \"$3\" \"$5\"\n";
        let runtime = b"the pinned runtime";
        let tools = AppImageTools {
            cache: scratch.join("cache"),
            appimagetool: ("https://example.test/appimagetool", digest(tool)),
            runtime: ("https://example.test/runtime", digest(runtime)),
            fetch: |url: &str, to: &Path| -> Result {
                std::fs::create_dir_all(to.parent().unwrap()).unwrap();
                let bytes: &[u8] = if url.ends_with("/runtime") {
                    runtime
                } else {
                    tool
                };
                std::fs::write(to, bytes).map_err(|cause| cause.to_string())
            },
        };
        let identity = Identity::new("2026.11.9".into());
        let (stage, out) = (scratch.join("stage"), scratch.join("out"));
        std::fs::create_dir_all(&out).unwrap();
        let image = appimage_with(&identity, &built, &stage, &out, &tools).unwrap();

        assert_eq!(image, out.join("KChess-2026.11.9-linux-x86_64.AppImage"));
        assert_eq!(std::fs::read(&image).unwrap(), runtime);
        let args = std::fs::read_to_string(image.with_extension("AppImage.args")).unwrap();
        let cached = tools.cache.join("appimage-runtime-x86_64");
        let app_dir = stage.join("AppDir");
        assert_eq!(
            args,
            format!(
                "x86_64\n1\n--no-appstream\n--runtime-file\n{}\n{}\n{}\n",
                cached.display(),
                app_dir.display(),
                image.display()
            )
        );
        for name in [
            "AppRun",
            "kchess",
            "kchess.desktop",
            "kchess.png",
            "stockfish",
            "fairy-stockfish",
            "kchess-service",
            "kchess-cli",
            "voice/libvosk.so",
            "THIRD_PARTY_LICENSES.txt",
        ] {
            assert!(
                app_dir.join(name).exists(),
                "{name} is missing from the AppDir"
            );
        }
    }

    #[test]
    fn the_appimage_pins_name_fixed_releases() {
        for url in [APPIMAGETOOL, APPIMAGE_RUNTIME] {
            assert!(!url.contains("/continuous/"), "{url} is a moving release");
        }
        for digest in [APPIMAGETOOL_SHA256, APPIMAGE_RUNTIME_SHA256] {
            assert_eq!(digest.len(), 64);
            assert!(
                digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            );
        }
    }

    #[test]
    fn the_snap_keeps_the_electron_snaps_name_and_plugs() {
        let text = snapcraft_yaml("2026.10.16");
        assert!(text.starts_with("name: kchess\n"));
        assert!(text.contains("version: '2026.10.16'"));
        assert!(text.contains("confinement: strict"));
        assert!(text.contains("- password-manager-service"));
        assert!(text.contains("- audio-record"));
        assert!(text.contains(
            "website: https://ketansingrole.github.io/KChess-V2/\nsource-code: https://github.com/ketansingrole/KChess-V2\n"
        ));
    }

    /// The .deb holds the staged programs under /opt/KChess, with the launcher and icon, and the
    /// control archive that replaces the Electron package, as dpkg installs them (read back as
    /// the update patches read a release's .deb).
    #[test]
    fn the_deb_installs_the_app_in_opt() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        let built = built(dir);
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let identity = Identity::new("2026.11.9".into());
        let package = deb(&identity, &built, &dir.join("stage"), &out).unwrap();
        assert!(package.ends_with("KChess-2026.11.9-linux-amd64.deb"));
        let (tree, (name, control)) = crate::deb::read(&std::fs::read(&package).unwrap()).unwrap();
        assert_eq!(name, "control.tar.gz");
        let mut control_tar = Vec::new();
        std::io::Read::read_to_end(
            &mut flate2::read::GzDecoder::new(control.as_slice()),
            &mut control_tar,
        )
        .unwrap();
        let control_text = String::from_utf8_lossy(&control_tar);
        assert!(control_text.contains("Package: kchess\nVersion: 2026.11.9\n"));
        assert!(control_text.contains("Replaces: kchess-electron\n"));
        let file_of = |path: &str| match tree.get(path) {
            Some(kchess_delta::Node::File { data, executable }) => (data.clone(), *executable),
            other => panic!("{path}: {other:?}"),
        };
        assert_eq!(file_of("opt/KChess/kchess"), (b"app".to_vec(), true));
        assert_eq!(
            file_of("opt/KChess/stockfish"),
            (b"stockfish".to_vec(), true)
        );
        assert_eq!(file_of("opt/KChess/kchess-cli"), (b"cli".to_vec(), true));
        assert_eq!(
            file_of("opt/KChess/voice/NOTICE.txt"),
            (b"notice".to_vec(), false)
        );
        assert!(tree.get("usr/share/applications/kchess.desktop").is_some());
        assert!(
            tree.get("usr/share/icons/hicolor/512x512/apps/kchess.png")
                .is_some()
        );
    }
}
