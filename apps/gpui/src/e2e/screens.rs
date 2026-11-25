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
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
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
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
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
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    super::environment();
    for (folder, appearance) in [
        ("content-light", Appearance::Light),
        ("content-dark", Appearance::Dark),
    ] {
        content(&Path::new(&out).join(folder), appearance);
    }
}

/// Following's timeline over the stand-in Lichess's players, wide and narrow, in both
/// appearances: `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::the_timeline --
/// --ignored` writes `<dir>/timeline/<appearance>-<width>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn the_timeline() {
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("timeline");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        for (width, height) in [(1320., 1500.), (760., 1500.)] {
            let profile = tempfile::tempdir().expect("a temporary profile");
            let (mut cx, window) = start(profile.path(), width, height);
            cx.update(|cx| {
                crate::settings::change(cx, move |s| s.frontend.appearance = appearance)
            });
            let players = kchess_contracts::check::FriendList(
                ["thibault", "EricRosen", "DrNykterstein", "penguingim1"]
                    .map(String::from)
                    .to_vec(),
            );
            let added = cx.update(|cx| {
                cx.global::<crate::core::AppCore>()
                    .request(kchess_contracts::calls::AddFriends { usernames: players })
            });
            cx.foreground_executor()
                .block_test(added)
                .expect("the players are added");
            // Synced, and then two of them finish a game: theirs are new.
            let synced = cx.update(|cx| {
                cx.global::<crate::core::AppCore>()
                    .request(kchess_contracts::calls::SyncGames { username: None })
            });
            cx.foreground_executor()
                .block_test(synced)
                .expect("the players are synced");
            super::fake_lichess::play("EricRosen", 300);
            super::fake_lichess::play("penguingim1", 300);
            cx.update_window(window, |_, window, cx| {
                crate::shell::open(Page::Friends, window, cx)
            })
            .expect("the window");
            // Lichess answers while the app runs, not while it sleeps.
            for _ in 0..10 {
                settle(&mut cx, window);
            }
            cx.capture_screenshot(window)
                .expect("a screenshot")
                .save(out.join(format!("{name}-{width}.png")))
                .expect("the screenshot saves");
            let quit = cx.update(crate::quit::quit);
            cx.foreground_executor().block_test(quit);
            crate::profile::TEST_DATA_DIR.set(None);
        }
    }
}

/// The sidebar while Settings is open, in both appearances, open and as a rail, at the default
/// window size and a short one that scrolls it: `KCHESS_SCREENSHOTS=<dir> cargo test -p
/// kchess-gpui e2e::screens::settings_sidebar -- --ignored` writes
/// `<dir>/settings-sidebar/<appearance>-<height>[-rail].png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn settings_sidebar() {
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("settings-sidebar");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        for height in [880., 560.] {
            let profile = tempfile::tempdir().expect("a temporary profile");
            let (mut cx, window) = start(profile.path(), 1320., height);
            cx.update(|cx| {
                crate::settings::change(cx, move |s| s.frontend.appearance = appearance)
            });
            cx.update_window(window, |_, window, cx| {
                crate::shell::open_settings(crate::pages::settings::Section::Sound, window, cx)
            })
            .expect("the window");
            settle(&mut cx, window);
            let shoot = |cx: &mut HeadlessAppContext, suffix: &str| {
                settle(cx, window);
                cx.capture_screenshot(window)
                    .expect("a screenshot")
                    .save(out.join(format!("{name}-{height}{suffix}.png")))
                    .expect("the screenshot saves");
            };
            shoot(&mut cx, "");
            // The last category, from the palette: the sidebar scrolls to it.
            cx.update_window(window, |_, window, cx| {
                crate::shell::open_settings(crate::pages::settings::Section::Updates, window, cx)
            })
            .expect("the window");
            shoot(&mut cx, "-updates");
            cx.update_window(window, |_, window, cx| window.click("sidebar-toggle", cx))
                .expect("the window");
            shoot(&mut cx, "-rail");
            let quit = cx.update(crate::quit::quit);
            cx.foreground_executor().block_test(quit);
            crate::profile::TEST_DATA_DIR.set(None);
        }
    }
}

/// A row ⌘K opened Settings at, scrolled to and lit, in both appearances:
/// `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::settings_reveal -- --ignored`
/// writes `<dir>/settings-reveal/<appearance>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn settings_reveal() {
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("settings-reveal");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), 1320., 880.);
        cx.update(|cx| crate::settings::change(cx, move |s| s.frontend.appearance = appearance));
        cx.update_window(window, |_, window, cx| {
            let section = crate::pages::settings::Section::Appearance;
            crate::shell::open_settings_at(section, "Piece set", window, cx)
        })
        .expect("the window");
        // Within the light's two seconds.
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("{name}.png")))
            .expect("the screenshot saves");
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// A setting just changed in Settings, in both appearances: the notice with its Undo, and the
/// header's Restore defaults. `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui
/// e2e::screens::settings_undo -- --ignored` writes `<dir>/settings-undo/<appearance>-<width>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn settings_undo() {
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("settings-undo");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        for width in [1320., 640.] {
            let profile = tempfile::tempdir().expect("a temporary profile");
            let (mut cx, window) = start(profile.path(), width, 880.);
            cx.update(|cx| {
                crate::settings::change(cx, move |s| s.frontend.appearance = appearance)
            });
            cx.update_window(window, |_, window, cx| {
                crate::shell::open_settings(crate::pages::settings::Section::Gameplay, window, cx)
            })
            .expect("the window");
            settle(&mut cx, window);
            cx.update_window(window, |_, window, cx| {
                crate::settings::change(cx, |s| s.frontend.premove = false);
                let edit = cx
                    .global::<crate::settings::SettingsHandle>()
                    .0
                    .read(cx)
                    .last_edit()
                    .cloned()
                    .expect("the change");
                crate::setting_notice::edited(&edit, window, cx);
            })
            .expect("the window");
            settle(&mut cx, window);
            cx.capture_screenshot(window)
                .expect("a screenshot")
                .save(out.join(format!("{name}-{width}.png")))
                .expect("the screenshot saves");
            let quit = cx.update(crate::quit::quit);
            cx.foreground_executor().block_test(quit);
            crate::profile::TEST_DATA_DIR.set(None);
        }
    }
}

/// Every Settings category, dark, wide and narrow: `KCHESS_SCREENSHOTS=<dir> cargo test -p
/// kchess-gpui e2e::screens::settings_sections -- --ignored` writes
/// `<dir>/settings/<width>-<category>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn settings_sections() {
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("settings");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for width in [1320., 900.] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), width, 880.);
        cx.update(|cx| crate::settings::change(cx, |s| s.frontend.appearance = Appearance::Dark));
        for (section, label) in crate::pages::settings::Section::ALL.map(|s| (s, s.label())) {
            cx.update_window(window, |_, window, cx| {
                crate::shell::open_settings(section, window, cx)
            })
            .expect("the window");
            settle(&mut cx, window);
            cx.capture_screenshot(window)
                .expect("a screenshot")
                .save(out.join(format!("{width}-{}.png", label.to_lowercase())))
                .expect("the screenshot saves");
        }
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// A Crazyhouse game over the board, with pieces in both pockets and one picked to drop, in
/// both appearances: `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::crazyhouse
/// -- --ignored` writes `<dir>/crazyhouse/<appearance>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn crazyhouse() {
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("crazyhouse");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), 1320., 880.);
        cx.update(|cx| crate::settings::change(cx, move |s| s.frontend.appearance = appearance));
        cx.update_window(window, |_, window, cx| {
            crate::shell::open(Page::Local, window, cx);
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.update_window(window, |_, window, cx| {
            let local = crate::shell::view::<crate::pages::local::LocalPage>(cx).expect("the page");
            local.update(cx, |local, cx| {
                let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
                let setup = kchess_domain::variant::Setup {
                    variant: kchess_domain::variant::Variant::Crazyhouse,
                    fen: fen.into(),
                };
                local.replace_game(setup, None, window, cx);
                for uci in [
                    "e2e4", "d7d5", "e4d5", "d8d5", "b1c3", "d5a5", "f1b5", "c7c6", "b5c6",
                ] {
                    local.play(uci, cx);
                }
                // Black takes back with the knight: White holds two pawns, Black a pawn and a
                // bishop.
                local.play("b8c6", cx);
            });
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.update_window(window, |_, window, cx| {
            window.click("pocket-White-Pawn", cx)
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("{name}.png")))
            .expect("the screenshot saves");
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// Moves written with figurines over the board and in Settings, in both appearances:
/// `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::figurines -- --ignored`
/// writes `<dir>/figurines/<appearance>-<page>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn figurines() {
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("figurines");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), 1320., 880.);
        cx.update(|cx| {
            crate::settings::change(cx, move |s| {
                s.frontend.appearance = appearance;
                s.frontend.figurine_notation = true;
            })
        });
        cx.update_window(window, |_, window, cx| {
            crate::shell::open(Page::Local, window, cx);
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.update_window(window, |_, _, cx| {
            let local = crate::shell::view::<crate::pages::local::LocalPage>(cx).expect("the page");
            local.update(cx, |local, cx| {
                for uci in [
                    "e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "a7a6", "b5c6", "d7c6", "e1g1", "f7f6",
                    "d2d4", "c8g4", "d4e5", "d8d1", "f1d1",
                ] {
                    local.play(uci, cx);
                }
            });
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("{name}-board.png")))
            .expect("the screenshot saves");
        cx.update_window(window, |_, window, cx| {
            crate::shell::open(Page::Settings, window, cx);
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("{name}-settings.png")))
            .expect("the screenshot saves");
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// A Crazyhouse game against the computer, named above the board, wide and at the narrowest
/// window: `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::computer_variant --
/// --ignored` writes `<dir>/computer-variant/<width>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn computer_variant() {
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("computer-variant");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (width, height) in [(1320., 880.), SIZES[0]] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), width, height);
        cx.update_window(window, |_, window, cx| {
            crate::shell::open(Page::Computer, window, cx);
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.update_window(window, |_, window, cx| {
            let page =
                crate::shell::view::<crate::pages::computer::ComputerPage>(cx).expect("the page");
            page.update(cx, |page, cx| {
                let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
                let setup = kchess_domain::variant::Setup {
                    variant: kchess_domain::variant::Variant::Crazyhouse,
                    fen: fen.into(),
                };
                page.start_from(setup, shakmaty::Color::White, window, cx);
            });
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("{width}.png")))
            .expect("the screenshot saves");
        // The game options open on the variant game, its FEN field under the variant.
        cx.update_window(window, |_, window, cx| {
            window.click(gpui_kit::ElementId::from("game-options"), cx);
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("options-{width}.png")))
            .expect("the screenshot saves");
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// Home as a new person, someone with games played here, and a connected Lichess account with
/// years of history, in both appearances, wide and narrow:
/// `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::home -- --ignored` writes
/// `<dir>/home/<state>-<appearance>-<width>.png`; `KCHESS_HOME_STATES` and `KCHESS_HOME_SIZES`
/// narrow it down, `KCHESS_HOME_RAIL=1` shoots it with the sidebar collapsed to its rail, and
/// `KCHESS_HOME_TURN=1` with your move waiting in a correspondence game.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn home() {
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("home");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    let only = kchess_host::env::var("KCHESS_HOME_STATES").unwrap_or_default();
    let rail = kchess_host::env::var_os("KCHESS_HOME_RAIL").is_some();
    let turn = kchess_host::env::var_os("KCHESS_HOME_TURN").is_some();
    for state in ["new", "finished", "played", "lichess"] {
        if !only.is_empty() && !only.split(',').any(|wanted| wanted == state) {
            continue;
        }
        for (appearance, name) in [(Appearance::Dark, "dark"), (Appearance::Light, "light")] {
            for (width, height) in home_sizes() {
                let profile = tempfile::tempdir().expect("a temporary profile");
                let (mut cx, window) = start(profile.path(), width, height);
                cx.update(|cx| {
                    crate::settings::change(cx, move |s| s.frontend.appearance = appearance)
                });
                if state != "new" {
                    seed_played(&mut cx, state != "finished");
                }
                if state == "lichess" {
                    connect_showcase(&mut cx, window, profile.path(), "Rattiestword5");
                }
                cx.update_window(window, |_, window, cx| {
                    crate::shell::open(Page::Dashboard, window, cx);
                    if turn {
                        crate::online::handle(cx).update(cx, |online, cx| {
                            online.ongoing = vec![
                                serde_json::from_value(serde_json::json!({
                                    "gameId": "corr1",
                                    "account": "",
                                    "opponent": { "name": "" },
                                    "color": "white",
                                    "fen": "",
                                    "variant": "standard",
                                    "speed": "correspondence",
                                    "rated": false,
                                    "isMyTurn": true,
                                }))
                                .expect("a game in progress"),
                            ];
                            cx.notify();
                        });
                    }
                    if rail {
                        crate::shell::toggle_sidebar(window, cx);
                    }
                })
                .expect("the window");
                for _ in 0..6 {
                    settle(&mut cx, window);
                }
                let suffix = if rail { "-rail" } else { "" };
                cx.capture_screenshot(window)
                    .expect("a screenshot")
                    .save(out.join(format!("{state}-{name}-{width}{suffix}.png")))
                    .expect("the screenshot saves");
                let quit = cx.update(crate::quit::quit);
                cx.foreground_executor().block_test(quit);
                crate::profile::TEST_DATA_DIR.set(None);
            }
        }
    }
}

/// Players' search before anyone was looked up, with recent names, while a name is typed, after a
/// name Lichess would refuse, and above a player, in both appearances, wide and narrow:
/// `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::players_search -- --ignored`
/// writes `<dir>/players-search/<state>-<appearance>-<width>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn players_search() {
    use gpui_kit::test::TestWindowExt as _;
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("players-search");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for state in ["first", "recent", "typing", "invalid", "player"] {
        for (appearance, name) in [(Appearance::Dark, "dark"), (Appearance::Light, "light")] {
            for (width, height) in [(1320., 900.), (760., 900.)] {
                let profile = tempfile::tempdir().expect("a temporary profile");
                let (mut cx, window) = start(profile.path(), width, height);
                cx.update(|cx| {
                    crate::settings::change(cx, move |s| s.frontend.appearance = appearance);
                    if state != "first" {
                        let recent = ["teknaS", "anicrus", "Anixris007", "bobadya", "Kunjshah45"];
                        crate::prefs::set(cx, "kchess:players-recent", recent);
                    }
                });
                cx.update_window(window, |_, window, cx| {
                    crate::shell::open(Page::Players, window, cx)
                })
                .expect("the window");
                settle(&mut cx, window);
                let typed = match state {
                    "typing" => Some("mag"),
                    "invalid" => Some("no one"),
                    _ => None,
                };
                if let Some(typed) = typed {
                    cx.update_window(window, |_, window, cx| {
                        use gpui_kit::{
                            InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
                        };
                        let field = window
                            .try_find_by_label("Lichess username")
                            .expect("the search field");
                        let position = field.bounds().center();
                        for event in [
                            MouseDownEvent {
                                button: MouseButton::Left,
                                position,
                                modifiers: Modifiers::default(),
                                click_count: 1,
                                first_mouse: false,
                            }
                            .to_platform_input(),
                            MouseUpEvent {
                                button: MouseButton::Left,
                                position,
                                modifiers: Modifiers::default(),
                                click_count: 1,
                            }
                            .to_platform_input(),
                        ] {
                            window.dispatch_event(event, cx);
                            super::draw(window, cx);
                        }
                        window.input(typed, cx);
                        if state == "invalid" {
                            window.press("enter", cx);
                        }
                    })
                    .expect("the window");
                }
                if state == "player" {
                    connect_showcase(&mut cx, window, profile.path(), "Rattiestword5");
                    cx.update_window(window, |_, window, cx| {
                        crate::pages::players::open_player("Rattiestword5", window, cx)
                    })
                    .expect("the window");
                }
                for _ in 0..8 {
                    std::thread::sleep(Duration::from_millis(60));
                    settle(&mut cx, window);
                }
                cx.capture_screenshot(window)
                    .expect("a screenshot")
                    .save(out.join(format!("{state}-{name}-{width}.png")))
                    .expect("the screenshot saves");
                let quit = cx.update(crate::quit::quit);
                cx.foreground_executor().block_test(quit);
                crate::profile::TEST_DATA_DIR.set(None);
            }
        }
    }
}

/// Game history's Lichess tab searched for an opening, in both appearances, wide and narrow:
/// `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::lichess_history_search --
/// --ignored` writes `<dir>/lichess-history/<state>-<appearance>-<width>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn lichess_history_search() {
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("lichess-history");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (state, search) in [("all", ""), ("searched", "najdorf"), ("none", "nobody")] {
        for (appearance, name) in [(Appearance::Dark, "dark"), (Appearance::Light, "light")] {
            for (width, height) in [(1320., 900.), (760., 900.)] {
                let profile = tempfile::tempdir().expect("a temporary profile");
                let (mut cx, window) = start(profile.path(), width, height);
                cx.update(|cx| {
                    crate::settings::change(cx, move |s| s.frontend.appearance = appearance)
                });
                connect_showcase(&mut cx, window, profile.path(), "Rattiestword5");
                cx.update_window(window, |_, window, cx| {
                    crate::shell::open(Page::History, window, cx);
                    if let Some(page) = crate::shell::view::<crate::pages::history::HistoryPage>(cx)
                    {
                        page.update(cx, |page, cx| page.search_lichess(search, window, cx));
                    }
                })
                .expect("the window");
                for _ in 0..8 {
                    std::thread::sleep(Duration::from_millis(60));
                    settle(&mut cx, window);
                }
                cx.capture_screenshot(window)
                    .expect("a screenshot")
                    .save(out.join(format!("{state}-{name}-{width}.png")))
                    .expect("the screenshot saves");
                let quit = cx.update(crate::quit::quit);
                cx.foreground_executor().block_test(quit);
                crate::profile::TEST_DATA_DIR.set(None);
            }
        }
    }
}

/// The window sizes Home is shot at: wide and narrow, or `KCHESS_HOME_SIZES` ("1920x1100,760x1400").
fn home_sizes() -> Vec<(f32, f32)> {
    let sizes: Vec<(f32, f32)> = kchess_host::env::var("KCHESS_HOME_SIZES")
        .unwrap_or_default()
        .split(',')
        .filter_map(|size| {
            let (width, height) = size.split_once('x')?;
            Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
        })
        .collect();
    if sizes.is_empty() {
        vec![(1320., 1100.), (760., 1400.)]
    } else {
        sizes
    }
}

/// A few finished games played here, and an unfinished one against the computer if `unfinished`.
fn seed_played(cx: &mut HeadlessAppContext, unfinished: bool) {
    use kchess_domain::games::Host;
    use kchess_domain::games::computer::ComputerGame;
    let host = Host {
        now: 0.,
        allowed: true,
        ready: false,
    };
    let saved = kchess_domain::games::computer::Saved {
        moves: Some(
            ["e2e4", "e7e5", "g1f3", "b8c6", "f1c4"]
                .map(str::to_string)
                .to_vec(),
        ),
        ..Default::default()
    };
    let session = kchess_contracts::types::SessionDocument::Computer(
        ComputerGame::resume(Some(saved), &host)
            .expect("the computer game")
            .session(host.now),
    );
    if unfinished {
        let saved = cx.update(|cx| {
            cx.global::<crate::core::AppCore>()
                .request(kchess_contracts::calls::SaveSession {
                    kind: kchess_contracts::types::SessionKind::Computer,
                    session: kchess_contracts::check::SessionObject::of(&session),
                    based_on: None,
                })
        });
        cx.foreground_executor()
            .block_test(saved)
            .expect("the computer game is saved");
    }
    let moves: Vec<String> = OPERA.split_whitespace().map(str::to_string).collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64);
    let games = [
        ("You", "Stockfish · Club Player", "1-0", "computer"),
        ("Stockfish · Club Player", "You", "1/2-1/2", "computer"),
        ("You", "Stockfish · Intermediate", "0-1", "computer"),
        ("You", "Stockfish · Club Player", "1-0", "computer"),
        ("Stockfish · Club Player", "You", "0-1", "computer"),
    ];
    cx.update(|cx| {
        let played = cx.global::<crate::played::PlayedHandle>().0.clone();
        played.update(cx, |played, cx| {
            for (index, (white, black, result, source)) in games.iter().enumerate() {
                let length = moves.len() - index * 2;
                let at = now - (index as u64 + 1) * 20 * 3_600_000;
                played.save(
                    serde_json::from_value(serde_json::json!({
                        "id": format!("home-{index}"),
                        "source": source,
                        "setup": { "variant": "standard", "fen": crate::game::STANDARD_FEN },
                        "moves": moves[..length],
                        "white": white,
                        "black": black,
                        "result": result,
                        "reason": "checkmate",
                        "finished": true,
                        "timeControl": "300+3",
                        "startedAt": at,
                        "updatedAt": at,
                    }))
                    .expect("a played game"),
                    cx,
                );
            }
        });
    });
}

/// The sidebar's account menu, open, in both appearances: with one account and with two, in the
/// open sidebar and on its rail. `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui
/// e2e::screens::account_menu -- --ignored` writes `<dir>/account-menu/<appearance>-<accounts>[-rail].png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn account_menu() {
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("account-menu");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        for accounts in [1, 2] {
            let profile = tempfile::tempdir().expect("a temporary profile");
            let (mut cx, window) = start(profile.path(), 1320., 880.);
            cx.update(|cx| {
                crate::settings::change(cx, move |s| s.frontend.appearance = appearance)
            });
            if accounts == 2 {
                connect_showcase(&mut cx, window, profile.path(), "Knightowl");
            }
            connect_showcase(&mut cx, window, profile.path(), "Rattiestword5");
            let shoot = |cx: &mut HeadlessAppContext, suffix: &str| {
                cx.update_window(window, |_, window, cx| window.click("account-menu", cx))
                    .expect("the window");
                settle(cx, window);
                cx.capture_screenshot(window)
                    .expect("a screenshot")
                    .save(out.join(format!("{name}-{accounts}{suffix}.png")))
                    .expect("the screenshot saves");
                // A second press closes it.
                cx.update_window(window, |_, window, cx| window.click("account-menu", cx))
                    .expect("the window");
                settle(cx, window);
            };
            shoot(&mut cx, "");
            cx.update_window(window, |_, window, cx| window.click("sidebar-toggle", cx))
                .expect("the window");
            settle(&mut cx, window);
            shoot(&mut cx, "-rail");
            let quit = cx.update(crate::quit::quit);
            cx.foreground_executor().block_test(quit);
            crate::profile::TEST_DATA_DIR.set(None);
        }
    }
}

/// The keyboard shortcuts dialog in both appearances, at the default and the smallest size:
/// `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::shortcuts -- --ignored`
/// writes `<dir>/shortcuts/<appearance>-<width>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn shortcuts() {
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("shortcuts");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        for (width, height) in [SIZES[2], SIZES[0]] {
            let profile = tempfile::tempdir().expect("a temporary profile");
            let (mut cx, window) = start(profile.path(), width, height);
            cx.update(|cx| {
                crate::settings::change(cx, move |s| s.frontend.appearance = appearance)
            });
            cx.update_window(window, |_, window, cx| crate::shortcuts::open(window, cx))
                .expect("the window");
            settle(&mut cx, window);
            cx.capture_screenshot(window)
                .expect("a screenshot")
                .save(out.join(format!("{name}-{width}.png")))
                .expect("the screenshot saves");
            let quit = cx.update(crate::quit::quit);
            cx.foreground_executor().block_test(quit);
            crate::profile::TEST_DATA_DIR.set(None);
        }
    }
}

/// An over-the-board game's clocks in their last ten seconds, in both appearances:
/// `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::low_clock -- --ignored`
/// writes `<dir>/low-clock/<appearance>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn low_clock() {
    use crate::pages::local::LocalPage;
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("low-clock");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), 1320., 880.);
        cx.update(|cx| crate::settings::change(cx, move |s| s.frontend.appearance = appearance));
        cx.update_window(window, |_, window, cx| {
            crate::shell::open(Page::Local, window, cx)
        })
        .expect("the window");
        // The saved game is read first: a game played before that was replaced by it, and the
        // pictures showed an empty board.
        for _ in 0..100 {
            if cx.update(|cx| {
                crate::shell::view::<LocalPage>(cx).is_some_and(|local| local.read(cx).resumed())
            }) {
                break;
            }
            settle(&mut cx, window);
        }
        cx.update_window(window, |_, window, cx| {
            let local = crate::shell::view::<LocalPage>(cx).expect("the page");
            local.update(cx, |local, cx| {
                let setup = kchess_domain::variant::Setup::standard();
                let clock = Some(kchess_domain::games::local::LocalClock::even(
                    kchess_domain::games::Clock {
                        minutes: 0.15,
                        increment: 0.,
                    },
                ));
                local.replace_game(setup, clock, window, cx);
                local.play("e2e4", cx);
            });
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("{name}.png")))
            .expect("the screenshot saves");
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// Over the board in both of its modes, a game under way on the shared board, at every size, so
/// switching between them can be compared by eye:
/// `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::over_the_board -- --ignored`
/// writes `<dir>/over-the-board/<width>-<mode>.png`, and `<width>-review.png` a move back
/// at the wider two.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn over_the_board() {
    use crate::pages::local::LocalPage;
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("over-the-board");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (width, height) in SIZES {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), width, height);
        cx.update(|cx| crate::settings::change(cx, |s| s.frontend.appearance = Appearance::Dark));
        cx.update_window(window, |_, window, cx| {
            crate::shell::open(Page::Local, window, cx);
            let local = crate::shell::view::<LocalPage>(cx).expect("the page");
            local.update(cx, |local, cx| {
                let setup = kchess_domain::variant::Setup::standard();
                let clock = Some(kchess_domain::games::local::LocalClock::even(
                    kchess_domain::games::Clock {
                        minutes: 5.,
                        increment: 3.,
                    },
                ));
                local.replace_game(setup, clock, window, cx);
            });
        })
        .expect("the window");
        // The new game starts once the page has its turn: moves played in the same update were
        // played on the game it replaced.
        settle(&mut cx, window);
        cx.update(|cx| {
            let local = crate::shell::view::<LocalPage>(cx).expect("the page");
            local.update(cx, |local, cx| {
                local.play("e2e4", cx);
                local.play("e7e5", cx);
            });
        });
        for (index, mode) in ["board", "clock"].into_iter().enumerate() {
            cx.update_window(window, |_, window, cx| {
                window.click(("local-mode", index), cx);
            })
            .expect("the window");
            settle(&mut cx, window);
            cx.capture_screenshot(window)
                .expect("a screenshot")
                .save(out.join(format!("{width}-{mode}.png")))
                .expect("the screenshot saves");
        }
        // A move back where the move list is beside the board: the turn line says so in its
        // own place, the moves where they were.
        if width >= 1024. {
            cx.update_window(window, |_, window, cx| {
                window.click(("local-mode", 0usize), cx);
            })
            .expect("the window");
            settle(&mut cx, window);
            cx.update_window(window, |_, window, cx| window.click("chevron-left", cx))
                .expect("the window");
            settle(&mut cx, window);
            cx.capture_screenshot(window)
                .expect("a screenshot")
                .save(out.join(format!("{width}-review.png")))
                .expect("the screenshot saves");
        }
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// The analysis board's import dialog at the default and the smallest size:
/// `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::import_dialog -- --ignored`
/// writes `<dir>/import/<width>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn import_dialog() {
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("import");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (width, height) in [SIZES[2], SIZES[0]] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), width, height);
        cx.update_window(window, |_, window, cx| {
            crate::shell::open(Page::Analysis, window, cx);
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.update_window(window, |_, window, cx| {
            let board = crate::pages::analysis::board(window, cx).expect("the analysis board");
            board.update(cx, |board, cx| board.open_import(window, cx));
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("{width}.png")))
            .expect("the screenshot saves");
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// A PGN file dragged over Puzzles, in both appearances:
/// `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::drop_zone -- --ignored`
/// writes `<dir>/drop/<appearance>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn drop_zone() {
    use gpui_kit::InputEvent as _;
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("drop");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (appearance, name) in [(Appearance::Dark, "dark"), (Appearance::Light, "light")] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (width, height) = SIZES[0];
        let (mut cx, window) = start(profile.path(), width, height);
        cx.update(|cx| crate::settings::change(cx, move |s| s.frontend.appearance = appearance));
        cx.update_window(window, |_, window, cx| {
            crate::shell::open(Page::Puzzles, window, cx)
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.update_window(window, |_, window, cx| {
            let position = gpui_kit::point(gpui_kit::px(width * 0.6), gpui_kit::px(height * 0.5));
            let paths = gpui_kit::ExternalPaths(std::iter::once("game.pgn".into()).collect());
            window.dispatch_event(
                gpui_kit::FileDropEvent::Entered { position, paths }.to_platform_input(),
                cx,
            );
            window.dispatch_event(
                gpui_kit::FileDropEvent::Pending { position }.to_platform_input(),
                cx,
            );
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("{name}.png")))
            .expect("the screenshot saves");
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// Connect the stand-in Lichess's showcase account as a person would, and sync its games.
fn connect_showcase(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    profile: &Path,
    username: &str,
) {
    let (code, session) = super::fake_lichess::session(username, false);
    session.lock().showcase = true;
    let opened = profile.join("opened-urls.txt");
    // The login pages opened so far: a second account's is the one opened after them.
    let logins = || -> Vec<String> {
        std::fs::read_to_string(&opened)
            .unwrap_or_default()
            .lines()
            .filter(|line| line.contains("/oauth"))
            .map(str::to_string)
            .collect()
    };
    let earlier = logins().len();
    cx.update_window(window, |_, window, cx| {
        crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.connect(window, cx))
    })
    .expect("the window");
    // Each wait fails the run rather than taking a picture of a showcase that never connected.
    let login = (0..400)
        .find_map(|_| {
            settle(cx, window);
            logins().into_iter().nth(earlier)
        })
        .expect("the login page opens");
    super::fake_lichess::return_from_login(&login, &code);
    let connected = (0..400).any(|_| {
        settle(cx, window);
        cx.update(|cx| {
            let accounts = crate::accounts::handle(cx);
            let accounts = accounts.read(cx);
            accounts.active(cx).as_deref() == Some(username) && !accounts.working()
        })
    });
    assert!(connected, "{username} connects and syncs");
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
                    serde_json::from_value(serde_json::json!({
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
                    }))
                    .expect("a played game"),
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
        kchess_board::analysis::open(kchess_board::analysis::Open::Line(line), window, cx)
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
    click(
        &mut cx,
        super::tab("watch-tabs", crate::pages::watch::Tab::Broadcasts).into(),
    );
    shoot(&mut cx, "watch-broadcasts", 5);
    open(&mut cx, Page::Players);
    shoot(&mut cx, "players-leaderboard", 4);
    cx.update_window(window, |_, window, cx| {
        crate::pages::players::open_player("thibault", window, cx)
    })
    .expect("the window");
    shoot(&mut cx, "player", 6);
    let quit = cx.update(crate::quit::quit);
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
        crate::notifications::init(cx);
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
    for page in Page::in_sidebar() {
        let label = page.info().label;
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
    for (section, label) in crate::pages::settings::Section::ALL.map(|s| (s, s.label())) {
        cx.update_window(window, |_, window, cx| {
            crate::shell::open_settings(section, window, cx)
        })
        .expect("the window");
        shoot(&mut cx, format!("settings--{}", slug(label)));
    }
    let quit = cx.update(crate::quit::quit);
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
    let pages: Vec<(Page, &'static str)> =
        Page::all().map(|page| (page, page.info().label)).collect();
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
    let quit = cx.update(crate::quit::quit);
    cx.foreground_executor().block_test(quit);
    crate::profile::TEST_DATA_DIR.set(None);
}

/// The board's coordinates in each mode, on the pages with a board, at the smallest and the default
/// window size: `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::board_coordinates
/// -- --ignored` writes `<dir>/coordinates/<page>-<mode>-<width>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn board_coordinates() {
    use kchess_contracts::types::CoordinateMode;
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("coordinates");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (width, height) in [SIZES[0], SIZES[2]] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), width, height);
        for (page, name) in [(Page::Analysis, "analysis"), (Page::Local, "local")] {
            cx.update_window(window, |_, window, cx| crate::shell::open(page, window, cx))
                .expect("the window");
            for (mode, label) in [
                (CoordinateMode::Inside, "inside"),
                (CoordinateMode::Outside, "outside"),
            ] {
                cx.update(|cx| crate::settings::change(cx, move |s| s.frontend.coordinates = mode));
                settle(&mut cx, window);
                cx.capture_screenshot(window)
                    .expect("a screenshot")
                    .save(out.join(format!("{name}-{label}-{width}.png")))
                    .expect("the screenshot saves");
            }
        }
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
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

/// The move box saying it cannot play a typed move, under the Openings board, wide and at the
/// minimum size: `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::move_box --
/// --ignored` writes `<dir>/move-box/<width>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn move_box() {
    use gpui_kit::test::TestWindowExt as _;
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("move-box");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (width, height) in [(1320., 880.), SIZES[0]] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), width, height);
        cx.update_window(window, |_, window, cx| {
            crate::shell::open(Page::Openings, window, cx);
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.update_window(window, |_, window, cx| {
            let page =
                crate::shell::view::<crate::pages::openings::OpeningsPage>(cx).expect("the page");
            page.update(cx, |page, cx| {
                page.show_line(Some(vec!["e2e4".into(), "e7e5".into()]), window, cx);
            });
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.update_window(window, |_, window, cx| {
            let field = window
                .try_find_by_label("Enter a chess move in SAN or UCI")
                .expect("the move box");
            use gpui_kit::{InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent};
            let position = field.bounds().center();
            for event in [
                MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }
                .to_platform_input(),
            ] {
                window.dispatch_event(event, cx);
                super::draw(window, cx);
            }
            window.input("Ka8", cx);
            window.press("enter", cx);
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("{width}.png")))
            .expect("the screenshot saves");
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// Voice turned on beside the move box, on a machine with no speech model (its error and Try
/// again under the board), at the two wider sizes: `KCHESS_SCREENSHOTS=<dir> cargo test -p
/// kchess-gpui e2e::screens::voice_on -- --ignored` writes `<dir>/voice/<width>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn voice_on() {
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("voice");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (width, height) in &SIZES[1..] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), *width, *height);
        cx.update(|cx| crate::settings::change(cx, |s| s.frontend.appearance = Appearance::Dark));
        cx.update_window(window, |_, window, cx| {
            crate::shell::open(Page::Computer, window, cx)
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.update_window(window, |_, window, cx| {
            window.click(("computer-voice", 0usize), cx)
        })
        .expect("the window");
        for _ in 0..5 {
            settle(&mut cx, window);
        }
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("{width}.png")))
            .expect("the screenshot saves");
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// Every state of What's new, in both appearances: after an update, before installing (with and
/// without what restarting would cost), the plain question once the notes are off, and the switch
/// in Settings → Updates. `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui
/// e2e::screens::whats_new -- --ignored` writes `<dir>/whats-new/<state>-<appearance>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn whats_new() {
    use gpui_kit::component::WindowExt as _;
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("whats-new");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    type Open = fn(&mut gpui_kit::Window, &mut gpui_kit::App);
    let states: [(&str, Open); 6] = [
        ("after-update", |window, cx| {
            let releases = crate::whats_new::releases().into_iter().take(2).collect();
            crate::whats_new::open_after_update(releases, window, cx)
        }),
        ("before-install", |window, cx| {
            let releases = crate::whats_new::releases().into_iter().take(2).collect();
            crate::whats_new::open_before_update(
                "2026.11.10".into(),
                releases,
                None,
                |_| {},
                window,
                cx,
            )
        }),
        ("before-install-live-game", |window, cx| {
            let releases = crate::whats_new::releases().into_iter().take(1).collect();
            let cost =
                "Your Lichess game is still on, and its clock keeps running while KChess restarts";
            crate::whats_new::open_before_update(
                "2026.11.10".into(),
                releases,
                Some(cost),
                |_| {},
                window,
                cx,
            )
        }),
        ("before-install-puzzle-run", |window, cx| {
            let releases = crate::whats_new::releases().into_iter().take(1).collect();
            crate::whats_new::open_before_update(
                "2026.11.10".into(),
                releases,
                Some("Your puzzle run ends"),
                |_| {},
                window,
                cx,
            )
        }),
        ("notes-off", |window, cx| {
            crate::settings::change(cx, |s| s.frontend.update_show_notes = false);
            let updates = cx.global::<crate::updates::UpdatesHandle>().0.clone();
            updates.update(cx, |updates, _| updates.version = Some("2026.11.10".into()));
            crate::pages::settings::updates::ask_restart(window, cx)
        }),
        ("settings", |window, cx| {
            crate::settings::change(cx, |s| s.frontend.update_show_notes = true);
            crate::shell::open_settings(crate::pages::settings::Section::Updates, window, cx)
        }),
    ];
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), 1100., 760.);
        cx.update(|cx| crate::settings::change(cx, move |s| s.frontend.appearance = appearance));
        for (state, open) in states {
            cx.update_window(window, |_, window, cx| {
                window.close_dialog(cx);
                open(window, cx)
            })
            .expect("the window");
            settle(&mut cx, window);
            cx.capture_screenshot(window)
                .expect("a screenshot")
                .save(out.join(format!("{state}-{name}.png")))
                .expect("the screenshot saves");
        }
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// Practice → Repertoire → Check against databases on a Sicilian repertoire, in both
/// appearances: `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::repertoire_check
/// -- --ignored` writes `<dir>/repertoire-check-<appearance>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn repertoire_check() {
    use kchess_contracts::calls;
    use kchess_contracts::check::{DatabaseName, PgnFilePath};
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).to_path_buf();
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    // Each line with its games' results, White's wins, draws and Black's wins.
    let lines: [(&str, [usize; 3]); 8] = [
        (
            "1. e4 c5 2. Nf3 d6 3. d4 cxd4 4. Nxd4 Nf6 5. Nc3 a6",
            [11, 10, 9],
        ),
        (
            "1. e4 c5 2. Nf3 d6 3. d4 cxd4 4. Nxd4 Nf6 5. Nc3 g6",
            [4, 2, 2],
        ),
        ("1. e4 c5 2. Nf3 Nc6", [8, 7, 5]),
        ("1. e4 c5 2. Nc3 Nc6", [9, 9, 7]),
        ("1. e4 c5 2. Nc3 e6", [1, 0, 0]),
        ("1. e4 c5 2. c3 Nf6", [8, 2, 2]),
        ("1. e4 c5 2. c3 d5", [8, 9, 8]),
        ("1. e4 e5", [15, 15, 10]),
    ];
    let mut pgn = String::new();
    for (moves, results) in lines {
        for (result, count) in ["1-0", "1/2-1/2", "0-1"].into_iter().zip(results) {
            pgn += &format!(
                "[WhiteElo \"2400\"]\n[BlackElo \"2300\"]\n[Result \"{result}\"]\n\n{moves} {result}\n\n"
            )
            .repeat(count);
        }
    }
    let files = tempfile::tempdir().expect("a folder for the file");
    let path = files.path().join("club.pgn");
    std::fs::write(&path, pgn).expect("the file");
    let repertoire = "1. e4 c5 2. Nf3 (2. Nc3 e6) (2. c3 Nf6) (2. d4 cxd4) d6 3. d4 cxd4 4. Nxd4 Nf6 5. Nc3 (5. Bd3 Nc6) a6 *";
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), 1100., 820.);
        cx.update(|cx| crate::settings::change(cx, move |s| s.frontend.appearance = appearance));
        let imported = cx.update(|cx| {
            cx.global::<crate::core::AppCore>()
                .request(calls::GameDbImport {
                    path: PgnFilePath(path.to_string_lossy().into_owned()),
                    into: None,
                    name: Some(DatabaseName("Club".into())),
                })
        });
        cx.foreground_executor()
            .block_test(imported)
            .expect("the games are imported");
        let saved = cx.update(|cx| {
            cx.global::<crate::studies::StudyHandle>()
                .0
                .read(cx)
                .command(kchess_contracts::types::StudyCommand::Save {
                    name: "Sicilian".into(),
                    pgn: repertoire.into(),
                    id: None,
                    chapter_id: None,
                })
        });
        let id = cx
            .foreground_executor()
            .block_test(saved)
            .expect("the study is saved")
            .id
            .expect("its id");
        cx.update_window(window, |_, window, cx| {
            crate::prefs::set(cx, "kchess:repertoire-color", "black");
            kchess_practice::repertoire::train_study(id, window, cx)
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.update_window(window, |_, window, cx| {
            let trainer = crate::shell::view::<crate::pages::practice::PracticePage>(cx)
                .and_then(|page| page.read(cx).repertoire())
                .expect("the repertoire trainer");
            trainer.update(cx, |trainer, cx| {
                trainer.check_against_databases(window, cx)
            })
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("repertoire-check-{name}.png")))
            .expect("the screenshot saves");
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}

/// Practice → Repertoire → Check → Against your games on a White repertoire, in both
/// appearances: `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui
/// e2e::screens::repertoire_your_games -- --ignored` writes
/// `<dir>/repertoire-your-games-<appearance>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn repertoire_your_games() {
    use kchess_contracts::calls;
    use kchess_contracts::check::ArchivedGame;
    use kchess_contracts::types::DatabaseSource;
    let out = kchess_host::env::var("KCHESS_SCREENSHOTS")
        .expect("KCHESS_SCREENSHOTS names the output folder");
    let out = Path::new(&out).to_path_buf();
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    let initial = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    // Your games with White: each line, its results and the day it was last played.
    let lines: [(&[&str], &[&str]); 6] = [
        (
            &["e2e4", "e7e5", "g1f3", "b8c6", "f1c4"],
            &["1-0", "0-1", "1-0", "1/2-1/2"],
        ),
        (
            &["e2e4", "c7c6", "d2d4", "d7d5"],
            &["0-1", "0-1", "1/2-1/2"],
        ),
        (&["e2e4", "e7e5", "g1f3", "d7d6", "d2d4"], &["1-0", "1-0"]),
        (&["e2e4", "c7c5", "b1c3", "b8c6"], &["1/2-1/2", "0-1"]),
        (&["e2e4", "e7e6", "d2d4", "d7d5", "e4e5"], &["1-0"]),
        (
            &["e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "a7a6"],
            &["1-0", "1-0", "0-1"],
        ),
    ];
    let repertoire =
        "1. e4 e5 (1... c5 2. Nf3 d6 3. d4) (1... e6 2. d4 d5 3. Nc3) 2. Nf3 Nc6 3. Bb5 *";
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), 1100., 820.);
        cx.update(|cx| crate::settings::change(cx, move |s| s.frontend.appearance = appearance));
        let mut count = 0;
        for (moves, results) in lines {
            for result in results {
                count += 1;
                let game = serde_json::json!({
                    "id": format!("game-{count}"), "source": "computer",
                    "startedAt": 1_761_091_200_000_i64 + count * 86_400_000,
                    "updatedAt": 1_761_091_260_000_i64 + count * 86_400_000,
                    "white": "You", "black": "Stockfish", "result": result,
                    "reason": "Checkmate", "timeControl": "-", "finished": true,
                    "setup": { "variant": "standard", "fen": initial }, "moves": moves,
                });
                let saved = cx.update(|cx| {
                    cx.global::<crate::core::AppCore>()
                        .request(calls::SaveArchivedGame {
                            game: ArchivedGame(game),
                        })
                });
                cx.foreground_executor()
                    .block_test(saved)
                    .expect("the game is saved");
            }
        }
        // Your games follows the games once they have stopped changing.
        let started = std::time::Instant::now();
        while !cx.update(|cx| {
            cx.global::<kchess_board::game_db::GameDbHandle>()
                .0
                .read(cx)
                .databases()
                .iter()
                .any(|database| database.source == DatabaseSource::Own)
        }) {
            assert!(started.elapsed() < Duration::from_secs(30), "Your games");
            settle(&mut cx, window);
        }
        let saved = cx.update(|cx| {
            cx.global::<crate::studies::StudyHandle>()
                .0
                .read(cx)
                .command(kchess_contracts::types::StudyCommand::Save {
                    name: "1. e4".into(),
                    pgn: repertoire.into(),
                    id: None,
                    chapter_id: None,
                })
        });
        let id = cx
            .foreground_executor()
            .block_test(saved)
            .expect("the study is saved")
            .id
            .expect("its id");
        cx.update_window(window, |_, window, cx| {
            crate::prefs::set(cx, "kchess:repertoire-color", "white");
            kchess_practice::repertoire::train_study(id, window, cx)
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.update_window(window, |_, window, cx| {
            let trainer = crate::shell::view::<crate::pages::practice::PracticePage>(cx)
                .and_then(|page| page.read(cx).repertoire())
                .expect("the repertoire trainer");
            trainer.update(cx, |trainer, cx| {
                trainer.check_against_your_games(window, cx)
            })
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("repertoire-your-games-{name}.png")))
            .expect("the screenshot saves");
    }
}

/// Practice → Repertoire → Check → Against Scouted_Player on a Black repertoire, their games
/// downloaded from the stand-in Lichess, in both appearances: `KCHESS_SCREENSHOTS=<dir> cargo
/// test -p kchess-gpui e2e::screens::repertoire_opponent -- --ignored` writes
/// `<dir>/repertoire-opponent-<appearance>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn repertoire_opponent() {
    use kchess_contracts::calls;
    use kchess_contracts::types::{GameSite, PlayerGames};
    let out = kchess_host::env::var("KCHESS_SCREENSHOTS")
        .expect("KCHESS_SCREENSHOTS names the output folder");
    let out = Path::new(&out).to_path_buf();
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    let repertoire = "1. e4 (1. d4 Nf6 2. c4 e6) c5 2. Nc3 (2. c3 Nf6) d6 *";
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), 1100., 820.);
        cx.update(|cx| crate::settings::change(cx, move |s| s.frontend.appearance = appearance));
        let installed = cx.update(|cx| {
            cx.global::<crate::core::AppCore>()
                .request(calls::GameDbPlayerInstall {
                    request: PlayerGames {
                        site: GameSite::Lichess,
                        username: super::fake_lichess::SCOUTED.into(),
                        months: None,
                        speeds: None,
                        rated: false,
                    },
                })
        });
        let database = cx
            .foreground_executor()
            .block_test(installed)
            .expect("their games are downloaded")
            .database
            .id;
        let saved = cx.update(|cx| {
            cx.global::<crate::studies::StudyHandle>()
                .0
                .read(cx)
                .command(kchess_contracts::types::StudyCommand::Save {
                    name: "Sicilian".into(),
                    pgn: repertoire.into(),
                    id: None,
                    chapter_id: None,
                })
        });
        let id = cx
            .foreground_executor()
            .block_test(saved)
            .expect("the study is saved")
            .id
            .expect("its id");
        cx.update_window(window, |_, window, cx| {
            crate::prefs::set(cx, "kchess:repertoire-color", "black");
            kchess_practice::repertoire::train_study(id, window, cx)
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.update_window(window, |_, window, cx| {
            let trainer = crate::shell::view::<crate::pages::practice::PracticePage>(cx)
                .and_then(|page| page.read(cx).repertoire())
                .expect("the repertoire trainer");
            trainer.update(cx, |trainer, cx| {
                trainer.check_against_player(
                    database,
                    super::fake_lichess::SCOUTED.into(),
                    window,
                    cx,
                )
            })
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("repertoire-opponent-{name}.png")))
            .expect("the screenshot saves");
    }
}

/// An event's table, a ten-player round robin with its crosstable, in both appearances:
/// `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::event_table -- --ignored`
/// writes `<dir>/event-table-<appearance>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn event_table() {
    use kchess_contracts::calls;
    use kchess_contracts::check::{DatabaseName, PgnFilePath};
    let out =
        kchess_host::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).to_path_buf();
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    const PLAYERS: [(&str, u32); 10] = [
        ("Gukesh D", 2794),
        ("Praggnanandhaa R", 2741),
        ("Abdusattorov, Nodirbek", 2768),
        ("Wei, Yi", 2743),
        ("Firouzja, Alireza", 2760),
        ("Giri, Anish", 2745),
        ("Erigaisi, Arjun", 2801),
        ("Keymer, Vincent", 2730),
        ("Van Foreest, Jorden", 2680),
        ("Mendonca, Leon Luke", 2650),
    ];
    // Each pair once, the result from their order and the round: a full table with a spread.
    let mut pgn = String::new();
    let mut round = 0;
    for (white, (w, we)) in PLAYERS.iter().enumerate() {
        for (black, (b, be)) in PLAYERS.iter().enumerate().skip(white + 1) {
            round += 1;
            let result = match (white * 7 + black * 3 + round) % 5 {
                0 | 1 => "1/2-1/2",
                2 | 3 => "1-0",
                _ => "0-1",
            };
            pgn += &format!(
                "[Event \"Tata Steel Masters\"]\n[Site \"Wijk aan Zee\"]\n\
                 [Date \"2025.01.{:02}\"]\n[Round \"{}\"]\n[White \"{w}\"]\n[Black \"{b}\"]\n\
                 [WhiteElo \"{we}\"]\n[BlackElo \"{be}\"]\n[Result \"{result}\"]\n\n1. e4 e5 {result}\n\n",
                18 + round % 13,
                1 + round % 13
            );
        }
    }
    let files = tempfile::tempdir().expect("a folder for the file");
    let path = files.path().join("tata.pgn");
    std::fs::write(&path, pgn).expect("the file");
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), 1100., 820.);
        cx.update(|cx| crate::settings::change(cx, move |s| s.frontend.appearance = appearance));
        let imported = cx.update(|cx| {
            cx.global::<crate::core::AppCore>()
                .request(calls::GameDbImport {
                    path: PgnFilePath(path.to_string_lossy().into_owned()),
                    into: None,
                    name: Some(DatabaseName("Tata Steel".into())),
                })
        });
        let database = cx
            .foreground_executor()
            .block_test(imported)
            .expect("the games are imported")
            .database
            .id;
        let page = cx.update(|cx| {
            cx.global::<crate::core::AppCore>()
                .request(calls::GameDbGames {
                    query: kchess_contracts::types::GameDbQuery {
                        database: database.clone(),
                        offset: 0,
                        limit: 1,
                        ..Default::default()
                    },
                })
        });
        let game = cx
            .foreground_executor()
            .block_test(page)
            .expect("the games")
            .games[0]
            .id;
        cx.update_window(window, |_, window, cx| {
            kchess_database::event_table(database, game, "Tata Steel Masters".into(), window, cx)
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("event-table-{name}.png")))
            .expect("the screenshot saves");
        let quit = cx.update(crate::quit::quit);
        cx.foreground_executor().block_test(quit);
        crate::profile::TEST_DATA_DIR.set(None);
    }
}
