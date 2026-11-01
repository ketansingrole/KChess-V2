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

#[gpui_kit::test]
fn switching_accounts_moves_the_event_stream(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, first) = Harness::signed_in(cx, false);
    app.wait_for("the first account's stream", |_, _| {
        first.lock().streams == 1
    });
    let second = app.connect("Second", false);
    let listens_to =
        |one: &Handle, other: &Handle| one.lock().stream_opened > other.lock().stream_opened;
    app.wait_for("the second account's stream", |_, _| {
        listens_to(&second, &first)
    });

    app.open("Play on Lichess");
    app.click_label("Account");
    super::analysis::settle(&mut app);
    app.press("down");
    app.press("enter");
    app.wait_for("the first account's stream again", |_, _| {
        listens_to(&first, &second)
    });
}
