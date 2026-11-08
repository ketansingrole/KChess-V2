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
    run_shown(command, &[])
}

/// Runs `program` with `args` like `run`, but never shows the `secrets` among them: a failure
/// message (printed and kept in build logs) names them `<redacted>`.
// Only notarization (macOS) passes secrets; its test runs on any Unix.
#[cfg(any(target_os = "macos", all(test, unix)))]
pub fn run_redacted(program: &str, args: &[&str], secrets: &[&str]) -> Result {
    run_shown(Command::new(program).args(args), secrets)
}

fn run_shown(command: &mut Command, secrets: &[&str]) -> Result {
    let mut shown = format!("{command:?}");
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        shown = shown.replace(secret, "<redacted>");
    }
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

/// Seconds since 1970 now, from the system clock.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs()) as i64
}

/// The UTC calendar date of `seconds` since 1970: (year, month, day), by Howard Hinnant's
/// civil_from_days.
pub fn civil_date(seconds: i64) -> (i64, u32, u32) {
    let z = seconds.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

/// `seconds` since 1970 as an ISO 8601 UTC timestamp with milliseconds.
pub fn iso_timestamp(seconds: i64) -> String {
    let (year, month, day) = civil_date(seconds);
    let rest = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
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
    fn dates_fall_on_the_right_day_across_leap_rules() {
        for (seconds, date) in [
            (0, (1970, 1, 1)),
            (-1, (1969, 12, 31)),
            (951_782_400, (2000, 2, 29)),
            (951_868_800, (2000, 3, 1)),
            (1_709_164_800, (2024, 2, 29)),
            (4_107_456_000, (2100, 2, 28)),
            (4_107_542_400, (2100, 3, 1)),
            (1_798_761_599, (2026, 12, 31)),
            (1_798_761_600, (2027, 1, 1)),
        ] {
            assert_eq!(civil_date(seconds), date, "{seconds}");
        }
    }

    #[test]
    fn timestamps_carry_the_time_of_day() {
        assert_eq!(iso_timestamp(1_762_605_296), "2025-11-08T12:34:56.000Z");
        assert_eq!(iso_timestamp(-1), "1969-12-31T23:59:59.000Z");
        assert_eq!(iso_timestamp(0), "1970-01-01T00:00:00.000Z");
        assert!(unix_now() > 1_790_000_000);
    }

    #[test]
    fn reads_the_workspace_version_only() {
        let manifest = "[workspace]\nmembers = []\n\n[workspace.package]\n# The release\nversion = \"2026.10.15\"\nedition = \"2024\"\n\n[package]\nversion = \"0.1.0\"\n";
        assert_eq!(workspace_version(manifest).as_deref(), Some("2026.10.15"));
        assert_eq!(workspace_version("[package]\nversion = \"1.0.0\"\n"), None);
    }

    /// A failed notarization must not print the app-specific password.
    #[cfg(unix)]
    #[test]
    fn a_failed_command_does_not_show_its_secret() {
        let failed = run_redacted(
            "sh",
            &["-c", "exit 3", "--password", "abcd-efgh"],
            &["abcd-efgh"],
        )
        .unwrap_err();
        assert!(!failed.contains("abcd-efgh"), "{failed}");
        assert!(failed.contains("<redacted>"), "{failed}");
    }
}
