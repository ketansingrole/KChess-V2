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
    let (mut app, lichess) = Harness::signed_in(cx, true);
    // This launch's own game stream opens first: under load it once arrived after the count
    // below was reset, and was counted against the next launch.
    let first = lichess.clone();
    app.wait_for("this launch's game stream", move |_, _| {
        first.lock().board_streams > 0
    });
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

/// When Lichess refuses the board stream of a game it listed as playable, the page says so in
/// Lichess's words, offers the game to watch and never a Reconnect that cannot work.
#[gpui_kit::test]
fn a_refused_game_offers_watching_instead_of_reconnecting(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = in_a_game(cx);
    lichess.lock().board_refused = true;
    app.find("online-watch");
    assert!(!app.shows("online-reconnect"));
    assert!(app.shows("online-another"));
    app.click("online-watch");
    assert_eq!(
        app.cx.update(|cx| crate::shell::current_page(cx)),
        Some(crate::pages::Page::Watch)
    );
}

/// The same refusal on launch, when the app resumes the game by itself: it once arrived before
/// the page was listening, and the board waited on a game that would never come.
#[gpui_kit::test]
fn a_game_refused_on_launch_offers_watching(cx: &mut gpui_kit::TestAppContext) {
    let (app, lichess) = Harness::signed_in(cx, true);
    lichess.lock().board_refused = true;
    let mut app = app.restart();
    app.online();
    app.open("Play on Lichess");
    app.find("online-watch");
    assert!(!app.shows("online-reconnect"));
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

/// Crazyhouse games were refused as needing a piece-drop board. Now a captured pawn waits in
/// the pocket beside the player, and picking it and a square drops it, sent as `P@e3`.
#[gpui_kit::test]
fn a_crazyhouse_pawn_is_dropped_from_the_pocket(cx: &mut gpui_kit::TestAppContext) {
    let (app, lichess) = Harness::signed_in(cx, true);
    {
        let mut session = lichess.lock();
        session.variant = Some("crazyhouse");
        // White took on d5 and Black took back: each holds a pawn, White to move.
        session.moves = ["e2e4", "d7d5", "e4d5", "d8d5"].map(String::from).to_vec();
    }
    let mut app = app.restart();
    app.online();
    app.open("Play on Lichess");
    app.find("online-resign");
    app.find("pocket-White-Pawn");
    app.find("pocket-Black-Pawn");
    lichess.lock().writes.clear();

    app.click("pocket-White-Pawn");
    app.click_square("e3", true);
    app.wait_for("the drop to reach Lichess", |_, _| {
        !lichess.lock().writes.is_empty()
    });
    assert_eq!(
        writes(&lichess),
        ["POST /api/board/game/E2eGame1/move/P%40e3"]
    );
}

/// Quick Pick offers the game's own actions: a draw offer goes to Lichess as the button sends
/// it, and resigning asks first there too.
#[gpui_kit::test]
fn quick_pick_offers_a_draw_and_asks_before_resigning(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = past_the_opening(cx);
    let run = |app: &mut Harness, label: &str| {
        app.press("secondary-k");
        app.type_text(&format!(">{label}"));
        super::palette::wait_for_first(app, label);
        app.press("enter");
    };
    run(&mut app, "Offer draw");
    let offered = lichess.clone();
    app.wait_for("the draw offer to reach Lichess", move |_, _| {
        offered
            .lock()
            .writes
            .contains(&"POST /api/board/game/E2eGame1/draw/yes".to_string())
    });
    run(&mut app, "Resign");
    app.click("cancel");
    app.wait_gone("cancel");
    assert!(
        !writes(&lichess)
            .iter()
            .any(|write| write.ends_with("/resign")),
        "{:?}",
        writes(&lichess)
    );
    assert!(app.shows("online-resign"));

    // The board turns round from there as its button turns it.
    run(&mut app, "Flip board");
    app.wait_for("the board to turn round", |_, cx| {
        crate::online::handle(cx).read(cx).flipped
    });
    // F turns it back, as on every board.
    app.click(("move", 0usize));
    app.press("f");
    app.wait_for("the board to turn back", |_, cx| {
        !crate::online::handle(cx).read(cx).flipped
    });
}

/// Help stays out of Quick Pick during the game; once it is over, Analyse there opens it.
#[gpui_kit::test]
fn a_finished_game_is_analysed_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (mut app, _lichess) = past_the_opening(cx);
    let listed = |app: &mut Harness| -> Vec<String> {
        app.press("secondary-k");
        app.type_text(">");
        super::palette::wait_for_first(app, "Zen mode");
        let labels = app.with_window(|window, _| {
            (0..8usize)
                .filter_map(|index| window.try_find(("palette-item", index)))
                .filter_map(|item| item.label().map(str::to_string))
                .collect()
        });
        app.press("escape");
        super::palette::wait_until_closed(app);
        labels
    };
    let playing = listed(&mut app);
    assert!(!playing.contains(&"Analyse".to_string()), "{playing:?}");
    app.click("online-resign");
    app.click("ok");
    app.find("online-analyse");
    let over = listed(&mut app);
    assert!(over.contains(&"New game".to_string()), "{over:?}");
    assert!(over.contains(&"Rematch".to_string()), "{over:?}");
    // While a rematch is being sent, Quick Pick cannot send another.
    app.with_window(|_, cx| {
        crate::online::handle(cx).update(cx, |online, cx| {
            online.busy = true;
            cx.notify();
        })
    });
    let sending = listed(&mut app);
    assert!(!sending.contains(&"Rematch".to_string()), "{sending:?}");
    app.with_window(|_, cx| {
        crate::online::handle(cx).update(cx, |online, cx| {
            online.busy = false;
            cx.notify();
        })
    });
    app.press("secondary-k");
    app.type_text(">Analyse");
    super::palette::wait_for_first(&mut app, "Analyse");
    app.press("enter");
    app.wait_for("the analysis board", |_, cx| {
        crate::shell::current_page(cx) == Some(crate::pages::Page::Analysis)
    });
}

/// While a seek waits for an opponent, Quick Pick cancels it as the page's Cancel does.
#[gpui_kit::test]
fn a_seek_is_cancelled_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    use crate::online::Phase;
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    app.open("Play on Lichess");
    app.wait_for("the startup check to finish", |_, cx| {
        crate::online::handle(cx).read(cx).phase != Phase::Checking
    });
    app.with_window(|_, cx| {
        crate::online::handle(cx).update(cx, |online, cx| {
            online.phase = Phase::Seeking;
            cx.notify();
        })
    });
    app.press("secondary-k");
    app.type_text(">Cancel seek");
    super::palette::wait_for_first(&mut app, "Cancel seek");
    app.press("enter");
    app.wait_for("the seek to end", |_, cx| {
        crate::online::handle(cx).read(cx).phase != Phase::Seeking
    });
}

/// A correspondence move cannot be taken back, so it waits on the board for Confirm (Return) and
/// can be cancelled (Esc) until then, as on Lichess; with the setting off it goes at once.
#[gpui_kit::test]
fn a_correspondence_move_waits_for_confirm(cx: &mut gpui_kit::TestAppContext) {
    let (app, lichess) = Harness::signed_in(cx, true);
    lichess.lock().correspondence = true;
    let mut app = app.restart();
    app.online();
    app.open("Play on Lichess");
    app.find("online-abort");

    app.click_square("e2", true);
    app.click_square("e4", true);
    app.find("move-confirm");
    app.click("move-cancel");
    app.wait_gone("move-confirm");

    app.click_square("e2", true);
    app.click_square("e4", true);
    app.find("move-confirm");
    app.press("escape");
    app.wait_gone("move-confirm");
    assert!(writes(&lichess).is_empty(), "{:?}", writes(&lichess));

    app.click_square("e2", true);
    app.click_square("e4", true);
    app.find("move-confirm");
    app.press("enter");
    app.wait_for_moves(&["e4", "e5"]);
    assert_eq!(
        writes(&lichess),
        ["POST /api/board/game/E2eGame1/move/e2e4"]
    );

    app.with_window(|_, cx| {
        crate::settings::change(cx, |settings| {
            settings.frontend.confirm_correspondence = false
        })
    });
    app.click_square("g1", true);
    app.click_square("f3", true);
    app.wait_for_moves(&["e4", "e5", "Nf3", "Nc6"]);
    assert!(!app.shows("move-confirm"));
}
