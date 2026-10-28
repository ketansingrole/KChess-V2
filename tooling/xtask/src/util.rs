//! Running commands and reading the repository.

use std::path::{Path, PathBuf};
use std::process::Command;

pub type Result<T = ()> = std::result::Result<T, String>;

/// The repository's root (this crate lives in `tooling/xtask`).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

pub fn say(message: &str) {
    eprintln!("[xtask] {message}");
}

/// Runs `program` in the repository, its output going to this terminal.
pub fn run(program: &str, args: &[&str]) -> Result {
    run_with(Command::new(program).args(args))
}

pub fn run_with(command: &mut Command) -> Result {
    let shown = format!("{command:?}");
    let status = command
        .current_dir(root())
        .status()
        .map_err(|cause| format!("Could not run {shown}: {cause}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{shown} failed ({status})."))
    }
}

/// Runs `program` and answers its standard output.
pub fn output(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(root())
        .output()
        .map_err(|cause| format!("Could not run {program}: {cause}"))?;
    if !out.status.success() {
        return Err(format!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether `program --version` runs.
pub fn available(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// The workspace's release version (`[workspace.package] version` in Cargo.toml).
pub fn version() -> Result<String> {
    let text = std::fs::read_to_string(root().join("Cargo.toml"))
        .map_err(|cause| format!("Cannot read Cargo.toml: {cause}"))?;
    workspace_version(&text).ok_or_else(|| "Cargo.toml has no workspace version.".into())
}

pub fn workspace_version(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[workspace.package]";
            continue;
        }
        if in_package
            && let Some(value) = line.strip_prefix("version")
            && let Some(value) = value.trim_start().strip_prefix('=')
        {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Downloads `url` to `path` with curl (on every CI image and developer machine).
pub fn download(url: &str, path: &Path) -> Result {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|cause| cause.to_string())?;
    }
    let target = path.to_string_lossy().to_string();
    run("curl", &["-fsSL", "--retry", "3", "-o", &target, url])
}

pub fn sha256_file(path: &Path) -> Result<String> {
    use sha2::Digest;
    let bytes =
        std::fs::read(path).map_err(|cause| format!("Cannot read {}: {cause}", path.display()))?;
    Ok(hex(&sha2::Sha256::digest(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_workspace_version_only() {
        let manifest = "[workspace]\nmembers = []\n\n[workspace.package]\n# The release\nversion = \"2026.10.15\"\nedition = \"2024\"\n\n[package]\nversion = \"0.1.0\"\n";
        assert_eq!(workspace_version(manifest).as_deref(), Some("2026.10.15"));
        assert_eq!(workspace_version("[package]\nversion = \"1.0.0\"\n"), None);
    }
}
