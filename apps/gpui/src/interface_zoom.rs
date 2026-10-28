//! Interface size: the whole
//! window scaled together, like a browser's page zoom, through the window zoom KChess adds to
//! GPUI (vendor/gpui-pre/KCHESS-PATCH.md). `auto` follows the display; the setting is the
//! profile's `interface-zoom.json`, which Electron keeps too. ⌘+ and ⌘− step it, ⌘0 returns to
//! Auto.

use std::path::{Path, PathBuf};

use gpui_kit::*;
use serde_json::{Value, json};

pub const PERCENTS: [u16; 13] = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300];

/// The workspace a 100% interface is designed for, in the display's own points.
const DESIGN_LONG_SIDE: f32 = 2560.;
const DESIGN_SHORT_SIDE: f32 = 1360.;
/// The top bar's height at 100%, which the macOS traffic lights centre on (`TOPBAR_HEIGHT`).
#[cfg(target_os = "macos")]
const TOPBAR: f32 = 52.;
/// Where Electron's traffic lights sit (`trafficLightPosition: { x: 12, y: 20 }`, which
/// hiddenInset draws a point lower): 12pt in, their frame 5pt above the top bar's middle.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_X: f32 = 12.;
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHT_ABOVE_MIDDLE: f32 = 5.;

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

    fn to_json(self) -> Value {
        match self {
            Setting::Auto => json!("auto"),
            Setting::Percent(percent) => json!(percent),
        }
    }
}

/// What `auto` picks for a work area: 100% up to a 1440p-sized workspace, then the largest step
/// that keeps the interface's share of the screen; never below 100% (`autoZoomPercent`).
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

/// The next step from `percent` in `direction`, held at the ends (`stepZoom`).
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

fn file(dir: &Path) -> PathBuf {
    dir.join("interface-zoom.json")
}

fn read(dir: &Path) -> Setting {
    match std::fs::read_to_string(file(dir)) {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .map_or(Setting::Auto, |saved| Setting::parse(&saved["zoom"])),
        Err(cause) => {
            if cause.kind() != std::io::ErrorKind::NotFound {
                crate::log::write(
                    kchess_core::Level::Warn,
                    "zoom",
                    &format!("Interface zoom unreadable; using Auto: {cause}"),
                );
            }
            Setting::Auto
        }
    }
}

fn write(dir: &Path, setting: Setting) {
    let target = file(dir);
    let temp = target.with_extension("json.tmp");
    let text = format!("{}\n", json!({ "zoom": setting.to_json() }));
    if let Err(cause) = std::fs::write(&temp, text).and_then(|_| std::fs::rename(&temp, &target)) {
        crate::log::write(
            kchess_core::Level::Warn,
            "zoom",
            &format!("Interface zoom not saved: {cause}"),
        );
    }
}

/// The saved setting, from anywhere: `cx.global::<InterfaceZoom>()`.
pub struct InterfaceZoom {
    dir: PathBuf,
    pub setting: Setting,
}

impl Global for InterfaceZoom {}

pub fn init(dir: PathBuf, cx: &mut App) {
    let setting = read(&dir);
    cx.set_global(InterfaceZoom { dir, setting });
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
    match cx.global::<InterfaceZoom>().setting {
        Setting::Auto => auto_for(window, cx),
        Setting::Percent(percent) => percent,
    }
}

/// Zoom the window as the setting says; the traffic lights stay centred on the top bar.
pub fn apply(window: &mut Window, cx: &App) {
    let factor = f32::from(percent(window, cx)) / 100.;
    window.set_zoom(factor);
    #[cfg(target_os = "macos")]
    window.set_traffic_light_position(traffic_lights(factor));
}

/// Where the traffic lights go in a window zoomed by `factor`: on the top bar's middle.
#[cfg(target_os = "macos")]
fn traffic_lights(factor: f32) -> Point<Pixels> {
    let y = (TOPBAR * factor / 2. - TRAFFIC_LIGHT_ABOVE_MIDDLE)
        .round()
        .max(4.);
    point(px(TRAFFIC_LIGHT_X), px(y))
}

/// Choose a setting, save it, and zoom `window` (the app's one window) to it.
pub fn choose(setting: Setting, window: &mut Window, cx: &mut App) {
    let zoom = cx.global_mut::<InterfaceZoom>();
    if zoom.setting != setting {
        zoom.setting = setting;
        write(&zoom.dir, setting);
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

    /// They sat 4pt right of and 3pt above Electron's (x12, y21 on screen at 100%).
    #[cfg(target_os = "macos")]
    #[test]
    fn traffic_lights_sit_where_electrons_do() {
        use gpui_kit::{point, px};
        assert_eq!(super::traffic_lights(1.), point(px(12.), px(21.)));
        assert_eq!(super::traffic_lights(2.), point(px(12.), px(47.)));
        assert_eq!(super::traffic_lights(0.5), point(px(12.), px(8.)));
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
