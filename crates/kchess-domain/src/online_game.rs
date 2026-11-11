//! How a Lichess game on the board changes with each server event, connection report and network
//! reply. The host keeps the `Model` and executes the effects; every decision lives here,
//! including which call to make and how its reply lands. Each transition takes the context the
//! host read at that moment (`Ctx`).

pub mod connection;
pub mod events;
pub mod model;
pub mod server;

use serde_json::Value;

use crate::js;
use connection::OnlineConnection;
use model::{
    Call, ChatEntry, Ctx, GamePhase, Left, Model, Offer, Op, Outcome, Pending, Reply, Run, View,
};
use serde::Deserialize as _;
use server::ServerEvent;

impl Model {
    /// The live position, the SAN history, the moves this side made, whether it is a
    /// correspondence game, and the rematch's options.
    pub fn view(&self) -> View {
        let replayed = self.replayed();
        View {
            position: replayed.setup,
            history: replayed.played,
            own_moves: self.own_moves(),
            correspondence: self.correspondence(),
            rematch: self.rematch(),
        }
    }

    /// A server event of the game stream.
    pub fn event(&mut self, event: &ServerEvent, ctx: &Ctx) -> Outcome {
        let mut run = Run::new(self, ctx);
        run.read_event(event);
        run.done()
    }

    /// The stream connected, failed or ended.
    pub fn connection(&mut self, report: &OnlineConnection, ctx: &Ctx) -> Outcome {
        let mut run = Run::new(self, ctx);
        run.read_connection(report);
        run.done()
    }

    /// Lichess is being asked for a game in progress.
    pub fn checking(&mut self, ctx: &Ctx) -> Outcome {
        let mut run = Run::new(self, ctx);
        run.checking();
        run.done()
    }

    /// Starts a network-backed operation: the state changes it makes before the call, the call,
    /// and what the driver must hand back to `settle`.
    pub fn begin(&mut self, op: Op, ctx: &Ctx) -> Outcome {
        let run = Run::new(self, ctx);
        match op {
            Op::Start(options) => start(run, options),
            Op::Stop => stop(run),
            Op::Reconnect => reconnect(run),
            Op::Open { account, id } => open(run, account, id),
            Op::Move(uci) => move_uci(run, uci),
            Op::Action(action) => act(run, action),
            Op::Chat(id) => chat(run, id),
        }
    }

    /// The core's reply to a call `begin` made.
    pub fn settle(&mut self, pending: &Pending, reply: Reply, ctx: &Ctx) -> Outcome {
        let run = Run::new(self, ctx);
        let value = match &reply {
            Reply::Ok(value) => Some(value),
            Reply::Failed => None,
        };
        let ok = value.is_some();
        match pending {
            Pending::Start {
                generation,
                correspondence,
                previous,
                left,
            } => {
                let current = *generation == run.m.generation;
                settle_start(run, *correspondence, *previous, left, value, current)
            }
            Pending::Stop if !ok => run.rethrow(),
            Pending::Stop => run.done(),
            Pending::Reconnect { generation } => {
                let current = *generation == run.m.generation;
                settle_reconnect(run, value, current)
            }
            Pending::Open {
                generation,
                account,
                id,
            } => {
                let current = *generation == run.m.generation;
                settle_open(run, account, id, ok, current)
            }
            Pending::Move {
                generation,
                id,
                epoch,
                sent,
            } => {
                let current = same_request(&run, *generation, id, *epoch);
                settle_move(run, *sent, ok, current)
            }
            Pending::Action {
                generation,
                id,
                epoch,
                action,
            } => {
                let current = same_request(&run, *generation, id, *epoch);
                settle_action(run, action, ok, current)
            }
            Pending::Chat { id } => settle_chat(run, id, value),
        }
    }
}

/// A seek or challenge. A game being played, a seek in progress, or Lichess
/// still being asked for a game in progress refuses it: the seek cut that check short, and its
/// end put the state back to idle while Lichess held the seek.
fn start(mut run: Run, options: Value) -> Outcome {
    let phase = run.m.online_phase;
    // What `checking` (and the core's report of the check) leaves; a report left over once the
    // phase moved on (a finished game, idle) does not hold seeks back.
    let checking = phase == GamePhase::Disconnected
        && run
            .m
            .online_connection
            .as_ref()
            .is_some_and(OnlineConnection::checking);
    let refused = (phase == GamePhase::Disconnected && !run.m.online_id.is_empty())
        || (phase == GamePhase::Playing && !run.m.correspondence())
        || phase == GamePhase::Seeking
        || checking;
    if refused {
        return run.answer(false);
    }
    let generation = run.next_generation();
    let correspondence = options.get("days").is_some();
    // A real-time seek from a correspondence game being played: the game it leaves, to go back to
    // if the seek fails.
    let left = (phase == GamePhase::Playing).then(|| Left {
        id: run.m.online_id.clone(),
        account: run.m.online_account.clone(),
    });
    if !correspondence {
        run.m.online_id.clear();
        run.m.online_account = options
            .get("account")
            .and_then(Value::as_str)
            .map_or_else(|| run.ctx.active_account.clone(), str::to_string);
        run.m.online_phase = GamePhase::Seeking;
        run.m.online_status = "Looking for an opponent…".into();
    }
    let pending = Pending::Start {
        generation,
        correspondence,
        previous: phase,
        left,
    };
    run.call(Call::StartOnline(options), pending)
}

/// Cancel the seek, and leave a game that is not being played.
fn stop(mut run: Run) -> Outcome {
    run.next_generation();
    if !matches!(
        run.m.online_phase,
        GamePhase::Disconnected | GamePhase::Playing
    ) {
        run.m.online_phase = GamePhase::Idle;
        run.m.online_status.clear();
    }
    run.call(Call::CancelOnline, Pending::Stop)
}

/// Ask Lichess for the game in progress.
fn reconnect(mut run: Run) -> Outcome {
    let generation = run.next_generation();
    run.checking();
    run.call(Call::ResumeOnline, Pending::Reconnect { generation })
}

/// Open one of the account's games on the board.
fn open(mut run: Run, account: String, id: String) -> Outcome {
    let generation = run.next_generation();
    let call = Call::OpenGame {
        account: account.clone(),
        id: id.clone(),
    };
    run.call(
        call,
        Pending::Open {
            generation,
            account,
            id,
        },
    )
}

/// A move on the board. It is sent once; a failed reply is never retried.
fn move_uci(run: Run, uci: String) -> Outcome {
    if run.m.online_id.is_empty() || run.m.online_phase != GamePhase::Playing {
        return run.done();
    }
    let id = run.m.online_id.clone();
    let pending = Pending::Move {
        generation: run.m.generation,
        id: id.clone(),
        epoch: run.m.state_epoch,
        sent: run.ctx.now,
    };
    run.call(Call::PlayOnline { id, uci }, pending)
}

/// Resign, offer or answer a draw or takeback, claim, berserk.
fn act(run: Run, action: String) -> Outcome {
    if run.m.online_id.is_empty() {
        return run.done();
    }
    let id = run.m.online_id.clone();
    let pending = Pending::Action {
        generation: run.m.generation,
        id: id.clone(),
        epoch: run.m.state_epoch,
        action: action.clone(),
    };
    run.call(Call::OnlineAction { id, action }, pending)
}

/// The chat so far. A courtesy: a failure leaves the game running.
fn chat(run: Run, id: String) -> Outcome {
    if !run.ctx.chat_enabled {
        return run.done();
    }
    let pending = Pending::Chat { id: id.clone() };
    run.call(Call::OnlineChat { id }, pending)
}

/// Whether a reply to a move or action still applies: the same request, game and epoch, and the
/// game not over. A late reply once it finished turned the result back into "The move could not
/// be confirmed", or showed a draw or takeback offer on a finished game.
fn same_request(run: &Run, generation: f64, id: &str, epoch: f64) -> bool {
    generation == run.m.generation
        && run.m.online_phase != GamePhase::Finished
        && id == run.m.online_id
        && epoch == run.m.state_epoch
}

fn settle_start(
    mut run: Run,
    correspondence: bool,
    previous: GamePhase,
    left: &Option<Left>,
    value: Option<&Value>,
    current: bool,
) -> Outcome {
    let Some(value) = value else {
        run.warn("[onlineGame] starting game failed:");
        if !current {
            return run.answer(false);
        }
        if !correspondence && run.m.online_phase == GamePhase::Seeking {
            run.m.online_phase = match previous {
                // Back to the correspondence game the seek left, which went on meanwhile.
                GamePhase::Playing if left.is_some() => GamePhase::Playing,
                GamePhase::Finished => GamePhase::Finished,
                _ => GamePhase::Idle,
            };
            if let Some(left) = left {
                run.m.online_id = left.id.clone();
                run.m.online_account = left.account.clone();
            }
            run.m.online_status.clear();
        }
        return run.rethrow();
    };
    if !current {
        return run.answer(false);
    }
    if !correspondence && run.m.online_phase == GamePhase::Seeking {
        run.m.online_status = if js::truthy(value.get("url")) {
            "Challenge sent. Waiting for acceptance…"
        } else {
            "Looking for an opponent…"
        }
        .into();
    }
    run.answer(true)
}

fn settle_reconnect(run: Run, value: Option<&Value>, current: bool) -> Outcome {
    let Some(value) = value else {
        return run.rethrow();
    };
    if current {
        if js::truthy(Some(value)) {
            run.m.online_account = js::to_string(value.get("account"));
            run.m.online_id = js::to_string(value.get("id"));
            run.m.online_status = "Reconnecting…".into();
        } else {
            run.m.online_phase = GamePhase::Idle;
            run.m.online_id.clear();
            run.m.online_status = "No game in progress. You can find another game.".into();
        }
    }
    run.done()
}

fn settle_open(mut run: Run, account: &str, id: &str, ok: bool, current: bool) -> Outcome {
    if !ok {
        return run.rethrow();
    }
    if current {
        if run.m.shown_game != id {
            run.reset_game();
        }
        run.m.shown_game.clear();
        run.m.online_account = account.to_string();
        run.m.online_id = id.to_string();
        run.m.online_status = "Opening game…".into();
        if run.m.online_phase != GamePhase::Playing {
            run.m.online_phase = GamePhase::Disconnected;
        }
    }
    run.done()
}

fn settle_move(mut run: Run, sent: f64, ok: bool, current: bool) -> Outcome {
    if ok {
        if current {
            let ms = js::round(run.ctx.now - sent);
            run.effect(model::Effect::MoveAcknowledged { ms });
        }
        return run.done();
    }
    // The server may have accepted the move before the reply was lost. Never replay it.
    run.warn("[onlineGame] request failed:");
    if !current {
        return run.done();
    }
    run.m.online_phase = GamePhase::Disconnected;
    run.m.online_status =
        "The move could not be confirmed. Reconnect to check the server position.".into();
    run.clock_pause();
    run.effect(model::Effect::Failed);
    run.done()
}

fn settle_action(mut run: Run, action: &str, ok: bool, current: bool) -> Outcome {
    if !ok {
        run.warn("[onlineGame] request failed:");
        run.effect(model::Effect::Failed);
        return run.done();
    }
    if current {
        match action {
            "offerDraw" => run.m.draw_offer = Offer::Mine,
            "declineDraw" | "acceptDraw" => run.m.draw_offer = Offer::None,
            "takeback" => {
                run.m.takeback_offer = if run.m.takeback_offer == Offer::Theirs {
                    Offer::None
                } else {
                    Offer::Mine
                }
            }
            "declineTakeback" => run.m.takeback_offer = Offer::None,
            "berserk" => run.m.berserked = true,
            _ => {}
        }
    }
    run.done()
}

fn settle_chat(mut run: Run, id: &str, value: Option<&Value>) -> Outcome {
    let Some(value) = value else {
        run.warn("[onlineGame] request failed:");
        return run.done();
    };
    if run.m.online_id == id {
        let mut chat: Vec<ChatEntry> = Vec::deserialize(value).unwrap_or_default();
        chat.extend(run.m.chat.iter().filter(|line| !line.players()).cloned());
        run.m.chat = chat;
    }
    run.done()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::model::{ClockConfig, Effect, Setup};
    use super::server::ServerEvent;
    use super::*;

    fn ctx(now: f64) -> Ctx {
        Ctx {
            now,
            active_account: "Alice".into(),
            chat_enabled: true,
        }
    }

    fn seek() -> Op {
        Op::Start(json!({ "minutes": 5, "increment": 0, "color": "random", "rated": false }))
    }

    fn playing() -> Model {
        let mut model = Model::idle();
        model.online_phase = GamePhase::Playing;
        model.online_id = "AbCd1234".into();
        model.online_setup = Setup::standard();
        model
    }

    #[test]
    fn a_seek_is_refused_while_a_game_is_played() {
        let mut model = playing();
        let result = model.begin(seek(), &ctx(0.0));
        assert_eq!(result.ret, Some(false));
        assert!(result.call.is_none());
    }

    /// Lichess is still asked for a game in progress (after signing in): a seek begun then cut
    /// the check short, and the check's end put the state back to idle while Lichess held the
    /// seek. Once Lichess has answered, a seek goes out.
    #[test]
    fn a_seek_waits_for_the_check_for_a_game_in_progress() {
        let mut model = Model::idle();
        model.checking(&ctx(0.0));
        let phase = model.online_phase;
        let refused = model.begin(seek(), &ctx(0.0));
        assert_eq!(refused.ret, Some(false));
        assert!(refused.call.is_none());
        assert_eq!(model.online_phase, phase);
        model.connection(
            &serde_json::from_value(json!({
                "session": 0, "account": "Alice", "gameId": "", "lane": "game", "phase": "idle",
            }))
            .unwrap(),
            &ctx(0.0),
        );
        let begun = model.begin(seek(), &ctx(0.0));
        assert!(matches!(begun.call, Some(Call::StartOnline(_))));
        assert_eq!(model.online_phase, GamePhase::Seeking);
    }

    #[test]
    fn a_seek_reply_after_a_stop_is_ignored() {
        let mut model = Model::idle();
        let begun = model.begin(seek(), &ctx(0.0));
        assert!(matches!(begun.call, Some(Call::StartOnline(_))));
        model.begin(Op::Stop, &ctx(0.0));
        let late = model.settle(
            &begun.pending.unwrap(),
            Reply::Ok(json!({ "url": "https://lichess.org/x" })),
            &ctx(0.0),
        );
        assert_eq!(late.ret, Some(false));
        assert_eq!(model.online_phase, GamePhase::Idle);
    }

    /// A draw offer answered after the game ended showed "offered a draw" on the finished game.
    #[test]
    fn a_late_offer_reply_leaves_a_finished_game_alone() {
        let mut model = playing();
        let begun = model.begin(Op::Action("offerDraw".into()), &ctx(10.0));
        model.online_phase = GamePhase::Finished;
        model.settle(&begun.pending.unwrap(), Reply::Ok(json!(true)), &ctx(20.0));
        assert_eq!(model.draw_offer, Offer::None);
    }

    /// A move whose reply failed after the game ended turned the result into "The move could not
    /// be confirmed. Reconnect…".
    #[test]
    fn a_late_failed_move_leaves_a_finished_game_alone() {
        let mut model = playing();
        let begun = model.begin(Op::Move("e2e4".into()), &ctx(10.0));
        model.online_phase = GamePhase::Finished;
        model.settle(&begun.pending.unwrap(), Reply::Failed, &ctx(20.0));
        assert_eq!(model.online_phase, GamePhase::Finished);
    }

    /// A real-time seek from a correspondence game cleared that game from the board; when the
    /// seek failed, the board went idle and the game was gone from it.
    #[test]
    fn a_failed_seek_goes_back_to_the_correspondence_game() {
        let mut model = playing();
        model.online_days_per_turn = Some(3.0);
        model.online_account = "Alice".into();
        let begun = model.begin(seek(), &ctx(0.0));
        assert_eq!(model.online_phase, GamePhase::Seeking);
        model.settle(&begun.pending.unwrap(), Reply::Failed, &ctx(1.0));
        assert_eq!(model.online_phase, GamePhase::Playing);
        assert_eq!(model.online_id, "AbCd1234");
        assert_eq!(model.online_account, "Alice");
    }

    /// The reply to a move says how long the server took to confirm it.
    #[test]
    fn a_confirmed_move_reports_how_long_the_server_took() {
        let mut model = playing();
        let begun = model.begin(Op::Move("e2e4".into()), &ctx(1000.0));
        let confirmed = model.settle(
            &begun.pending.unwrap(),
            Reply::Ok(json!(true)),
            &ctx(1250.0),
        );
        assert!(
            confirmed
                .effects
                .contains(&Effect::MoveAcknowledged { ms: 250.0 })
        );
    }

    #[test]
    fn an_unconfirmed_move_fails_and_is_not_retried() {
        let mut model = playing();
        let begun = model.begin(Op::Move("e2e4".into()), &ctx(10.0));
        assert_eq!(
            begun.call,
            Some(Call::PlayOnline {
                id: "AbCd1234".into(),
                uci: "e2e4".into()
            })
        );
        let failed = model.settle(&begun.pending.unwrap(), Reply::Failed, &ctx(20.0));
        assert_eq!(model.online_phase, GamePhase::Disconnected);
        assert_eq!(
            failed.effects,
            vec![
                Effect::Warn {
                    message: "[onlineGame] request failed:"
                },
                Effect::ClockPause,
                Effect::Failed
            ]
        );
        let again = model.begin(Op::Move("e2e4".into()), &ctx(30.0));
        assert!(again.call.is_none());
    }

    #[test]
    fn a_finished_game_says_who_won_and_how_in_one_line() {
        use super::events::outcome_line;
        use crate::color::Color::{Black, White};
        assert_eq!(
            outcome_line(Some("white"), "mate", White),
            "You won by checkmate"
        );
        assert_eq!(
            outcome_line(Some("white"), "outoftime", Black),
            "You lost on time"
        );
        assert_eq!(outcome_line(None, "stalemate", White), "Draw by stalemate");
        assert_eq!(outcome_line(None, "draw", Black), "Draw by agreement");
        assert_eq!(outcome_line(None, "aborted", White), "Game aborted");
        assert_eq!(
            outcome_line(Some("black"), "unknownFinish", Black),
            "You won"
        );
        assert_eq!(outcome_line(None, "finished", White), "Game over");
        // A status that contradicts the result is left out rather than stated.
        assert_eq!(outcome_line(None, "mate", White), "Game over");
        assert_eq!(outcome_line(Some("white"), "stalemate", White), "You won");
    }

    /// A 5+5 game: neither clock runs until both sides have moved, as on Lichess. A clock that
    /// ran from the start snapped back to 5:00 when the first move arrived.
    #[test]
    fn the_clocks_start_once_both_sides_have_moved() {
        let mut model = playing();
        model.online_account = "Alice".into();
        model.online_color = crate::color::Color::White;
        let mut ticking_after = |moves: &str| {
            let result = model.event(
                &ServerEvent::read(&json!({
                    "type": "gameState",
                    "moves": moves,
                    "wtime": 300_000,
                    "btime": 300_000,
                    "status": "started",
                })),
                &ctx(0.0),
            );
            result
                .effects
                .into_iter()
                .find_map(|effect| match effect {
                    Effect::ClockSet { ticking, .. } => Some(ticking),
                    _ => None,
                })
                .expect("the clocks are set")
        };
        assert_eq!(ticking_after(""), None, "before White's first move");
        assert_eq!(ticking_after("e2e4"), None, "before Black's first move");
        assert_eq!(ticking_after("e2e4 e7e5"), Some("white"));
    }

    #[test]
    fn a_game_reads_both_players_ratings() {
        let mut model = playing();
        model.online_account = "Alice".into();
        model.event(
            &ServerEvent::read(&json!({
                "type": "gameFull",
                "id": "AbCd1234",
                "white": { "id": "bob", "name": "Bob", "rating": 1590 },
                "black": { "id": "alice", "name": "Alice", "rating": 1580 },
                "state": { "moves": "" },
            })),
            &ctx(0.0),
        );
        assert_eq!(model.online_color, crate::color::Color::Black);
        assert_eq!(model.online_rating, Some(1580.0));
        assert_eq!(model.online_opponent_rating, Some(1590.0));
    }

    #[test]
    fn a_finished_game_announces_its_result_and_is_kept_for_a_rematch() {
        let mut model = playing();
        model.opponent_id = "bob".into();
        model.online_opponent = "Bob".into();
        model.online_color = crate::color::Color::White;
        model.online_account = "Alice".into();
        model.online_clock_config = Some(ClockConfig {
            initial: 300.0,
            increment: 3.0,
        });
        let context = ctx(0.0);
        let mut run = Run::new(&mut model, &context);
        run.finish(Some("black"), "resign", true);
        let result = run.done();
        assert_eq!(model.online_phase, GamePhase::Finished);
        assert_eq!(model.online_status, "You lost by resignation");
        // Exports and saved games need the result itself, not only the sentence.
        assert_eq!(model.online_result, "0-1");
        assert_eq!(
            result.effects[1],
            Effect::Notify {
                kind: crate::misc::constants::NotificationKind::GameEvents,
                title: "You lost".into(),
                body: "Against Bob · resign.".into(),
            }
        );
        let rematch = model.view().rematch.unwrap();
        assert_eq!(rematch["color"], json!("black"));
        assert_eq!(rematch["minutes"], json!(5.0));
        assert_eq!(rematch["increment"], json!(3.0));
    }
}
