//! Microphone access: whether the system lets KChess record, and
//! the system's own request for it. On macOS this is AVCaptureDevice's authorization, one of the
//! two places the app calls a platform API directly (and so an `unsafe` exception, below; the Dock
//! icon in `dock_icon.rs` is the other).
//! Windows keeps its switches in the registry (`windows`), and it never asks: a desktop app is
//! allowed unless one of them is off. Linux always records.
//!
//! The app reads all of it through `host`, which a test answers in place of the machine
//! (`Pinned`): Settings → Voice input showed another state on a Mac than on CI's runners, so a
//! layout fault in one state passed CI.

use gpui_kit::App;

/// What the system says about recording (`MicrophoneStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(any(target_os = "macos", target_os = "windows", test)),
    allow(dead_code, reason = "Linux reports no refusal before recording starts")
)]
pub enum Access {
    Granted,
    Denied,
    Restricted,
    NotDetermined,
    Unknown,
}

/// What the system says about the microphone, as Settings → Voice input shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Host {
    pub access: Access,
    /// Whether the system asks on behalf of the app that started KChess (`launched_from_terminal`).
    pub from_terminal: bool,
    /// The system's microphone privacy settings, where there are any to open.
    pub privacy_settings: Option<&'static str>,
}

const MACOS_PRIVACY: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";

impl Host {
    /// What a test sees unless it pins another: not asked yet, with the system's privacy settings
    /// to open, which shows the most controls (`Allow microphone` beside `Privacy settings`).
    #[cfg(any(test, feature = "test-support"))]
    pub const TEST: Host = Host {
        access: Access::NotDetermined,
        from_terminal: false,
        privacy_settings: Some(MACOS_PRIVACY),
    };

    /// What this machine says.
    #[cfg_attr(
        any(test, feature = "test-support"),
        allow(dead_code, reason = "tests never read the machine")
    )]
    fn system() -> Host {
        Host {
            access: status(),
            from_terminal: launched_from_terminal(),
            privacy_settings: privacy_settings_url(),
        }
    }
}

/// The microphone as a test pins it, whatever the machine running the test says.
#[cfg(any(test, feature = "test-support"))]
pub struct Pinned(pub Host);

#[cfg(any(test, feature = "test-support"))]
impl gpui_kit::Global for Pinned {}

/// What the test pinned, `Host::TEST` unless it pinned another.
#[cfg(any(test, feature = "test-support"))]
fn pinned(cx: &App) -> Host {
    cx.try_global::<Pinned>()
        .map_or(Host::TEST, |pinned| pinned.0)
}

/// What the system says about the microphone: in a test, what it pinned (`Host::TEST` unless it
/// pinned another), never the machine's.
///
/// On macOS this asks the privacy service, a round trip of about 6 ms: read it when Settings
/// shows the microphone or the window comes back, never while drawing. A view drawn every frame
/// that called it (the voice bar, for its Privacy settings link) took the analysis board's frames
/// from 1.4 to 9 ms; `privacy_settings` answers that without asking. A test fails if a frame
/// reads it (`slow_calls`).
#[track_caller]
pub fn host(cx: &App) -> Host {
    crate::slow_calls::call("the system's microphone access (microphone::host)");
    #[cfg(any(test, feature = "test-support"))]
    {
        HOST_READS.set(HOST_READS.get() + 1);
        pinned(cx)
    }
    #[cfg(not(any(test, feature = "test-support")))]
    {
        let _ = cx;
        Host::system()
    }
}

/// How many times this thread read `host`, so a test can check that drawing never asks the system.
#[cfg(any(test, feature = "test-support"))]
pub fn host_reads() -> usize {
    HOST_READS.get()
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static HOST_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The system's microphone privacy settings, where there are any to open (`Host::privacy_settings`),
/// without asking the system anything: cheap enough to read while drawing.
pub fn privacy_settings(cx: &App) -> Option<&'static str> {
    #[cfg(any(test, feature = "test-support"))]
    return pinned(cx).privacy_settings;
    #[cfg(not(any(test, feature = "test-support")))]
    {
        let _ = cx;
        privacy_settings_url()
    }
}

/// Ask the system for access when it has not asked yet; `done` hears the status after, perhaps on
/// another thread. A test's pinned answer stands: no system request opens.
pub fn ask(cx: &App, done: impl FnOnce(Access) + Send + 'static) {
    #[cfg(any(test, feature = "test-support"))]
    return done(host(cx).access);
    #[cfg(not(any(test, feature = "test-support")))]
    {
        let _ = cx;
        request(done)
    }
}

/// The system's microphone privacy settings, where there are any to open.
fn privacy_settings_url() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some(MACOS_PRIVACY)
    } else if cfg!(target_os = "windows") {
        Some("ms-settings:privacy-microphone")
    } else {
        None
    }
}

/// A development build is attributed to the app that started it (a terminal or editor), which is
/// the one macOS asks about, even from the bundle `tooling/macos-dev-runner.sh` runs a debug build
/// in: only a packaged app has its own entry.
fn launched_from_terminal() -> bool {
    cfg!(target_os = "macos")
        && (cfg!(debug_assertions)
            || !std::env::current_exe()
                .is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/")))
}

/// What the system says about recording now.
fn status() -> Access {
    #[cfg(target_os = "macos")]
    return mac::status();
    #[cfg(target_os = "windows")]
    return win::status();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    if cfg!(target_os = "linux") {
        Access::Granted
    } else {
        Access::Unknown
    }
}

/// What Windows' three microphone switches add up to, each `Allow`, `Deny` or not set (`None`,
/// which Windows takes as allowed): `machine` is the device's, which only an administrator turns
/// on again; `user` is "Microphone access" and `desktop` is "Let desktop apps access your
/// microphone", the one switch every app installed outside the Store shares. Windows lists no
/// switch of KChess's own, so the privacy page has nothing more to change while all three allow.
#[cfg_attr(
    not(any(target_os = "windows", test)),
    allow(dead_code, reason = "only Windows keeps these switches")
)]
fn windows_access(machine: Option<&str>, user: Option<&str>, desktop: Option<&str>) -> Access {
    let denies =
        |value: Option<&str>| value.is_some_and(|value| value.eq_ignore_ascii_case("Deny"));
    if denies(machine) {
        Access::Restricted
    } else if denies(user) || denies(desktop) {
        Access::Denied
    } else {
        Access::Granted
    }
}

#[cfg(target_os = "windows")]
mod win {
    use windows_registry::{CURRENT_USER, Key, LOCAL_MACHINE};

    use super::Access;

    const MICROPHONE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

    /// A switch's `Value`; `None` where it was never set (its key or value missing). Any other
    /// failure to read it is `Err`, and the status is then unknown.
    fn switch(root: &Key, path: &str) -> windows_registry::Result<Option<String>> {
        let key = match root.open(path) {
            Ok(key) => key,
            Err(error) if error.code().0 == NOT_FOUND => return Ok(None),
            Err(error) => return Err(error),
        };
        match key.get_string("Value") {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.code().0 == NOT_FOUND => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// ERROR_FILE_NOT_FOUND as an HRESULT: the key or value was never set.
    #[allow(
        clippy::cast_possible_wrap,
        reason = "an HRESULT is the same bits, signed"
    )]
    const NOT_FOUND: i32 = 0x8007_0002_u32 as i32;

    pub fn status() -> Access {
        let read = || -> windows_registry::Result<Access> {
            let machine = switch(LOCAL_MACHINE, MICROPHONE)?;
            let user = switch(CURRENT_USER, MICROPHONE)?;
            let desktop = switch(CURRENT_USER, &format!(r"{MICROPHONE}\NonPackaged"))?;
            Ok(super::windows_access(
                machine.as_deref(),
                user.as_deref(),
                desktop.as_deref(),
            ))
        };
        read().unwrap_or(Access::Unknown)
    }
}

/// Ask the system for access when it has not asked yet; `done` hears the status after.
#[cfg_attr(
    any(test, feature = "test-support"),
    allow(dead_code, reason = "tests never ask the machine")
)]
fn request(done: impl FnOnce(Access) + Send + 'static) {
    #[cfg(target_os = "macos")]
    return mac::request(done);
    #[cfg(not(target_os = "macos"))]
    done(status());
}

#[cfg(target_os = "macos")]
#[allow(
    unsafe_code,
    reason = "AVFoundation's bindings are unsafe to call; nothing else in the app needs it"
)]
mod mac {
    use std::sync::Mutex;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};

    use super::Access;

    pub fn status() -> Access {
        // SAFETY: AVMediaTypeAudio is an immutable framework constant, linked with AVFoundation.
        let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
            return Access::Unknown;
        };
        // SAFETY: a class method that only reads the app's privacy state; audio is one of the
        // two media types it accepts, so it raises no exception.
        let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) };
        match status {
            AVAuthorizationStatus::Authorized => Access::Granted,
            AVAuthorizationStatus::Denied => Access::Denied,
            AVAuthorizationStatus::Restricted => Access::Restricted,
            AVAuthorizationStatus::NotDetermined => Access::NotDetermined,
            _ => Access::Unknown,
        }
    }

    /// Show the system's request when it has not asked yet; `done` hears
    /// the status after it closes, on another thread.
    pub fn request(done: impl FnOnce(Access) + Send + 'static) {
        if status() != Access::NotDetermined {
            return done(status());
        }
        // SAFETY: as in `status`.
        let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
            return done(Access::Unknown);
        };
        let done = Mutex::new(Some(done));
        let handler = RcBlock::new(move |_granted: Bool| {
            if let Some(done) = done.lock().ok().and_then(|mut done| done.take()) {
                done(status());
            }
        });
        // SAFETY: audio is an accepted media type; the block owns everything it uses and may
        // run on any queue, as the handler is documented to.
        unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &handler) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test's pinned microphone is what the app hears: its privacy settings, and the answer to
    /// asking for access, with no system request opened.
    #[gpui_kit::test]
    fn a_pinned_microphone_answers_for_the_system(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            assert_eq!(privacy_settings(cx), Some(MACOS_PRIVACY));
            cx.set_global(Pinned(Host {
                access: Access::Denied,
                from_terminal: false,
                privacy_settings: Some("settings:microphone"),
            }));
            assert_eq!(privacy_settings(cx), Some("settings:microphone"));
        });
        let (heard, hear) = std::sync::mpsc::channel();
        cx.update(|cx| ask(cx, move |access| heard.send(access).expect("heard")));
        assert!(hear.recv().expect("an answer") == Access::Denied);
    }

    /// Windows showed "Unknown" with every switch on: the app never read them. Any one off blocks
    /// the microphone, and a switch never set counts as on, as Windows takes it.
    #[test]
    fn windows_switches_add_up_to_the_access_shown() {
        let (on, off) = (Some("Allow"), Some("Deny"));
        assert_eq!(windows_access(on, on, on), Access::Granted);
        assert_eq!(windows_access(None, None, None), Access::Granted);
        assert_eq!(windows_access(on, off, on), Access::Denied);
        assert_eq!(windows_access(on, on, off), Access::Denied);
        assert_eq!(windows_access(off, on, on), Access::Restricted);
        assert_eq!(windows_access(off, off, off), Access::Restricted);
    }

    /// This machine's own switches read without failing.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_switches_read_on_this_machine() {
        assert_ne!(super::win::status(), Access::Unknown);
    }
}
