//! `cargo xtask package --mas`: the Mac App Store's KChess, an installer package App Store Connect
//! takes (`KChess-<version>-mac-arm64.pkg`, in `apps/gpui/dist/mas/`).
//!
//! It is the app the `.dmg` carries, run in the App Sandbox: the core service, Stockfish and the
//! voice helper inherit the app's sandbox, and the command line stays out (outside the sandbox it
//! cannot reach the sandboxed core). In the sandbox the app leaves updates to the App Store and
//! runs only the Stockfish it comes with (`kchess_core::app_store`). It carries no Electron bridge
//! and no update manifest: nothing ever updated a store install but the store.
//!
//! Signing (`KCHESS_MAS_APP_IDENTITY`, `KCHESS_MAS_INSTALLER_IDENTITY`, `KCHESS_MAS_PROFILE`, the
//! Mac App Store provisioning profile, and `APPLE_TEAM_ID`): the app with an Apple Distribution
//! identity and the profile embedded, the package with a Mac Installer Distribution one. Without
//! them the bundle is signed ad hoc with the same sandbox, to try it here, and no package is made.

use std::path::{Path, PathBuf};

use super::mac::{assemble, store_plist, text};
use super::{Built, Identity};
use crate::util::{Result, run, say};

const PLIST_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n";

fn entitlements(keys: &[(&str, Option<&str>)]) -> String {
    let mut body = String::from(PLIST_HEAD);
    for (key, value) in keys {
        match value {
            None => body.push_str(&format!("  <key>{key}</key>\n  <true/>\n")),
            Some(value) => {
                body.push_str(&format!("  <key>{key}</key>\n  <string>{value}</string>\n"))
            }
        }
    }
    body.push_str("</dict>\n</plist>\n");
    body
}

/// The app's: Lichess and the downloads (client), the Lichess sign-in's loopback listener
/// (server), voice input, and the PGNs and exports a person picks. With a team, the identifier
/// the provisioning profile grants.
pub fn app_entitlements(app_id: &str, team: Option<&str>) -> String {
    let identifier = team.map(|team| format!("{team}.{app_id}"));
    let mut keys = vec![
        ("com.apple.security.app-sandbox", None),
        ("com.apple.security.network.client", None),
        ("com.apple.security.network.server", None),
        ("com.apple.security.device.audio-input", None),
        ("com.apple.security.device.microphone", None),
        ("com.apple.security.files.user-selected.read-write", None),
    ];
    if let (Some(identifier), Some(team)) = (identifier.as_deref(), team) {
        keys.push(("com.apple.application-identifier", Some(identifier)));
        keys.push(("com.apple.developer.team-identifier", Some(team)));
    }
    entitlements(&keys)
}

/// The programs the app starts: its sandbox, and nothing of their own (the only keys the store
/// allows beside `inherit`).
pub fn inherited_entitlements() -> String {
    entitlements(&[
        ("com.apple.security.app-sandbox", None),
        ("com.apple.security.inherit", None),
    ])
}

struct Signing {
    app: String,
    installer: String,
    profile: PathBuf,
    team: String,
}

fn signing() -> Result<Option<Signing>> {
    let var = |name: &str| std::env::var(name).unwrap_or_default();
    let (app, installer, profile, team) = (
        var("KCHESS_MAS_APP_IDENTITY"),
        var("KCHESS_MAS_INSTALLER_IDENTITY"),
        var("KCHESS_MAS_PROFILE"),
        var("APPLE_TEAM_ID"),
    );
    if [&app, &installer, &profile, &team]
        .iter()
        .all(|value| value.is_empty())
    {
        return Ok(None);
    }
    if [&app, &installer, &profile, &team]
        .iter()
        .any(|value| value.is_empty())
    {
        return Err("The App Store package needs KCHESS_MAS_APP_IDENTITY, KCHESS_MAS_INSTALLER_IDENTITY, KCHESS_MAS_PROFILE and APPLE_TEAM_ID together.".into());
    }
    let profile = PathBuf::from(profile);
    if !profile.is_file() {
        return Err(format!("No provisioning profile at {}.", profile.display()));
    }
    Ok(Some(Signing {
        app,
        installer,
        profile,
        team,
    }))
}

fn write(path: &Path, text: &str) -> Result {
    std::fs::write(path, text).map_err(|cause| cause.to_string())
}

/// Sign the nested code, then the app that seals it.
fn sign(
    app: &Path,
    nested: &[PathBuf],
    identity: &str,
    app_entitlements: &Path,
    stage: &Path,
) -> Result {
    let inherited = stage.join("inherit.entitlements");
    write(&inherited, &inherited_entitlements())?;
    let timestamp = if identity == "-" {
        "--timestamp=none"
    } else {
        "--timestamp"
    };
    for path in nested {
        let path = text(path);
        // A library is loaded into a sandboxed program; only programs carry entitlements.
        if path.ends_with(".dylib") {
            run(
                "codesign",
                &["--force", timestamp, "--sign", identity, &path],
            )?;
        } else {
            run(
                "codesign",
                &[
                    "--force",
                    timestamp,
                    "--sign",
                    identity,
                    "--entitlements",
                    &text(&inherited),
                    &path,
                ],
            )?;
        }
    }
    run(
        "codesign",
        &[
            "--force",
            timestamp,
            "--sign",
            identity,
            "--entitlements",
            &text(app_entitlements),
            &text(app),
        ],
    )
}

/// Every program in `app` is sandboxed (App Store Connect refuses the build otherwise), and the
/// signature holds.
fn verify(app: &Path, nested: &[PathBuf]) -> Result {
    run("codesign", &["--verify", "--deep", "--strict", &text(app)])?;
    for path in nested.iter().map(|path| path.as_path()).chain([app]) {
        if text(path).ends_with(".dylib") {
            continue;
        }
        let shown = std::process::Command::new("codesign")
            .args(["--display", "--entitlements", "-", "--xml"])
            .arg(path)
            .output()
            .map_err(|cause| cause.to_string())?;
        let shown = String::from_utf8_lossy(&shown.stdout);
        if !shown.contains("com.apple.security.app-sandbox") {
            return Err(format!("{} is not sandboxed.", path.display()));
        }
    }
    Ok(())
}

pub fn package(
    identity: &Identity,
    built: &Built,
    stage: &Path,
    out: &Path,
) -> Result<Vec<PathBuf>> {
    if std::env::consts::ARCH != "aarch64" {
        return Err("macOS packages are Apple Silicon only.".into());
    }
    let signing = signing()?;
    let out = out.join("mas");
    std::fs::create_dir_all(&out).map_err(|cause| cause.to_string())?;
    let app = stage.join(format!("{}.app", identity.name));
    let nested = assemble(built, &app, &store_plist(identity), false)?;
    let app_entitlements = stage.join("app.entitlements");
    write(
        &app_entitlements,
        &app_entitlements_for(identity, signing.as_ref()),
    )?;
    let Some(signing) = signing else {
        say("Signing ad hoc in the App Sandbox (no KCHESS_MAS_APP_IDENTITY); no package…");
        sign(&app, &nested, "-", &app_entitlements, stage)?;
        verify(&app, &nested)?;
        let local = out.join(format!("{}.app", identity.name));
        run("ditto", &[&text(&app), &text(&local)])?;
        say(&format!("Sandboxed app to try: {}", local.display()));
        return Ok(Vec::new());
    };
    std::fs::copy(
        &signing.profile,
        app.join("Contents/embedded.provisionprofile"),
    )
    .map_err(|cause| cause.to_string())?;
    say(&format!("Signing as {} in the App Sandbox…", signing.app));
    sign(&app, &nested, &signing.app, &app_entitlements, stage)?;
    verify(&app, &nested)?;
    let pkg = out.join(format!(
        "{}-{}-mac-arm64.pkg",
        identity.prefix(),
        identity.version
    ));
    say("Making the installer package…");
    run(
        "productbuild",
        &[
            "--component",
            &text(&app),
            "/Applications",
            "--sign",
            &signing.installer,
            &text(&pkg),
        ],
    )?;
    run("pkgutil", &["--check-signature", &text(&pkg)])?;
    Ok(vec![pkg])
}

fn app_entitlements_for(identity: &Identity, signing: Option<&Signing>) -> String {
    app_entitlements(
        identity.app_id,
        signing.map(|signing| signing.team.as_str()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_is_sandboxed_with_what_kchess_uses() {
        let plist = app_entitlements("com.kchess.app", Some("4NBN7U2PZL"));
        for key in [
            "com.apple.security.app-sandbox",
            "com.apple.security.network.client",
            "com.apple.security.network.server",
            "com.apple.security.device.audio-input",
            "com.apple.security.files.user-selected.read-write",
        ] {
            assert!(
                plist.contains(&format!("<key>{key}</key>\n  <true/>")),
                "{key}"
            );
        }
        assert!(plist.contains("<string>4NBN7U2PZL.com.kchess.app</string>"));
        // Ad hoc: no identifier a profile would have to grant.
        assert!(!app_entitlements("com.kchess.app", None).contains("application-identifier"));
    }

    #[test]
    fn the_programs_it_starts_only_inherit_the_sandbox() {
        let plist = inherited_entitlements();
        assert_eq!(plist.matches("<key>").count(), 2);
        assert!(plist.contains("com.apple.security.inherit"));
    }

    #[test]
    fn the_store_plist_declares_exempt_encryption_only() {
        let identity = Identity::new("2026.11.3".into());
        let plist = store_plist(&identity);
        assert!(plist.contains("<key>ITSAppUsesNonExemptEncryption</key>\n  <false/>"));
        assert!(!super::super::mac::info_plist(&identity).contains("ITSAppUses"));
    }
}
