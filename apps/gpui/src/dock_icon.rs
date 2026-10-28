//! The Dock, app switcher and Mission Control icon (`refreshAppIcon`): the light or dark tile
//! `tooling/make-icons.py` writes, following the system appearance. A development build has no
//! bundle to take an icon from, and a packaged one shows its `.icns` until this applies.

use gpui_kit::{App, Global, WindowAppearance};

/// Whether the dark tile is the one shown.
struct Shown(bool);

impl Global for Shown {}

/// Show the tile for the system's appearance now, unless it already shows.
pub fn apply(cx: &mut App) {
    let dark = matches!(
        cx.window_appearance(),
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    );
    if cx
        .try_global::<Shown>()
        .is_some_and(|shown| shown.0 == dark)
    {
        return;
    }
    if set(dark) {
        cx.set_global(Shown(dark));
    }
}

#[cfg(target_os = "macos")]
#[allow(
    unsafe_code,
    reason = "AppKit's setter for the application icon is unsafe to call"
)]
fn set(dark: bool) -> bool {
    use objc2::{AllocAnyThread as _, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let png: &[u8] = if dark {
        include_bytes!("../build/icon-dark.png")
    } else {
        include_bytes!("../build/icon-light.png")
    };
    let Some(main) = MainThreadMarker::new() else {
        return false;
    };
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(png)) else {
        return false;
    };
    // SAFETY: on the main thread, with an image that is never nil.
    unsafe { NSApplication::sharedApplication(main).setApplicationIconImage(Some(&image)) };
    true
}

/// Windows and Linux take the icon from the package (the executable's resources, the desktop
/// entry).
#[cfg(not(target_os = "macos"))]
fn set(_: bool) -> bool {
    true
}
