//! Screenshots of every page at the minimum, a mid and the default window size, rendered offscreen
//! by the platform's headless renderer with the app's own assets, for comparing layouts by eye.
//! Each page is also drawn again after only the shell redrew, from its cached frame, and must look
//! the same as when it was drawn afresh.
//!
//! `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens -- --ignored` writes
//! `<dir>/<width>x<height>/<page>.png` on macOS, and in `<dir>/tabs-light` and `<dir>/tabs-dark`
//! every tab of every page and every Settings section in a tall window, so a long page shows
//! whole; it needs a GPU, so it is not part of `check`. `every_screen_with_content`, run on its
//! own with `KCHESS_E2E_LICHESS=https://lichess.org`, seeds games and reads the real Lichess into
//! `<dir>/content-light` and `-dark`.

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

#[test]
#[ignore = "needs a GPU, KCHESS_SCREENSHOTS and KCHESS_E2E_LICHESS"]
fn every_screen_with_content() {
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    super::environment();
    for (folder, appearance) in [
        ("content-light", Appearance::Light),
        ("content-dark", Appearance::Dark),
    ] {
        content(&Path::new(&out).join(folder), appearance);
    }
}

/// The Opera Game, a real game with a clear story for a review to tell.
const OPERA: &str = "e2e4 e7e5 g1f3 d7d6 d2d4 c8g4 d4e5 g4f3 d1f3 d6e5 f1c4 g8f6 f3b3 d8e7 b1c3 c7c6 \
                     c1g5 b7b5 c3b5 c6b5 c4b5 b8d7 e1c1 a8d8 d1d7 d8d7 h1d1 e7e6 b5d7 f6d7 b3b8 \
                     d7b8 d1d8";

fn content(out: &Path, appearance: Appearance) {
    std::fs::create_dir_all(out).expect("the screenshot folder");
    let profile = tempfile::tempdir().expect("a temporary profile");
    let (mut cx, window) = start(profile.path(), 1320., 1500.);
    cx.update(|cx| crate::settings::change(cx, move |s| s.frontend.appearance = appearance));
    let moves: Vec<String> = OPERA.split_whitespace().map(str::to_string).collect();
    let results = ["1-0", "0-1", "1/2-1/2", "1-0", "0-1", "1-0"];
    cx.update(|cx| {
        let played = cx.global::<crate::played::PlayedHandle>().0.clone();
        played.update(cx, |played, cx| {
            for (index, result) in results.iter().enumerate() {
                let length = moves.len() - index * 2;
                played.save(
                    serde_json::json!({
                        "id": format!("seed-{index}"),
                        "source": "computer",
                        "setup": { "variant": "standard", "fen": crate::game::STANDARD_FEN },
                        "moves": moves[..length],
                        "white": "You",
                        "black": "Stockfish · Club Player",
                        "result": result,
                        "reason": "checkmate",
                        "finished": true,
                        "timeControl": "300+3",
                        "startedAt": 1_790_000_000_000u64 + index as u64 * 90_000_000,
                        "updatedAt": 1_790_000_000_000u64 + index as u64 * 90_000_000,
                    }),
                    cx,
                );
            }
        });
    });
    let shoot = |cx: &mut HeadlessAppContext, name: &str, wait: u64| {
        std::thread::sleep(Duration::from_secs(wait));
        settle(cx, window);
        let image = cx.capture_screenshot(window).expect("a screenshot");
        image
            .save(out.join(format!("{name}.png")))
            .expect("the screenshot saves");
    };
    let open = |cx: &mut HeadlessAppContext, page: Page| {
        cx.update_window(window, |_, window, cx| crate::shell::open(page, window, cx))
            .expect("the window");
    };
    let click = |cx: &mut HeadlessAppContext, id: gpui_kit::ElementId| {
        cx.update_window(window, |_, window, cx| {
            if window.try_find(id.clone()).is_some() {
                window.click(id.clone(), cx);
            }
        })
        .expect("the window");
    };
    shoot(&mut cx, "home-loading", 1);
    open(&mut cx, Page::Dashboard);
    shoot(&mut cx, "home", 1);
    open(&mut cx, Page::History);
    shoot(&mut cx, "history", 1);
    let line = moves.clone();
    cx.update_window(window, move |_, window, cx| {
        crate::shell::open_analysis_line(line, window, cx)
    })
    .expect("the window");
    shoot(&mut cx, "analysis-moves", 1);
    click(&mut cx, "Review".into());
    shoot(&mut cx, "analysis-review-start", 1);
    click(&mut cx, "review-start".into());
    shoot(&mut cx, "analysis-reviewed", 20);
    click(&mut cx, "Explorer".into());
    shoot(&mut cx, "analysis-explorer", 4);
    for (page, name) in [
        (Page::Puzzles, "puzzles"),
        (Page::Tournaments, "tournaments"),
        (Page::Watch, "watch-tv"),
    ] {
        open(&mut cx, page);
        shoot(&mut cx, name, 5);
    }
    click(&mut cx, ("watch-tabs", 2usize).into());
    shoot(&mut cx, "watch-broadcasts", 5);
    open(&mut cx, Page::Players);
    shoot(&mut cx, "players-leaderboard", 4);
    cx.update_window(window, |_, window, cx| {
        crate::shell::open_player("thibault", window, cx)
    })
    .expect("the window");
    shoot(&mut cx, "player", 6);
    let quit = cx.update(crate::core::quit);
    cx.foreground_executor().block_test(quit);
    crate::profile::TEST_DATA_DIR.set(None);
}

/// The app in a window of this size, over `profile`.
fn start(profile: &Path, width: f32, height: f32) -> (HeadlessAppContext, AnyWindowHandle) {
    let state = serde_json::json!({ "x": 0, "y": 0, "width": width, "height": height });
    std::fs::write(profile.join("window-state.json"), state.to_string()).expect("the window state");
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
    label
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric(), "-")
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
