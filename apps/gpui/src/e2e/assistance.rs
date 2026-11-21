//! Engine help and the check for a Lichess game in progress: the app follows the core's lock,
//! says why it is on, and checks again until Lichess answers.

use std::time::Duration;

use super::Harness;
use super::fake_lichess::Handle;
use crate::online::Pause;

fn pause(app: &mut Harness) -> Option<Pause> {
    app.with_window(|_, cx| crate::online::handle(cx).read(cx).pause())
}

fn allowed(app: &mut Harness) -> bool {
    app.with_window(|_, cx| crate::online::handle(cx).read(cx).assistance_allowed())
}

/// Signed in while Lichess fails the check for a game in progress.
fn check_failed(cx: &mut gpui_kit::TestAppContext) -> (Harness, Handle) {
    let mut app = Harness::start(cx);
    let lichess = app.connect_with("Tester", false, |session| session.fail_check = true);
    app.wait_for("the check to fail", |_, cx| {
        crate::online::handle(cx).read(cx).pause() == Some(Pause::Unverified)
    });
    (app, lichess)
}

/// 2026-11-20: when the check for a game in progress failed (offline, Lichess down), the core
/// kept engine help locked until a restart while the app took it as allowed: Play vs Computer
/// and the analysis engine were refused "during a live Lichess game" with nobody playing.
#[gpui_kit::test]
fn a_failed_check_is_explained_and_checked_again(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = check_failed(cx);
    assert!(!allowed(&mut app), "the core's lock is the app's");

    app.open("Analysis board");
    // Still failing: it stays locked, and says so.
    lichess.lock().checked = false;
    app.click("engine-check-again");
    app.wait_for("the check to run again", |_, _| lichess.lock().checked);
    app.wait_for("the check to fail again", |_, cx| {
        crate::online::handle(cx).read(cx).pause() == Some(Pause::Unverified)
    });
    assert!(!allowed(&mut app));
    // The review says why too.
    app.click("Review");
    app.find("assistance-check-again");

    lichess.lock().fail_check = false;
    app.click("assistance-check-again");
    app.wait_for("engine help to come back", |_, cx| {
        crate::online::handle(cx).read(cx).assistance_allowed()
    });
    app.wait_gone("assistance-check-again");
    app.wait_gone("engine-check-again");
    assert_eq!(pause(&mut app), None);
}

/// After a failed check, the app asks Lichess again on its own, a little later each time.
#[gpui_kit::test]
fn a_failed_check_is_retried_on_its_own(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = check_failed(cx);
    lichess.lock().fail_check = false;
    app.cx.executor().advance_clock(Duration::from_secs(16));
    app.wait_for("the retry to unlock engine help", |_, cx| {
        crate::online::handle(cx).read(cx).assistance_allowed()
    });
}

/// Back on a network, the app asks Lichess again at once.
#[gpui_kit::test]
fn a_failed_check_is_retried_when_the_network_returns(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = check_failed(cx);
    app.set_online(false);
    lichess.lock().fail_check = false;
    app.set_online(true);
    app.wait_for("the network's return to unlock engine help", |_, cx| {
        crate::online::handle(cx).read(cx).assistance_allowed()
    });
}

/// The explorer looks its position up again once the check lets help back: a lookup made just
/// after connecting, before the check had answered, was refused and stayed so behind Try again
/// (Windows CI, 2026-11-22, `the_explorer_offers_connect_lichess_and_looks_up_once_connected`).
#[gpui_kit::test]
fn a_refused_lookup_is_made_again_once_the_check_answers(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::openings::OpeningsPage;
    let (mut app, lichess) = check_failed(cx);
    app.open("Openings");
    // 1. d4 d5, where the stand-in Lichess's 1. e4 is a move White can play.
    app.with_window(|window, cx| {
        crate::shell::view::<OpeningsPage>(cx)
            .expect("the page")
            .update(cx, |page, cx| {
                page.show_line(Some(vec!["d2d4".into(), "d7d5".into()]), window, cx)
            })
    });
    app.find("explorer-retry");
    lichess.lock().fail_check = false;
    app.cx.executor().advance_clock(Duration::from_secs(16));
    app.wait_for("engine help to come back", |_, cx| {
        crate::online::handle(cx).read(cx).assistance_allowed()
    });
    app.find(("explorer-tile", 0usize));
    assert!(!app.shows("explorer-retry"));
}

/// Play with computer says why the computer is not moving, and offers to check again.
#[gpui_kit::test]
fn the_computer_waits_with_a_reason(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = check_failed(cx);
    app.open("Play with computer");
    app.find("assistance-check-again");
    lichess.lock().fail_check = false;
    app.click("assistance-check-again");
    // The banner goes as the check starts, before Lichess answers: wait for the answer.
    app.wait_for("engine help to come back", |_, cx| {
        crate::online::handle(cx).read(cx).assistance_allowed()
    });
    app.wait_gone("assistance-check-again");
}

/// The core's refusals say what is going on: no game is being played, the check is not done.
#[gpui_kit::test]
fn a_refusal_names_the_check_not_a_live_game(cx: &mut gpui_kit::TestAppContext) {
    use crate::core::AppCore;
    use kchess_contracts::calls;
    use kchess_contracts::check::Moves;
    use kchess_contracts::types::EngineLevel;
    let (app, _lichess) = check_failed(cx);
    let core = app.cx.update(|cx| cx.global::<AppCore>().bridge.clone());
    let refused = core
        .request_now(calls::BestMove {
            moves: Moves(Vec::new()),
            level: EngineLevel::Max,
            options: None,
        })
        .expect_err("refused while unverified");
    // Told by its kind, so a lookup refused for it is made again once help is lent.
    assert_eq!(refused.kind, Some(kchess_contracts::ErrorKind::Withheld));
    assert_eq!(
        refused.message,
        "Engine assistance is unavailable until your Lichess game status is verified."
    );
}
