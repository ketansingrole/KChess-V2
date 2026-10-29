//! macOS: the `.app` bundle, signed (ad hoc, or Developer ID with the hardened runtime and
//! notarization), as a `.zip` (what the updaters install) and a `.dmg` (what people download).
//!
//! The zip also holds `legacy/KChess.app`, the Electron bridge (`apps/electron-bridge`): Electron
//! releases before 2026.10.12 were `com.kchess.electron`, and Squirrel.Mac only installs a bundle
//! with the running app's identifier, which it looks for anywhere in the zip. The bridge has that
//! identifier and puts this KChess in its place. Its executable is named like the app because
//! ShipIt names the installed folder after the update's executable (`SQRLInstaller.m`), so with
//! any other name an update left `/Applications/<executable>.app`, which Spotlight never finds.

use std::path::{Path, PathBuf};

use super::{Built, Identity, copy_program, copy_voice};
use crate::util::{Result, root, run, say};

const MICROPHONE: &str =
    "KChess uses your microphone to recognize spoken chess moves and square names on this device.";

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The identifier of the Electron releases before 2026.10.12, which the bridge carries.
const ELECTRON_APP_ID: &str = "com.kchess.electron";

pub fn info_plist(identity: &Identity) -> String {
    plist(identity, identity.app_id, "kchess", false)
}

/// The bridge's: Electron's identifier, and no Dock icon while it works.
fn bridge_plist(identity: &Identity) -> String {
    plist(identity, ELECTRON_APP_ID, identity.name, true)
}

fn plist(identity: &Identity, app_id: &str, executable: &str, background: bool) -> String {
    let strings = [
        ("CFBundleName", identity.name),
        ("CFBundleDisplayName", identity.name),
        ("CFBundleIdentifier", app_id),
        ("CFBundleExecutable", executable),
        ("CFBundleIconFile", "icon"),
        ("CFBundlePackageType", "APPL"),
        ("CFBundleShortVersionString", &identity.version),
        ("CFBundleVersion", &identity.version),
        (
            "LSApplicationCategoryType",
            "public.app-category.board-games",
        ),
        ("LSMinimumSystemVersion", "12.0"),
        ("NSHumanReadableCopyright", super::COPYRIGHT),
        ("NSMicrophoneUsageDescription", MICROPHONE),
    ];
    let mut body = String::new();
    for (key, value) in strings {
        body.push_str(&format!(
            "  <key>{key}</key>\n  <string>{}</string>\n",
            escape(value)
        ));
    }
    body.push_str("  <key>NSHighResolutionCapable</key>\n  <true/>\n");
    if background {
        body.push_str("  <key>LSUIElement</key>\n  <true/>\n");
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n{body}</dict>\n</plist>\n"
    )
}

fn text(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn sign_identity() -> String {
    std::env::var("KCHESS_SIGN_IDENTITY").unwrap_or_default()
}

/// Sign the nested programs before the app that seals them.
fn sign(app: &Path, nested: &[PathBuf], stage: &Path) -> Result {
    let identity = sign_identity();
    if identity.is_empty() {
        say("Signing ad hoc (no KCHESS_SIGN_IDENTITY)…");
        for path in nested {
            run("codesign", &["--force", "--sign", "-", &text(path)])?;
        }
        return run("codesign", &["--force", "--sign", "-", &text(app)]);
    }
    say(&format!("Signing as {identity} with the hardened runtime…"));
    // The microphone is for voice input; nothing else needs an exception to the hardened runtime.
    let entitlements = stage.join("entitlements.plist");
    std::fs::write(
        &entitlements,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>com.apple.security.device.audio-input</key>\n  <true/>\n</dict>\n</plist>\n",
    )
    .map_err(|cause| cause.to_string())?;
    let flags = [
        "--force",
        "--timestamp",
        "--options",
        "runtime",
        "--sign",
        identity.as_str(),
    ];
    for path in nested {
        let mut args: Vec<&str> = flags.to_vec();
        let path = text(path);
        args.push(&path);
        run("codesign", &args)?;
    }
    let mut args: Vec<&str> = flags.to_vec();
    let (entitlements_text, app_text) = (text(&entitlements), text(app));
    args.extend(["--entitlements", &entitlements_text, &app_text]);
    run("codesign", &args)?;
    let _ = std::fs::remove_file(&entitlements);
    Ok(())
}

/// Notarize `apps` in one submission and staple each; nothing to do when signed ad hoc.
fn notarize(apps: &[&Path], stage: &Path) -> Result {
    if sign_identity().is_empty() {
        return Ok(());
    }
    let var = |name: &str| std::env::var(name).unwrap_or_default();
    let (apple_id, password, team) = (
        var("APPLE_ID"),
        var("APPLE_APP_SPECIFIC_PASSWORD"),
        var("APPLE_TEAM_ID"),
    );
    if apple_id.is_empty() || password.is_empty() || team.is_empty() {
        return Err("Signed Mac packages require APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD and APPLE_TEAM_ID for notarization.".into());
    }
    say("Notarizing…");
    let folder = stage.join("notarize");
    std::fs::create_dir_all(&folder).map_err(|cause| cause.to_string())?;
    for (index, app) in apps.iter().enumerate() {
        run(
            "ditto",
            &[&text(app), &text(&folder.join(format!("{index}.app")))],
        )?;
    }
    let upload = text(&stage.join("notarize.zip"));
    run("ditto", &["-c", "-k", &text(&folder), &upload])?;
    run(
        "xcrun",
        &[
            "notarytool",
            "submit",
            &upload,
            "--apple-id",
            &apple_id,
            "--password",
            &password,
            "--team-id",
            &team,
            "--wait",
        ],
    )?;
    let _ = std::fs::remove_file(&upload);
    let _ = std::fs::remove_dir_all(&folder);
    for app in apps {
        run("xcrun", &["stapler", "staple", &text(app)])?;
    }
    Ok(())
}

/// The Electron bridge's bundle at `app`, signed.
fn bridge(identity: &Identity, built: &Built, app: &Path, stage: &Path) -> Result {
    let contents = app.join("Contents");
    copy_program(&built.bridge, &contents.join("MacOS").join(identity.name))?;
    std::fs::create_dir_all(contents.join("Resources")).map_err(|cause| cause.to_string())?;
    std::fs::copy(
        root().join("apps/gpui/build/icon.icns"),
        contents.join("Resources/icon.icns"),
    )
    .map_err(|cause| cause.to_string())?;
    std::fs::write(contents.join("Info.plist"), bridge_plist(identity))
        .map_err(|cause| cause.to_string())?;
    sign(app, &[], stage)
}

/// The `.zip` and `.dmg`, as `(file name, path)`.
pub fn package(
    identity: &Identity,
    built: &Built,
    stage: &Path,
    out: &Path,
) -> Result<Vec<PathBuf>> {
    if std::env::consts::ARCH != "aarch64" {
        return Err("macOS packages are Apple Silicon only.".into());
    }
    let app = stage.join(format!("{}.app", identity.name));
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    // The app finds Stockfish, the core service and the voice runtime beside itself
    // (apps/gpui/src/profile.rs, core.rs).
    copy_program(&built.app, &macos.join("kchess"))?;
    copy_program(&built.stockfish, &macos.join("stockfish"))?;
    copy_program(&built.service, &macos.join("kchess-service"))?;
    copy_program(&built.cli, &macos.join("kchess-cli"))?;
    let resources = contents.join("Resources");
    // Resources, not MacOS: the notices beside the helper are not code, which codesign rejects there.
    let voice = copy_voice(built, &resources)?;
    std::fs::copy(
        root().join("apps/gpui/build/icon.icns"),
        resources.join("icon.icns"),
    )
    .map_err(|cause| cause.to_string())?;
    std::fs::copy(&built.licenses, resources.join("THIRD_PARTY_LICENSES.txt"))
        .map_err(|cause| cause.to_string())?;
    std::fs::write(contents.join("Info.plist"), info_plist(identity))
        .map_err(|cause| cause.to_string())?;
    let mut nested = voice;
    nested.extend([
        macos.join("kchess-service"),
        macos.join("kchess-cli"),
        macos.join("stockfish"),
    ]);
    sign(&app, &nested, stage)?;
    let bridged = stage.join("zip");
    let bridge_app = bridged
        .join("legacy")
        .join(format!("{}.app", identity.name));
    bridge(identity, built, &bridge_app, stage)?;
    notarize(&[&app, &bridge_app], stage)?;
    run(
        "ditto",
        &[
            &text(&app),
            &text(&bridged.join(format!("{}.app", identity.name))),
        ],
    )?;

    let base = format!("{}-{}-mac-arm64", identity.prefix(), identity.version);
    let zip = out.join(format!("{base}.zip"));
    say("Making the zip…");
    // ditto keeps the bundles' symlinks, permissions and signatures, as Squirrel.Mac expects.
    run(
        "ditto",
        &["-c", "-k", "--sequesterRsrc", &text(&bridged), &text(&zip)],
    )?;
    say("Making the disk image…");
    let image = stage.join("dmg");
    std::fs::create_dir_all(&image).map_err(|cause| cause.to_string())?;
    run(
        "ditto",
        &[
            &text(&app),
            &text(&image.join(format!("{}.app", identity.name))),
        ],
    )?;
    std::os::unix::fs::symlink("/Applications", image.join("Applications"))
        .map_err(|cause| cause.to_string())?;
    let dmg = out.join(format!("{base}.dmg"));
    run(
        "hdiutil",
        &[
            "create",
            "-volname",
            identity.name,
            "-srcfolder",
            &text(&image),
            "-fs",
            "HFS+",
            "-format",
            "UDZO",
            "-imagekey",
            "zlib-level=9",
            "-ov",
            &text(&dmg),
        ],
    )?;
    verify_zip(identity, &zip)?;
    Ok(vec![zip, dmg])
}

/// The value of `key` in a plist written by `plist`.
fn plist_value<'a>(plist: &'a str, key: &str) -> Option<&'a str> {
    let after = plist.split_once(&format!("<key>{key}</key>"))?.1;
    let value = after.trim_start().strip_prefix("<string>")?;
    value.split_once("</string>").map(|(value, _)| value)
}

/// What every Mac install updates from must be in the zip: this KChess, and the bridge an
/// Electron release before 2026.10.12 installs (with its identifier, and its executable named so
/// ShipIt installs it as `KChess.app`), each signed. `entry` reads one file of the zip.
fn check_zip(identity: &Identity, mut entry: impl FnMut(&str) -> Option<Vec<u8>>) -> Result {
    let app = format!("{}.app/Contents", identity.name);
    let bundles = [
        (app.clone(), identity.app_id, "kchess"),
        (format!("legacy/{app}"), ELECTRON_APP_ID, identity.name),
    ];
    for (contents, app_id, executable) in bundles {
        let plist = entry(&format!("{contents}/Info.plist"))
            .ok_or_else(|| format!("The zip has no {contents}/Info.plist."))?;
        let plist = String::from_utf8_lossy(&plist);
        let expected = [
            ("CFBundleIdentifier", app_id),
            ("CFBundleExecutable", executable),
            ("CFBundleShortVersionString", identity.version.as_str()),
        ];
        for (key, value) in expected {
            if plist_value(&plist, key) != Some(value) {
                return Err(format!(
                    "{contents}/Info.plist has {key} {:?}, not {value:?}.",
                    plist_value(&plist, key)
                ));
            }
        }
        for file in [
            format!("MacOS/{executable}"),
            "_CodeSignature/CodeResources".into(),
        ] {
            if entry(&format!("{contents}/{file}")).is_none() {
                return Err(format!("The zip has no {contents}/{file}."));
            }
        }
    }
    Ok(())
}

fn verify_zip(identity: &Identity, zip: &Path) -> Result {
    let file = std::fs::File::open(zip).map_err(|cause| cause.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|cause| cause.to_string())?;
    check_zip(identity, |name| {
        use std::io::Read as _;
        let mut file = archive.by_name(name).ok()?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).ok()?;
        Some(bytes)
    })?;
    say("The zip holds KChess and the Electron bridge.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bridge_executable_is_named_like_the_app() {
        // ShipIt installs the update as `<executable>.app`; 2026.10.18 landed as
        // `/Applications/kchess-electron-bridge.app`.
        let identity = Identity::new("2026.10.19".into());
        let plist = bridge_plist(&identity);
        assert!(plist.contains("<key>CFBundleExecutable</key>\n  <string>KChess</string>"));
        assert!(plist.contains("<string>com.kchess.electron</string>"));
    }

    fn zip_of(identity: &Identity, bridge: String) -> std::collections::HashMap<String, Vec<u8>> {
        let mut files = std::collections::HashMap::new();
        for (dir, plist, executable) in [
            ("KChess.app", info_plist(identity), "kchess"),
            ("legacy/KChess.app", bridge, "KChess"),
        ] {
            files.insert(format!("{dir}/Contents/Info.plist"), plist.into_bytes());
            files.insert(format!("{dir}/Contents/MacOS/{executable}"), vec![0]);
            files.insert(
                format!("{dir}/Contents/_CodeSignature/CodeResources"),
                vec![0],
            );
        }
        files
    }

    #[test]
    fn the_zip_must_carry_the_bridge_as_electron_installs_it() {
        let identity = Identity::new("2026.10.19".into());
        let good = zip_of(&identity, bridge_plist(&identity));
        assert_eq!(check_zip(&identity, |name| good.get(name).cloned()), Ok(()));

        // 2026.10.18's bridge, whose executable ShipIt installed as kchess-electron-bridge.app.
        let renamed = bridge_plist(&identity).replace(
            "<key>CFBundleExecutable</key>\n  <string>KChess</string>",
            "<key>CFBundleExecutable</key>\n  <string>kchess-electron-bridge</string>",
        );
        let bad = zip_of(&identity, renamed);
        assert!(check_zip(&identity, |name| bad.get(name).cloned()).is_err());

        let mut missing = zip_of(&identity, bridge_plist(&identity));
        missing.retain(|name, _| !name.starts_with("legacy/"));
        assert!(check_zip(&identity, |name| missing.get(name).cloned()).is_err());
    }
}
