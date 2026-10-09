//! `core/src/domain/onlineGame.ts`: how a Lichess game on the board changes with each server
//! event, connection report and network reply (see RUST_MIGRATION.md, "Porting a stateful module").
//! The class keeps the state object and executes the effects; every decision lives here, including
//! which call to make and how its reply lands. Each method takes the state, its input, and the
//! context the host read at that moment (`now`, `activeAccount`, `chatEnabled`) as the last argument.

mod events;
mod model;

use serde_json::{Value, json};

use crate::js;
use model::{Call, Ctx, Model, Run, at, js_round, string_or, text_of, truthy};

type Out = Result<Value, String>;

static NULL: Value = Value::Null;

/// This module's methods for `api::call`; None when the method is not one of them.
pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "onlineGameView" => view(args),
        "onlineGameEvent" => event(args),
        "onlineGameConnection" => connection(args),
        "onlineGameChecking" => checking(args),
        "onlineGameReset" => reset(args),
        "onlineGameFinish" => finish(args),
        "onlineGameBegin" => begin(args),
        "onlineGameSettle" => settle(args),
        _ => return None,
    })
}

/// Argument `i`; a missing argument reads as null.
fn argument(args: &[Value], i: usize) -> &Value {
    args.get(i).unwrap_or(&NULL)
}

fn state(args: &[Value]) -> Result<Model, String> {
    serde_json::from_value(argument(args, 0).clone()).map_err(|e| e.to_string())
}

fn context(args: &[Value], i: usize) -> Result<Ctx, String> {
    serde_json::from_value(argument(args, i).clone()).map_err(|e| e.to_string())
}

/// The state as the rules read it, with the context the host supplied at `i`.
fn run(args: &[Value], i: usize) -> Result<Run, String> {
    Ok(Run::new(state(args)?, context(args, i)?))
}

/// `position`, `history`, `ownMoves`, `correspondence` and `rematchOptions` for the state.
fn view(args: &[Value]) -> Out {
    let m = state(args)?;
    let replayed = m.replayed();
    Ok(json!({
        "position": { "variant": replayed.setup.variant, "fen": replayed.setup.fen },
        "history": replayed.played,
        "ownMoves": m.own_moves(),
        "correspondence": m.correspondence(),
        "rematch": m.rematch(),
    }))
}

fn event(args: &[Value]) -> Out {
    let mut run = run(args, 2)?;
    run.read_event(argument(args, 1));
    run.done(None, None, None, false)
}

fn connection(args: &[Value]) -> Out {
    let mut run = run(args, 2)?;
    run.read_connection(argument(args, 1));
    run.done(None, None, None, false)
}

fn checking(args: &[Value]) -> Out {
    let mut run = run(args, 1)?;
    run.checking();
    run.done(None, None, None, false)
}

fn reset(args: &[Value]) -> Out {
    let mut run = run(args, 1)?;
    run.reset_game();
    run.done(None, None, None, false)
}

/// `finish(winner, reason, notify)`: the public call, with its arguments in one object.
fn finish(args: &[Value]) -> Out {
    let mut run = run(args, 2)?;
    let input = argument(args, 1);
    let reason = js::to_string(input.get("reason"));
    let notify = truthy(input.get("notify"));
    run.finish(text_of(input.get("winner")), &reason, notify);
    run.done(None, None, None, false)
}

/// Starts a network-backed operation: the state changes it makes before the call, the call, and
/// what the driver must hand back to `settle`. `op` names the public method; `input` is its argument.
fn begin(args: &[Value]) -> Out {
    let run = run(args, 3)?;
    let input = argument(args, 2);
    match text_of(Some(argument(args, 1))).unwrap_or("") {
        "start" => start(run, input),
        "stop" => stop(run),
        "reconnect" => reconnect(run),
        "open" => open(run, input),
        "move" => move_uci(run, input),
        "action" => action(run, input),
        "chat" => chat(run, input),
        other => Err(format!("unknown online game operation {other}")),
    }
}

/// `start(options)`: a seek or challenge. A game being played, or a seek in progress, refuses it.
fn start(mut run: Run, options: &Value) -> Out {
    let phase = run.m.online_phase.clone();
    let refused = (phase == "disconnected" && !run.m.online_id.is_empty())
        || (phase == "playing" && !run.m.correspondence())
        || phase == "seeking";
    if refused {
        return run.done(Some(json!(false)), None, None, false);
    }
    let generation = run.next_generation();
    let correspondence = options.get("days").is_some();
    if !correspondence {
        run.m.online_id.clear();
        run.m.online_account = string_or(options.get("account"), &run.ctx.active_account);
        run.m.online_phase = "seeking".into();
        run.m.online_status = "Looking for an opponent…".into();
    }
    let pending = json!({
        "op": "start",
        "generation": generation,
        "correspondence": correspondence,
        "previous": phase,
    });
    run.done(
        None,
        Some(Call::new("startOnline", vec![options.clone()])),
        Some(pending),
        false,
    )
}

/// `stop()`: cancel the seek, and leave a game that is not being played.
fn stop(mut run: Run) -> Out {
    run.next_generation();
    if run.m.online_phase != "disconnected" && run.m.online_phase != "playing" {
        run.m.online_phase = "idle".into();
        run.m.online_status.clear();
    }
    run.done(
        None,
        Some(Call::new("cancelOnline", Vec::new())),
        Some(json!({ "op": "stop" })),
        false,
    )
}

/// `reconnect()`: ask Lichess for the game in progress.
fn reconnect(mut run: Run) -> Out {
    let generation = run.next_generation();
    run.checking();
    run.done(
        None,
        Some(Call::new("resumeOnline", Vec::new())),
        Some(json!({ "op": "reconnect", "generation": generation })),
        false,
    )
}

/// `open(account, id)`: open one of the account's games on the board.
fn open(mut run: Run, input: &Value) -> Out {
    let generation = run.next_generation();
    let account = text_of(input.get("account")).unwrap_or("").to_string();
    let id = text_of(input.get("id")).unwrap_or("").to_string();
    run.done(
        None,
        Some(Call::new(
            "openGame",
            vec![account.clone().into(), id.clone().into()],
        )),
        Some(json!({ "op": "open", "generation": generation, "account": account, "id": id })),
        false,
    )
}

/// `onlineMove(uci)`: a move on the board. It is sent once; a failed reply is never retried.
fn move_uci(run: Run, input: &Value) -> Out {
    if run.m.online_id.is_empty() || run.m.online_phase != "playing" {
        return run.done(None, None, None, false);
    }
    let id = run.m.online_id.clone();
    let uci = text_of(Some(input)).unwrap_or("").to_string();
    let pending = json!({
        "op": "move",
        "generation": run.m.generation,
        "id": id,
        "epoch": run.m.state_epoch,
        "sent": run.ctx.now,
    });
    run.done(
        None,
        Some(Call::new("playOnline", vec![id.into(), uci.into()])),
        Some(pending),
        false,
    )
}

/// `onlineAction(action)`: resign, offer or answer a draw or takeback, claim, berserk.
fn action(run: Run, input: &Value) -> Out {
    if run.m.online_id.is_empty() {
        return run.done(None, None, None, false);
    }
    let id = run.m.online_id.clone();
    let action = text_of(Some(input)).unwrap_or("").to_string();
    let pending = json!({
        "op": "action",
        "generation": run.m.generation,
        "id": id,
        "epoch": run.m.state_epoch,
        "action": action,
    });
    run.done(
        None,
        Some(Call::new("onlineAction", vec![id.into(), action.into()])),
        Some(pending),
        false,
    )
}

/// `loadChat(id)`: the chat so far. A courtesy: a failure leaves the game running.
fn chat(run: Run, input: &Value) -> Out {
    if !run.ctx.chat_enabled {
        return run.done(None, None, None, false);
    }
    let id = text_of(Some(input)).unwrap_or("").to_string();
    let pending = json!({ "op": "chat", "id": id });
    run.done(
        None,
        Some(Call::new("onlineChat", vec![id.into()])),
        Some(pending),
        false,
    )
}

/// The network reply to a call `begin` made: `outcome` is `{ ok, value }`, or `{ ok: false }`
/// when the call threw (the driver keeps the cause for the effects and any rethrow).
fn settle(args: &[Value]) -> Out {
    let run = run(args, 3)?;
    let pending = argument(args, 1);
    let outcome = argument(args, 2);
    let ok = outcome.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let value = outcome.get("value");
    let current = pending.get("generation").and_then(Value::as_f64) == Some(run.m.generation);
    match text_of(pending.get("op")).unwrap_or("") {
        "start" => settle_start(run, pending, ok, value, current),
        "stop" => {
            if !ok {
                return run.done(None, None, None, true);
            }
            run.done(None, None, None, false)
        }
        "reconnect" => settle_reconnect(run, ok, value, current),
        "open" => settle_open(run, pending, ok, current),
        "move" => settle_move(run, pending, ok, current),
        "action" => settle_action(run, pending, ok, current),
        "chat" => settle_chat(run, pending, ok, value),
        other => Err(format!("unknown online game operation {other}")),
    }
}

/// Whether a reply to a move or action still applies: the same request, game and epoch.
fn same_request(run: &Run, pending: &Value, current: bool) -> bool {
    current
        && text_of(pending.get("id")) == Some(run.m.online_id.as_str())
        && pending.get("epoch").and_then(Value::as_f64) == Some(run.m.state_epoch)
}

fn settle_start(
    mut run: Run,
    pending: &Value,
    ok: bool,
    value: Option<&Value>,
    current: bool,
) -> Out {
    let correspondence = pending
        .get("correspondence")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !ok {
        run.warn("[onlineGame] starting game failed:");
        if !current {
            return run.done(Some(json!(false)), None, None, false);
        }
        if !correspondence && run.m.online_phase == "seeking" {
            run.m.online_phase = if text_of(pending.get("previous")) == Some("finished") {
                "finished"
            } else {
                "idle"
            }
            .into();
            run.m.online_status.clear();
        }
        return run.done(None, None, None, true);
    }
    if !current {
        return run.done(Some(json!(false)), None, None, false);
    }
    if !correspondence && run.m.online_phase == "seeking" {
        run.m.online_status = if truthy(at(value, "url")) {
            "Challenge sent. Waiting for acceptance…"
        } else {
            "Looking for an opponent…"
        }
        .into();
    }
    run.done(Some(json!(true)), None, None, false)
}

fn settle_reconnect(mut run: Run, ok: bool, value: Option<&Value>, current: bool) -> Out {
    if !ok {
        return run.done(None, None, None, true);
    }
    if current {
        match value.filter(|resumed| truthy(Some(*resumed))) {
            Some(resumed) => {
                run.m.online_account = js::to_string(resumed.get("account"));
                run.m.online_id = js::to_string(resumed.get("id"));
                run.m.online_status = "Reconnecting…".into();
            }
            None => {
                run.m.online_phase = "idle".into();
                run.m.online_id.clear();
                run.m.online_status = "No game in progress. You can find another game.".into();
            }
        }
    }
    run.done(None, None, None, false)
}

fn settle_open(mut run: Run, pending: &Value, ok: bool, current: bool) -> Out {
    if !ok {
        return run.done(None, None, None, true);
    }
    if current {
        let id = text_of(pending.get("id")).unwrap_or("").to_string();
        if run.m.shown_game != id {
            run.reset_game();
        }
        run.m.shown_game.clear();
        run.m.online_account = text_of(pending.get("account")).unwrap_or("").to_string();
        run.m.online_id = id;
        run.m.online_status = "Opening game…".into();
        if run.m.online_phase != "playing" {
            run.m.online_phase = "disconnected".into();
        }
    }
    run.done(None, None, None, false)
}

fn settle_move(mut run: Run, pending: &Value, ok: bool, current: bool) -> Out {
    let current = same_request(&run, pending, current);
    if ok {
        if current {
            let sent = pending.get("sent").and_then(Value::as_f64).unwrap_or(0.0);
            let ms = js_round(run.ctx.now - sent);
            run.effect(model::Effect::MoveAcknowledged { ms });
        }
        return run.done(None, None, None, false);
    }
    // The server may have accepted the move before the reply was lost. Never replay it.
    run.warn("[onlineGame] request failed:");
    if !current {
        return run.done(None, None, None, false);
    }
    run.m.online_phase = "disconnected".into();
    run.m.online_status =
        "The move could not be confirmed. Reconnect to check the server position.".into();
    run.clock_pause();
    run.effect(model::Effect::Failed);
    run.done(None, None, None, false)
}

fn settle_action(mut run: Run, pending: &Value, ok: bool, current: bool) -> Out {
    if !ok {
        run.warn("[onlineGame] request failed:");
        run.effect(model::Effect::Failed);
        return run.done(None, None, None, false);
    }
    if same_request(&run, pending, current) {
        match text_of(pending.get("action")).unwrap_or("") {
            "offerDraw" => run.m.draw_offer = "mine".into(),
            "declineDraw" | "acceptDraw" => run.m.draw_offer = "none".into(),
            "takeback" => {
                run.m.takeback_offer = if run.m.takeback_offer == "theirs" {
                    "none"
                } else {
                    "mine"
                }
                .into()
            }
            "declineTakeback" => run.m.takeback_offer = "none".into(),
            "berserk" => run.m.berserked = true,
            _ => {}
        }
    }
    run.done(None, None, None, false)
}

fn settle_chat(mut run: Run, pending: &Value, ok: bool, value: Option<&Value>) -> Out {
    if !ok {
        run.warn("[onlineGame] request failed:");
        return run.done(None, None, None, false);
    }
    let id = text_of(pending.get("id")).unwrap_or("");
    if run.m.online_id == id {
        let mut chat: Vec<Value> = value.and_then(Value::as_array).cloned().unwrap_or_default();
        chat.extend(
            run.m
                .chat
                .iter()
                .filter(|line| line.get("room") != Some(&Value::from("player")))
                .cloned(),
        );
        run.m.chat = chat;
    }
    run.done(None, None, None, false)
}

#[cfg(test)]
mod tests {
    use super::model::Setup;
    use super::*;

    fn state_json(model: &Model) -> Value {
        serde_json::to_value(model).unwrap()
    }

    fn ctx(now: f64) -> Value {
        json!({ "now": now, "activeAccount": "Alice", "chatEnabled": true })
    }

    fn transition(method: &str, args: Vec<Value>) -> Value {
        call(method, &args).unwrap().unwrap()
    }

    fn effects(result: &Value) -> Vec<Value> {
        result["effects"].as_array().unwrap().clone()
    }

    #[test]
    fn a_seek_is_refused_while_a_game_is_played() {
        let mut model = Model::idle();
        model.online_phase = "playing".into();
        model.online_id = "AbCd1234".into();
        let result = transition(
            "onlineGameBegin",
            vec![
                state_json(&model),
                json!("start"),
                json!({ "minutes": 5, "increment": 0, "color": "random", "rated": false }),
                ctx(0.0),
            ],
        );
        assert_eq!(result["ret"], json!(false));
        assert!(result.get("call").is_none());
    }

    #[test]
    fn a_seek_reply_after_a_stop_is_ignored() {
        let model = Model::idle();
        let begun = transition(
            "onlineGameBegin",
            vec![
                state_json(&model),
                json!("start"),
                json!({ "minutes": 5, "increment": 0, "color": "random", "rated": false }),
                ctx(0.0),
            ],
        );
        assert_eq!(begun["call"]["method"], json!("startOnline"));
        let stopped = transition(
            "onlineGameBegin",
            vec![begun["state"].clone(), json!("stop"), Value::Null, ctx(0.0)],
        );
        let late = transition(
            "onlineGameSettle",
            vec![
                stopped["state"].clone(),
                begun["pending"].clone(),
                json!({ "ok": true, "value": { "url": "https://lichess.org/x" } }),
                ctx(0.0),
            ],
        );
        assert_eq!(late["ret"], json!(false));
        assert_eq!(late["state"]["onlinePhase"], json!("idle"));
    }

    #[test]
    fn an_unconfirmed_move_fails_and_is_not_retried() {
        let mut model = Model::idle();
        model.online_phase = "playing".into();
        model.online_id = "AbCd1234".into();
        model.online_setup = Setup::standard();
        let begun = transition(
            "onlineGameBegin",
            vec![state_json(&model), json!("move"), json!("e2e4"), ctx(10.0)],
        );
        assert_eq!(begun["call"]["args"], json!(["AbCd1234", "e2e4"]));
        let failed = transition(
            "onlineGameSettle",
            vec![
                begun["state"].clone(),
                begun["pending"].clone(),
                json!({ "ok": false }),
                ctx(20.0),
            ],
        );
        assert_eq!(failed["state"]["onlinePhase"], json!("disconnected"));
        let names: Vec<Value> = effects(&failed)
            .iter()
            .map(|e| e["effect"].clone())
            .collect();
        assert_eq!(
            names,
            vec![json!("warn"), json!("clockPause"), json!("failed")]
        );
        let again = transition(
            "onlineGameBegin",
            vec![
                failed["state"].clone(),
                json!("move"),
                json!("e2e4"),
                ctx(30.0),
            ],
        );
        assert!(again.get("call").is_none());
    }

    #[test]
    fn a_finished_game_announces_its_result_and_is_kept_for_a_rematch() {
        let mut model = Model::idle();
        model.online_phase = "playing".into();
        model.online_id = "AbCd1234".into();
        model.opponent_id = "bob".into();
        model.online_opponent = "Bob".into();
        model.online_color = "white".into();
        model.online_account = "Alice".into();
        model.online_clock_config = Some(model::ClockConfig {
            initial: 300.0,
            increment: 3.0,
        });
        let result = transition(
            "onlineGameFinish",
            vec![
                state_json(&model),
                json!({ "winner": "black", "reason": "resign", "notify": true }),
                ctx(0.0),
            ],
        );
        assert_eq!(result["state"]["onlinePhase"], json!("finished"));
        assert_eq!(result["state"]["onlineStatus"], json!("Game ended: resign"));
        assert_eq!(
            effects(&result)[1],
            json!({
                "effect": "notify",
                "kind": "gameEvents",
                "title": "You lost",
                "body": "Against Bob · resign.",
            })
        );
        let view = call("onlineGameView", &[result["state"].clone()])
            .unwrap()
            .unwrap();
        assert_eq!(view["rematch"]["color"], json!("black"));
        assert_eq!(view["rematch"]["minutes"], json!(5.0));
        assert_eq!(view["rematch"]["increment"], json!(3.0));
    }
}
