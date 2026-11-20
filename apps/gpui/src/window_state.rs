//! Where the window opens: as it was left, in the view preference `kchess:window-state` the
//! core keeps, unless no connected display can still show it.

use std::path::Path;
use std::time::Duration;

use gpui_kit::*;
use serde::{Deserialize, Serialize};

pub const DEFAULT_SIZE: (f32, f32) = (1320., 880.);
pub const MIN_SIZE: (f32, f32) = (640., 620.);
/// The top strip that must stay on a screen so the window can still be grabbed and moved.
const GRAB_HEIGHT: f32 = 52.;
const GRAB_WIDTH: f32 = 120.;
/// Saved this long after the last move or resize.
const SAVE_DELAY: Duration = Duration::from_millis(500);
/// The view preference holding how the window was left.
pub const KEY: &str = "kchess:window-state";
/// Where releases before it was a view preference kept it.
const LEGACY_FILE: &str = "window-state.json";

/// How the window was left: its normal bounds and its mode.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub full_screen: bool,
}

/// A rectangle as `x, y, width, height`.
pub type Area = (f32, f32, f32, f32);

/// A saved state, or `None` when it is damaged or empty.
pub fn parse(text: &str) -> Option<WindowState> {
    usable(serde_json::from_str(text).ok()?)
}

/// `state` in whole points, or `None` when it has no size or is not a number.
fn usable(state: WindowState) -> Option<WindowState> {
    let finite = [state.x, state.y, state.width, state.height]
        .iter()
        .all(|n| n.is_finite());
    (finite && state.width > 0. && state.height > 0.).then_some(WindowState {
        x: state.x.round(),
        y: state.y.round(),
        width: state.width.round(),
        height: state.height.round(),
        ..state
    })
}

/// The display area that holds the window's title strip, if any still does.
fn grabbable(state: &WindowState, areas: &[Area]) -> Option<Area> {
    areas.iter().copied().find(|&(x, y, width, height)| {
        let overlap = (state.x + state.width).min(x + width) - state.x.max(x);
        overlap >= GRAB_WIDTH.min(state.width)
            && state.y >= y
            && state.y + GRAB_HEIGHT <= y + height
    })
}

/// Where to open: the saved place, shrunk and slid onto its display; else the default size,
/// centred. `None` for the position means centred.
pub fn restored(
    saved: Option<&WindowState>,
    areas: &[Area],
    primary: Area,
) -> (Option<(f32, f32)>, (f32, f32)) {
    let fit = |(width, height): (f32, f32), area: Area| {
        (
            width.min(area.2).max(MIN_SIZE.0),
            height.min(area.3).max(MIN_SIZE.1),
        )
    };
    let Some((saved, area)) = saved.and_then(|saved| Some((saved, grabbable(saved, areas)?)))
    else {
        return (None, fit(DEFAULT_SIZE, primary));
    };
    let (width, height) = fit((saved.width, saved.height), area);
    let x = saved.x.min(area.0 + area.2 - width).max(area.0);
    let y = saved.y.min(area.1 + area.3 - height).max(area.1);
    (Some((x, y)), (width, height))
}

/// How the window was left, if that is known and usable.
pub fn read(cx: &App) -> Option<WindowState> {
    crate::prefs::get::<Option<WindowState>>(cx, KEY, None).and_then(usable)
}

/// Move the state out of the file an earlier release kept it in (`profile`'s
/// `window-state.json`), once, after the view preferences are read and before the window opens.
pub fn init(profile: &Path, cx: &mut App) {
    crate::prefs::take_file(cx, KEY, &profile.join(LEGACY_FILE), |text| {
        serde_json::to_value(parse(text)?).ok()
    });
}

/// The window's options for opening where it was left.
pub fn bounds(cx: &App) -> (Option<WindowBounds>, WindowState) {
    let saved = read(cx);
    let area = |bounds: Bounds<Pixels>| {
        (
            bounds.origin.x.as_f32(),
            bounds.origin.y.as_f32(),
            bounds.size.width.as_f32(),
            bounds.size.height.as_f32(),
        )
    };
    let areas: Vec<Area> = cx
        .displays()
        .iter()
        .map(|display| area(display.visible_bounds()))
        .collect();
    let primary = cx
        .primary_display()
        .map(|display| area(display.visible_bounds()))
        .unwrap_or((0., 0., DEFAULT_SIZE.0, DEFAULT_SIZE.1));
    let (place, (width, height)) = restored(saved.as_ref(), &areas, primary);
    let windowed = match place {
        Some((x, y)) => Bounds::new(point(px(x), px(y)), size(px(width), px(height))),
        None => Bounds::centered(None, size(px(width), px(height)), cx),
    };
    let state = WindowState {
        x: windowed.origin.x.as_f32(),
        y: windowed.origin.y.as_f32(),
        width,
        height,
        maximized: saved.is_some_and(|s| s.maximized),
        full_screen: saved.is_some_and(|s| s.full_screen),
    };
    let bounds = if state.full_screen {
        WindowBounds::Fullscreen(windowed)
    } else if state.maximized {
        WindowBounds::Maximized(windowed)
    } else {
        WindowBounds::Windowed(windowed)
    };
    (Some(bounds), state)
}

/// Saves the window's bounds and mode shortly after each change, through the core, which writes
/// one at a time for every app on the profile. (Each app writing its own file under one shared
/// temporary name, two apps saving at once lost one of the saves.)
pub fn track<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> Subscription {
    // The newest save replaces the one waiting, which is dropped and so never runs.
    let pending: std::rc::Rc<std::cell::RefCell<Option<Task<()>>>> = Default::default();
    cx.observe_window_bounds(window, move |_, window, cx| {
        let bounds = window.window_bounds();
        let (normal, maximized, full_screen) = match bounds {
            WindowBounds::Windowed(bounds) => (bounds, false, false),
            WindowBounds::Maximized(bounds) => (bounds, true, false),
            WindowBounds::Fullscreen(bounds) => (bounds, false, true),
        };
        let state = WindowState {
            x: normal.origin.x.as_f32(),
            y: normal.origin.y.as_f32(),
            width: normal.size.width.as_f32(),
            height: normal.size.height.as_f32(),
            maximized,
            full_screen,
        };
        *pending.borrow_mut() = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            cx.update(|cx| crate::prefs::keep(cx, KEY, state));
        }));
    })
}

#[cfg(test)]
mod tests {
    use super::{MIN_SIZE, WindowState, parse, restored};

    const SCREEN: (f32, f32, f32, f32) = (0., 25., 1440., 875.);

    fn state(x: f32, y: f32, width: f32, height: f32) -> WindowState {
        WindowState {
            x,
            y,
            width,
            height,
            maximized: false,
            full_screen: false,
        }
    }

    #[test]
    fn a_saved_state_reads_or_is_ignored() {
        let saved =
            parse(r#"{"x":10.4,"y":40,"width":1200,"height":800,"maximized":true}"#).unwrap();
        assert_eq!((saved.x, saved.width), (10., 1200.));
        assert!(saved.maximized && !saved.full_screen);
        assert!(parse(r#"{"x":0,"y":0,"width":0,"height":800}"#).is_none());
        assert!(parse("damaged").is_none());
    }

    #[test]
    fn the_window_opens_where_it_was_left_if_a_display_can_show_it() {
        let saved = state(100., 60., 1200., 800.);
        assert_eq!(
            restored(Some(&saved), &[SCREEN], SCREEN),
            (Some((100., 60.)), (1200., 800.))
        );
        // An unplugged monitor: the default size, centred.
        let gone = state(3000., 60., 1200., 800.);
        assert_eq!(restored(Some(&gone), &[SCREEN], SCREEN).0, None);
        // Too big for the display: shrunk and slid back on.
        let big = state(400., 30., 2000., 2000.);
        let (place, size) = restored(Some(&big), &[SCREEN], SCREEN);
        assert_eq!(size, (1440., 875.));
        assert_eq!(place, Some((0., 25.)));
        // Never under the minimum.
        let small = state(100., 60., 100., 100.);
        assert_eq!(restored(Some(&small), &[SCREEN], SCREEN).1, MIN_SIZE);
    }
}
