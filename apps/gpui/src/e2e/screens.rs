//! Screenshots of every page at the minimum, a mid and the default window size, rendered offscreen
//! by the platform's headless renderer with the app's own assets, for comparing layouts by eye.
//! Each page is also drawn again after only the shell redrew, from its cached frame, and must look
//! the same as when it was drawn afresh.
//!
//! `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens -- --ignored` writes
//! `<dir>/<width>x<height>/<page>.png` on macOS, and in `<dir>/tabs-light` and `<dir>/tabs-dark`
//! every tab of every page and every Settings section in a tall window, so a long page shows
//! whole; it needs a GPU, so it is not part of `check`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, HeadlessAppContext, Role};
use kchess_contracts::types::Appearance;

use super::SIZES;
use crate::pages::Page;

#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn every_page_at_every_size() {
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    super::environment();
    let mut differ = Vec::new();
    for (width, height) in SIZES {
        capture(
            &Path::new(&out).join(format!("{width}x{height}")),
            width,
            height,
            &mut differ,
        );
    }
    assert!(
        differ.is_empty(),
        "drawn from their cached frames, these pages look different (see their -reused.png):\n  {}",
        differ.join("\n  ")
    );
}

#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn every_tab_in_both_appearances() {
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    super::environment();
    for (folder, appearance) in [
        ("tabs-light", Appearance::Light),
        ("tabs-dark", Appearance::Dark),
    ] {
        tabs(&Path::new(&out).join(folder), appearance);
    }
}

/// The app in a window of this size, over `profile`.
fn start(profile: &Path, width: f32, height: f32) -> (HeadlessAppContext, AnyWindowHandle) {
    let state = serde_json::json!({ "x": 0, "y": 0, "width": width, "height": height });
    std::fs::write(profile.join("window-state.json"), state.to_string())
        .expect("the window state");
    crate::profile::TEST_DATA_DIR.set(Some(profile.to_path_buf()));
    let text = Arc::new(gpui_apple::AppleTextSystem::new());
    let mut cx = HeadlessAppContext::with_platform(
        text,
        Arc::new(crate::assets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::menu::init(cx);
    });
    cx.update(|cx| crate::core::start(profile.to_path_buf(), cx))
        .expect("the core opens the profile");
    cx.update(crate::startup::start);
    cx.run_until_parked();
    let window = cx
        .update(|cx| cx.windows().first().copied())
        .expect("the main window opened");
    (cx, window)
}

fn tabs(out: &Path, appearance: Appearance) {
    std::fs::create_dir_all(out).expect("the screenshot folder");
    let profile = tempfile::tempdir().expect("a temporary profile");
    let (mut cx, window) = start(profile.path(), 1320., 1500.);
    cx.update(|cx| crate::settings::change(cx, move |s| s.frontend.appearance = appearance));
    settle(&mut cx, window);
    let shoot = |cx: &mut HeadlessAppContext, name: String| {
        settle(cx, window);
        let image = cx.capture_screenshot(window).expect("a screenshot");
        image
            .save(out.join(format!("{name}.png")))
            .expect("the screenshot saves");
    };
    for (page, label, _) in crate::shell::nav().iter() {
        let (page, label) = (*page, *label);
        cx.update_window(window, |_, window, cx| crate::shell::open(page, window, cx))
            .expect("the window");
        let name = slug(label);
        shoot(&mut cx, name.clone());
        let tabs = cx
            .update_window(window, |_, window, _| {
                window
                    .elements()
                    .into_iter()
                    .filter(|e| e.visible() && e.role() == Some(Role::Tab))
                    .filter_map(|e| Some((e.path().last()?.clone(), e.label()?.to_string())))
                    .collect::<Vec<_>>()
            })
            .expect("the window");
        for (id, tab) in tabs {
            cx.update_window(window, |_, window, cx| {
                if window.try_find(id.clone()).is_some() {
                    window.click(id.clone(), cx);
                }
            })
            .expect("the window");
            shoot(&mut cx, format!("{name}--{}", slug(&tab)));
        }
    }
    for (section, label, _) in crate::pages::settings::sections() {
        cx.update_window(window, |_, window, cx| {
            crate::shell::open_settings(section, window, cx)
        })
        .expect("the window");
        shoot(&mut cx, format!("settings--{}", slug(label)));
    }
    let quit = cx.update(crate::core::quit);
    cx.foreground_executor().block_test(quit);
    crate::profile::TEST_DATA_DIR.set(None);
}

fn slug(label: &str) -> String {
    label.to_lowercase().replace(
        |c: char| !c.is_alphanumeric(),
        "-",
    )
}

fn capture(out: &Path, width: f32, height: f32, differ: &mut Vec<String>) {
    std::fs::create_dir_all(out).expect("the screenshot folder");
    let profile = tempfile::tempdir().expect("a temporary profile");
    let (mut cx, window) = start(profile.path(), width, height);
    let mut pages: Vec<(Page, &str)> = crate::shell::nav()
        .iter()
        .map(|(page, label, _)| (*page, *label))
        .collect();
    pages.push((Page::Settings, "Settings"));
    for (page, label) in pages {
        cx.update_window(window, |_, window, cx| crate::shell::open(page, window, cx))
            .expect("the window");
        settle(&mut cx, window);
        let image = cx.capture_screenshot(window).expect("a screenshot");
        let name = slug(label);
        image
            .save(out.join(format!("{name}.png")))
            .expect("the screenshot saves");
        // Only the shell redraws: the page's last frame is reused, as on screen.
        cx.update(crate::shell::notify_shell);
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
            .expect("the window");
        let reused = cx.capture_screenshot(window).expect("a screenshot");
        if reused.as_raw() != image.as_raw() {
            reused
                .save(out.join(format!("{name}-reused.png")))
                .expect("the screenshot saves");
            differ.push(format!("{label} at {width}×{height}"));
        }
    }
    let quit = cx.update(crate::core::quit);
    cx.foreground_executor().block_test(quit);
    crate::profile::TEST_DATA_DIR.set(None);
}

/// Let the core answer and animations end before a picture.
fn settle(cx: &mut HeadlessAppContext, window: AnyWindowHandle) {
    for _ in 0..60 {
        std::thread::sleep(Duration::from_millis(5));
        cx.advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .expect("the window");
}
