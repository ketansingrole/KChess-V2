//! macOS: the `.app` bundle, signed (ad hoc, or Developer ID with the hardened runtime and
//! notarization), as a `.zip` (what the updater installs) and a `.dmg` (what people download).
//! Electron installations never install this zip: they step through the frozen release
//! (`electron_step`), whose zip carries the bridge for `com.kchess.electron`.

use std::path::{Path, PathBuf};

use kchess_delta::Asset;

use super::{Built, Identity, copy_program, copy_voice};
use crate::util::{Result, root, run, run_redacted, say};

const MICROPHONE: &str =
    "KChess uses your microphone to recognize spoken chess moves and square names on this device.";

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn info_plist(identity: &Identity) -> String {
    plist(identity, identity.app_id, "kchess", false, false)
}

/// The Mac App Store's: it also says KChess uses only exempt encryption (HTTPS), which App Store
/// Connect otherwise asks about for every build.
pub fn store_plist(identity: &Identity) -> String {
    plist(identity, identity.app_id, "kchess", false, true)
}

fn plist(
    identity: &Identity,
    app_id: &str,
    executable: &str,
    background: bool,
    store: bool,
) -> String {
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
    if store {
        body.push_str("  <key>ITSAppUsesNonExemptEncryption</key>\n  <false/>\n");
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n{body}</dict>\n</plist>\n"
    )
}

pub fn text(path: &Path) -> String {
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
    run_redacted(
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
        &[&password],
    )?;
    let _ = std::fs::remove_file(&upload);
    let _ = std::fs::remove_dir_all(&folder);
    for app in apps {
        run("xcrun", &["stapler", "staple", &text(app)])?;
    }
    Ok(())
}

/// The unsigned bundle at `app`, with `plist`; the programs and libraries in it that are signed
/// before it. The app finds Stockfish, Fairy-Stockfish, the core service and the voice runtime beside itself
/// (apps/gpui/src/profile.rs, core.rs); the command line only where it can reach the core, which
/// a sandboxed app's core is not.
pub fn assemble(built: &Built, app: &Path, plist: &str, cli: bool) -> Result<Vec<PathBuf>> {
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    copy_program(&built.app, &macos.join("kchess"))?;
    copy_program(&built.stockfish, &macos.join("stockfish"))?;
    copy_program(&built.fairy, &macos.join("fairy-stockfish"))?;
    copy_program(&built.service, &macos.join("kchess-service"))?;
    if cli {
        copy_program(&built.cli, &macos.join("kchess-cli"))?;
    }
    let resources = contents.join("Resources");
    // Resources, not MacOS: the notices beside the helper are not code, which codesign rejects there.
    let mut nested = copy_voice(built, &resources)?;
    std::fs::copy(
        root().join("apps/gpui/build/icon.icns"),
        resources.join("icon.icns"),
    )
    .map_err(|cause| cause.to_string())?;
    std::fs::copy(&built.licenses, resources.join("THIRD_PARTY_LICENSES.txt"))
        .map_err(|cause| cause.to_string())?;
    std::fs::write(contents.join("Info.plist"), plist).map_err(|cause| cause.to_string())?;
    nested.push(macos.join("kchess-service"));
    if cli {
        nested.push(macos.join("kchess-cli"));
    }
    nested.push(macos.join("stockfish"));
    nested.push(macos.join("fairy-stockfish"));
    Ok(nested)
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
    let nested = assemble(built, &app, &info_plist(identity), true)?;
    sign(&app, &nested, stage)?;
    notarize(&[&app], stage)?;
    let zipped = stage.join("zip");
    run(
        "ditto",
        &[
            &text(&app),
            &text(&zipped.join(format!("{}.app", identity.name))),
        ],
    )?;

    let zip = out.join(identity.asset(Asset::MacZip));
    say("Making the zip…");
    // ditto keeps the bundles' symlinks, permissions and signatures, as Squirrel.Mac expects.
    run(
        "ditto",
        &["-c", "-k", "--sequesterRsrc", &text(&zipped), &text(&zip)],
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
    let dmg = out.join(identity.asset(Asset::MacDmg));
    // hdiutil sometimes finds the disk-image driver busy ("Resource busy") on a runner that just
    // made the zip; it succeeds when tried again.
    retrying(3, std::time::Duration::from_secs(10), || {
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
        )
    })?;
    verify_zip(identity, &zip)?;
    Ok(vec![zip, dmg])
}

/// `work`, tried up to `attempts` times with `pause` between: the last failure is the error.
fn retrying(attempts: u32, pause: std::time::Duration, mut work: impl FnMut() -> Result) -> Result {
    let mut tried = 1;
    loop {
        match work() {
            Err(cause) if tried < attempts => {
                say(&format!("{cause}; trying again ({tried} of {attempts})…"));
                std::thread::sleep(pause);
                tried += 1;
            }
            done => return done,
        }
    }
}

/// The value of `key` in a plist written by `plist`.
fn plist_value<'a>(plist: &'a str, key: &str) -> Option<&'a str> {
    let after = plist.split_once(&format!("<key>{key}</key>"))?.1;
    let value = after.trim_start().strip_prefix("<string>")?;
    value.split_once("</string>").map(|(value, _)| value)
}

/// What the updater installs must be in the zip: this KChess, signed. `entry` reads one file of
/// the zip.
fn check_zip(identity: &Identity, mut entry: impl FnMut(&str) -> Option<Vec<u8>>) -> Result {
    let app = format!("{}.app/Contents", identity.name);
    let bundles = [(app, identity.app_id, "kchess")];
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
    say("The zip holds KChess.");
    Ok(())
}

#[cfg(test)]
mod tests {
    /// 2026-11-07: `hdiutil create` failed with "Resource busy" on a CI runner, once, and
    /// failed the package job; a second try makes the image.
    #[test]
    fn a_busy_tool_is_tried_again() {
        let mut calls = 0;
        let made = super::retrying(3, std::time::Duration::ZERO, || {
            calls += 1;
            if calls < 2 {
                Err("hdiutil: create failed - Resource busy".into())
            } else {
                Ok(())
            }
        });
        assert!(made.is_ok());
        assert_eq!(calls, 2);
        let mut calls = 0;
        let failed = super::retrying(3, std::time::Duration::ZERO, || {
            calls += 1;
            Err(format!("busy {calls}"))
        });
        assert_eq!(failed.unwrap_err(), "busy 3");
    }

    use super::*;

    fn zip_of(identity: &Identity) -> std::collections::HashMap<String, Vec<u8>> {
        let mut files = std::collections::HashMap::new();
        let dir = "KChess.app";
        files.insert(
            format!("{dir}/Contents/Info.plist"),
            info_plist(identity).into_bytes(),
        );
        files.insert(format!("{dir}/Contents/MacOS/kchess"), vec![0]);
        files.insert(
            format!("{dir}/Contents/_CodeSignature/CodeResources"),
            vec![0],
        );
        files
    }

    #[test]
    fn the_zip_must_carry_the_signed_app() {
        let identity = Identity::new("2026.12.0".into());
        let good = zip_of(&identity);
        assert_eq!(check_zip(&identity, |name| good.get(name).cloned()), Ok(()));

        let mut unsigned = zip_of(&identity);
        unsigned.retain(|name, _| !name.contains("_CodeSignature"));
        assert!(check_zip(&identity, |name| unsigned.get(name).cloned()).is_err());

        let older = zip_of(&Identity::new("2026.11.0".into()));
        assert!(check_zip(&identity, |name| older.get(name).cloned()).is_err());
    }
}
