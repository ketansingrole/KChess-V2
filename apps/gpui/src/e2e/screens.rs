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

/// Following's timeline over the stand-in Lichess's players, wide and narrow, in both
/// appearances: `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::the_timeline --
/// --ignored` writes `<dir>/timeline/<appearance>-<width>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn the_timeline() {
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
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
            let players =
                serde_json::json!(["thibault", "EricRosen", "DrNykterstein", "penguingim1"]);
            let added = cx.update(|cx| {
                cx.global::<crate::core::AppCore>()
                    .call("addFriends", vec![players])
            });
            cx.foreground_executor()
                .block_test(added)
                .expect("the players are added");
            // Synced, and then two of them finish a game: theirs are new.
            let synced = cx.update(|cx| {
                cx.global::<crate::core::AppCore>()
                    .call("syncGames", vec![])
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
            let quit = cx.update(crate::core::quit);
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
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
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
            let quit = cx.update(crate::core::quit);
            cx.foreground_executor().block_test(quit);
            crate::profile::TEST_DATA_DIR.set(None);
        }
    }
}

/// A Crazyhouse game over the board, with pieces in both pockets and one picked to drop, in
/// both appearances: `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::crazyhouse
/// -- --ignored` writes `<dir>/crazyhouse/<appearance>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn crazyhouse() {
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
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
            let local = crate::shell::view_of::<crate::pages::local::LocalPage>(Page::Local, cx)
                .expect("the page");
            local.update(cx, |local, cx| {
                let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
                let setup = serde_json::json!({ "variant": "crazyhouse", "fen": fen });
                local.replace_game(setup, serde_json::Value::Null, window, cx);
                for uci in [
                    "e2e4", "d7d5", "e4d5", "d8d5", "b1c3", "d5a5", "f1b5", "c7c6", "b5c6",
                ] {
                    local.play(uci, window, cx);
                }
                // Black takes back with the knight: White holds two pawns, Black a pawn and a
                // bishop.
                local.play("b8c6", window, cx);
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
        let quit = cx.update(crate::core::quit);
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
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
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
                crate::shell::view_of::<crate::pages::computer::ComputerPage>(Page::Computer, cx)
                    .expect("the page");
            page.update(cx, |page, cx| {
                let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
                let setup = serde_json::json!({ "variant": "crazyhouse", "fen": fen });
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
        let quit = cx.update(crate::core::quit);
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
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("home");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    let only = std::env::var("KCHESS_HOME_STATES").unwrap_or_default();
    let rail = std::env::var_os("KCHESS_HOME_RAIL").is_some();
    let turn = std::env::var_os("KCHESS_HOME_TURN").is_some();
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
                            online.ongoing = vec![serde_json::json!({
                                "gameId": "corr1",
                                "speed": "correspondence",
                                "isMyTurn": true,
                            })];
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
                let quit = cx.update(crate::core::quit);
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
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
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
                        crate::shell::open_player("Rattiestword5", window, cx)
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
                let quit = cx.update(crate::core::quit);
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
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
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
                    if let Some(page) = crate::shell::view_of::<crate::pages::history::HistoryPage>(
                        Page::History,
                        cx,
                    ) {
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
                let quit = cx.update(crate::core::quit);
                cx.foreground_executor().block_test(quit);
                crate::profile::TEST_DATA_DIR.set(None);
            }
        }
    }
}

/// The window sizes Home is shot at: wide and narrow, or `KCHESS_HOME_SIZES` ("1920x1100,760x1400").
fn home_sizes() -> Vec<(f32, f32)> {
    let sizes: Vec<(f32, f32)> = std::env::var("KCHESS_HOME_SIZES")
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
    use crate::game::computer::{ComputerGame, Host};
    let host = Host {
        now: 0.,
        allowed: true,
        ready: false,
    };
    let session = serde_json::json!({
        "moves": ["e2e4", "e7e5", "g1f3", "b8c6", "f1c4"],
        "color": "white",
        "level": "club",
    });
    let snapshot = ComputerGame::new(Some(&session), &host)
        .expect("the computer game")
        .snapshot(&host);
    if unfinished {
        let saved = cx.update(|cx| {
            cx.global::<crate::core::AppCore>()
                .call("saveSession", vec![serde_json::json!("computer"), snapshot])
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
                    serde_json::json!({
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
                    }),
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
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
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
            let quit = cx.update(crate::core::quit);
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
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
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
            let quit = cx.update(crate::core::quit);
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
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
    let out = Path::new(&out).join("low-clock");
    std::fs::create_dir_all(&out).expect("the screenshot folder");
    super::environment();
    for (name, appearance) in [("dark", Appearance::Dark), ("light", Appearance::Light)] {
        let profile = tempfile::tempdir().expect("a temporary profile");
        let (mut cx, window) = start(profile.path(), 1320., 880.);
        cx.update(|cx| crate::settings::change(cx, move |s| s.frontend.appearance = appearance));
        cx.update_window(window, |_, window, cx| {
            crate::shell::open(Page::Local, window, cx);
            let local = crate::shell::view_of::<LocalPage>(Page::Local, cx).expect("the page");
            local.update(cx, |local, cx| {
                let setup =
                    serde_json::json!({ "variant": "standard", "fen": crate::game::STANDARD_FEN });
                let clock = serde_json::json!({
                    "white": { "minutes": 0.15, "increment": 0 },
                    "black": { "minutes": 0.15, "increment": 0 },
                });
                local.replace_game(setup, clock, window, cx);
                local.play("e2e4", window, cx);
            });
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("{name}.png")))
            .expect("the screenshot saves");
        let quit = cx.update(crate::core::quit);
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
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
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
            let board = crate::shell::analysis_board(window, cx).expect("the analysis board");
            board.update(cx, |board, cx| board.open_import(window, cx));
        })
        .expect("the window");
        settle(&mut cx, window);
        cx.capture_screenshot(window)
            .expect("a screenshot")
            .save(out.join(format!("{width}.png")))
            .expect("the screenshot saves");
        let quit = cx.update(crate::core::quit);
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
    for _ in 0..400 {
        settle(cx, window);
        if let Some(url) = logins().into_iter().nth(earlier) {
            super::fake_lichess::return_from_login(&url, &code);
            break;
        }
    }
    for _ in 0..400 {
        settle(cx, window);
        let done = cx.update(|cx| {
            let accounts = crate::accounts::handle(cx);
            let accounts = accounts.read(cx);
            accounts.active(cx).as_deref() == Some(username) && !accounts.busy
        });
        if done {
            break;
        }
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

/// The board's coordinates in each mode, on the pages with a board, at the smallest and the default
/// window size: `KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens::board_coordinates
/// -- --ignored` writes `<dir>/coordinates/<page>-<mode>-<width>.png`.
#[test]
#[ignore = "needs a GPU and KCHESS_SCREENSHOTS"]
fn board_coordinates() {
    use kchess_contracts::types::CoordinateMode;
    let out = std::env::var_os("KCHESS_SCREENSHOTS").expect("KCHESS_SCREENSHOTS names a folder");
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
        let quit = cx.update(crate::core::quit);
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
