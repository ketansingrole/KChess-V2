//! The update manifests the Electron app's updater (electron-updater) reads from each release:
//! `latest-mac.yml`, `latest.yml` (Windows) and `latest-linux.yml`, naming the packages with their
//! SHA-512 (base64) and size. Only the final build writes them: they are how every Electron
//! installation finds the app that replaces it. Without block maps the updater downloads the whole
//! package, as it does for any first download.

use std::path::{Path, PathBuf};

use base64::Engine;
use sha2::Digest;

use super::Identity;
use crate::util::Result;

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// The manifest's text for `files` as `(name, sha512 base64, size)`; `path` is the first.
pub fn text(version: &str, files: &[(String, String, u64)], release_date: &str) -> String {
    let mut out = format!("version: {version}\nfiles:\n");
    for (name, sha512, size) in files {
        out.push_str(&format!(
            "  - url: {name}\n    sha512: {sha512}\n    size: {size}\n"
        ));
    }
    if let Some((name, sha512, _)) = files.first() {
        out.push_str(&format!("path: {name}\nsha512: {sha512}\n"));
    }
    out.push_str(&format!("releaseDate: '{release_date}'\n"));
    out
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

pub fn write(identity: &Identity, packages: &[PathBuf], out: &Path) -> Result {
    let name = if cfg!(target_os = "macos") {
        "latest-mac.yml"
    } else if cfg!(windows) {
        "latest.yml"
    } else {
        "latest-linux.yml"
    };
    let mut files = Vec::new();
    for path in packages {
        let bytes = std::fs::read(path).map_err(|cause| cause.to_string())?;
        let sha512 = base64::engine::general_purpose::STANDARD.encode(sha2::Sha512::digest(&bytes));
        files.push((file_name(path), sha512, bytes.len() as u64));
    }
    std::fs::write(out.join(name), text(&identity.version, &files, &now_iso()))
        .map_err(|cause| cause.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_electron_updaters_manifest() {
        let files = [
            (
                "KChess-2026.10.16-mac-arm64.zip".to_string(),
                "abc=".to_string(),
                10,
            ),
            (
                "KChess-2026.10.16-mac-arm64.dmg".to_string(),
                "def=".to_string(),
                20,
            ),
        ];
        assert_eq!(
            text("2026.10.16", &files, "2026-10-29T00:00:00.000Z"),
            "version: 2026.10.16\nfiles:\n  - url: KChess-2026.10.16-mac-arm64.zip\n    sha512: abc=\n    size: 10\n  - url: KChess-2026.10.16-mac-arm64.dmg\n    sha512: def=\n    size: 20\npath: KChess-2026.10.16-mac-arm64.zip\nsha512: abc=\nreleaseDate: '2026-10-29T00:00:00.000Z'\n"
        );
        assert_eq!(now_iso().len(), 24);
    }
}
