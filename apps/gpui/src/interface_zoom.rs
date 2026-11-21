//! Interface size: the whole
//! window scaled together, like a browser's page zoom, through the window zoom KChess adds to
//! GPUI (vendor/gpui-pre/KCHESS-PATCH.md). `auto` follows the display; the setting is the view
//! preference `kchess:interface-zoom`, kept by the core. ⌘+ and ⌘− step it, ⌘0 returns to
//! Auto.

use std::path::Path;

use gpui_kit::*;
#[cfg(target_os = "macos")]
use kchess_nav::shell::TOPBAR_HEIGHT;
use serde_json::{Value, json};

/// The view preference holding a chosen percent; Auto when it is missing.
const KEY: &str = "kchess:interface-zoom";
/// Where releases before the setting was a view preference kept it, as `{"zoom": …}`.
const LEGACY_FILE: &str = "interface-zoom.json";

pub const PERCENTS: [u16; 13] = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300];

/// The workspace a 100% interface is designed for, in the display's own points.
const DESIGN_LONG_SIDE: f32 = 2560.;
const DESIGN_SHORT_SIDE: f32 = 1360.;
/// Where the traffic lights sit: 12pt in, beside the sidebar toggle.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_X: f32 = 12.;

/// `auto`, or one of `PERCENTS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Auto,
    Percent(u16),
}

impl Setting {
    fn parse(value: &Value) -> Setting {
        match value {
            Value::Number(number) => number
                .as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .filter(|n| PERCENTS.contains(n))
                .map_or(Setting::Auto, Setting::Percent),
            _ => Setting::Auto,
        }
    }
}

/// What `auto` picks for a work area: 100% up to a 1440p-sized workspace, then the largest step
/// that keeps the interface's share of the screen; never below 100%.
pub fn auto_percent(width: f32, height: f32) -> u16 {
    let (long, short) = (width.max(height), width.min(height));
    let ratio = (long / DESIGN_LONG_SIDE).min(short / DESIGN_SHORT_SIDE) * 100.;
    // A few percent of slack: a taskbar along the long side must not cost a step.
    PERCENTS
        .into_iter()
        .rev()
        .find(|step| *step >= 100 && f32::from(*step) <= ratio + 3.)
        .unwrap_or(100)
}

/// The next step from `percent` in `direction`, held at the ends.
pub fn step(percent: u16, direction: i8) -> u16 {
    if direction > 0 {
        PERCENTS
            .into_iter()
            .find(|step| *step > percent)
            .unwrap_or(300)
    } else {
        PERCENTS
            .into_iter()
            .rev()
            .find(|step| *step < percent)
            .unwrap_or(PERCENTS[0])
    }
}

/// The setting chosen, from anywhere.
pub fn setting(cx: &App) -> Setting {
    Setting::parse(&crate::prefs::get(cx, KEY, Value::Null))
}

/// Move the setting out of the file an earlier release kept it in (`profile`'s
/// `interface-zoom.json`), once, after the view preferences are read and before the window opens.
pub fn init(profile: &Path, cx: &mut App) {
    crate::prefs::take_file(cx, KEY, &profile.join(LEGACY_FILE), |text| {
        let saved: Value = serde_json::from_str(text).ok()?;
        Some(match Setting::parse(&saved["zoom"]) {
            Setting::Auto => json!("auto"),
            Setting::Percent(percent) => json!(percent),
        })
    });
}

/// What `auto` picks for the display the window is on.
pub fn auto_for(window: &Window, cx: &App) -> u16 {
    window.display(cx).map_or(100, |display| {
        let area = display.visible_bounds().size;
        auto_percent(area.width.as_f32(), area.height.as_f32())
    })
}

/// The percent in effect in `window`.
pub fn percent(window: &Window, cx: &App) -> u16 {
    match setting(cx) {
        Setting::Auto => auto_for(window, cx),
        Setting::Percent(percent) => percent,
    }
}

/// Zoom the window as the setting says; the traffic lights stay centred on the top bar.
pub fn apply(window: &mut Window, cx: &App) {
    let factor = f32::from(percent(window, cx)) / 100.;
    window.set_zoom(factor);
    #[cfg(target_os = "macos")]
    window.set_traffic_light_centre(traffic_lights(factor));
}

/// Where the traffic lights' middle goes in a window zoomed by `factor`: on the top bar's
/// middle (`TOPBAR_HEIGHT` at 100%). macOS centres them by their own height, which differs between its releases.
#[cfg(target_os = "macos")]
fn traffic_lights(factor: f32) -> Point<Pixels> {
    point(px(TRAFFIC_LIGHT_X), TOPBAR_HEIGHT * factor / 2.)
}

/// Choose a setting, save it, and zoom `window` (the app's one window) to it.
pub fn choose(chosen: Setting, window: &mut Window, cx: &mut App) {
    match chosen {
        Setting::Auto => crate::prefs::forget(cx, KEY),
        Setting::Percent(percent) => crate::prefs::set(cx, KEY, percent),
    }
    apply(window, cx);
}

/// One step larger or smaller than what `window` shows, or (0) back to Auto.
pub fn step_window(direction: i8, window: &mut Window, cx: &mut App) {
    let setting = if direction == 0 {
        Setting::Auto
    } else {
        Setting::Percent(step(percent(window, cx), direction))
    };
    choose(setting, window, cx);
}

#[cfg(test)]
mod tests {
    use super::{Setting, auto_percent, step};
    use serde_json::json;

    #[test]
    fn auto_scales_up_only_for_workspaces_larger_than_1440p() {
        assert_eq!(auto_percent(1280., 760.), 100);
        assert_eq!(auto_percent(1920., 1040.), 100);
        assert_eq!(auto_percent(2560., 1400.), 100);
        // An ultrawide 1440p screen is wider, not denser.
        assert_eq!(auto_percent(3440., 1400.), 100);
        assert_eq!(auto_percent(3840., 2120.), 150);
        assert_eq!(auto_percent(2160., 3800.), 150);
        assert_eq!(auto_percent(5120., 2850.), 200);
        assert_eq!(auto_percent(7680., 4280.), 300);
        assert_eq!(auto_percent(2560., 1410.), 100);
    }

    /// They once sat 4pt right of where they belong, and on macOS 27 2pt below the top bar's
    /// middle: their top was placed for 10pt buttons. Their middle is the top bar's, 12pt in.
    #[cfg(target_os = "macos")]
    #[test]
    fn traffic_lights_centre_on_the_top_bar_12pt_in() {
        use gpui_kit::px;
        for factor in [0.5, 0.76, 1., 1.25, 2.] {
            let centre = super::traffic_lights(factor);
            assert_eq!(centre.x, px(12.), "beside the toggle at {factor}");
            let middle = super::TOPBAR_HEIGHT * factor / 2.;
            assert_eq!(centre.y, middle, "on the top bar's middle at {factor}");
        }
    }

    #[test]
    fn steps_hold_at_the_ends() {
        assert_eq!(step(100, 1), 110);
        assert_eq!(step(100, -1), 90);
        assert_eq!(step(300, 1), 300);
        assert_eq!(step(50, -1), 50);
        assert_eq!(step(133, 1), 150);
    }

    #[test]
    fn only_auto_and_the_steps_are_read() {
        assert_eq!(Setting::parse(&json!("auto")), Setting::Auto);
        assert_eq!(Setting::parse(&json!(125)), Setting::Percent(125));
        assert_eq!(Setting::parse(&json!(133)), Setting::Auto);
        assert_eq!(Setting::parse(&json!("125")), Setting::Auto);
    }
}
