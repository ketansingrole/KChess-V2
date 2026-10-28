//! macOS: the `.app` bundle, signed (ad hoc, or Developer ID with the hardened runtime and
//! notarization), as a `.zip` (what the updaters install) and a `.dmg` (what people download).

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

pub fn info_plist(identity: &Identity) -> String {
    let strings = [
        ("CFBundleName", identity.name),
        ("CFBundleDisplayName", identity.name),
        ("CFBundleIdentifier", identity.app_id),
        ("CFBundleExecutable", "kchess"),
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
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n{body}</dict>\n</plist>\n"
    )
}

fn text(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// Sign the nested programs before the app that seals them.
fn sign(app: &Path, nested: &[PathBuf], stage: &Path) -> Result {
    let identity = std::env::var("KCHESS_SIGN_IDENTITY").unwrap_or_default();
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
    let upload = text(&stage.join("notarize.zip"));
    run("ditto", &["-c", "-k", "--keepParent", &app_text, &upload])?;
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
    run("xcrun", &["stapler", "staple", &app_text])
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

    let base = format!("{}-{}-mac-arm64", identity.prefix(), identity.version);
    let zip = out.join(format!("{base}.zip"));
    say("Making the zip…");
    // ditto keeps the bundle's symlinks, permissions and signature, as Squirrel.Mac expects.
    run(
        "ditto",
        &[
            "-c",
            "-k",
            "--sequesterRsrc",
            "--keepParent",
            &text(&app),
            &text(&zip),
        ],
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
    Ok(vec![zip, dmg])
}
