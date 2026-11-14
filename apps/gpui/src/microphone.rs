//! Microphone access: whether the system lets KChess record, and
//! the system's own request for it. On macOS this is AVCaptureDevice's authorization, one of the
//! two places the app calls a platform API directly (and so an `unsafe` exception, below; the Dock
//! icon in `dock_icon.rs` is the other).
//! Elsewhere the system has no such switch the app can read: Linux always records, and Windows
//! shows its denial when the microphone fails to open.
//!
//! The app reads all of it through `host`, which a test answers in place of the machine
//! (`Pinned`): Settings → Voice input showed another state on a Mac than on CI's runners, so a
//! layout fault in one state passed CI.

use gpui_kit::App;

/// What the system says about recording (`MicrophoneStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(any(target_os = "macos", test)),
    allow(
        dead_code,
        reason = "only macOS reports a refusal before recording starts"
    )
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
    #[cfg(test)]
    pub const TEST: Host = Host {
        access: Access::NotDetermined,
        from_terminal: false,
        privacy_settings: Some(MACOS_PRIVACY),
    };

    /// What this machine says.
    #[cfg_attr(test, allow(dead_code, reason = "tests never read the machine"))]
    fn system() -> Host {
        Host {
            access: status(),
            from_terminal: launched_from_terminal(),
            privacy_settings: privacy_settings_url(),
        }
    }
}

/// The microphone as a test pins it, whatever the machine running the test says.
#[cfg(test)]
pub struct Pinned(pub Host);

#[cfg(test)]
impl gpui_kit::Global for Pinned {}

/// What the system says about the microphone: in a test, what it pinned (`Host::TEST` unless it
/// pinned another), never the machine's.
pub fn host(cx: &App) -> Host {
    #[cfg(test)]
    return cx
        .try_global::<Pinned>()
        .map_or(Host::TEST, |pinned| pinned.0);
    #[cfg(not(test))]
    {
        let _ = cx;
        Host::system()
    }
}

/// Ask the system for access when it has not asked yet; `done` hears the status after, perhaps on
/// another thread. A test's pinned answer stands: no system request opens.
pub fn ask(cx: &App, done: impl FnOnce(Access) + Send + 'static) {
    #[cfg(test)]
    return done(host(cx).access);
    #[cfg(not(test))]
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
    #[cfg(not(target_os = "macos"))]
    if cfg!(target_os = "linux") {
        Access::Granted
    } else {
        Access::Unknown
    }
}

/// Ask the system for access when it has not asked yet; `done` hears the status after.
#[cfg_attr(test, allow(dead_code, reason = "tests never ask the machine"))]
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
