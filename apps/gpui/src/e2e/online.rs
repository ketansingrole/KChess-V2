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
    wait_until_enabled(&mut app, "tournaments-try-again");
    // The failure takes the list's place, without the key to a schedule that is not there.
    app.find("tournaments-error");
    assert!(!app.shows("tournaments-legend"));
    // It offers to try again in its place, as the other pages' failed loads do, and only there:
    // Refresh beside it would be a second button for the same thing.
    assert!(!app.shows("tournaments-refresh"));
    app.click("tournaments-try-again");
    wait_until_enabled(&mut app, "tournaments-try-again");
    assert!(!app.shows("tournaments-refresh"));

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
    // Neither the channels nor the feed load: the page says so once, and can be asked again.
    assert!(app.enabled("watch-try-again"));
    assert!(!app.shows("watch-retry"));
    assert!(!app.shows("watch-channels-refresh"));
    app.click("watch-try-again");
    app.find("watch-try-again");

    app.click(("watch-tabs", 1usize));
    assert_eq!(app.find(("watch-tabs", 1usize)).selected(), Some(true));
    assert!(app.enabled("watch-friends-refresh"));
    // Nobody is followed: the header says where to follow players, and nothing more.
    assert!(!app.shows("watch-friends-none"));

    app.click(("watch-tabs", 2usize));
    assert_eq!(app.find(("watch-tabs", 2usize)).selected(), Some(true));
    assert!(app.enabled("broadcasts-retry"));
    // TV's failure stays on TV: Broadcasts says only its own.
    assert!(!app.shows("watch-error"));

    app.click(("watch-tabs", 0usize));
    assert!(app.enabled("watch-try-again"));
}

#[gpui_kit::test]
fn a_player_lookup_checks_the_name_and_survives_a_failed_request(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut app = open(cx, "Players", true);
    app.click_label("Lichess username");
    app.type_text("!");
    app.click("players-lookup");
    wait_for_label(
        &mut app,
        "players-error",
        "Lichess usernames are 2–30 letters, digits, - or _",
    );

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
            .is_some_and(|error| !error.is_empty() && !error.starts_with("Lichess usernames"))
    });
    assert!(app.enabled("players-lookup"));
}

#[gpui_kit::test]
fn the_player_search_drops_a_stale_message_and_forgets_recent_names(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut app = open(cx, "Players", true);
    // Nothing typed: nothing to look up.
    assert!(!app.enabled("players-lookup"));
    app.cx.update(|cx| {
        crate::prefs::set(cx, "kchess:players-recent", ["alice", "bob"]);
    });
    app.click_label("Lichess username");
    app.type_text("!");
    app.press("enter");
    wait_for_label(
        &mut app,
        "players-error",
        "Lichess usernames are 2–30 letters, digits, - or _",
    );
    // Typing again takes the message about the old text away.
    app.type_text("x");
    app.wait_for("the message to go", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window.try_find("players-error").is_none()
    });
    assert!(app.shows(("players-suggestion", 1usize)));
    app.click("players-recent-clear");
    app.wait_for("the recent names to go", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window.try_find(("players-suggestion", 0usize)).is_none()
    });
    let kept: Vec<String> = app
        .cx
        .update(|cx| crate::prefs::get(cx, "kchess:players-recent", vec!["?".to_string()]));
    assert!(kept.is_empty());
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

/// The Following page's buttons are in Quick Pick: a typed name is followed from there.
#[gpui_kit::test]
fn a_player_is_followed_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    let mut app = open(cx, "Following", true);
    app.click_label("Lichess username to follow");
    app.type_text("carol");
    app.press("secondary-k");
    app.type_text(">Follow player");
    super::palette::wait_for_first(&mut app, "Follow player");
    app.press("enter");
    super::palette::wait_until_closed(&mut app);
    app.find("friends-sync");
}

/// The timeline is what the followed players did, from their public activity: Lichess's own
/// timeline answers only its mobile app, so it always read as "Nothing new".
#[gpui_kit::test]
fn the_timeline_shows_what_followed_players_did(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    app.open("Following");
    follow(&mut app, "carol");
    let line = app
        .find(gpui_kit::ElementId::NamedInteger(
            "friends-timeline-player".into(),
            1,
        ))
        .bounds();
    // The day's date sits beside its first line, on its baseline, not over it.
    let day = app
        .find(gpui_kit::ElementId::NamedInteger(
            "friends-timeline-day".into(),
            0,
        ))
        .bounds();
    assert!(day.right() < line.left(), "{day:?} {line:?}");
    assert!(
        (day.bottom() - line.bottom()).abs() < gpui_kit::px(4.),
        "{day:?} {line:?}"
    );
}

/// A timeline that could not be read is tried again from where its failure shows.
#[gpui_kit::test]
fn a_failed_timeline_can_be_tried_again(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    app.open("Following");
    follow(&mut app, "frank");
    // A text button is still a 24 px target (WCAG 2.5.8).
    let retry = app.find("friends-timeline-retry").bounds();
    assert!(retry.size.height >= gpui_kit::px(24.), "{retry:?}");
    app.click("friends-timeline-retry");
    app.find(gpui_kit::ElementId::NamedInteger(
        "friends-timeline-player".into(),
        1,
    ));
    assert!(!app.shows("friends-timeline-retry"));
}

/// A sync of a big account takes minutes: following a player meanwhile works at once, and Sync
/// all waits for the running sync. (A single busy flag used to disable Follow until it ended.)
#[gpui_kit::test]
fn following_works_while_games_sync(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = Harness::signed_in(cx, false);
    app.open("Following");
    // The sync held below must be a new one. Signing in syncs the account too, and a sync asked
    // for while that one runs joins it: on Windows CI (PR #85) the sign-in's sync was past its
    // games download, so the joined sync ended at once, Sync all never waited and the test
    // timed out.
    app.wait_for("the sign-in's sync to end", |_, cx| {
        !crate::accounts::handle(cx).read(cx).working()
    });
    lichess.lock().hold_games = true;
    app.with_window(|window, cx| {
        crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.sync(None, window, cx))
    });
    app.wait_for("the sync to start", |_, cx| {
        crate::accounts::handle(cx).read(cx).syncing()
    });
    follow(&mut app, "dana");
    app.wait_for("dana to be followed", |_, cx| {
        crate::accounts::handle(cx)
            .read(cx)
            .tracked()
            .iter()
            .any(|friend| friend["username"].as_str() == Some("dana"))
    });
    app.wait_for("Sync all to wait for the sync", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find(gpui_kit::ElementId::from("friends-sync"))
            .is_some_and(|sync| sync.disabled() == Some(true))
    });
    lichess.lock().hold_games = false;
    app.wait_for("the syncs to end", |_, cx| {
        !crate::accounts::handle(cx).read(cx).working()
    });
}

/// A player who finished a game since their sync is marked, and so is Sync all, from their profile
/// alone; syncing takes the mark away.
#[gpui_kit::test]
fn players_with_games_to_sync_are_marked(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    app.open("Following");
    follow(&mut app, "dana");
    let new_games = ("friend-new-games", 0usize);
    app.wait_for("the follow's sync", |_, cx| {
        let accounts = crate::accounts::handle(cx).read(cx);
        !accounts.working()
            && accounts
                .tracked()
                .iter()
                .any(|friend| friend["syncedPlayTime"].is_number())
    });
    assert!(!app.shows(new_games), "the sync downloaded everything");
    assert_eq!(app.find("friends-sync").label(), Some("Sync all"));

    super::fake_lichess::play("dana", 300);
    let mut app = app.restart();
    app.open("Following");
    app.find(new_games);
    assert_eq!(app.find("friends-sync").label(), Some("Sync all · 1 new"));

    app.click("friends-sync");
    app.wait_gone(new_games);
    assert_eq!(app.find("friends-sync").label(), Some("Sync all"));
}

#[gpui_kit::test]
fn insights_without_games_lead_to_following_or_analysis(cx: &mut gpui_kit::TestAppContext) {
    let mut app = open(cx, "Insights", true);
    // With no games there is nothing to filter.
    app.find("insights-follow");
    assert!(app.with_window(|window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window.try_find_by_label("Speed").is_none()
    }));
    app.click("insights-analyse");
    assert_eq!(page(&mut app), Some(Page::Analysis));
    app.open("Insights");
    app.click("insights-follow");
    assert_eq!(page(&mut app), Some(Page::Friends));
}

/// A saved title the leaderboard lacks says so and offers everyone back; then titles have their
/// own column, so every name starts at one place.
#[gpui_kit::test]
fn the_leaderboard_lines_up_names_and_offers_everyone_back(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    app.cx
        .update(|cx| crate::prefs::set(cx, "kchess:leaderboard-title", "WGM".to_string()));
    app.open("Players");
    app.click("players-board-all");
    let starts =
        [0usize, 1, 2].map(|row| app.find(("players-board-player", row)).bounds().origin.x);
    assert!(starts.iter().all(|x| *x == starts[0]), "{starts:?}");
    let shown: String = app
        .cx
        .update(|cx| crate::prefs::get(cx, "kchess:leaderboard-title", String::new()));
    assert_eq!(shown, "all");
}

/// One title shows only its players, each at their rank in the whole board.
#[gpui_kit::test]
fn the_leaderboard_shows_one_title(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    app.cx
        .update(|cx| crate::prefs::set(cx, "kchess:leaderboard-title", "GM".to_string()));
    app.open("Players");
    app.find(("players-board-row", 0usize));
    app.find(("players-board-row", 3usize));
    assert!(!app.shows(("players-board-row", 1usize)));
    assert!(!app.shows(("players-board-row", 2usize)));
}

/// An open player's buttons are in Quick Pick: a message is written from there.
#[gpui_kit::test]
fn a_player_is_messaged_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    app.open("Players");
    app.find("players-lookup");
    app.cx.update(|cx| {
        let page = crate::shell::view_of::<crate::pages::players::PlayersPage>(
            crate::pages::Page::Players,
            cx,
        )
        .expect("the Players page");
        page.update(cx, |page, cx| {
            page.show_profile(
                serde_json::json!({ "id": "rival", "username": "Rival", "count": { "all": 3 } }),
                cx,
            )
        });
    });
    app.find("players-challenge");
    app.press("secondary-k");
    app.type_text(">");
    super::palette::wait_for_first(&mut app, "Follow player");
    let listed: Vec<String> = app.with_window(|window, _| {
        (0..6usize)
            .filter_map(|index| window.try_find(("palette-item", index)))
            .filter_map(|item| item.label().map(str::to_string))
            .collect()
    });
    assert_eq!(&listed[..3], ["Follow player", "Challenge", "Message"]);
    app.type_text("Message");
    super::palette::wait_for_first(&mut app, "Message");
    app.press("enter");
    app.find("message-send");
}

/// With players followed and none of them playing, Watch's Following tab says so.
#[gpui_kit::test]
fn watch_says_when_no_followed_player_is_playing(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    app.open("Following");
    follow(&mut app, "dana");
    app.wait_for("the follow", |_, cx| {
        !crate::accounts::handle(cx).read(cx).tracked().is_empty()
    });
    // The follow's toasts are still up; they leave the page's tabs in reach.
    app.open("Watch");
    app.click(("watch-tabs", 1usize));
    assert_eq!(app.find(("watch-tabs", 1usize)).selected(), Some(true));
    app.find("watch-friends-none");
}

/// A leaderboard that could not load says so with Try again, which loads it once Lichess answers.
#[gpui_kit::test]
fn a_failed_leaderboard_can_be_tried_again(cx: &mut gpui_kit::TestAppContext) {
    // The stand-in Lichess fails Racing Kings' leaderboard once.
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    app.cx
        .update(|cx| crate::prefs::set(cx, "kchess:leaderboard-perf", "racingKings".to_string()));
    app.open("Players");
    app.click("players-board-retry");
    app.find(("players-board-row", 0usize));
}

/// Follow `name` from the Following page. Follow waits while the accounts are busy, as they are
/// with signing in's first sync, so the click waits for it too.
fn follow(app: &mut Harness, name: &str) {
    app.click_label("Lichess username to follow");
    app.type_text(name);
    app.wait_for("Follow", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find(gpui_kit::ElementId::from("friends-follow"))
            .is_some_and(|follow| follow.disabled() != Some(true))
    });
    app.click("friends-follow");
}

/// A restarted core counts its online sessions from the start again: its reports are followed,
/// not dropped as older than the previous core's.
#[gpui_kit::test]
fn a_restarted_core_is_followed_from_its_first_session(cx: &mut gpui_kit::TestAppContext) {
    use crate::core::{AppCore, CoreEvent, RECONNECTED};
    use serde_json::{Value, json};
    let mut app = Harness::start(cx);
    let send = |app: &mut Harness, name: &str, payload: Value| {
        app.cx.update(|cx| {
            let events = cx.global::<AppCore>().events.clone();
            events.update(cx, |_, cx| {
                cx.emit(CoreEvent {
                    name: name.into(),
                    payload,
                })
            });
        });
        app.cx.run_until_parked();
    };
    let epoch = |app: &mut Harness| {
        app.cx
            .update(|cx| crate::online::handle(cx).read(cx).state_epoch())
    };
    let report = |session: u64| json!({ "session": session, "account": "", "lane": "events", "phase": "connected" });
    send(&mut app, "online:state", report(40));
    assert_eq!(epoch(&mut app), 40.);
    send(&mut app, RECONNECTED, Value::Null);
    send(&mut app, "online:state", report(39));
    assert_eq!(epoch(&mut app), 39., "the new core's report is followed");
}
