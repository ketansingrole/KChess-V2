//! Game history: finished games listed, opened and kept.

use super::Harness;
use crate::pages::Page;

const SCHOLARS_MATE: [&str; 7] = ["e4", "e5", "Qh5", "Nc6", "Bc4", "Nf6", "Qxf7#"];

/// Play moves over the board by typing them, from wherever the app is.
fn play_over_the_board(app: &mut Harness, moves: &[&str]) {
    app.open("Over the board");
    for (played, text) in moves.iter().enumerate() {
        app.click_label("Enter a chess move in SAN or UCI");
        app.type_text(text);
        app.press("enter");
        app.wait_for_moves(&moves[..=played]);
    }
}

/// Game history with the games played over the board listed.
fn board_games(app: &mut Harness) {
    app.open("Game history");
    app.click(("history-source", 1usize));
}

/// A finished game of Scholar's mate, then history over the board.
fn mate_in_history(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    play_over_the_board(&mut app, &SCHOLARS_MATE);
    board_games(&mut app);
    app
}

#[gpui_kit::test]
fn an_empty_history_offers_a_game_to_play(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Game history");
    assert_eq!(app.find("history-play").label(), Some("Play the computer"));
    app.click("history-play");
    app.find("takeback");

    app.open("Game history");
    app.click(("history-source", 1usize));
    assert_eq!(
        app.find("history-play").label(),
        Some("Play over the board")
    );
    app.click("history-play");
    app.find("local-new");
}

#[gpui_kit::test]
fn a_finished_board_game_is_listed_and_opens_to_its_moves(cx: &mut gpui_kit::TestAppContext) {
    let mut app = mate_in_history(cx);
    assert_eq!(
        app.find(("history-row", 0usize)).label(),
        Some("White vs Black")
    );
    assert!(!app.shows(("history-row", 1usize)));
    app.click(("history-row", 0usize));
    app.wait_for_moves(&SCHOLARS_MATE);
    // The game opens at its last move.
    assert_eq!(app.find(("move", 6usize)).selected(), Some(true));
    assert!(!app.enabled("chevron-right"));
}

#[gpui_kit::test]
fn an_open_game_is_stepped_through_and_closed_to_the_list(cx: &mut gpui_kit::TestAppContext) {
    let mut app = mate_in_history(cx);
    app.click(("history-row", 0usize));
    app.click("skip-back");
    assert!(!app.enabled("skip-back"));
    app.click("chevron-right");
    assert_eq!(app.find(("move", 0usize)).selected(), Some(true));
    app.click("skip-forward");
    assert_eq!(app.find(("move", 6usize)).selected(), Some(true));

    app.click("all-games");
    app.find(("history-row", 0usize));
    assert!(!app.shows("all-games"));
}

/// Escape closes an open game to the list, as All games does.
#[gpui_kit::test]
fn escape_closes_an_open_game(cx: &mut gpui_kit::TestAppContext) {
    let mut app = mate_in_history(cx);
    app.click(("history-row", 0usize));
    app.wait_for_moves(&SCHOLARS_MATE);
    app.press("escape");
    app.wait_gone("all-games");
    app.find(("history-row", 0usize));
}

#[gpui_kit::test]
fn finished_games_are_kept_when_the_app_restarts(cx: &mut gpui_kit::TestAppContext) {
    let app = mate_in_history(cx);
    let mut app = app.restart();
    board_games(&mut app);
    app.click(("history-row", 0usize));
    app.wait_for_moves(&SCHOLARS_MATE);
}

#[gpui_kit::test]
fn a_saved_game_opens_on_the_analysis_board(cx: &mut gpui_kit::TestAppContext) {
    let mut app = mate_in_history(cx);
    app.click(("history-row", 0usize));
    app.click("open-analysis");
    // The whole game is in the tree.
    app.find("tree-e2e4");
    app.find("tree-e2e4 e7e5 d1h5 b8c6 f1c4 g8f6 h5f7");
}

/// An open game's buttons are in Quick Pick: it opens on the analysis board from there.
#[gpui_kit::test]
fn an_open_game_is_analysed_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = mate_in_history(cx);
    app.click(("history-row", 0usize));
    app.wait_for_moves(&SCHOLARS_MATE);
    app.press("secondary-k");
    app.type_text(">");
    super::palette::wait_for_first(&mut app, "All games");
    let listed: Vec<String> = app.with_window(|window, _| {
        (0..6usize)
            .filter_map(|index| window.try_find(("palette-item", index)))
            .filter_map(|item| item.label().map(str::to_string))
            .collect()
    });
    assert_eq!(&listed[..3], ["All games", "Export PGN", "Open analysis"]);
    app.type_text("Open analysis");
    super::palette::wait_for_first(&mut app, "Open analysis");
    app.press("enter");
    super::palette::wait_until_closed(&mut app);
    app.find("tree-e2e4 e7e5 d1h5 b8c6 f1c4 g8f6 h5f7");
}

/// Home's _Review_ is in Quick Pick: the last game opens in Game history. Home adds no Sync games
/// of its own there.
#[gpui_kit::test]
fn the_last_game_is_reviewed_from_home_in_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = mate_in_history(cx);
    app.open("Home");
    app.find("home-review");
    app.press("secondary-k");
    app.type_text(">");
    super::palette::wait_for_first(&mut app, "Review last game");
    let listed: Vec<String> = app.with_window(|window, _| {
        (0..8usize)
            .filter_map(|index| window.try_find(("palette-item", index)))
            .filter_map(|item| item.label().map(str::to_string))
            .collect()
    });
    assert!(!listed.iter().any(|label| label == "Sync games"));
    app.press("enter");
    super::palette::wait_until_closed(&mut app);
    app.wait_for_moves(&SCHOLARS_MATE);
    app.find("open-analysis");
}

/// Game history's Lichess tab is in Quick Pick from any page.
#[gpui_kit::test]
fn lichess_games_open_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text("Lichess games");
    super::palette::wait_for_first(&mut app, "Lichess games");
    app.press("enter");
    super::palette::wait_until_closed(&mut app);
    assert_eq!(app.find(("history-source", 2usize)).selected(), Some(true));
}

#[gpui_kit::test]
fn a_resigned_computer_game_is_listed_with_the_computer_games(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Play with computer");
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4"]);
    app.click("resign");
    app.click("ok");
    app.find("new-game");

    app.open("Game history");
    app.find(("history-row", 0usize));
    assert!(!app.shows(("history-row", 1usize)));
    app.click(("history-row", 0usize));
    app.wait_for_moves(&["e4"]);

    // Over the board has none of it.
    app.click("all-games");
    app.click(("history-source", 1usize));
    app.find("history-play");
    assert!(!app.shows(("history-row", 0usize)));
}

#[gpui_kit::test]
fn the_list_is_narrowed_by_result(cx: &mut gpui_kit::TestAppContext) {
    let mut app = mate_in_history(cx);
    // A second game, left unfinished after one move.
    app.open("Over the board");
    app.click("local-new");
    app.click("start-local");
    app.wait_for_moves(&[]);
    play_over_the_board(&mut app, &["e4"]);
    app.open("Game history");
    app.find(("history-row", 1usize));

    app.click(("history-filter", 1usize));
    assert_eq!(app.find(("history-filter", 1usize)).selected(), Some(true));
    app.find(("history-row", 0usize));
    assert!(!app.shows(("history-row", 1usize)));
    app.click(("history-row", 0usize));
    app.wait_for_moves(&SCHOLARS_MATE);
    app.click("all-games");

    app.click(("history-filter", 4usize));
    app.find(("history-row", 0usize));
    assert!(!app.shows(("history-row", 1usize)));
    app.click(("history-row", 0usize));
    app.wait_for_moves(&["e4"]);
    app.click("all-games");

    // No game was won by Black, so that filter leads nowhere.
    app.click(("history-filter", 2usize));
    assert_eq!(app.find(("history-filter", 2usize)).selected(), Some(false));
    assert_eq!(app.find(("history-filter", 4usize)).selected(), Some(true));
    app.click(("history-filter", 0usize));
    app.find(("history-row", 1usize));
}

#[gpui_kit::test]
fn deleting_a_game_asks_first_and_is_kept_after_a_restart(cx: &mut gpui_kit::TestAppContext) {
    let mut app = mate_in_history(cx);
    let actions = "Actions for the game from ";
    let label = app.with_window(|window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window
            .elements()
            .iter()
            .filter_map(|element| element.label())
            .find(|label| label.starts_with(actions))
            .map(str::to_string)
    });
    let label = label.expect("the game's actions");

    // Deleting asks nothing: its notice's Undo brings the game back, and it stays.
    app.click_label(&label);
    app.click_label("Delete");
    app.find("history-play");
    app.click("undo-delete-game");
    app.find(("history-row", 0usize));
    let mut app = app.restart();
    board_games(&mut app);
    app.find(("history-row", 0usize));

    app.click_label(&label);
    app.click_label("Delete");
    app.find("history-play");
    assert!(!app.shows(("history-row", 0usize)));

    let mut app = app.restart();
    board_games(&mut app);
    app.find("history-play");
}

/// A Crazyhouse game in the history shows what each side held in its pocket at the move looked
/// at, and opens on the analysis board in Crazyhouse.
#[gpui_kit::test]
fn a_crazyhouse_game_shows_its_pockets(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::Page;
    use crate::pages::local::LocalPage;
    let mut app = Harness::start(cx);
    app.open("Over the board");
    app.with_window(|window, cx| {
        let local = crate::shell::view_of::<LocalPage>(Page::Local, cx).expect("the page");
        local.update(cx, |local, cx| {
            let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
            let setup = serde_json::json!({ "variant": "crazyhouse", "fen": fen });
            local.replace_game(setup, serde_json::Value::Null, window, cx);
        });
    });
    let moves = ["e4", "d5", "exd5", "Qxd5"];
    for (played, text) in moves.iter().enumerate() {
        app.click_label("Enter a chess move in SAN or UCI");
        app.type_text(text);
        app.press("enter");
        app.wait_for_moves(&moves[..=played]);
    }
    app.click("local-resign");
    app.click("ok");
    board_games(&mut app);
    app.click(("history-row", 0usize));
    app.wait_for_moves(&moves);
    let held = |app: &mut Harness, id: &'static str| app.find(id).label().map(str::to_string);
    assert!(held(&mut app, "pocket-White-Pawn").is_some_and(|label| label.ends_with(": 1")));
    assert!(held(&mut app, "pocket-Black-Pawn").is_some_and(|label| label.ends_with(": 1")));
    // Before Black took back, only White held a pawn.
    app.click(("move", 2usize));
    assert!(held(&mut app, "pocket-Black-Pawn").is_some_and(|label| label.ends_with(": 0")));
    // The analysis board takes the game in Crazyhouse.
    app.click("open-analysis");
    app.find("tree-e2e4 d7d5 e4d5 d8d5");
}

/// Insights offer a Variant choice only once the account has variant games, and choosing one asks
/// for that variant's games alone.
#[gpui_kit::test]
fn insights_choose_a_variant_only_when_one_was_played(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::{Page, insights::InsightsPage};
    use gpui_kit::test::TestQueryExt as _;
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    app.open("Lichess insights");
    let page = |app: &mut Harness| {
        app.cx
            .update(|cx| crate::shell::view_of::<InsightsPage>(Page::Insights, cx))
            .expect("the Insights page")
    };
    app.wait_for("the first report", |_, cx| page_loaded(cx));
    assert!(app.with_window(|window, _| window.try_find_by_label("Variant").is_none()));

    // The account turns out to have Chess960 games.
    let insights = page(&mut app);
    app.cx
        .update(|cx| insights.update(cx, |page, cx| page.show_report_variants(&["chess960"], cx)));
    app.wait_for("the Variant choice", |window, _| {
        window.try_find_by_label("Variant").is_some()
    });
    assert_eq!(
        app.cx.update(|cx| insights.read(cx).query(cx))["variant"],
        serde_json::Value::Null,
        "standard chess until a variant is chosen"
    );
    app.with_window(|window, cx| {
        insights.update(cx, |page, cx| page.choose_variant("chess960", window, cx))
    });
    assert_eq!(
        app.cx.update(|cx| insights.read(cx).query(cx))["variant"],
        "chess960"
    );

    fn page_loaded(cx: &gpui_kit::App) -> bool {
        crate::shell::view_of::<InsightsPage>(Page::Insights, cx)
            .is_some_and(|page| page.read(cx).loaded())
    }
}

/// The Lichess tab's games, once listed: the total and each game's opening.
fn lichess_listed(app: &mut Harness, what: &str, ready: impl Fn(usize, &[String]) -> bool) {
    app.wait_for(what, |_, cx| {
        crate::shell::view_of::<crate::pages::history::HistoryPage>(Page::History, cx)
            .and_then(|page| page.read(cx).lichess_listed())
            .is_some_and(|(total, openings)| ready(total, &openings))
    });
}

/// Hundreds of synced games are found by opponent or opening: the search narrows the list and
/// its total, and Clear filters empties it.
#[gpui_kit::test]
fn lichess_games_are_searched_by_opponent_or_opening(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in_showcase(cx);
    app.with_window(|window, cx| crate::shell::open(Page::History, window, cx));
    app.click(("history-source", 2usize));
    lichess_listed(&mut app, "the synced games", |total, _| total == 40);

    app.click_label("Search by opponent or opening");
    app.type_text("najdorf");
    lichess_listed(&mut app, "the Najdorf games", |total, openings| {
        total == 10
            && !openings.is_empty()
            && openings.iter().all(|opening| opening.contains("Najdorf"))
    });

    app.click("lichess-clear");
    lichess_listed(&mut app, "every game again", |total, _| total == 40);
    // A name is found in any case.
    app.click_label("Search by opponent or opening");
    app.type_text("TACTICALTOM");
    lichess_listed(&mut app, "TacticalTom's games", |total, _| {
        total > 0 && total < 40
    });
}

/// An open game steps to the games above and below it in the list without going back to it,
/// within the list's filter.
#[gpui_kit::test]
fn an_open_game_steps_to_the_next_and_previous(cx: &mut gpui_kit::TestAppContext) {
    let mut app = mate_in_history(cx);
    // A newer game, left unfinished after one move: it is listed first.
    app.open("Over the board");
    app.click("local-new");
    app.click("start-local");
    app.wait_for_moves(&[]);
    play_over_the_board(&mut app, &["e4"]);
    app.open("Game history");
    app.find(("history-row", 1usize));

    app.click(("history-row", 0usize));
    app.wait_for_moves(&["e4"]);
    assert!(!app.enabled("previous-game"));
    app.click("next-game");
    app.wait_for_moves(&SCHOLARS_MATE);
    assert!(!app.enabled("next-game"));
    app.click("previous-game");
    app.wait_for_moves(&["e4"]);

    // Under White won, the unfinished game is not listed: Scholar's mate has no neighbours.
    app.click("all-games");
    app.click(("history-filter", 1usize));
    app.click(("history-row", 0usize));
    app.wait_for_moves(&SCHOLARS_MATE);
    assert!(!app.enabled("previous-game"));
    assert!(!app.enabled("next-game"));
}

/// A synced Lichess game steps to its neighbours on the list's page too.
#[gpui_kit::test]
fn a_lichess_game_steps_to_the_next_and_previous(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in_showcase(cx);
    app.with_window(|window, cx| crate::shell::open(Page::History, window, cx));
    app.click(("history-source", 2usize));
    lichess_listed(&mut app, "the synced games", |total, _| total == 40);
    let open = |app: &mut Harness| {
        app.cx.update(|cx| {
            crate::shell::view_of::<crate::pages::history::HistoryPage>(Page::History, cx)
                .map(|page| page.read(cx).lichess_open())
                .unwrap_or_default()
        })
    };
    let wait_open = |app: &mut Harness, row: usize| {
        app.wait_for("the game to open", |_, cx| {
            crate::shell::view_of::<crate::pages::history::HistoryPage>(Page::History, cx)
                .map(|page| page.read(cx).lichess_open())
                .is_some_and(|(open, ids)| open.as_ref() == ids.get(row))
        });
    };

    app.with_window(|window, cx| {
        let page = crate::shell::view_of::<crate::pages::history::HistoryPage>(Page::History, cx)
            .expect("the history page");
        page.update(cx, |page, cx| page.open_first_lichess(window, cx));
    });
    wait_open(&mut app, 0);
    assert!(!app.enabled("lichess-previous-game"));
    app.click("lichess-next-game");
    wait_open(&mut app, 1);
    app.click("lichess-next-game");
    wait_open(&mut app, 2);
    app.click("lichess-previous-game");
    wait_open(&mut app, 1);
    assert!(open(&mut app).0.is_some());
}
