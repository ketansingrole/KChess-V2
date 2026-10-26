//! Microphone access (`electron/main/microphone.ts`): whether the system lets KChess record, and
//! the system's own request for it. On macOS this is AVCaptureDevice's authorization, one of the
//! two places the app calls a platform API directly (and so an `unsafe` exception, below; the Dock
//! icon in `dock_icon.rs` is the other).
//! Elsewhere the system has no such switch the app can read: Linux always records, and Windows
//! shows its denial when the microphone fails to open.

/// What the system says about recording (`MicrophoneStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(target_os = "macos"),
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

/// A development build is attributed to the app that started it (a terminal or editor), which is
/// the one macOS asks about: only a packaged app has its own entry.
pub fn launched_from_terminal() -> bool {
    cfg!(target_os = "macos")
        && !std::env::current_exe()
            .is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"))
}

#[cfg(target_os = "macos")]
pub use mac::{request, status};

#[cfg(not(target_os = "macos"))]
pub fn status() -> Access {
    if cfg!(target_os = "linux") {
        Access::Granted
    } else {
        Access::Unknown
    }
}

/// Ask the system for access when it has not asked yet; `done` hears the status after.
#[cfg(not(target_os = "macos"))]
pub fn request(done: impl FnOnce(Access) + Send + 'static) {
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

    /// Show the system's request when it has not asked yet (`askForMediaAccess`); `done` hears
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
