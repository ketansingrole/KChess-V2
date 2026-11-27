use super::describe_control;
use kchess_board::testing::Started;

/// A board page's app with the live game of its own, its Lichess at `origin`.
fn started(cx: &mut gpui_kit::TestAppContext, origin: Option<String>) -> Started {
    let mut app = Started::bare(cx, origin);
    app.with_window(|_, cx| super::init(cx));
    app
}
use kchess_contracts::types::ChallengeTimeControl;
use serde_json::Number;

fn clock(limit: u64, increment: u64) -> ChallengeTimeControl {
    ChallengeTimeControl::Clock {
        limit: Number::from(limit),
        increment: Number::from(increment),
    }
}

#[test]
fn controls_read_like_lichess() {
    assert_eq!(describe_control(&clock(300, 3)), "5+3");
    assert_eq!(describe_control(&clock(90, 0)), "1.5+0");
    let days = ChallengeTimeControl::Correspondence {
        days: Number::from(1),
    };
    assert_eq!(describe_control(&days), "1 day per move");
    assert_eq!(describe_control(&ChallengeTimeControl::Unlimited), "∞");
}

/// A resume is counted as it starts, so an answer to an earlier one is known for stale.
#[gpui_kit::test]
fn each_resume_is_counted_as_it_starts(cx: &mut gpui_kit::TestAppContext) {
    let mut app = started(cx, None);
    let online = app.with_window(|_, cx| super::handle(cx));
    let before = app.with_window(|_, cx| online.read(cx).resumes);
    app.with_window(|_, cx| super::resume_quietly(online.clone(), true, cx));
    app.with_window(|_, cx| super::resume(online.clone(), cx));
    assert_eq!(app.with_window(|_, cx| online.read(cx).resumes), before + 2);
}

/// A game being played redraws its clocks as they tick, and nothing else does.
#[gpui_kit::test]
fn a_game_being_played_redraws_as_its_clocks_tick(cx: &mut gpui_kit::TestAppContext) {
    use kchess_domain::online_game::model::GamePhase;
    let mut app = started(cx, None);
    let online = app.with_window(|_, cx| super::handle(cx));
    let redraws = std::rc::Rc::new(std::cell::Cell::new(0));
    let _watch = app.with_window(|_, cx| {
        let redraws = redraws.clone();
        cx.observe(&online, move |_, _| redraws.set(redraws.get() + 1))
    });
    app.with_window(|_, cx| online.update(cx, |online, cx| online.tick(cx)));
    app.cx().run_until_parked();
    let idle = redraws.get();
    app.with_window(|_, cx| {
        online.update(cx, |online, cx| online.set_phase(GamePhase::Playing, cx))
    });
    app.cx().run_until_parked();
    let before = redraws.get();
    app.with_window(|_, cx| online.update(cx, |online, cx| online.tick(cx)));
    app.cx().run_until_parked();
    assert_eq!((idle, redraws.get()), (0, before + 1));
}

/// How long the core may take to answer a status read, which it answers at once: a read
/// that never lands fails the test promptly instead of hanging until the runner's timeout.
const ANSWER: std::time::Duration = std::time::Duration::from_secs(10);

/// The board's online state once its startup check has ended and the core lends help.
fn settled(app: &mut Started) -> gpui_kit::Entity<super::OnlineState> {
    app.wait_for_engine_help();
    app.with_window(|_, cx| super::handle(cx))
}

/// A connection report in `phase`, for the state to hold.
fn connection(lane: &str, phase: &str) -> kchess_domain::online_game::connection::OnlineConnection {
    serde_json::from_value(serde_json::json!({
        "session": 1, "account": "me", "gameId": "", "lane": lane, "phase": phase,
    }))
    .expect("a connection report")
}

#[test]
fn each_pause_says_why() {
    use super::Pause;
    assert_eq!(
        Pause::Checking.text(),
        "Checking Lichess for a game in progress…"
    );
    assert_eq!(
        Pause::Unverified.text(),
        "Couldn't check Lichess for a game in progress"
    );
    assert_eq!(Pause::LiveGame.text(), "Paused during your Lichess game");
}

/// Engine help needs both: no game of the account's on the board, and the core lending it. A
/// check that failed is told apart from a check under way and from a live game, which is
/// what decides whether Check again is offered.
#[gpui_kit::test]
fn engine_help_follows_both_the_board_and_the_core(cx: &mut gpui_kit::TestAppContext) {
    use super::{Pause, Phase};
    use kchess_contracts::types::Assistance;
    let mut app = started(cx, None);
    let online = settled(&mut app);
    let all = [
        Phase::Checking,
        Phase::Idle,
        Phase::Seeking,
        Phase::Playing,
        Phase::Finished,
        Phase::Disconnected,
    ];
    let lent = [
        Assistance::Allowed,
        Assistance::Unverified,
        Assistance::LiveGame,
    ];
    let seen = app.with_window(|_, cx| {
        online.update(cx, |state, _| {
            let mut seen = Vec::new();
            for phase in all {
                for assistance in lent {
                    state.phase = phase;
                    state.assistance = assistance;
                    seen.push((phase, assistance, state.assistance_allowed(), state.pause()));
                }
            }
            // The core reporting its check of the game lane counts as checking, whatever the
            // board's phase.
            state.phase = Phase::Idle;
            state.assistance = Assistance::Unverified;
            state.model.online_connection = Some(connection("game", "checking"));
            let reported = state.pause();
            state.model.online_connection = Some(connection("events", "checking"));
            let other_lane = state.pause();
            state.model.online_connection = None;
            (seen, reported, other_lane)
        })
    });
    let (seen, reported, other_lane) = seen;
    let expected = |phase: Phase, assistance: Assistance| match (phase, assistance) {
        (Phase::Idle | Phase::Finished, Assistance::Allowed) => (true, None),
        (Phase::Checking, _) => (false, Some(Pause::Checking)),
        (Phase::Idle | Phase::Finished, Assistance::Unverified) => (false, Some(Pause::Unverified)),
        _ => (false, Some(Pause::LiveGame)),
    };
    for (phase, assistance, allowed, pause) in seen {
        assert_eq!(
            (allowed, pause),
            expected(phase, assistance),
            "{phase:?} with the core's {assistance:?}"
        );
    }
    assert_eq!(reported, Some(Pause::Checking));
    assert_eq!(other_lane, Some(Pause::Unverified));
}

/// Each change the core reports is taken and counted, so a status read asked before it is
/// known for stale; a read with nothing reported since sets the state to the core's.
#[gpui_kit::test]
fn the_cores_lock_is_followed(cx: &mut gpui_kit::TestAppContext) {
    use kchess_contracts::events::CoreEvent;
    use kchess_contracts::types::Assistance;
    let mut app = started(cx, None);
    let online = settled(&mut app);
    let (before, after, taken) = app.with_window(|_, cx| {
        online.update(cx, |state, cx| {
            let before = state.assistance_reports;
            state.receive(&CoreEvent::Assistance(Assistance::LiveGame), cx);
            (before, state.assistance_reports, state.assistance)
        })
    });
    assert_eq!((after, taken), (before + 1, Assistance::LiveGame));
    // The core still lends help (nothing changed there): reading it corrects the state.
    app.with_window(|_, cx| online.update(cx, |state, cx| state.read_assistance(cx)));
    app.wait_within(ANSWER, "the core's answer", |_, cx| {
        online.read(cx).assistance == Assistance::Allowed
    });
}

/// A failed check is tried again later, twice as late each time up to five minutes; not
/// while Lichess refuses the login, which connecting the account again checks itself.
#[gpui_kit::test]
fn a_failed_check_is_retried_later_each_time(cx: &mut gpui_kit::TestAppContext) {
    use std::time::Duration;
    let mut app = started(cx, None);
    let online = settled(&mut app);
    let waits = app.with_window(|_, cx| {
        online.update(cx, |state, cx| {
            state.recheck_after = super::RECHECK_FIRST;
            state._recheck = None;
            state.schedule_recheck(cx);
            let first = (state.recheck_after, state._recheck.is_some());
            state.recheck_after = Duration::from_secs(4 * 60);
            state.schedule_recheck(cx);
            let capped = state.recheck_after;
            state.recheck_after = super::RECHECK_FIRST;
            state.model.online_connection = Some(connection("events", "auth-required"));
            state.schedule_recheck(cx);
            let refused = (state.recheck_after, state._recheck.is_some());
            state.model.online_connection = None;
            (first, capped, refused)
        })
    });
    assert_eq!(
        waits,
        (
            (Duration::from_secs(30), true),
            Duration::from_secs(5 * 60),
            (Duration::from_secs(15), false),
        )
    );
}

/// Check again asks Lichess once more after a failed check, and only then.
#[gpui_kit::test]
fn check_again_follows_only_a_failed_check(cx: &mut gpui_kit::TestAppContext) {
    use super::Phase;
    use kchess_contracts::types::Assistance;
    let mut app = started(cx, None);
    let online = settled(&mut app);
    let asked = |app: &mut Started, assistance: Assistance| {
        app.with_window(|_, cx| {
            let before = online.update(cx, |state, _| {
                state.model.online_connection = None;
                state.phase = Phase::Idle;
                state.assistance = assistance;
                state.resumes
            });
            super::check_again(cx);
            let state = online.read(cx);
            (state.resumes - before, state.pause())
        })
    };
    assert_eq!(asked(&mut app, Assistance::Allowed), (0, None));
    assert_eq!(
        asked(&mut app, Assistance::Unverified),
        (1, Some(super::Pause::Checking))
    );
}

/// A retry that came due while the core was still checking was dropped, and with that check
/// failing too nothing asked again: engine help stayed locked (macOS CI, 2026-11-24,
/// `e2e::assistance::a_failed_check_is_retried_on_its_own`). It is made once that check fails.
#[gpui_kit::test]
fn a_retry_due_during_a_check_is_made_once_it_fails(cx: &mut gpui_kit::TestAppContext) {
    use super::{Pause, Phase};
    use kchess_contracts::events::CoreEvent;
    use kchess_contracts::types::Assistance;
    let mut app = started(cx, None);
    let online = settled(&mut app);
    let before = app.with_window(|_, cx| {
        online.update(cx, |state, _| {
            state.phase = Phase::Idle;
            state.assistance = Assistance::Unverified;
            state.model.online_connection = Some(connection("game", "checking"));
            state.resumes
        })
    });
    // The retry timer fires while the core's check is underway.
    let during = app.with_window(|_, cx| {
        super::check_again(cx);
        let state = online.read(cx);
        (state.resumes - before, state.pause())
    });
    assert_eq!(during, (0, Some(Pause::Checking)));
    // That check fails as well.
    app.with_window(|_, cx| {
        online.update(cx, |state, cx| {
            let failed = CoreEvent::OnlineState(connection("game", "disconnected"));
            state.receive(&failed, cx);
        })
    });
    app.cx().run_until_parked();
    let after = app.with_window(|_, cx| online.read(cx).resumes - before);
    assert_eq!(after, 1, "the retry is made once the check has failed");
}

/// Signing out while the games in progress loaded kept that request current: its answer
/// brought the account's games back, and an earlier failure stayed on the empty list.
#[gpui_kit::test]
fn games_in_progress_from_before_signing_out_do_not_come_back(cx: &mut gpui_kit::TestAppContext) {
    let mut app = started(cx, None);
    let (stale, error) = app.with_window(|_, cx| {
        super::handle(cx).update(cx, |online, cx| {
            online.ongoing_loading.show_error("Lichess did not answer");
            let asked = online.ongoing_loading.current();
            // No account is connected in a fresh profile.
            online.refresh_ongoing(cx);
            (
                online.ongoing_loading.is_current(asked),
                online.ongoing_loading.error().cloned(),
            )
        })
    });
    assert!(!stale);
    assert_eq!(error, None);
}

fn ongoing(id: &str, speed: &str, my_turn: bool) -> kchess_contracts::types::OngoingGame {
    serde_json::from_value(serde_json::json!({
        "gameId": id,
        "account": "alice",
        "opponent": { "name": "bob" },
        "fen": "8/8/8/8/8/8/8/8 w - - 0 1",
        "isMyTurn": my_turn,
        "variant": "standard",
        "speed": speed,
        "rated": false,
    }))
    .expect("an ongoing game")
}

/// A correspondence game announces the player's move only when it just turned to them, in a
/// game not being played in the window: each other case stays quiet.
#[test]
fn only_a_correspondence_game_that_just_turned_to_you_is_your_move() {
    let game = ongoing("abc", "correspondence", true);
    assert!(super::is_your_move_now(&game, Some(false), "other"));
    // Not correspondence.
    assert!(!super::is_your_move_now(
        &ongoing("abc", "blitz", true),
        Some(false),
        "other"
    ));
    // Still the opponent's turn.
    assert!(!super::is_your_move_now(
        &ongoing("abc", "correspondence", false),
        Some(false),
        "other"
    ));
    // Already the player's move when last seen, or never seen.
    assert!(!super::is_your_move_now(&game, Some(true), "other"));
    assert!(!super::is_your_move_now(&game, None, "other"));
    // The game open in the window needs no announcement.
    assert!(!super::is_your_move_now(&game, Some(false), "abc"));
}

/// Asking for the games in progress with an account connected marks the list as read once the
/// current request's answer lands, failed or not.
#[gpui_kit::test]
fn the_current_answer_for_games_in_progress_lands(cx: &mut gpui_kit::TestAppContext) {
    let server = kchess_testkit::Server::start(|_, out| {
        kchess_testkit::respond(out, 500, "text/plain", b"")
    })
    .expect("a stand-in Lichess");
    let mut app = started(cx, Some(server.base().to_string()));
    app.with_account("alice", true);
    app.with_window(|_, cx| {
        super::handle(cx).update(cx, |online, cx| {
            assert!(!online.ongoing_loaded);
            online.refresh_ongoing(cx);
        });
    });
    app.wait_for("the answer to land", |_, cx| {
        super::handle(cx).read(cx).ongoing_loaded
    });
}

/// Starting a game asks the core at once: the state is busy until its answer comes.
#[gpui_kit::test]
fn starting_a_game_is_busy_until_the_core_answers(cx: &mut gpui_kit::TestAppContext) {
    let server = kchess_testkit::Server::start(|_, out| {
        kchess_testkit::respond(out, 500, "text/plain", b"")
    })
    .expect("a stand-in Lichess");
    let mut app = started(cx, Some(server.base().to_string()));
    let busy = app.with_window(|_, cx| {
        super::handle(cx).update(cx, |online, cx| {
            online.start(None, cx);
            online.busy
        })
    });
    assert!(busy);
    app.wait_for("the start to end", |_, cx| !super::handle(cx).read(cx).busy);
}

/// Accepting a live challenge the core refuses (here no account is connected) leaves the
/// board on the game it was following, so that game's moves keep arriving.
#[gpui_kit::test]
fn a_refused_accept_leaves_the_board_on_its_game(cx: &mut gpui_kit::TestAppContext) {
    use kchess_contracts::types::ChallengeInfo;
    let mut app = started(cx, None);
    let online = settled(&mut app);
    let challenge: ChallengeInfo = serde_json::from_value(serde_json::json!({
        "id": "Chal5678", "account": "other", "direction": "in", "opponent": { "name": "Bob" },
        "variant": "standard", "variantName": "Standard", "rated": false, "speed": "blitz",
        "timeControl": { "type": "clock", "limit": 300, "increment": 3 }, "color": "white",
        "playable": true, "receivedAt": 0,
    }))
    .expect("a challenge");
    let task = app.with_window(|_, cx| {
        online.update(cx, |state, cx| {
            state.model.online_account = "me".into();
            state.model.online_id = "Live1234".into();
            state.accept(&challenge, cx)
        })
    });
    let accepted = std::rc::Rc::new(std::cell::Cell::new(None));
    let seen = accepted.clone();
    app.cx()
        .spawn(async move |_| seen.set(Some(task.await)))
        .detach();
    app.wait_within(ANSWER, "the core's refusal", |_, _| {
        accepted.get().is_some()
    });
    let (account, id) = app.with_window(|_, cx| {
        let model = &online.read(cx).model;
        (model.online_account.clone(), model.online_id.clone())
    });
    assert_eq!(accepted.get(), Some(false));
    assert_eq!((account.as_str(), id.as_str()), ("me", "Live1234"));
}

/// Accepting a live challenge while a real-time game is on the board leaves the board on that
/// game until the core answers: moved to the challenge meanwhile, it dropped the game's moves.
#[gpui_kit::test]
fn an_accept_during_a_live_game_leaves_the_board_on_it(cx: &mut gpui_kit::TestAppContext) {
    use kchess_contracts::types::ChallengeInfo;
    use kchess_domain::online_game::model::GamePhase;
    let mut app = started(cx, None);
    let online = settled(&mut app);
    let challenge: ChallengeInfo = serde_json::from_value(serde_json::json!({
        "id": "Chal5678", "account": "other", "direction": "in", "opponent": { "name": "Bob" },
        "variant": "standard", "variantName": "Standard", "rated": false, "speed": "blitz",
        "timeControl": { "type": "clock", "limit": 300, "increment": 3 }, "color": "white",
        "playable": true, "receivedAt": 0,
    }))
    .expect("a challenge");
    let id = app.with_window(|_, cx| {
        online.update(cx, |state, cx| {
            state.model.online_account = "me".into();
            state.model.online_id = "Live1234".into();
            state.set_phase(GamePhase::Playing, cx);
            state.accept(&challenge, cx).detach();
            state.model.online_id.clone()
        })
    });
    assert_eq!(id, "Live1234");
}
