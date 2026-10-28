//! Puzzles with no Lichess account, Lichess out of reach and no puzzle database downloaded.

use super::Harness;

fn puzzles(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    app.open("Puzzles");
    app
}

/// Open a tab of the page ("Daily" is 1).
fn tab(app: &mut Harness, index: usize) {
    app.click(("puzzle-tabs", index));
    assert_eq!(app.find(("puzzle-tabs", index)).selected(), Some(true));
}

/// Wait until a mode's pill is (not) the chosen one.
fn wait_for_mode(app: &mut Harness, index: usize, chosen: bool) {
    use gpui_kit::test::TestWindowExt as _;
    app.wait_for(&format!("mode {index} chosen: {chosen}"), |window, _| {
        window
            .try_find(("puzzle-mode", index))
            .is_some_and(|pill| pill.selected() == Some(chosen))
    });
}

#[gpui_kit::test]
fn training_offline_offers_a_retry_and_the_downloaded_puzzles(cx: &mut gpui_kit::TestAppContext) {
    let mut app = puzzles(cx);
    wait_for_mode(&mut app, 0, true);
    assert!(app.enabled("try-again"));
    app.click("try-again");
    app.find("try-again");

    // Without a database the downloaded puzzles ask for it.
    app.click("error-offline");
    wait_for_mode(&mut app, 1, true);
    assert!(app.enabled("db-install"));
    assert!(!app.shows("chess-board"));

    // Back to Lichess practice, which is still unreachable.
    app.click(("puzzle-mode", 0usize));
    app.find("try-again");
}

#[gpui_kit::test]
fn rated_puzzles_ask_for_an_account_and_offer_another_mode(cx: &mut gpui_kit::TestAppContext) {
    let mut app = puzzles(cx);
    // The headless text is wide: the last mode needs the room the sidebar takes to be clicked.
    app.click("sidebar-toggle");
    app.click(("puzzle-mode", 2usize));
    wait_for_mode(&mut app, 2, true);
    app.click("other-mode");
    wait_for_mode(&mut app, 2, false);
    assert!(!app.shows("other-mode"));
}

#[gpui_kit::test]
fn the_daily_puzzle_offline_can_fall_back_to_downloaded_puzzles(cx: &mut gpui_kit::TestAppContext) {
    let mut app = puzzles(cx);
    tab(&mut app, 1);
    assert!(app.enabled("daily-retry"));
    app.click("daily-downloaded");
    wait_for_mode(&mut app, 1, true);
    assert_eq!(app.find(("puzzle-tabs", 0usize)).selected(), Some(true));
    app.find("db-install");
}

#[gpui_kit::test]
fn storm_streak_and_rush_wait_for_the_downloaded_puzzles(cx: &mut gpui_kit::TestAppContext) {
    let mut app = puzzles(cx);
    tab(&mut app, 2);
    // A run needs the downloaded puzzles; a score is only saved to an account.
    app.find("db-install");
    assert!(!app.enabled("rush-start"));
    app.find("storm-connect");
}

#[gpui_kit::test]
fn lichess_stats_ask_for_an_account(cx: &mut gpui_kit::TestAppContext) {
    let mut app = puzzles(cx);
    tab(&mut app, 3);
    assert!(app.enabled("stats-connect"));
}

#[gpui_kit::test]
fn an_empty_history_leads_back_to_training(cx: &mut gpui_kit::TestAppContext) {
    let mut app = puzzles(cx);
    tab(&mut app, 4);
    app.click("puzzle-history-train");
    assert_eq!(app.find(("puzzle-tabs", 0usize)).selected(), Some(true));
    app.find(("puzzle-mode", 0usize));
}

#[gpui_kit::test]
fn the_chosen_tab_is_kept_when_the_app_restarts(cx: &mut gpui_kit::TestAppContext) {
    let mut app = puzzles(cx);
    tab(&mut app, 4);
    let mut app = app.restart();
    app.open("Puzzles");
    assert_eq!(app.find(("puzzle-tabs", 4usize)).selected(), Some(true));
    app.find("puzzle-history-train");
}
