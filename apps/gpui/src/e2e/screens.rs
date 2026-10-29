//! Screenshots of every page at the minimum, a mid and the default window size, rendered offscreen
//! by the platform's headless renderer with the app's own assets, for comparing layouts by eye.
//!
//! `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens -- --ignored` writes
//! `<dir>/<width>x<height>/<page>.png` on macOS; it needs a GPU, so it is not part of `check`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, HeadlessAppContext};

use super::SIZES;
use crate::pages::Page;

#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn every_page_at_every_size() {
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    super::environment();
    for (width, height) in SIZES {
        capture(
            &Path::new(&out).join(format!("{width}x{height}")),
            width,
            height,
        );
    }
}

fn capture(out: &Path, width: f32, height: f32) {
    std::fs::create_dir_all(out).expect("the screenshot folder");
    let profile = tempfile::tempdir().expect("a temporary profile");
    let state = serde_json::json!({ "x": 0, "y": 0, "width": width, "height": height });
    std::fs::write(profile.path().join("window-state.json"), state.to_string())
        .expect("the window state");
    crate::profile::TEST_DATA_DIR.set(Some(profile.path().to_path_buf()));
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
    cx.update(|cx| crate::core::start(profile.path().to_path_buf(), cx))
        .expect("the core opens the profile");
    cx.update(crate::startup::start);
    cx.run_until_parked();
    let window = cx
        .update(|cx| cx.windows().first().copied())
        .expect("the main window opened");
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
        let name = label.to_lowercase().replace([' ', '/'], "-");
        image
            .save(out.join(format!("{name}.png")))
            .expect("the screenshot saves");
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
