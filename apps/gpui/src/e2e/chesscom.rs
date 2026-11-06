//! Chess.com players: added by name in Settings → Accounts, their games listed in History's
//! Online tab and counted in Insights, and removed with their games.

use super::Harness;
use crate::pages::Page;

/// The Online tab's games, once listed: the total and each game's opening.
fn listed(app: &mut Harness, what: &str, ready: impl Fn(usize) -> bool) {
    app.wait_for(what, |_, cx| {
        crate::shell::view_of::<crate::pages::history::HistoryPage>(Page::History, cx)
            .and_then(|page| page.read(cx).lichess_listed())
            .is_some_and(|(total, _)| ready(total))
    });
}

#[gpui_kit::test]
fn a_chesscom_player_is_synced_listed_counted_and_removed(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.online();
    app.open_settings("Accounts");
    assert!(!app.enabled("chesscom-add"), "nothing typed yet");
    app.click_label("Chess.com username");
    app.type_text("e2e_carol");
    app.click("chesscom-add");
    app.find("chesscom-sync-e2e_carol");

    // Both games are in History, under the player's Chess.com account.
    app.with_window(|window, cx| crate::shell::open(Page::History, window, cx));
    app.click(("history-source", 2usize));
    listed(&mut app, "the Chess.com games", |total| total == 2);
    app.click(("lichess-game", 0usize));
    app.find("lichess-all-games");
    app.click("lichess-all-games");

    // Insights count them for that account.
    app.open("Insights");
    app.wait_for("the Chess.com report", |_, cx| {
        crate::shell::view_of::<crate::pages::insights::InsightsPage>(Page::Insights, cx)
            .is_some_and(|page| {
                let page = page.read(cx);
                page.loaded()
                    && page.query(cx)["account"] == "e2e_carol@chess.com"
                    && page.report_total() == Some(2.)
            })
    });

    // Removing the player removes their games.
    app.open_settings("Accounts");
    app.click("chesscom-remove-e2e_carol");
    app.click("ok");
    app.wait_gone("chesscom-sync-e2e_carol");
    app.with_window(|window, cx| crate::shell::open(Page::History, window, cx));
    app.find("history-chesscom");
}

#[gpui_kit::test]
fn an_unknown_chesscom_player_is_not_added(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.online();
    app.open_settings("Accounts");
    app.click_label("Chess.com username");
    app.type_text("nobody_at_all");
    app.click("chesscom-add");
    app.wait_for("the add to finish", |_, cx| {
        !crate::accounts::handle(cx).read(cx).syncing()
    });
    assert!(!app.shows("chesscom-sync-nobody_at_all"));
    assert!(
        app.cx
            .update(|cx| crate::accounts::handle(cx).read(cx).chesscom().is_empty())
    );
}
