//! The Lichess pages with no account while Lichess cannot be reached: each shows what it can
//! without one, and asks to connect only when an action needs an account.

use super::Harness;
use crate::pages::Page;

/// Open a page, with the computer taken to be on a network or off it (what the machine running
/// the test says would change the pages).
fn open(cx: &mut gpui_kit::TestAppContext, label: &str, online: bool) -> Harness {
    let mut app = Harness::start(cx);
    app.cx.update(|cx| {
        let network = cx.global::<crate::network::NetworkHandle>().0.clone();
        network.update(cx, |network, cx| {
            network.online = online;
            cx.notify();
        });
    });
    app.open(label);
    app
}

fn page(app: &mut Harness) -> Option<Page> {
    app.cx.update(|cx| crate::shell::current_page(cx))
}

/// Wait until the element's accessible label is `label`.
fn wait_for_label(app: &mut Harness, id: &'static str, label: &str) {
    use gpui_kit::test::TestWindowExt as _;
    app.wait_for(&format!("{id} to read {label:?}"), |window, _| {
        window
            .try_find(id)
            .is_some_and(|element| element.label() == Some(label))
    });
}

/// Wait until the pill or tab is the chosen one.
fn wait_until_selected(app: &mut Harness, id: (&'static str, usize)) {
    use gpui_kit::test::TestWindowExt as _;
    app.wait_for(&format!("{id:?} to be chosen"), |window, _| {
        window
            .try_find(id)
            .is_some_and(|pill| pill.selected() == Some(true))
    });
}

/// Wait until the control takes clicks again: nothing is left loading.
fn wait_until_enabled(app: &mut Harness, id: &'static str) {
    use gpui_kit::test::TestWindowExt as _;
    app.wait_for(&format!("{id} to be enabled"), |window, _| {
        window
            .try_find(id)
            .is_some_and(|button| button.disabled() != Some(true))
    });
}

#[gpui_kit::test]
fn play_online_asks_to_connect_and_offers_the_computer(cx: &mut gpui_kit::TestAppContext) {
    let mut app = open(cx, "Play on Lichess", true);
    assert!(app.enabled("online-connect"));
    app.click("online-computer");
    assert_eq!(page(&mut app), Some(Page::Computer));
}

#[gpui_kit::test]
fn a_tournament_list_that_cannot_load_can_be_refreshed(cx: &mut gpui_kit::TestAppContext) {
    let mut app = open(cx, "Tournaments", true);
    wait_until_enabled(&mut app, "tournaments-refresh");
    app.click("tournaments-refresh");
    wait_until_enabled(&mut app, "tournaments-refresh");

    // The list shows every tournament, or only those that can be played here.
    assert_eq!(
        app.find(("tournaments-filter", 1usize)).selected(),
        Some(true)
    );
    app.click(("tournaments-filter", 0usize));
    wait_until_selected(&mut app, ("tournaments-filter", 0usize));
    assert_eq!(
        app.find(("tournaments-filter", 1usize)).selected(),
        Some(false)
    );
}

#[gpui_kit::test]
fn creating_an_arena_asks_for_an_account_and_can_be_declined(cx: &mut gpui_kit::TestAppContext) {
    let mut app = open(cx, "Tournaments", true);
    app.click("tournaments-create");
    // The dialog asks to connect; declining keeps browsing.
    app.find("ok");
    app.click("cancel");
    app.wait_for("the dialog to close", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window.try_find("ok").is_none()
    });
    assert!(app.shows("tournaments-create"));
}

#[gpui_kit::test]
fn watch_shows_its_feed_failing_and_the_following_and_broadcast_tabs(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut app = open(cx, "Watch", true);
    assert_eq!(app.find(("watch-tabs", 0usize)).selected(), Some(true));
    // The TV feed cannot connect: it can be asked again.
    assert!(app.enabled("watch-retry"));
    assert!(app.enabled("watch-channels-refresh"));

    app.click(("watch-tabs", 1usize));
    assert_eq!(app.find(("watch-tabs", 1usize)).selected(), Some(true));
    assert!(app.enabled("watch-friends-refresh"));

    app.click(("watch-tabs", 2usize));
    assert_eq!(app.find(("watch-tabs", 2usize)).selected(), Some(true));
    assert!(app.enabled("broadcasts-retry"));

    app.click(("watch-tabs", 0usize));
    assert!(app.enabled("watch-channels-refresh"));
}

#[gpui_kit::test]
fn a_player_lookup_checks_the_name_and_survives_a_failed_request(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut app = open(cx, "Players", true);
    app.click_label("Lichess username");
    app.type_text("!");
    app.click("players-lookup");
    wait_for_label(&mut app, "players-error", "Enter a Lichess username");

    // A real name goes out, fails and leaves the lookup ready for another try.
    app.click_label("Lichess username");
    app.press("secondary-a");
    app.type_text("carol");
    app.click("players-lookup");
    // Lookup spins while the request is out (a refused connection takes a while on Windows),
    // so wait for the failure itself.
    use gpui_kit::test::TestWindowExt as _;
    app.wait_for("the failed lookup's message", |window, _| {
        window
            .try_find("players-error")
            .and_then(|error| error.label().map(str::to_string))
            .is_some_and(|error| !error.is_empty() && error != "Enter a Lichess username")
    });
    assert!(app.enabled("players-lookup"));
}

#[gpui_kit::test]
fn players_cannot_be_looked_up_offline(cx: &mut gpui_kit::TestAppContext) {
    let mut app = open(cx, "Players", false);
    assert!(!app.enabled("players-lookup"));
}

#[gpui_kit::test]
fn a_player_is_followed_without_an_account_and_stays_followed(cx: &mut gpui_kit::TestAppContext) {
    let mut app = open(cx, "Following", true);
    assert!(!app.enabled("friends-follow"));
    assert!(!app.shows("friends-sync"));
    app.click_label("Lichess username to follow");
    app.type_text("carol");
    assert!(app.enabled("friends-follow"));
    app.click("friends-follow");
    app.find("friends-sync");
    assert!(!app.enabled("friends-follow"), "the name was taken");

    let mut app = app.restart();
    app.open("Following");
    app.find("friends-sync");
}

#[gpui_kit::test]
fn insights_without_games_lead_to_following_or_analysis(cx: &mut gpui_kit::TestAppContext) {
    let mut app = open(cx, "Lichess insights", true);
    app.click("insights-analyse");
    assert_eq!(page(&mut app), Some(Page::Analysis));
    app.open("Lichess insights");
    app.click("insights-follow");
    assert_eq!(page(&mut app), Some(Page::Friends));
}
