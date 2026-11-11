//! Child processes the core starts (the engine, its probe, the voice helper) are console
//! programs. On Windows one started by a process with no console of its own, such as the
//! detached service, gets a new console window; `spawn` keeps it hidden.

use std::time::Duration;

use tokio::process::{Child, Command};

/// How long `spawn` keeps retrying a busy executable, in all.
const BUSY_FOR: Duration = Duration::from_secs(1);

/// Starts `command` without a console window on Windows.
///
/// On Linux, a process forked elsewhere in this process while the executable was still open for
/// writing (a fresh copy or download) holds that descriptor until it execs, and `execve` fails
/// with `ETXTBSY` meanwhile. The window is brief, so a busy spawn is retried for up to
/// `BUSY_FOR`; callers are synchronous, so the waits block (only ever on this error).
pub(crate) fn spawn(command: &mut Command) -> std::io::Result<Child> {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let started = std::time::Instant::now();
    let mut delay = Duration::from_millis(5);
    loop {
        match command.spawn() {
            Err(cause)
                if cause.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && started.elapsed() < BUSY_FOR =>
            {
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(200));
            }
            spawned => return spawned,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CI regression (Linux, `core_wiring::a_lease_never_waits_on_an_unanswered_battery_request`):
    /// "Could not start the engine: Text file busy" when a test's fresh engine copy was still
    /// held open for writing by a child another thread had just forked. The spawn waits it out.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_busy_executable_is_waited_out() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engine");
        let mut writer = std::fs::File::create(&path).unwrap();
        writer.write_all(b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let busy = Command::new(&path).spawn().unwrap_err();
        assert_eq!(busy.kind(), std::io::ErrorKind::ExecutableFileBusy);
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            drop(writer);
        });
        let mut child = spawn(&mut Command::new(&path)).unwrap();
        assert!(child.wait().await.unwrap().success());
        release.join().unwrap();
    }

    #[tokio::test]
    async fn a_missing_executable_is_an_error_at_once() {
        let started = std::time::Instant::now();
        let missing = spawn(&mut Command::new("/nonexistent/kchess-engine"));
        assert_eq!(missing.unwrap_err().kind(), std::io::ErrorKind::NotFound);
        assert!(started.elapsed() < BUSY_FOR);
    }
}
