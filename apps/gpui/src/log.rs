//! The app's log: core and app lines, `[scope] message`, to stderr and to the profile's bounded
//! `logs/kchess.log` (`DiagnosticLog` in `crates/kchess-node/js/diagnosticLog.ts`, which Electron
//! writes to the same files), for Settings → Data & storage → Export diagnostics. Lines are
//! redacted as they are written: tokens, credentials and email addresses never reach the file.
//! Debug lines appear only with `KCHESS_DEBUG=1`.

use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use kchess_core::Level;
use regex::Regex;

/// Each file's bound; two older files are kept beside it.
const MAX_BYTES: u64 = 512 * 1024;

struct FileLog {
    dir: PathBuf,
    size: u64,
}

static LOG: OnceLock<Mutex<FileLog>> = OnceLock::new();

impl FileLog {
    fn path(&self, index: u32) -> PathBuf {
        path(&self.dir, index)
    }

    fn append(&mut self, line: &str) -> std::io::Result<()> {
        let bytes = &line.as_bytes()[..line.len().min(MAX_BYTES as usize)];
        if self.size + bytes.len() as u64 > MAX_BYTES {
            if fs::metadata(self.path(0)).is_ok_and(|meta| meta.len() > 0) {
                let _ = fs::remove_file(self.path(2));
                let _ = fs::rename(self.path(1), self.path(2));
                fs::rename(self.path(0), self.path(1))?;
            }
            self.size = 0;
        }
        open(&self.path(0))?.write_all(bytes)?;
        self.size += bytes.len() as u64;
        Ok(())
    }
}

fn path(dir: &Path, index: u32) -> PathBuf {
    if index == 0 {
        dir.join("kchess.log")
    } else {
        dir.join(format!("kchess.{index}.log"))
    }
}

fn open(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// Start writing to `<data dir>/logs`; until then lines go to stderr only.
pub fn init(data_dir: &Path) {
    let dir = data_dir.join("logs");
    if let Err(cause) = fs::create_dir_all(&dir) {
        eprintln!("WARN [diagnostics] Diagnostics unavailable: {cause}");
        return;
    }
    #[cfg(unix)]
    let _ = fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    let size = fs::metadata(path(&dir, 0)).map_or(0, |meta| meta.len());
    let _ = LOG.set(Mutex::new(FileLog { dir, size }));
}

pub fn write(level: Level, scope: &str, message: &str) {
    if level == Level::Debug && std::env::var("KCHESS_DEBUG").as_deref() != Ok("1") {
        return;
    }
    eprintln!("{} [{scope}] {message}", level.name());
    if let Some(log) = LOG.get()
        && let Ok(mut log) = log.lock()
    {
        let time = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let line = redact(&format!(
            "{time} {} [{scope}] {message}\n",
            level.name().to_uppercase()
        ));
        // Disk trouble never stops the app or recurses into the log.
        let _ = log.append(&line);
    }
}

/// This launch's id, so a shared log can be split by session when handed over.
pub fn session_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let mut bytes = [0u8; 16];
        if getrandom::fill(&mut bytes).is_err() {
            let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
            bytes[..8].copy_from_slice(&now.to_le_bytes());
        }
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    })
}

/// The log files, oldest first, redacted again (`DiagnosticLog.read`).
pub fn read() -> Vec<String> {
    let Some(log) = LOG.get() else {
        return Vec::new();
    };
    let Ok(log) = log.lock() else {
        return Vec::new();
    };
    [2, 1, 0]
        .into_iter()
        .filter_map(|index| fs::read_to_string(log.path(index)).ok())
        .map(|text| redact(&text))
        .collect()
}

/// `redactDiagnostics`: the home folder, bearer tokens, Lichess tokens, credential fields and
/// email addresses.
pub fn redact(text: &str) -> String {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        [
            (r"(?i)\bBearer\s+[A-Za-z0-9._~+/-]+=*", "Bearer [redacted]"),
            (r"\b(?:lip|lio)_[A-Za-z0-9_-]+\b", "[redacted]"),
            (
                r#"(?i)(["']?(?:access_token|refresh_token|id_token|token|password|authorization|cookie|code_verifier|code_challenge|client_secret|verifier|challenge|code|state)["']?\s*[:=]\s*)(["']?)[^\s,"'&}]+(["']?)"#,
                "${1}${2}[redacted]${3}",
            ),
            (r"[\w.+-]+@[\w-]+\.[\w.]+", "[redacted]"),
        ]
        .into_iter()
        .filter_map(|(pattern, with)| Regex::new(pattern).ok().map(|rule| (rule, with)))
        .collect()
    });
    let mut text = match dirs::home_dir() {
        Some(home) if !home.as_os_str().is_empty() => text.replace(&*home.to_string_lossy(), "~"),
        _ => text.to_string(),
    };
    for (rule, with) in rules {
        text = rule.replace_all(&text, *with).into_owned();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn secrets_never_reach_the_log() {
        let line = redact(
            "GET /api/account Authorization: Bearer lip_abc123 token=xyz \"password\": \"hunter2\" by me@example.com",
        );
        assert!(!line.contains("abc123"), "{line}");
        assert!(!line.contains("xyz"), "{line}");
        assert!(!line.contains("hunter2"), "{line}");
        assert!(!line.contains("example.com"), "{line}");
        assert!(line.contains("GET /api/account"));
        // Usernames stay, so logs can still be matched to an account.
        assert!(redact("synced @KChessTester").contains("KChessTester"));
    }
}
