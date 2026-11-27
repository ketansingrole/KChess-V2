//! What a crash leaves in the log, as Zed keeps its panics and crash reports: a panic's message,
//! where it happened and its backtrace, and on the next start a line saying the last run ended
//! without quitting (a native crash, a force quit, power lost), naming macOS's crash report.
//!
//! Each running app holds a lock on its own marker in the profile's `logs/` (`running-<session>`),
//! made at start and removed when it quits. A marker nobody holds is a run that ended without
//! quitting: the system releases a lock when its process ends, however it ends, so a second app
//! open on the same profile holds its marker and is never taken for a crash.

use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};
use std::time::SystemTime;

use kchess_contracts::Level;

use crate::log::{IoResultExt as _, ResultExt as _};

/// The marker of this run, removed when it quits.
static MARKER: Mutex<Option<Marker>> = Mutex::new(None);
const PREFIX: &str = "running-";

/// Log every panic in this process from now on, keeping the previous hook (stderr) as well. The
/// core logs its own panics where it runs; this is for an app whose core is the profile's service
/// (in an embedded core, the core's hook already sees the app's panics).
pub fn report_panics() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            crate::log::write(Level::Error, "panic", &describe(info));
            previous(info);
        }));
    });
}

/// The panic as Rust's own hook words it, then the backtrace.
fn describe(info: &std::panic::PanicHookInfo<'_>) -> String {
    let backtrace = std::backtrace::Backtrace::force_capture();
    format!("{}\n{backtrace}", kchess_host::panic_message(info))
}

/// Report the runs before this one that ended without quitting, then mark this one as running.
/// `version` is this app's, written in the marker for the next start to name.
pub fn watch(data_dir: &Path, version: &str) {
    let dir = data_dir.join("logs");
    for line in report_leftovers(&dir, crash_reports().as_deref()) {
        crate::log::write(Level::Warn, "diagnostics", &line);
    }
    if let Some(marker) = Marker::create(&dir, crate::log::session_id(), version).warn_on_err()
        && let Ok(mut current) = MARKER.lock()
    {
        *current = Some(marker);
    }
}

/// This run is quitting: its marker goes. Called as the quit begins, so a quit that outlasts the
/// time the system gives it is not taken for a crash.
pub fn closed() {
    let marker = MARKER.lock().ok().and_then(|mut current| current.take());
    if let Some(marker) = marker {
        marker.remove().warn_on_err();
    }
}

/// A run's marker, locked for as long as it runs.
struct Marker {
    file: File,
    path: PathBuf,
}

impl Marker {
    fn create(dir: &Path, session: &str, version: &str) -> std::io::Result<Marker> {
        let path = dir.join(format!("{PREFIX}{session}"));
        let mut file = File::create(&path)?;
        file.try_lock().map_err(std::io::Error::other)?;
        writeln!(file, "{version}")?;
        Ok(Marker { file, path })
    }

    fn remove(self) -> std::io::Result<()> {
        // Closed first: Windows removes no file that is open.
        drop(self.file);
        fs::remove_file(&self.path).ok_if_missing()
    }
}

/// One line for each marker in `dir` that no running app holds, which it removes; with the newest
/// crash report in `reports` written since that run started.
fn report_leftovers(dir: &Path, reports: Option<&Path>) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for path in entries.flatten().map(|entry| entry.path()) {
        let marked = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(PREFIX));
        if !marked {
            continue;
        }
        let Ok(mut file) = fs::OpenOptions::new().read(true).write(true).open(&path) else {
            continue;
        };
        // Held: another app is running on this profile.
        if file.try_lock().is_err() {
            continue;
        }
        let mut version = String::new();
        std::io::Read::read_to_string(&mut file, &mut version).warn_on_err();
        let started = file.metadata().and_then(|meta| meta.modified()).ok();
        drop(file);
        fs::remove_file(&path).ok_if_missing().warn_on_err();
        let version = version.trim();
        let version = if version.is_empty() { "?" } else { version };
        let mut line = format!("KChess {version} did not quit cleanly the last time it ran");
        if let Some(report) = reports
            .zip(started)
            .and_then(|(dir, at)| newest_report(dir, at))
        {
            line.push_str(&format!("; the system's crash report is {report}"));
        }
        lines.push(line);
    }
    lines
}

/// Where the system keeps crash reports: macOS's `~/Library/Logs/DiagnosticReports`.
fn crash_reports() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let home = kchess_host::env::var_os("HOME")?;
    Some(Path::new(&home).join("Library/Logs/DiagnosticReports"))
}

/// The name of the newest KChess crash report in `dir` written at or after `since`.
fn newest_report(dir: &Path, since: SystemTime) -> Option<String> {
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let ours = name.starts_with("KChess") && name.ends_with(".ips");
            let written = entry.metadata().and_then(|meta| meta.modified()).ok()?;
            (ours && written >= since).then_some((written, name))
        })
        .max()
        .map(|(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The log, started once for the test process (whichever test starts it first): every line
    /// written goes to it.
    fn logged() -> String {
        static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let dir = DIR.get_or_init(|| tempfile::tempdir().expect("a temporary folder"));
        crate::log::init(dir.path());
        crate::log::read().join("")
    }

    /// A marker no app holds is reported once with its version, and removed; a held one is not.
    #[test]
    fn a_run_that_did_not_quit_is_reported_once_and_a_running_one_never() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let gone = Marker::create(dir.path(), "gone", "2026.11.7").expect("a marker");
        // The process ended without quitting: the system released its lock, the file stayed.
        drop(gone.file);
        let running = Marker::create(dir.path(), "running", "2026.11.8").expect("a marker");
        fs::write(dir.path().join("kchess.log"), "a log line\n").expect("a log");
        let lines = report_leftovers(dir.path(), None);
        assert_eq!(
            lines,
            ["KChess 2026.11.7 did not quit cleanly the last time it ran"]
        );
        assert!(!dir.path().join("running-gone").exists());
        assert!(running.path.exists(), "a running app's marker stays");
        assert!(dir.path().join("kchess.log").exists());
        assert!(
            report_leftovers(dir.path(), None).is_empty(),
            "reported once"
        );
        running.remove().expect("removed");
        assert!(!dir.path().join("running-running").exists());
        assert!(report_leftovers(dir.path(), None).is_empty());
    }

    /// The report names the newest KChess crash report written since that run started.
    #[test]
    fn the_report_names_the_newest_crash_report_since_the_run_began() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let reports = tempfile::tempdir().expect("a temporary folder");
        let at = |name: &str, seconds: u64| {
            let path = reports.path().join(name);
            fs::write(&path, "{}").expect("a report");
            let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
            File::options()
                .write(true)
                .open(&path)
                .and_then(|file| file.set_modified(time))
                .expect("its time");
        };
        at("KChess-2026-11-01-old.ips", 1_000);
        at("KChess-2026-11-02-new.ips", 3_000);
        at("KChess-2026-11-02-newer.ips", 4_000);
        at("Other-2026-11-02-newest.ips", 5_000);
        at("KChess-2026-11-02-newest.txt", 5_000);
        let since = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2_000);
        assert_eq!(
            newest_report(reports.path(), since).as_deref(),
            Some("KChess-2026-11-02-newer.ips")
        );
        let later = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(4_500);
        assert_eq!(newest_report(reports.path(), later), None);
        drop(
            Marker::create(dir.path(), "crashed", "1.0")
                .expect("a marker")
                .file,
        );
        // The marker is written now, after every report above: none is named.
        assert_eq!(
            report_leftovers(dir.path(), Some(reports.path())),
            ["KChess 1.0 did not quit cleanly the last time it ran"]
        );
        at("KChess-2026-11-20-crash.ips", 4_102_444_800);
        drop(
            Marker::create(dir.path(), "crashed", "1.0")
                .expect("a marker")
                .file,
        );
        assert_eq!(
            report_leftovers(dir.path(), Some(reports.path())),
            [
                "KChess 1.0 did not quit cleanly the last time it ran; the system's crash report is KChess-2026-11-20-crash.ips"
            ]
        );
    }

    /// A panic reaches the log with its thread, place, message and a backtrace that names its
    /// functions.
    #[test]
    fn a_panic_is_logged_with_where_it_happened_and_its_backtrace() {
        logged();
        report_panics();
        let line = line!() + 3;
        let caught = std::thread::Builder::new()
            .name("crashing".into())
            .spawn(|| panic!("the board {}", "broke"))
            .expect("a thread")
            .join();
        assert!(caught.is_err());
        let log = logged();
        let at = format!(
            "ERROR [panic] thread 'crashing' panicked at {}:{line}: the board broke",
            file!()
        );
        let start = log.find(&at).unwrap_or_else(|| panic!("{at} not in {log}"));
        // Its frames by name, on Linux too: Linux builds were once linked with every symbol
        // stripped (`.cargo/config.toml`), and each frame read `<unknown>`.
        let after = &log[start..];
        assert!(after.contains("\n   0: "), "a backtrace follows: {after}");
        // `describe` on macOS (from the line tables), `kchess_app::crashes::describe` on Linux.
        let named = |frame: &str| {
            frame
                .split_once(": ")
                .is_some_and(|(_, name)| name.ends_with("describe"))
        };
        assert!(after.lines().any(named), "its frames are named: {after}");
    }

    /// macOS keeps its crash reports in the user's `Library/Logs/DiagnosticReports`; elsewhere
    /// there is no folder to look in.
    #[test]
    fn crash_reports_are_looked_for_where_macos_keeps_them() {
        kchess_host::env::set("HOME", "/Users/someone");
        let found = crash_reports();
        kchess_host::env::reset("HOME");
        let expected = cfg!(target_os = "macos")
            .then(|| PathBuf::from("/Users/someone/Library/Logs/DiagnosticReports"));
        assert_eq!(found, expected);
    }

    /// Watching marks this run, held, with its version; closing removes the mark; a second close
    /// does nothing. The mark is read only as the next start reads it: Windows refuses to read a
    /// file another handle has locked, the running app's own included.
    #[test]
    fn this_run_is_marked_until_it_quits() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let logs = dir.path().join("logs");
        fs::create_dir_all(&logs).expect("logs");
        let marker = logs.join(format!("{PREFIX}{}", crate::log::session_id()));
        watch(dir.path(), "9.9.9");
        assert!(marker.exists(), "marked");
        assert!(
            report_leftovers(&logs, None).is_empty(),
            "held while it runs"
        );
        closed();
        assert!(!marker.exists());
        closed();

        // Ended without quitting: the system releases the lock, the mark stays.
        watch(dir.path(), "9.9.9");
        let held = MARKER.lock().expect("the marker").take().expect("marked");
        drop(held.file);
        assert_eq!(
            report_leftovers(&logs, None),
            ["KChess 9.9.9 did not quit cleanly the last time it ran"]
        );
        assert!(!marker.exists());
    }
}
