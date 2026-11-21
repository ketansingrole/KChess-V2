//! Whether a downloaded package is signed as the running app is: the same signer, so an update
//! never swaps in a build someone else made.

use super::*;

/// Who signed a program: whether its signature holds, the signer (a macOS team or a Windows
/// certificate's subject; none when ad hoc or unsigned) and, on macOS, the app's identifier.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Signer {
    pub(super) valid: bool,
    pub(super) identity: Option<String>,
    pub(super) app: Option<String>,
}

/// An update must be signed as well as the app it replaces, by the same signer, as the same app.
/// An unsigned app (a Linux package, or a build without credentials) relies on the checksum.
pub(super) fn signed_alike(running: &Signer, update: &Signer) -> Result<(), String> {
    if running.valid && !update.valid {
        return Err("The update's signature is not valid".into());
    }
    if running.identity.is_some() && update.identity != running.identity {
        return Err("The update is not signed by KChess's developer".into());
    }
    if running.app.is_some() && update.app != running.app {
        return Err("The update is not KChess".into());
    }
    Ok(())
}

/// The signature of `path`, a bundle or program of `package`'s kind.
#[allow(
    clippy::disallowed_methods,
    reason = "the updater's own thread runs it (`on_runtime`); allowed, not expected, since some platforms' builds have no such call"
)]
pub(super) fn signer(package: Package, path: &Path) -> Signer {
    match package {
        Package::MacZip => {
            let valid = Command::new("codesign")
                .args(["--verify", "--deep", "--strict"])
                .arg(path)
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            let details = Command::new("codesign")
                .args(["--display", "--verbose=2"])
                .arg(path)
                .output()
                .map(|output| String::from_utf8_lossy(&output.stderr).to_string())
                .unwrap_or_default();
            let field = |key: &str| {
                details
                    .lines()
                    .find_map(|line| line.strip_prefix(key))
                    .map(str::trim)
                    .filter(|value| !value.is_empty() && *value != "not set")
                    .map(str::to_string)
            };
            Signer {
                valid,
                identity: field("TeamIdentifier="),
                app: field("Identifier="),
            }
        }
        Package::WindowsInstaller => {
            // The path goes by the environment, never into the script's text.
            let mut powershell = Command::new("powershell");
            #[cfg(windows)]
            {
                // No console window flashes up beside the app while the signature is checked.
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                powershell.creation_flags(CREATE_NO_WINDOW);
            }
            let output = powershell
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "$s = Get-AuthenticodeSignature -LiteralPath $env:KCHESS_SIGNED; \
                     \"$($s.Status)`n$($s.SignerCertificate.Subject)\"",
                ])
                .env("KCHESS_SIGNED", path)
                .output()
                .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
                .unwrap_or_default();
            let mut lines = output.lines().map(str::trim);
            let valid = lines.next() == Some("Valid");
            Signer {
                valid,
                identity: lines
                    .next()
                    .filter(|subject| valid && !subject.is_empty())
                    .map(str::to_string),
                app: None,
            }
        }
        Package::AppImage | Package::Deb => Signer::default(),
    }
}

/// The running app's signature, which an update for `install` must share.
pub(super) fn running_signer(install: &Install) -> Signer {
    match install.package {
        Package::WindowsInstaller => std::env::current_exe()
            .map(|exe| signer(install.package, &exe))
            .unwrap_or_default(),
        package => signer(package, &install.path),
    }
}

/// Whether `staged`, an update for `install` of `package`'s kind, is signed as the running app.
pub(super) fn signed_as_running(
    package: Package,
    install: &Install,
    staged: &Path,
) -> Result<(), String> {
    let running = running_signer(install);
    signed_alike(&running, &signer(package, staged))?;
    if package == Package::MacZip
        && let (Some(team), Some(app)) = (&running.identity, &running.app)
        && !meets_requirement(staged, &developer_requirement(app, team))
    {
        return Err("The update is not signed by KChess's developer".into());
    }
    Ok(())
}

/// What a Developer ID build's update must satisfy: signed through Apple's own certificate
/// chain, as `app`, by `team`. The team and identifier `codesign --display` prints are the
/// bundle's own claims, which a self-signed certificate can make; this requirement is not.
pub(super) fn developer_requirement(app: &str, team: &str) -> String {
    format!(
        "=anchor apple generic and identifier \"{app}\" and certificate leaf[subject.OU] = \"{team}\""
    )
}

/// Whether the bundle at `path` passes `codesign --verify` against `requirement`.
#[allow(
    clippy::disallowed_methods,
    reason = "the updater's own thread runs it (`on_runtime`); allowed, not expected, since some platforms' builds have no such call"
)]
pub(super) fn meets_requirement(path: &Path, requirement: &str) -> bool {
    Command::new("codesign")
        .args(["--verify", "--deep", "--strict", "-R", requirement])
        .arg(path)
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
