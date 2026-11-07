//! `electron-step <packages>`: how the Electron app's installations still update.
//!
//! Electron's updater (electron-updater 6.8, every Electron release up to 2026.10.15) reads
//! `latest-mac.yml`, `latest.yml` (Windows) or `latest-linux.yml` from the newest published GitHub
//! release, so every release must carry them. They name the stepping-stone release `STEP`, frozen:
//! its installers take over the Electron profile at their first start
//! (`crates/kchess-core/src/electron_migration.rs`, the Mac bridge for `com.kchess.electron`), and
//! its own updater then brings the installation to the newest release.
//!
//! Each file's `url` is relative to the newest release's download folder
//! (`GitHubProvider.resolveFiles`), and `../<STEP tag>/<name>` resolves into the step's own
//! release: Electron downloads the step's original, signed bytes, under their own names (the Mac
//! bridge finds the zip electron-updater kept by that name). Nothing is copied. The manifest's
//! `version` is the new release's, so Electron sees an update, and the sha512 and size are the
//! step's, which electron-updater checks before installing; Squirrel.Mac and NSIS then check the
//! step's signature as they did for every earlier release.
//!
//! The step's files are pinned here and checked, before a release is made, against what its
//! release serves and against its `SHA256SUMS.txt` signed by the release key. Never change `STEP`:
//! Electron installations from any earlier version depend on it.

use std::path::Path;

use base64::Engine as _;
use sha2::Digest as _;

use crate::util::{Result, hex, output, say};

/// One of the stepping stone's installers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pinned {
    /// The manifest naming it (`latest-mac.yml`, `latest.yml`, `latest-linux.yml`).
    pub manifest: &'static str,
    /// Its asset name in the step's release.
    pub name: &'static str,
    pub size: u64,
    /// Hex, as `SHA256SUMS.txt` lists it.
    pub sha256: &'static str,
    /// Base64, as electron-updater checks it.
    pub sha512: &'static str,
}

/// The stepping-stone release.
#[derive(Debug, Clone, Copy)]
pub struct Step {
    pub tag: &'static str,
    pub files: &'static [Pinned],
}

/// The release every Electron installation steps through. Frozen: see the module documentation.
pub const STEP: Step = Step {
    tag: "v2026.11.4",
    files: &[
        Pinned {
            manifest: "latest-mac.yml",
            name: "KChess-2026.11.4-mac-arm64.zip",
            size: 120421156,
            sha256: "7aaec152573d3d853ea48b87d78fbdc1dc58af93606ff4162585f0bc1e704f9c",
            sha512: "7pN5eUJF2PSVNivhRMdRMRqx5Bgf0jSlYE2VwkQYH7BCrtMWS7MXzOMph/vE4joOCR3qVlBc91cSnMPakjULpw==",
        },
        Pinned {
            manifest: "latest-mac.yml",
            name: "KChess-2026.11.4-mac-arm64.dmg",
            size: 120946147,
            sha256: "a5422e4a8849e0f00c7218b23eb2501e7917a3fed1fb2a2fe1ee90cc748f14f5",
            sha512: "oMmbLh6kmwYFNWCvLopNTJRO2CiHtoANLwJYuDm9aaT9JM37Xb2BkW10DtvbEEwbNCX+3idLBiCfqpQYKkA+7w==",
        },
        Pinned {
            manifest: "latest.yml",
            name: "KChess-2026.11.4-win-x64.exe",
            size: 112831401,
            sha256: "103e75ccf1cb4d68ff15a92e6bb60163378143e3213768da2fdd68c4ef86425c",
            sha512: "cRbGt7YYkgG01oPyftWqfwYvIYeD6oXov5m9+9TP/Td1QflYRo4+svTjG7qm55g58b3exdOkQvsD2YGPnKachg==",
        },
        Pinned {
            manifest: "latest-linux.yml",
            name: "KChess-2026.11.4-linux-x86_64.AppImage",
            size: 131537400,
            sha256: "9ac4e968663829cbf4d280ba8894fa917aa256b157e583c4b9d02b151ba36d4b",
            sha512: "O9UqcRMfH8oc1OaOIVO48E82kcjQPf7eQkJumBUAKg3QJiaA/9/PuWCeD+NE/4HvlZgUNPlAdR7rTHwW9YISNA==",
        },
        Pinned {
            manifest: "latest-linux.yml",
            name: "KChess-2026.11.4-linux-amd64.deb",
            size: 136720666,
            sha256: "1a285e5fa8d189b980d8db8a7a1b563c9aed84607070b9b6bf3dc2aee56980da",
            sha512: "8kzptm+sFzx1UhOsefMN/yLWUHQE2zMHchpgZEfafXN07BUzaRenv2taoxCstciuqIkTyaEHwRW97Qp4nxjNuQ==",
        },
    ],
};

const MANIFESTS: [&str; 3] = ["latest-mac.yml", "latest.yml", "latest-linux.yml"];

/// The manifest `manifest` for release `version`: the step's files it names, with their url into
/// the step's release, sha512 and size. The first is the one `path` names.
pub fn manifest_text(step: &Step, manifest: &str, version: &str, release_date: &str) -> String {
    let files: Vec<&Pinned> = step
        .files
        .iter()
        .filter(|file| file.manifest == manifest)
        .collect();
    let url = |file: &Pinned| format!("../{}/{}", step.tag, file.name);
    let mut out = format!("version: {version}\nfiles:\n");
    for file in &files {
        out.push_str(&format!(
            "  - url: {}\n    sha512: {}\n    size: {}\n",
            url(file),
            file.sha512,
            file.size
        ));
    }
    if let Some(first) = files.first() {
        out.push_str(&format!("path: {}\nsha512: {}\n", url(first), first.sha512));
    }
    out.push_str(&format!("releaseDate: '{release_date}'\n"));
    out
}

/// What is wrong with `step` itself, before anything is downloaded.
pub fn check_step(step: &Step) -> Result {
    let version = step
        .tag
        .strip_prefix('v')
        .ok_or("The step's tag must start with v.")?;
    crate::release::parse(version)?;
    for manifest in MANIFESTS {
        if !step.files.iter().any(|file| file.manifest == manifest) {
            return Err(format!("The step names no file for {manifest}."));
        }
    }
    for file in step.files {
        // The step's own installers, never a later release's: the updater of B takes it on.
        if !file.name.starts_with(&format!("KChess-{version}-")) {
            return Err(format!(
                "{} is not one of {}'s installers.",
                file.name, step.tag
            ));
        }
        if file.name.ends_with(".yml") || file.name.starts_with("SHA256SUMS") {
            return Err(format!("{} is not an installer.", file.name));
        }
        if file.sha256.len() != 64 || file.size == 0 {
            return Err(format!("{} is pinned incompletely.", file.name));
        }
        base64::engine::general_purpose::STANDARD
            .decode(file.sha512)
            .ok()
            .filter(|digest| digest.len() == 64)
            .ok_or_else(|| format!("{}'s sha512 is not base64 of 64 bytes.", file.name))?;
    }
    Ok(())
}

/// Whether `bytes` are `file`'s pinned bytes.
pub fn matches(file: &Pinned, bytes: &[u8]) -> Result {
    if bytes.len() as u64 != file.size {
        return Err(format!(
            "{} is {} bytes, not the pinned {}.",
            file.name,
            bytes.len(),
            file.size
        ));
    }
    if hex(&sha2::Sha256::digest(bytes)) != file.sha256 {
        return Err(format!("{}'s SHA-256 is not the pinned one.", file.name));
    }
    let sha512 = base64::engine::general_purpose::STANDARD.encode(sha2::Sha512::digest(bytes));
    if sha512 != file.sha512 {
        return Err(format!("{}'s SHA-512 is not the pinned one.", file.name));
    }
    Ok(())
}

/// Whether the step's `SHA256SUMS.txt` (`sums`) is signed by `key` (`signature`, base64) and
/// lists each pinned file with its pinned digest.
pub fn check_sums(step: &Step, key: &[u8], sums: &[u8], signature: &str) -> Result {
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature.trim())
        .map_err(|_| "The step's checksum signature is not base64.".to_string())?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(sums, &signature)
        .map_err(|_| "The step's SHA256SUMS.txt is not signed by the release key.".to_string())?;
    let text = String::from_utf8_lossy(sums);
    for file in step.files {
        let listed: Vec<&str> = text
            .lines()
            .filter_map(|line| line.split_once("  "))
            .filter(|(_, name)| *name == file.name)
            .map(|(digest, _)| digest)
            .collect();
        if listed != [file.sha256] {
            return Err(format!(
                "The step's SHA256SUMS.txt does not list {} with its pinned digest.",
                file.name
            ));
        }
    }
    Ok(())
}

fn download(tag: &str, name: &str, into: &Path) -> Result<Vec<u8>> {
    let repo = std::env::var("GH_REPO").unwrap_or_else(|_| "ketansingrole/KChess-V2".into());
    output(
        "gh",
        &[
            "release",
            "download",
            tag,
            "--repo",
            &repo,
            "--pattern",
            name,
            "--dir",
            &into.to_string_lossy(),
            "--clobber",
        ],
    )
    .map_err(|cause| format!("The step's {name} could not be downloaded: {cause}"))?;
    std::fs::read(into.join(name)).map_err(|cause| format!("{name}: {cause}"))
}

/// `electron-step <packages>`: check the step's installers and write the manifests into
/// `packages`, replacing any there.
pub fn write(packages: &Path) -> Result {
    let step = STEP;
    check_step(&step)?;
    let key = crate::update_key::public_key()
        .ok_or("apps/gpui/update-key.pub holds no key to check the step with.")?;
    let work = tempfile::tempdir().map_err(|cause| cause.to_string())?;
    let sums = download(step.tag, "SHA256SUMS.txt", work.path())?;
    let signature = download(step.tag, "SHA256SUMS.txt.sig", work.path())?;
    check_sums(&step, &key, &sums, &String::from_utf8_lossy(&signature))?;
    for file in step.files {
        let bytes = download(step.tag, file.name, work.path())?;
        matches(file, &bytes)?;
        say(&format!("{} matches the pinned step.", file.name));
    }
    let version = crate::util::version()?;
    let date = now_iso();
    std::fs::create_dir_all(packages).map_err(|cause| cause.to_string())?;
    for manifest in MANIFESTS {
        std::fs::write(
            packages.join(manifest),
            manifest_text(&step, manifest, &version, &date),
        )
        .map_err(|cause| cause.to_string())?;
    }
    say(&format!(
        "Wrote the Electron manifests: {version} installs {}.",
        step.tag
    ));
    Ok(())
}

fn now_iso() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs()) as i64;
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::KeyPair as _;

    fn pinned(manifest: &'static str, name: &'static str, bytes: &[u8]) -> Pinned {
        Pinned {
            manifest,
            name,
            size: bytes.len() as u64,
            sha256: Box::leak(hex(&sha2::Sha256::digest(bytes)).into_boxed_str()),
            sha512: Box::leak(
                base64::engine::general_purpose::STANDARD
                    .encode(sha2::Sha512::digest(bytes))
                    .into_boxed_str(),
            ),
        }
    }

    fn step() -> Step {
        Step {
            tag: "v2026.11.4",
            files: Box::leak(Box::new([
                pinned("latest-mac.yml", "KChess-2026.11.4-mac-arm64.zip", b"zip"),
                pinned("latest.yml", "KChess-2026.11.4-win-x64.exe", b"exe"),
                pinned(
                    "latest-linux.yml",
                    "KChess-2026.11.4-linux-x86_64.AppImage",
                    b"appimage",
                ),
                pinned(
                    "latest-linux.yml",
                    "KChess-2026.11.4-linux-amd64.deb",
                    b"deb",
                ),
            ])),
        }
    }

    /// The manifest names exactly the step's bytes, in the step's release, as the new version.
    #[test]
    fn the_manifest_names_the_steps_own_files() {
        let step = step();
        check_step(&step).unwrap();
        let mac = manifest_text(
            &step,
            "latest-mac.yml",
            "2026.12.0",
            "2026-12-01T00:00:00.000Z",
        );
        let zip = step.files[0];
        assert_eq!(
            mac,
            format!(
                "version: 2026.12.0\nfiles:\n  - url: ../v2026.11.4/KChess-2026.11.4-mac-arm64.zip\n    sha512: {}\n    size: 3\npath: ../v2026.11.4/KChess-2026.11.4-mac-arm64.zip\nsha512: {}\nreleaseDate: '2026-12-01T00:00:00.000Z'\n",
                zip.sha512, zip.sha512
            )
        );
        let linux = manifest_text(&step, "latest-linux.yml", "2026.12.0", "x");
        assert!(linux.contains("../v2026.11.4/KChess-2026.11.4-linux-x86_64.AppImage"));
        assert!(linux.contains("../v2026.11.4/KChess-2026.11.4-linux-amd64.deb"));
        assert!(!linux.contains("mac-arm64"));
    }

    /// electron-updater resolves each url under `/releases/download/<newest tag>/`, and WHATWG
    /// URL parsing (Node's `new URL`) removes the dot segment: the step's own release.
    #[test]
    fn the_url_resolves_into_the_steps_release() {
        let base = "/ketansingrole/KChess-V2/releases/download/v2026.12.0/";
        let joined = format!("{base}../v2026.11.4/KChess-2026.11.4-win-x64.exe");
        let mut parts: Vec<&str> = Vec::new();
        for segment in joined.split('/') {
            match segment {
                ".." => {
                    parts.pop();
                }
                "." => {}
                other => parts.push(other),
            }
        }
        assert_eq!(
            parts.join("/"),
            "/ketansingrole/KChess-V2/releases/download/v2026.11.4/KChess-2026.11.4-win-x64.exe"
        );
    }

    /// The step's assets are its own installers, never names the app's updater or a later release
    /// uses (`apps/gpui/src/updates.rs` asks for `KChess-<newest version>-…`).
    #[test]
    fn the_step_names_only_its_own_installers() {
        let mut bad = step();
        let mut files = bad.files.to_vec();
        files[1].name = "KChess-2026.12.0-win-x64.exe";
        bad.files = Box::leak(files.into_boxed_slice());
        assert!(check_step(&bad).is_err());

        let mut missing = step();
        missing.files = &missing.files[..1];
        assert!(check_step(&missing).unwrap_err().contains("latest.yml"));
    }

    #[test]
    fn a_release_fails_when_the_steps_bytes_differ() {
        let step = step();
        assert_eq!(matches(&step.files[0], b"zip"), Ok(()));
        assert!(matches(&step.files[0], b"zap").is_err());
        assert!(matches(&step.files[0], b"zip!").is_err());
    }

    #[test]
    fn the_steps_checksums_must_be_signed_and_list_each_file() {
        let step = step();
        let random = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&random).unwrap();
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let key = pair.public_key().as_ref().to_vec();
        let sums: String = step
            .files
            .iter()
            .map(|file| format!("{}  {}\n", file.sha256, file.name))
            .collect();
        let sign = |text: &str| {
            base64::engine::general_purpose::STANDARD.encode(pair.sign(text.as_bytes()).as_ref())
        };
        assert_eq!(
            check_sums(&step, &key, sums.as_bytes(), &sign(&sums)),
            Ok(())
        );
        // Signed by another key.
        let other = ring::signature::Ed25519KeyPair::from_pkcs8(
            ring::signature::Ed25519KeyPair::generate_pkcs8(&random)
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        let foreign =
            base64::engine::general_purpose::STANDARD.encode(other.sign(sums.as_bytes()).as_ref());
        assert!(check_sums(&step, &key, sums.as_bytes(), &foreign).is_err());
        // Signed, but listing a different digest.
        let changed = sums.replacen(step.files[0].sha256, &"0".repeat(64), 1);
        assert!(check_sums(&step, &key, changed.as_bytes(), &sign(&changed)).is_err());
    }

    /// Never shipped unpinned: a release with the placeholder step fails.
    #[test]
    fn the_shipped_step_is_complete() {
        check_step(&STEP).unwrap();
    }
}
