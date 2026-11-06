//! The system's reduced-motion setting, carried into GPUI's `reduce_motion`: the components read
//! it (the spinner stands still, dialogs, popovers and tooltips appear at rest) and so does the
//! app's own motion (the board's slides, the sidebar's easing). Read at start and again whenever
//! a window becomes active, since changing it means leaving KChess for the system's settings.

use gpui_kit::{App, Global};

/// Set by the app itself: the end-to-end harness fixes `reduce_motion` and must not have the
/// test machine's setting replace it.
struct FollowSystem;

impl Global for FollowSystem {}

/// Follow the system's setting from now on.
pub fn init(cx: &mut App) {
    cx.set_global(FollowSystem);
    refresh(cx);
}

/// Read the system's setting again (a window became active).
pub fn refresh(cx: &mut App) {
    if !cx.has_global::<FollowSystem>() {
        return;
    }
    // Linux asks another process; the others answer at once.
    #[cfg(target_os = "linux")]
    cx.spawn(async move |cx| {
        let reduce = cx
            .background_executor()
            .spawn(async { system_reduces_motion() })
            .await;
        cx.update(|cx| apply(reduce, cx));
    })
    .detach();
    #[cfg(not(target_os = "linux"))]
    apply(system_reduces_motion(), cx);
}

fn apply(reduce: bool, cx: &mut App) {
    if reduce != cx.reduce_motion() {
        crate::log::write(
            kchess_core::Level::Debug,
            "motion",
            &format!("Reduced motion {}", if reduce { "on" } else { "off" }),
        );
        cx.set_reduce_motion(reduce);
    }
}

/// System Settings › Accessibility › Display › Reduce motion.
#[cfg(target_os = "macos")]
fn system_reduces_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// Settings › Accessibility › Visual effects › Animation effects, off.
#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "SystemParametersInfoW writes the setting through a pointer"
)]
fn system_reduces_motion() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    let mut animate: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to the pointer, which lives on the stack
    // for the call.
    let read = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut animate).cast(), 0)
    };
    read != 0 && animate == 0
}

/// GNOME's (and its derivatives') Animations switch, else KDE's animation speed at "Instant".
#[cfg(target_os = "linux")]
fn system_reduces_motion() -> bool {
    let gnome = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| gnome_reduces(&String::from_utf8_lossy(&output.stdout)));
    gnome.unwrap_or_else(|| {
        dirs::config_dir()
            .and_then(|dir| std::fs::read_to_string(dir.join("kdeglobals")).ok())
            .is_some_and(|globals| kde_reduces(&globals))
    })
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn system_reduces_motion() -> bool {
    false
}

/// `gsettings get … enable-animations` printed `false`; `None` when it printed neither.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn gnome_reduces(printed: &str) -> Option<bool> {
    match printed.trim() {
        "false" => Some(true),
        "true" => Some(false),
        _ => None,
    }
}

/// `kdeglobals` sets `AnimationDurationFactor=0` under `[KDE]`.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn kde_reduces(globals: &str) -> bool {
    let mut in_kde = false;
    for line in globals.lines().map(str::trim) {
        if line.starts_with('[') {
            in_kde = line == "[KDE]";
        } else if in_kde && let Some(factor) = line.strip_prefix("AnimationDurationFactor=") {
            return factor
                .trim()
                .parse::<f32>()
                .is_ok_and(|factor| factor == 0.);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_animations_off_reduces_motion() {
        assert_eq!(gnome_reduces("false\n"), Some(true));
        assert_eq!(gnome_reduces("true\n"), Some(false));
        assert_eq!(gnome_reduces("No such schema\n"), None);
    }

    #[test]
    fn kde_instant_animations_reduce_motion() {
        assert!(kde_reduces(
            "[General]\nfoo=1\n[KDE]\nAnimationDurationFactor=0\n"
        ));
        assert!(!kde_reduces("[KDE]\nAnimationDurationFactor=0.5\n"));
        // The factor counts only in its own group.
        assert!(!kde_reduces(
            "[Other]\nAnimationDurationFactor=0\n[KDE]\nfoo=1\n"
        ));
        assert!(!kde_reduces(""));
    }

    #[gpui_kit::test]
    fn the_harness_keeps_its_own_setting(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            cx.set_reduce_motion(true);
            // Without `init`, a window becoming active leaves the setting alone.
            refresh(cx);
            assert!(cx.reduce_motion());
        });
    }
}
