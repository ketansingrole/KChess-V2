//! The online game's transitions, by the names the recorded suites called them.

use serde_json::Value;

use kchess_domain::js;
use kchess_domain::online_game::{self, model::Run};

use super::Out;

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "onlineGameView" => online_game::view(args),
        "onlineGameEvent" => online_game::event(args),
        "onlineGameConnection" => online_game::connection(args),
        "onlineGameChecking" => online_game::checking(args),
        "onlineGameReset" => reset(args),
        "onlineGameFinish" => finish(args),
        "onlineGameBegin" => online_game::begin(args),
        "onlineGameSettle" => online_game::settle(args),
        _ => return None,
    })
}

static NULL: Value = Value::Null;

fn argument(args: &[Value], i: usize) -> &Value {
    args.get(i).unwrap_or(&NULL)
}

/// The state at 0 as the rules read it, with the context the host supplied at `i`.
fn run(args: &[Value], i: usize) -> Result<Run, String> {
    fn read<T: serde::de::DeserializeOwned>(args: &[Value], i: usize) -> Result<T, String> {
        serde_json::from_value(argument(args, i).clone()).map_err(|e| e.to_string())
    }
    Ok(Run::new(read(args, 0)?, read(args, i)?))
}

/// The board left without its game.
fn reset(args: &[Value]) -> Out {
    let mut run = run(args, 1)?;
    run.reset_game();
    run.done(None, None, None, false)
}

/// The game finished by `{ winner, reason, notify }`.
fn finish(args: &[Value]) -> Out {
    let mut run = run(args, 2)?;
    let input = argument(args, 1);
    let reason = js::to_string(input.get("reason"));
    let notify = js::truthy(input.get("notify"));
    run.finish(js::string(input.get("winner")), &reason, notify);
    run.done(None, None, None, false)
}
