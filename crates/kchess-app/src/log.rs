//! The app's log: core and app lines, `[scope] message`, to stderr and to the profile's bounded
//! `logs/kchess.log`, for Settings → Data & storage → Export diagnostics. Lines are
//! redacted as they are written: tokens, credentials and email addresses never reach the file.
//! Debug lines appear only with `KCHESS_DEBUG=1`.

use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use kchess_contracts::Level;

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
    if level == Level::Debug && kchess_host::env::var("KCHESS_DEBUG").as_deref() != Some("1") {
        return;
    }
    // Not `eprintln!`: it panics when stderr is gone (a terminal closed under a running app).
    let _ = writeln!(std::io::stderr(), "{} [{scope}] {message}", level.name());
    if let Some(log) = LOG.get()
        && let Ok(mut log) = log.lock()
    {
        let time = kchess_ui::dates::rfc3339_millis(jiff::Timestamp::now());
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
            let now = jiff::Timestamp::now().as_nanosecond() as i64;
            bytes[..8].copy_from_slice(&now.to_le_bytes());
        }
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    })
}

/// The log files, oldest first, redacted again.
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

use kchess_host::redact;

/// A failure nobody returns is logged, with the file and line that dropped it, rather than lost to
/// `let _ =` (Zed's `log_err`). The value comes back as an `Option` for the caller to go on with.
pub trait ResultExt<T> {
    /// The value, or `None` with the error logged as an error: a failure needing attention.
    #[track_caller]
    fn log_err(self) -> Option<T>;
    /// The value, or `None` with the error logged as a warning: a failure the app recovers from
    /// (a leftover file, a cache it rebuilds).
    #[track_caller]
    fn warn_on_err(self) -> Option<T>;
}

impl<T, E: std::fmt::Display> ResultExt<T> for Result<T, E> {
    #[track_caller]
    fn log_err(self) -> Option<T> {
        let at = std::panic::Location::caller();
        self.map_err(|error| dropped(Level::Error, &error, at)).ok()
    }

    #[track_caller]
    fn warn_on_err(self) -> Option<T> {
        let at = std::panic::Location::caller();
        self.map_err(|error| dropped(Level::Warn, &error, at)).ok()
    }
}

fn dropped(level: Level, error: &dyn std::fmt::Display, at: &std::panic::Location<'_>) {
    write(
        level,
        "app",
        &format!("{error} (at {}:{})", at.file(), at.line()),
    );
}

/// Removing what may already be gone: its absence is success.
pub trait IoResultExt {
    /// `Ok` when the operation failed only because the file or folder is not there.
    fn ok_if_missing(self) -> std::io::Result<()>;
}

impl IoResultExt for std::io::Result<()> {
    fn ok_if_missing(self) -> std::io::Result<()> {
        match self {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The tests' log, one per test process: the log starts once, so every test writes to it.
    pub(crate) fn logged() -> String {
        static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
        let dir = DIR.get_or_init(|| tempfile::tempdir().expect("a temporary folder"));
        init(dir.path());
        read().join("")
    }

    /// A line reaches the profile's log with its time and level, redacted as it is written.
    #[test]
    fn a_line_is_written_to_the_profiles_log_redacted() {
        logged();
        write(Level::Warn, "test", "sent by me@example.com");
        let log = logged();
        let line = log
            .lines()
            .find(|line| line.contains("[test]"))
            .unwrap_or_else(|| panic!("no line written: {log:?}"));
        assert!(line.contains(" WARN [test] sent by [redacted]"), "{line}");
        assert!(line.starts_with("20"), "the time first: {line}");
    }

    /// A dropped failure is logged at its level with the line that dropped it; a success passes.
    #[test]
    fn a_dropped_failure_is_logged_with_where_it_was_dropped() {
        logged();
        assert_eq!(Ok::<_, String>(7).log_err(), Some(7));
        assert_eq!(Ok::<_, String>(8).warn_on_err(), Some(8));
        let line = line!() + 1;
        assert_eq!(Err::<u8, _>("the disk is full").log_err(), None);
        assert_eq!(Err::<u8, _>("a leftover stayed").warn_on_err(), None);
        let log = logged();
        let at = format!("(at {}:{line})", file!());
        assert!(
            log.lines()
                .any(|entry| entry.contains(" ERROR [app] the disk is full ")
                    && entry.ends_with(&at)),
            "{log}"
        );
        let next = format!("(at {}:{})", file!(), line + 1);
        assert!(
            log.lines()
                .any(|entry| entry.contains(" WARN [app] a leftover stayed ")
                    && entry.ends_with(&next)),
            "{log}"
        );
    }

    /// Removing what is already gone succeeds; any other failure stays one.
    #[test]
    fn only_a_missing_file_counts_as_removed() {
        let missing = std::fs::remove_file("/nonexistent/kchess/never-there");
        assert!(missing.ok_if_missing().is_ok());
        let denied = Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(denied.ok_if_missing().is_err());
        assert!(Ok(()).ok_if_missing().is_ok());
    }
}
