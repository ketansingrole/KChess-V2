//! A live Lichess game, against the stand-in (`fake_lichess`): moves go out once and answers come
//! back, the game's offers and endings show, and no help appears until it is over.

use super::Harness;
use super::fake_lichess::Handle;

/// Signed in by an earlier launch with a game under way, on its page.
fn in_a_game(cx: &mut gpui_kit::TestAppContext) -> (Harness, Handle) {
    let (app, lichess) = Harness::signed_in(cx, true);
    let mut app = app.restart();
    app.online();
    app.open("Play on Lichess");
    app.find("online-abort");
    (app, lichess)
}

/// In the game with a move each played: from here it can be resigned rather than aborted.
fn past_the_opening(cx: &mut gpui_kit::TestAppContext) -> (Harness, Handle) {
    let (mut app, lichess) = in_a_game(cx);
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4", "e5"]);
    lichess.lock().writes.clear();
    (app, lichess)
}

#[gpui_kit::test]
fn a_game_lichess_keeps_from_apps_is_offered_to_watch(cx: &mut gpui_kit::TestAppContext) {
    // A game started on another device that the Board API cannot play once opened Play on
    // Lichess on "Connection interrupted" with Lichess's 400 and a Reconnect that never worked.
    let (app, lichess) = Harness::signed_in(cx, true);
    {
        // From the next launch on, the game is one Lichess keeps from apps.
        let mut session = lichess.lock();
        session.elsewhere = true;
        session.board_streams = 0;
    }
    let mut app = app.restart();
    app.online();
    app.open("Play on Lichess");
    {
        use gpui_kit::test::TestWindowExt as _;
        app.wait_for("the game to be listed to watch", |window, _| {
            window
                .try_find(("ongoing-play", 0usize))
                .is_some_and(|button| button.label() == Some("Watch"))
        });
    }
    assert!(
        !app.shows("online-abort"),
        "the game is not taken to the board"
    );
    assert_eq!(lichess.lock().board_streams, 0);
    app.click(("ongoing-play", 0usize));
    assert_eq!(
        app.cx.update(|cx| crate::shell::current_page(cx)),
        Some(crate::pages::Page::Watch)
    );
}

fn writes(lichess: &Handle) -> Vec<String> {
    lichess.lock().writes.clone()
}

#[gpui_kit::test]
fn a_move_goes_to_lichess_once_and_the_reply_comes_back(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = in_a_game(cx);
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4", "e5"]);
    assert_eq!(
        writes(&lichess),
        ["POST /api/board/game/E2eGame1/move/e2e4"]
    );
}

#[gpui_kit::test]
fn no_help_is_offered_until_the_game_is_over(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = past_the_opening(cx);
    assert!(!app.shows("online-analyse"));

    app.click("online-resign");
    app.click("ok");
    app.find("online-analyse");
    assert_eq!(writes(&lichess), ["POST /api/board/game/E2eGame1/resign"]);
    assert!(!app.shows("online-resign"));
}

#[gpui_kit::test]
fn resigning_asks_first(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = past_the_opening(cx);
    app.click("online-resign");
    app.click("cancel");
    app.wait_gone("cancel");
    assert!(writes(&lichess).is_empty());
    assert!(app.shows("online-resign"));
}

#[gpui_kit::test]
fn an_opponents_draw_offer_is_accepted_and_ends_the_game(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = in_a_game(cx);
    {
        let mut session = lichess.lock();
        session.draw_offered = true;
        session.changed();
    }
    app.click("draw-accept");
    app.find("online-analyse");
    assert_eq!(writes(&lichess), ["POST /api/board/game/E2eGame1/draw/yes"]);
}

#[gpui_kit::test]
fn an_opponents_draw_offer_can_be_declined_and_the_game_goes_on(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = in_a_game(cx);
    {
        let mut session = lichess.lock();
        session.draw_offered = true;
        session.changed();
    }
    app.click("draw-decline");
    app.wait_gone("draw-accept");
    assert_eq!(
        writes(&lichess),
        ["POST /api/board/game/E2eGame1/draw/false"]
    );
    assert!(app.shows("online-abort"));
}

#[gpui_kit::test]
fn an_opponents_takeback_request_is_answered(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = past_the_opening(cx);
    {
        let mut session = lichess.lock();
        session.takeback_offered = true;
        session.changed();
    }
    app.click("takeback-accept");
    app.wait_for_moves(&["e4"]);
    assert_eq!(
        writes(&lichess).last().map(String::as_str),
        Some("POST /api/board/game/E2eGame1/takeback/yes")
    );
}

#[gpui_kit::test]
fn a_game_lost_on_lichess_ends_here_too(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = in_a_game(cx);
    {
        let mut session = lichess.lock();
        session.finished = Some("outoftime");
        session.winner = Some("black");
        session.changed();
    }
    app.find("online-analyse");
    assert!(!app.shows("online-abort"));
    assert!(writes(&lichess).is_empty());
}

#[gpui_kit::test]
fn a_game_without_moves_is_aborted(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = in_a_game(cx);
    assert!(!app.shows("online-resign"));
    app.click("online-abort");
    app.find("online-new");
    assert_eq!(writes(&lichess), ["POST /api/board/game/E2eGame1/abort"]);
}

#[gpui_kit::test]
fn a_game_under_way_comes_to_the_board_when_signing_in(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in(cx, true);
    app.open("Play on Lichess");
    app.find("online-abort");
}

#[gpui_kit::test]
fn signing_in_starts_listening_for_challenges(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = Harness::signed_in(cx, false);
    app.wait_for("the event stream to open", |_, _| {
        lichess.lock().streams == 1
    });
}

/// The app ends up reading the active account's stream and no other, and a connect does not open
/// the previous account's again. It did: the resume a connect starts reopened the stream the core
/// last kept, the old account's, before the app named the new one. That abandoned request reached
/// Lichess ten seconds late on a PC whose network filter holds loopback connections, and the test,
/// which compared when streams opened, took it for the stream the app kept.
#[gpui_kit::test]
fn switching_accounts_moves_the_event_stream(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, first) = Harness::signed_in(cx, false);
    app.wait_for("the first account's stream", |_, _| {
        first.lock().streams == 1
    });
    let second = app.connect("Second", false);
    let listens_to =
        |one: &Handle, other: &Handle| one.lock().listening > 0 && other.lock().listening == 0;
    app.wait_for("the second account's stream", |_, _| {
        listens_to(&second, &first)
    });
    assert_eq!(
        first.lock().streams,
        1,
        "the old account's stream opened again"
    );

    app.open("Play on Lichess");
    app.click_label("Account");
    super::analysis::settle(&mut app);
    app.press("down");
    app.press("enter");
    app.wait_for("the first account's stream again", |_, _| {
        listens_to(&first, &second)
    });
}

#[gpui_kit::test]
fn a_refreshed_login_brings_back_the_event_stream(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, expired) = Harness::signed_in(cx, false);
    app.wait_for("the event stream to open", |_, _| {
        expired.lock().streams == 1
    });
    expired.lock().revoked = true;
    app.wait_for("the app to stop syncing", |_, cx| {
        !crate::accounts::handle(cx).read(cx).busy
    });

    let refreshed = app.connect("Tester", false);
    app.wait_for("the event stream with the new login", |_, _| {
        refreshed.lock().streams == 1
    });
}
