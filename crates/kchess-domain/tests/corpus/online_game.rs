//! The online game's transitions, by the names the recorded suites called them: `[state, …,
//! context]` in, `{ state, effects, ret?, call?, pending?, rethrow }` out.

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use kchess_domain::js;
use kchess_domain::online_game::model::{Call, Ctx, Left, Model, Op, Outcome, Pending, Reply, Run};
use kchess_domain::online_game::server::ServerEvent;

use super::Out;

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    Some(match method {
        "onlineGameView" => read::<Model>(args, 0).and_then(|model| json_of(&model.view())),
        "onlineGameEvent" => step(args, 2, |model, ctx| {
            Ok(model.event(&ServerEvent::read(argument(args, 1)), ctx))
        }),
        "onlineGameConnection" => step(args, 2, |model, ctx| {
            Ok(model.connection(argument(args, 1), ctx))
        }),
        "onlineGameChecking" => step(args, 1, |model, ctx| Ok(model.checking(ctx))),
        "onlineGameReset" => step(args, 1, |model, ctx| {
            let mut run = Run::new(model, ctx);
            run.reset_game();
            Ok(run.done())
        }),
        "onlineGameFinish" => step(args, 2, |model, ctx| {
            let input = argument(args, 1);
            let reason = js::to_string(input.get("reason"));
            let notify = js::truthy(input.get("notify"));
            let mut run = Run::new(model, ctx);
            run.finish(js::string(input.get("winner")), &reason, notify);
            Ok(run.done())
        }),
        "onlineGameBegin" => step(args, 3, |model, ctx| {
            Ok(model.begin(op(argument(args, 1), argument(args, 2))?, ctx))
        }),
        "onlineGameSettle" => step(args, 3, |model, ctx| {
            let pending = pending(argument(args, 1))?;
            let outcome = argument(args, 2);
            let reply = if outcome.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                Reply::Ok(outcome.get("value").cloned().unwrap_or(Value::Null))
            } else {
                Reply::Failed
            };
            Ok(model.settle(&pending, reply, ctx))
        }),
        _ => return None,
    })
}

static NULL: Value = Value::Null;

fn argument(args: &[Value], i: usize) -> &Value {
    args.get(i).unwrap_or(&NULL)
}

fn read<T: DeserializeOwned>(args: &[Value], i: usize) -> Result<T, String> {
    serde_json::from_value(argument(args, i).clone()).map_err(|e| e.to_string())
}

fn json_of(value: &impl serde::Serialize) -> Out {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// One transition of the state at 0 with the context at `i`, as the recorded shape.
fn step(
    args: &[Value],
    i: usize,
    body: impl FnOnce(&mut Model, &Ctx) -> Result<Outcome, String>,
) -> Out {
    let mut model: Model = read(args, 0)?;
    let ctx: Ctx = read(args, i)?;
    let outcome = body(&mut model, &ctx)?;
    let mut out = json_of(&outcome)?;
    if let Some(call) = &outcome.call {
        let (method, args) = method_and_args(call);
        out["call"] = json!({ "method": method, "args": args });
    }
    out["state"] = json_of(&model)?;
    Ok(out)
}

/// A call as the recorded suites wrote it: the core method's name and arguments.
fn method_and_args(call: &Call) -> (&'static str, Vec<Value>) {
    match call {
        Call::StartOnline(options) => ("startOnline", vec![options.clone()]),
        Call::CancelOnline => ("cancelOnline", Vec::new()),
        Call::ResumeOnline => ("resumeOnline", Vec::new()),
        Call::OpenGame { account, id } => ("openGame", vec![json!(account), json!(id)]),
        Call::PlayOnline { id, uci } => ("playOnline", vec![json!(id), json!(uci)]),
        Call::OnlineAction { id, action } => ("onlineAction", vec![json!(id), json!(action)]),
        Call::OnlineChat { id } => ("onlineChat", vec![json!(id)]),
    }
}

fn text(value: Option<&Value>) -> String {
    js::string(value).unwrap_or("").to_string()
}

fn unknown(op: &str) -> String {
    format!("unknown online game operation {op}")
}

/// `op` with its input as the suites passed them.
fn op(name: &Value, input: &Value) -> Result<Op, String> {
    Ok(match js::string(Some(name)).unwrap_or("") {
        "start" => Op::Start(input.clone()),
        "stop" => Op::Stop,
        "reconnect" => Op::Reconnect,
        "open" => Op::Open {
            account: text(input.get("account")),
            id: text(input.get("id")),
        },
        "move" => Op::Move(text(Some(input))),
        "action" => Op::Action(text(Some(input))),
        "chat" => Op::Chat(text(Some(input))),
        other => return Err(unknown(other)),
    })
}

/// A pending token as the suites handed it back. A token without its request's identity never
/// matches the current one.
fn pending(token: &Value) -> Result<Pending, String> {
    let number = |key: &str| token.get(key).and_then(Value::as_f64).unwrap_or(f64::NAN);
    let generation = number("generation");
    // A move or action without a game id is not the current request.
    let request = |generation: f64| match js::string(token.get("id")) {
        Some(_) => generation,
        None => f64::NAN,
    };
    Ok(match js::string(token.get("op")).unwrap_or("") {
        "start" => Pending::Start {
            generation,
            correspondence: token
                .get("correspondence")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            previous: text(token.get("previous")),
            left: token
                .get("left")
                .filter(|left| left.is_object())
                .map(|left| Left {
                    id: text(left.get("id")),
                    account: text(left.get("account")),
                }),
        },
        "stop" => Pending::Stop,
        "reconnect" => Pending::Reconnect { generation },
        "open" => Pending::Open {
            generation,
            account: text(token.get("account")),
            id: text(token.get("id")),
        },
        "move" => Pending::Move {
            generation: request(generation),
            id: text(token.get("id")),
            epoch: number("epoch"),
            sent: token.get("sent").and_then(Value::as_f64).unwrap_or(0.0),
        },
        "action" => Pending::Action {
            generation: request(generation),
            id: text(token.get("id")),
            epoch: number("epoch"),
            action: text(token.get("action")),
        },
        "chat" => Pending::Chat {
            id: text(token.get("id")),
        },
        other => return Err(unknown(other)),
    })
}
