use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use reqwest::blocking::Client;
use serde::Deserialize;

pub const STOCKFISH_RELEASES_API: &str =
    "https://api.github.com/repos/official-stockfish/Stockfish/releases/latest";
pub const STOCKFISH_DOWNLOAD_PAGE: &str = "https://stockfishchess.org/download/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallResult {
    pub release_tag: String,
    pub installed_path: PathBuf,
    pub asset_name: String,
}

#[derive(Debug)]
pub enum InstallError {
    Http(reqwest::Error),
    Io(io::Error),
    NoMacOsArm64Asset,
    UnsupportedArchive(String),
    ExtractionFailed(String),
    StockfishBinaryNotFound,
    DownloadFailed(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(err) => write!(f, "network error: {err}"),
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::NoMacOsArm64Asset => write!(f, "no macOS arm64 asset found in latest release"),
            Self::UnsupportedArchive(name) => write!(f, "unsupported archive format: {name}"),
            Self::ExtractionFailed(reason) => write!(f, "failed to extract archive: {reason}"),
            Self::StockfishBinaryNotFound => {
                write!(f, "could not locate stockfish executable in archive")
            }
            Self::DownloadFailed(reason) => write!(f, "download failed: {reason}"),
        }
    }
}

impl std::error::Error for InstallError {}

impl From<reqwest::Error> for InstallError {
    fn from(value: reqwest::Error) -> Self {
        Self::Http(value)
    }
}

impl From<io::Error> for InstallError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

pub fn default_engine_base_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(Path::new(&home).join(".kchess"))
}

pub fn default_managed_engine_path() -> Option<PathBuf> {
    Some(
        default_engine_base_dir()?
            .join("engines")
            .join("stockfish")
            .join("current")
            .join("stockfish"),
    )
}

pub fn select_macos_arm64_asset(assets: &[ReleaseAsset]) -> Option<ReleaseAsset> {
    let matches = |name: &str, needles: &[&str]| {
        let lowered = name.to_ascii_lowercase();
        needles.iter().all(|needle| lowered.contains(needle))
    };

    assets
        .iter()
        .find(|asset| matches(&asset.name, &["macos", "m1-apple-silicon"]))
        .cloned()
        .or_else(|| {
            assets
                .iter()
                .find(|asset| matches(&asset.name, &["macos", "apple-silicon"]))
                .cloned()
        })
        .or_else(|| {
            assets
                .iter()
                .find(|asset| matches(&asset.name, &["macos", "arm64"]))
                .cloned()
        })
}

pub fn install_latest_stockfish_macos_arm64(
    base_dir: &Path,
) -> Result<InstallResult, InstallError> {
    let client = Client::builder().build()?;
    let release = fetch_latest_release(&client)?;

    let assets = release
        .assets
        .into_iter()
        .map(|asset| ReleaseAsset {
            name: asset.name,
            download_url: asset.browser_download_url,
        })
        .collect::<Vec<_>>();

    let asset = select_macos_arm64_asset(&assets).ok_or(InstallError::NoMacOsArm64Asset)?;

    let tmp_dir = base_dir.join("tmp");
    fs::create_dir_all(&tmp_dir)?;
    let download_path = tmp_dir.join(format!("{}-{}", unique_suffix(), asset.name));
    let extract_dir = tmp_dir.join(format!("stockfish-extract-{}", unique_suffix()));

    download_file(&client, &asset.download_url, &download_path)?;
    extract_archive(&download_path, &extract_dir, &asset.name)?;

    let source_bin =
        find_stockfish_binary(&extract_dir)?.ok_or(InstallError::StockfishBinaryNotFound)?;

    let installed_path = base_dir
        .join("engines")
        .join("stockfish")
        .join("current")
        .join("stockfish");

    if let Some(parent) = installed_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::copy(&source_bin, &installed_path)?;
    make_executable(&installed_path)?;

    let _ = fs::remove_file(&download_path);
    let _ = fs::remove_dir_all(&extract_dir);

    Ok(InstallResult {
        release_tag: release.tag_name,
        installed_path,
        asset_name: asset.name,
    })
}

fn fetch_latest_release(client: &Client) -> Result<GithubRelease, InstallError> {
    let response = client
        .get(STOCKFISH_RELEASES_API)
        .header("User-Agent", "kchess/0.1.0")
        .header("Accept", "application/vnd.github+json")
        .send()?;

    let response = response
        .error_for_status()
        .map_err(|err| InstallError::DownloadFailed(err.to_string()))?;

    Ok(response.json()?)
}

fn download_file(client: &Client, url: &str, destination: &Path) -> Result<(), InstallError> {
    let response = client
        .get(url)
        .header("User-Agent", "kchess/0.1.0")
        .send()?;
    let response = response
        .error_for_status()
        .map_err(|err| InstallError::DownloadFailed(err.to_string()))?;

    let bytes = response.bytes().map_err(InstallError::Http)?;
    fs::write(destination, &bytes)?;
    Ok(())
}

fn extract_archive(
    archive_path: &Path,
    extract_dir: &Path,
    archive_name: &str,
) -> Result<(), InstallError> {
    fs::create_dir_all(extract_dir)?;
    let lowered = archive_name.to_ascii_lowercase();

    if lowered.ends_with(".zip") {
        run_command(
            Command::new("/usr/bin/unzip")
                .arg("-o")
                .arg(archive_path)
                .arg("-d")
                .arg(extract_dir),
        )
    } else if lowered.ends_with(".tar.gz") || lowered.ends_with(".tgz") {
        run_command(
            Command::new("/usr/bin/tar")
                .arg("-xzf")
                .arg(archive_path)
                .arg("-C")
                .arg(extract_dir),
        )
    } else if lowered.ends_with(".tar") {
        run_command(
            Command::new("/usr/bin/tar")
                .arg("-xf")
                .arg(archive_path)
                .arg("-C")
                .arg(extract_dir),
        )
    } else {
        Err(InstallError::UnsupportedArchive(archive_name.to_string()))
    }
}

fn run_command(command: &mut Command) -> Result<(), InstallError> {
    let output = command.output()?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(InstallError::ExtractionFailed(stderr.trim().to_string()))
    }
}

fn find_stockfish_binary(root: &Path) -> Result<Option<PathBuf>, InstallError> {
    let mut fallback = None;
    let mut stack = vec![root.to_path_buf()];

    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let entry_path = entry.path();
            let file_type = entry.file_type()?;

            if file_type.is_dir() {
                stack.push(entry_path);
                continue;
            }

            if !file_type.is_file() {
                continue;
            }

            let Some(file_name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };

            if file_name == "stockfish" {
                return Ok(Some(entry_path));
            }

            if file_name.to_ascii_lowercase().starts_with("stockfish") && fallback.is_none() {
                fallback = Some(entry_path);
            }
        }
    }

    Ok(fallback)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), InstallError> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), InstallError> {
    Ok(())
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.to_string(),
            download_url: "https://example.test/stockfish".to_string(),
        }
    }

    #[test]
    fn asset_selection_prefers_m1_apple_silicon() {
        let assets = vec![
            asset("stockfish-macos-arm64.tar"),
            asset("stockfish-macos-m1-apple-silicon.tar"),
        ];

        let selected = select_macos_arm64_asset(&assets).expect("asset");
        assert!(selected.name.contains("m1-apple-silicon"));
    }

    #[test]
    fn asset_selection_falls_back_to_apple_silicon_then_arm64() {
        let assets = vec![
            asset("stockfish-macos-arm64.zip"),
            asset("stockfish-macos-apple-silicon.zip"),
        ];

        let selected = select_macos_arm64_asset(&assets).expect("asset");
        assert!(selected.name.contains("apple-silicon"));
    }

    #[test]
    fn asset_selection_returns_none_when_no_match() {
        let assets = vec![asset("stockfish-windows-x86-64.zip")];
        assert!(select_macos_arm64_asset(&assets).is_none());
    }
}
