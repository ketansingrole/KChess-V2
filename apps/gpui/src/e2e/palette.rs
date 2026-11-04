//! Quick Pick (⌘K, ⇧⌘P) and the interface zoom shortcuts.

use super::Harness;
use crate::pages::Page;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use serde_json::json;

fn page(app: &mut Harness) -> Option<Page> {
    app.cx.update(|cx| crate::shell::current_page(cx))
}

/// Whether a result with this name is listed.
fn lists(app: &mut Harness, name: &str) -> bool {
    app.with_window(|window, _| {
        (0..64usize)
            .map_while(|index| window.try_find(("palette-item", index)))
            .any(|item| item.label() == Some(name))
    })
}

/// Wait until a result with this name is listed.
fn wait_until_listed(app: &mut Harness, name: &str) {
    app.wait_for(&format!("{name:?} to be listed"), |window, _| {
        !window.find_all_by_label(name).is_empty()
    });
}

/// Wait until the best result is named `name`.
pub(super) fn wait_for_first(app: &mut Harness, name: &str) {
    app.wait_for(&format!("{name:?} as the first result"), |window, _| {
        window
            .try_find(("palette-item", 0usize))
            .is_some_and(|item| item.label() == Some(name))
    });
}

/// Wait until the palette is closed.
pub(super) fn wait_until_closed(app: &mut Harness) {
    app.wait_for("the palette to close", |window, _| {
        window.try_find(("palette-item", 0usize)).is_none()
    });
}

#[gpui_kit::test]
fn quick_pick_opens_with_its_modes_and_closes_on_escape(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    assert!(!lists(&mut app, "Run a command"));
    app.press("secondary-k");
    for mode in ["Run a command", "Find a player", "Change a setting"] {
        app.click_label(mode);
        // A mode row switches the mode and leaves the palette open.
        app.press("backspace");
    }
    app.press("escape");
    wait_until_closed(&mut app);
}

#[gpui_kit::test]
fn a_press_beside_the_open_box_closes_it(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text("puzzles");
    wait_for_first(&mut app, "Puzzles");
    // The title bar's empty side, left of the box (its icon and padding are 32px): nothing
    // there takes the focus.
    let field = app.with_window(|window, _| {
        window
            .try_find_by_label("Search pages and commands")
            .expect("the search field")
            .bounds()
    });
    app.click_point(gpui_kit::point(
        field.origin.x - gpui_kit::px(52.),
        field.center().y,
    ));
    wait_until_closed(&mut app);
    // It gave up the focus too: typing does not reopen it.
    app.type_text("p");
    assert!(!lists(&mut app, "Puzzles"));
}

#[gpui_kit::test]
fn a_page_is_found_by_name_and_opened(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text("puzzles");
    wait_for_first(&mut app, "Puzzles");
    app.press("enter");
    wait_until_closed(&mut app);
    assert_eq!(page(&mut app), Some(Page::Puzzles));
}

#[gpui_kit::test]
fn acronyms_and_several_words_find_pages_and_commands(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text("otb");
    wait_for_first(&mut app, "Over the board");
    app.press("enter");
    wait_until_closed(&mut app);
    assert_eq!(page(&mut app), Some(Page::Local));

    app.press("secondary-k");
    app.type_text("stockfish black");
    app.wait_for("a Stockfish game as the first result", |window, _| {
        window.try_find(("palette-item", 0usize)).and_then(|item| {
            item.label()
                .map(|label| label.starts_with("Play Stockfish"))
        }) == Some(true)
    });
}

#[gpui_kit::test]
fn renamed_settings_categories_are_found_by_their_old_names(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    for (old, name) in [
        ("app theme", "Themes"),
        ("my accounts", "Accounts"),
        ("analysis", "Cloud evaluation"),
    ] {
        app.press("secondary-k");
        app.type_text(old);
        wait_until_listed(&mut app, name);
        app.press("escape");
        wait_until_closed(&mut app);
    }
}

#[gpui_kit::test]
fn commands_are_run_from_the_command_mode(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.click_label("Run a command");
    wait_until_listed(&mut app, "New analysis board");
    app.type_text(" new analysis");
    wait_for_first(&mut app, "New analysis board");
    app.press("enter");
    wait_until_closed(&mut app);
    assert_eq!(page(&mut app), Some(Page::Analysis));

    // The shortcut opens straight into the commands; Backspace goes back to searching pages.
    app.press("shift-secondary-p");
    wait_until_listed(&mut app, "New analysis board");
    assert!(!lists(&mut app, "Home"));
    app.press("backspace");
    wait_until_listed(&mut app, "Run a command");
}

#[gpui_kit::test]
fn a_setting_is_changed_from_the_palette_and_saved(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    assert_eq!(app.saved_settings()["soundEnabled"], json!(true));
    app.press("secondary-k");
    app.type_text("#sound");
    wait_for_first(&mut app, "Sound");
    app.click(("palette-item", 0usize));
    wait_until_closed(&mut app);
    app.wait_for_saved("soundEnabled", json!(false));
}

#[gpui_kit::test]
fn the_commands_used_last_are_offered_first_and_kept(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    assert!(!lists(&mut app, "New analysis board"));
    app.press("escape");
    wait_until_closed(&mut app);

    app.press("shift-secondary-p");
    app.type_text(" new analysis");
    wait_for_first(&mut app, "New analysis board");
    app.press("enter");
    wait_until_closed(&mut app);

    app.press("secondary-k");
    wait_until_listed(&mut app, "New analysis board");
    app.press("escape");
    wait_until_closed(&mut app);

    // A restart keeps it.
    let mut app = app.restart();
    app.press("secondary-k");
    wait_until_listed(&mut app, "New analysis board");
}

#[gpui_kit::test]
fn the_interface_zoom_steps_with_the_keys_and_is_kept(cx: &mut gpui_kit::TestAppContext) {
    use crate::interface_zoom::{InterfaceZoom, Setting};
    fn setting(app: &mut Harness) -> Setting {
        app.cx.update(|cx| cx.global::<InterfaceZoom>().setting)
    }
    fn saved(app: &Harness) -> serde_json::Value {
        let file = app.profile.as_ref().expect("the profile").path();
        let text = std::fs::read_to_string(file.join("interface-zoom.json")).unwrap_or_default();
        serde_json::from_str(&text).unwrap_or_default()
    }
    fn wait_for_setting(app: &mut Harness, wanted: Setting) {
        let mut shown = setting(app);
        app.wait_for(&format!("the zoom to be {wanted:?}"), |_, cx| {
            shown = cx.global::<InterfaceZoom>().setting;
            shown == wanted
        });
    }

    let mut app = Harness::start(cx);
    assert_eq!(setting(&mut app), Setting::Auto);
    let start = app.with_window(|window, cx| crate::interface_zoom::percent(window, cx));

    app.press("secondary-=");
    let larger = crate::interface_zoom::step(start, 1);
    wait_for_setting(&mut app, Setting::Percent(larger));
    assert_eq!(saved(&app)["zoom"], json!(larger));

    app.press("secondary--");
    app.press("secondary--");
    let smaller = crate::interface_zoom::step(start, -1);
    wait_for_setting(&mut app, Setting::Percent(smaller));
    assert_eq!(saved(&app)["zoom"], json!(smaller));

    // The choice is there again after a restart, and ⌘0 goes back to Auto.
    let mut app = app.restart();
    assert_eq!(setting(&mut app), Setting::Percent(smaller));
    app.press("secondary-0");
    wait_for_setting(&mut app, Setting::Auto);
    assert_eq!(saved(&app)["zoom"], json!("auto"));
}

/// A FEN typed or pasted into Quick Pick is analysed or edited from there; a Lichess game link
/// or numbered moves open that game, and a profile link that player.
#[gpui_kit::test]
fn a_pasted_position_or_game_link_opens_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.online();
    app.press("secondary-k");
    app.type_text("4k3/8/8/8/8/8/4P3/4K3 w - - 0 1");
    wait_for_first(&mut app, "Analyse this position");
    assert!(lists(&mut app, "Edit this position"));
    app.press("enter");
    wait_until_closed(&mut app);
    assert_eq!(page(&mut app), Some(Page::Analysis));
    // Only this position's king reaches d2.
    app.click_label("Enter a chess move in SAN or UCI");
    app.type_text("Kd2");
    app.press("enter");
    app.find("tree-e1d2");

    app.press("secondary-k");
    app.type_text(&format!(
        "lichess.org/{}#2",
        super::fake_lichess::LINKED_GAME
    ));
    wait_for_first(&mut app, "Open this game on the analysis board");
    app.press("enter");
    wait_until_closed(&mut app);
    // The board held unsaved moves, so it asks first.
    app.click("ok");
    app.find("tree-e2e4 e7e5 g1f3 b8c6 f1b5 a7a6");
    assert_eq!(app.find("tree-e2e4 e7e5").selected(), Some(true));

    // A profile link opens that player.
    app.press("secondary-k");
    app.type_text("https://lichess.org/@/SomeIm/perf/blitz");
    wait_for_first(&mut app, "Open SomeIm’s profile");
    app.press("enter");
    wait_until_closed(&mut app);
    assert_eq!(page(&mut app), Some(Page::Players));

    // Numbered moves open as a game, at its last move.
    app.press("secondary-k");
    app.type_text("1. d4 d5 2. c4");
    wait_for_first(&mut app, "Analyse these moves");
    app.press("enter");
    wait_until_closed(&mut app);
    app.click("ok");
    assert_eq!(app.find("tree-d2d4 d7d5 c2c4").selected(), Some(true));
}

/// A Lichess broadcast link opens on Watch → Broadcasts: a round's link at that round, though
/// the tournament would open at its live one.
#[gpui_kit::test]
fn a_pasted_broadcast_link_opens_its_round(cx: &mut gpui_kit::TestAppContext) {
    use super::fake_lichess::{BROADCAST_ROUNDS, BROADCAST_TOUR};
    let mut app = Harness::start(cx);
    app.online();
    let wait_for_round = |app: &mut Harness, wanted: &str| {
        let wanted = wanted.to_string();
        app.wait_for(&format!("round {wanted}"), move |_, cx| {
            crate::shell::view_of::<crate::pages::watch::WatchPage>(Page::Watch, cx)
                .is_some_and(|watch| watch.read(cx).broadcast_round() == wanted)
        });
    };

    app.press("secondary-k");
    app.type_text(&format!(
        "https://lichess.org/broadcast/kchess-open/round-1/{}",
        BROADCAST_ROUNDS[0]
    ));
    wait_for_first(&mut app, "Watch this broadcast");
    app.press("enter");
    wait_until_closed(&mut app);
    assert_eq!(page(&mut app), Some(Page::Watch));
    wait_for_round(&mut app, BROADCAST_ROUNDS[0]);
    assert_eq!(app.find(("watch-tabs", 2usize)).selected(), Some(true));

    // The tournament's own link opens it at its live round.
    app.press("secondary-k");
    app.type_text(&format!(
        "lichess.org/broadcast/kchess-open/{BROADCAST_TOUR}"
    ));
    wait_for_first(&mut app, "Watch this broadcast");
    app.press("enter");
    wait_until_closed(&mut app);
    wait_for_round(&mut app, BROADCAST_ROUNDS[1]);
}

/// A Lichess tournament link opens the event on the Tournaments page, signed in or not.
#[gpui_kit::test]
fn a_pasted_tournament_link_opens_the_event(cx: &mut gpui_kit::TestAppContext) {
    use super::fake_lichess::ARENA;
    let mut app = Harness::start(cx);
    app.online();
    app.press("secondary-k");
    app.type_text(&format!("https://lichess.org/tournament/{ARENA}"));
    wait_for_first(&mut app, "Open this tournament");
    app.press("enter");
    wait_until_closed(&mut app);
    assert_eq!(page(&mut app), Some(Page::Tournaments));
    app.find("tournament-join");
    app.click("tournament-back");
    app.wait_gone("tournament-join");
}

/// A Lichess TV link opens that channel on Watch → TV, from whichever tab was showing.
#[gpui_kit::test]
fn a_pasted_tv_link_watches_its_channel(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.online();
    app.with_window(|window, cx| crate::shell::open(Page::Watch, window, cx));
    app.click(("watch-tabs", 2usize));
    app.press("secondary-k");
    app.type_text("https://lichess.org/tv/racingKings");
    wait_for_first(&mut app, "Watch this TV channel");
    app.press("enter");
    wait_until_closed(&mut app);
    assert_eq!(page(&mut app), Some(Page::Watch));
    assert_eq!(app.find(("watch-tabs", 0usize)).selected(), Some(true));
    app.wait_for("the channel to be kept", |_, cx| {
        crate::prefs::get::<String>(cx, crate::pages::watch::CHANNEL_KEY, String::new())
            == "racingKings"
    });
}

/// On the analysis board, Quick Pick copies the position and flips the board, as its menu and F do.
#[gpui_kit::test]
fn the_analysis_boards_actions_are_in_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Analysis board");
    app.find("chess-board");
    app.click_square("e2", true);
    app.click_square("e4", true);
    let run = |app: &mut Harness, label: &str| {
        app.press("secondary-k");
        app.type_text(label);
        wait_for_first(app, label);
        app.press("enter");
        wait_until_closed(app);
    };
    run(&mut app, "Copy FEN");
    assert_eq!(
        app.cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1")
    );
    run(&mut app, "Flip board");
    // Black's pawn is now at the board's bottom.
    app.click_square("e7", false);
    app.click_square("e5", false);
    assert_eq!(app.find("tree-e2e4 e7e5").selected(), Some(true));
}

/// On Play with computer, Quick Pick takes back a move only when the Take back button would.
#[gpui_kit::test]
fn a_computer_game_takes_back_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Play with computer");
    app.wait_for("the game to take moves", |window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window
            .try_find_by_label("Enter a chess move in SAN or UCI")
            .is_some_and(|entry| entry.disabled() != Some(true))
    });
    // Before a move there is nothing to take back: the page's actions lead the list without it.
    let shown = |app: &mut Harness| {
        app.press("secondary-k");
        app.type_text(">");
        wait_for_first(app, "Zen mode");
        let labels: Vec<String> = app.with_window(|window, _| {
            (0..6usize)
                .filter_map(|index| window.try_find(("palette-item", index)))
                .filter_map(|item| item.label().map(str::to_string))
                .collect()
        });
        app.press("escape");
        wait_until_closed(app);
        labels
    };
    let before = shown(&mut app);
    assert!(before.contains(&"New game".to_string()), "{before:?}");
    assert!(!before.contains(&"Take back".to_string()), "{before:?}");

    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4"]);
    app.press("secondary-k");
    app.type_text(">Take back");
    wait_for_first(&mut app, "Take back");
    app.press("enter");
    wait_until_closed(&mut app);
    app.wait_for_moves(&[]);
    assert!(!app.enabled("takeback"));
}

/// Over the board, Quick Pick offers the panel's actions as they apply, and runs them.
#[gpui_kit::test]
fn a_game_over_the_board_is_drawn_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Over the board");
    app.find("chess-board");
    let shown = |app: &mut Harness| {
        app.press("secondary-k");
        app.type_text(">");
        wait_for_first(app, "Zen mode");
        let labels: Vec<String> = app.with_window(|window, _| {
            (0..10usize)
                .filter_map(|index| window.try_find(("palette-item", index)))
                .filter_map(|item| item.label().map(str::to_string))
                .collect()
        });
        app.press("escape");
        wait_until_closed(app);
        labels
    };
    let before = shown(&mut app);
    assert!(before.contains(&"Flip board".to_string()), "{before:?}");
    assert!(!before.contains(&"Draw".to_string()), "{before:?}");
    assert!(!before.contains(&"Take back".to_string()), "{before:?}");

    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4"]);
    let during = shown(&mut app);
    for label in ["Take back", "Draw", "Resign", "Copy PGN"] {
        assert!(during.contains(&label.to_string()), "{label}: {during:?}");
    }
    app.press("secondary-k");
    app.type_text(">Draw");
    wait_for_first(&mut app, "Draw");
    app.press("enter");
    wait_until_closed(&mut app);
    app.wait_gone("local-draw");
    let after = shown(&mut app);
    assert!(!after.contains(&"Draw".to_string()), "{after:?}");
}

/// While a TV game is watched, Quick Pick offers its panel's actions, and Stop ends it.
#[gpui_kit::test]
fn a_watched_game_is_stopped_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::watch::WatchPage;
    let mut app = Harness::start(cx);
    app.with_window(|window, cx| crate::shell::open(Page::Watch, window, cx));
    let listed = |app: &mut Harness| -> Vec<String> {
        app.press("secondary-k");
        app.type_text(">");
        app.wait_for("the commands", |window, _| {
            window.try_find(("palette-item", 0usize)).is_some()
        });
        let labels = app.with_window(|window, _| {
            (0..6usize)
                .filter_map(|index| window.try_find(("palette-item", index)))
                .filter_map(|item| item.label().map(str::to_string))
                .collect()
        });
        app.press("escape");
        wait_until_closed(app);
        labels
    };
    assert!(!listed(&mut app).contains(&"Stop watching".to_string()));
    app.with_window(|_, cx| {
        let page = crate::shell::view_of::<WatchPage>(Page::Watch, cx).expect("Watch");
        page.update(cx, |page, cx| {
            page.show_frame(
                serde_json::json!({
                    "source": "tv", "channel": "atomic", "variant": "atomic", "gameId": "tv1",
                    "fen": "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
                    "white": { "name": "A" }, "black": { "name": "B" },
                }),
                cx,
            )
        });
    });
    // Atomic cannot go to the analysis board, so Analyse is left out.
    let watching = listed(&mut app);
    assert!(watching.contains(&"Flip board".to_string()), "{watching:?}");
    assert!(!watching.contains(&"Analyse".to_string()), "{watching:?}");
    app.press("secondary-k");
    app.type_text(">Stop watching");
    wait_for_first(&mut app, "Stop watching");
    app.press("enter");
    wait_until_closed(&mut app);
    assert!(!listed(&mut app).contains(&"Stop watching".to_string()));
}
