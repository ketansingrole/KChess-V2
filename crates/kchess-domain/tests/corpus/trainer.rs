//! The puzzle session's transitions and the analysis tree's helpers.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use kchess_domain::position;
use kchess_domain::trainer::analysis_tree::{self as tree, MoveInput, Score};
use kchess_domain::trainer::puzzle_session::{
    self as session, Detail, PuzzleSession, Reply, Resume, Selection, Step,
};

use super::{Out, arg};

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    puzzle_session(method, args).or_else(|| analysis_tree(method, args))
}

/// The session's transitions as the recorded suites called them: `{ state, internal, day, … }`
/// in, `{ state, internal, call, record, warn }` out.
fn puzzle_session(method: &str, args: &[Value]) -> Option<Out> {
    let input = args.first().unwrap_or(&Value::Null);
    Some(match method {
        "puzzleSessionInitial" => {
            serde_json::to_value(PuzzleSession::new()).map_err(|e| e.to_string())
        }
        "isReconnect" => Ok(json!(args.first().is_some_and(session::is_reconnect))),
        "puzzleSessionInvalidate" => step(input, |session, _, day| Ok(session.invalidate(day))),
        "puzzleSessionLoadBegin" => step(input, |session, arg, day| {
            let selection: Selection = take(arg, "selection")?;
            let online = arg.get("online").and_then(Value::as_bool).unwrap_or(false);
            Ok(session.load_begin(&selection, online, day))
        }),
        "puzzleSessionLoadSettled" => step(input, |session, arg, day| {
            let token: Resume = take(arg, "resume")?;
            let now: Selection = take(arg, "selection")?;
            let outcome = reply(arg)?;
            Ok(session.load_settled(token, outcome, &now, day))
        }),
        "puzzleSessionRetry" => step(input, |session, arg, day| {
            let puzzle = arg.get("puzzle").cloned().unwrap_or(Value::Null);
            let selection: Selection = take(arg, "selection")?;
            Ok(session.retry(puzzle, &selection, day))
        }),
        "puzzleSessionShow" => step(input, |session, arg, day| {
            let puzzle = arg.get("puzzle").cloned().unwrap_or(Value::Null);
            let selection: Selection = take(arg, "selection")?;
            Ok(session.show(puzzle, &selection, day))
        }),
        "puzzleSessionReportBegin" => step(input, |session, arg, _| {
            let win = arg.get("win").and_then(Value::as_bool).unwrap_or(false);
            let detail: Option<Detail> = take(arg, "detail")?;
            let selection: Selection = take(arg, "selection")?;
            Ok(session.report_begin(win, detail, &selection))
        }),
        "puzzleSessionMove" => step(input, |session, arg, _| {
            let index = arg.get("index").and_then(Value::as_u64).unwrap_or(u64::MAX);
            let correct = arg.get("correct").and_then(Value::as_bool).unwrap_or(false);
            let uci = arg.get("uci").and_then(Value::as_str);
            Ok(session.played(usize::try_from(index).unwrap_or(usize::MAX), correct, uci))
        }),
        "puzzleSessionEnd" => step(input, |session, _, day| Ok(session.end(day))),
        "puzzleSessionReportSettled" => step(input, |session, arg, _| {
            let token: Resume = take(arg, "resume")?;
            let now: Selection = take(arg, "selection")?;
            let outcome = reply(arg)?;
            Ok(session.report_settled(token, outcome, &now))
        }),
        "puzzleSessionResetSession" => step(input, |session, _, _| Ok(session.reset_session())),
        _ => return None,
    })
}

fn take<T: DeserializeOwned>(arg: &Map<String, Value>, key: &str) -> Result<T, String> {
    serde_json::from_value(arg.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|error| format!("{key}: {error}"))
}

/// A reply as `{ ok: value }` or `{ error: message }`; an unreadable shape is an error.
fn reply(arg: &Map<String, Value>) -> Result<Reply, String> {
    let outcome = arg
        .get("outcome")
        .and_then(Value::as_object)
        .ok_or("outcome must be an object")?;
    if let Some(text) = outcome.get("error") {
        return Ok(Err(text.as_str().unwrap_or_default().to_string()));
    }
    Ok(Ok(outcome.get("ok").cloned().unwrap_or(Value::Null)))
}

/// One step over the `{ state, internal, day, … }` input.
fn step(
    input: &Value,
    body: impl FnOnce(&mut PuzzleSession, &Map<String, Value>, &str) -> Result<Step, String>,
) -> Out {
    let arg = input
        .as_object()
        .ok_or("the session input must be an object")?;
    let _: session::State = take(arg, "state")?;
    let mut session: PuzzleSession = serde_json::from_value(json!({
        "state": arg.get("state"),
        "internal": arg.get("internal").cloned().unwrap_or(Value::Null),
    }))
    .map_err(|error| format!("internal: {error}"))?;
    let day = arg.get("day").and_then(Value::as_str).unwrap_or_default();
    let step = body(&mut session, arg, day)?;
    let mut out = serde_json::to_value(&session).map_err(|e| e.to_string())?;
    let step = step_json(step)?;
    if let (Some(out), Some(step)) = (out.as_object_mut(), step.as_object()) {
        out.extend(step.clone());
    }
    Ok(out)
}

/// A step as the recorded suites wrote it: its call by method name and argument.
fn step_json(step: Step) -> Result<Value, String> {
    let call = match step.call {
        None => Value::Null,
        Some(call) => {
            let (method, args) = match &call.request {
                session::Request::LocalPuzzles(query) => ("localPuzzles", json!(query)),
                session::Request::PuzzleNext(draw) => ("puzzleNext", json!(draw)),
                session::Request::PuzzleSolve(solve) => ("puzzleSolve", json!(solve)),
            };
            let resume = serde_json::to_value(&call.resume).map_err(|e| e.to_string())?;
            json!({ "method": method, "args": args, "resume": resume })
        }
    };
    Ok(json!({ "call": call, "record": step.record, "warn": step.warn }))
}

fn number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64)
}

/// A JSON array of numbers; absent or null is empty.
fn numbers(value: Option<&Value>) -> Vec<f64> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default()
}

fn texts(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Child indices along a path, as unsigned numbers.
fn indices(value: Option<&Value>) -> Vec<u64> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default()
}

fn text(value: Option<&Value>) -> &str {
    value.and_then(Value::as_str).unwrap_or("")
}

fn add_move_plan(input: Option<&Value>) -> Out {
    let input = input
        .and_then(Value::as_object)
        .ok_or("addMove input must be an object")?;
    let field = |key: &str| text(input.get(key));
    let child_ucis = texts(input.get("childUcis"));
    let plan = tree::add_move_plan(&MoveInput {
        path: field("path"),
        uci: field("uci"),
        fen: field("fen"),
        ply: number(input.get("ply")).unwrap_or(f64::NAN),
        child_ucis: &child_ucis,
        variant: super::variant(field("variant"))?,
    });
    serde_json::to_value(plan).map_err(|e| e.to_string())
}

fn analysis_tree(method: &str, args: &[Value]) -> Option<Out> {
    let arg = |i: usize| arg(args, i);
    let score = |i: usize| arg(i).and_then(Score::of_line);
    let result = match method {
        "moveGlyph" => Ok(json!(tree::move_glyph(&numbers(arg(0))))),
        "setMoveGlyph" => Ok(json!(tree::set_move_glyph(
            &numbers(arg(0)),
            number(arg(1))
        ))),
        "pathOf" => Ok(json!(tree::path_of(&texts(arg(0))))),
        "movesOf" => Ok(json!(tree::moves_of(text(arg(0))))),
        "moveNumber" => Ok(json!(tree::move_number(
            number(arg(0)).unwrap_or(f64::NAN),
            arg(1).and_then(Value::as_bool).unwrap_or(false)
        ))),
        "formatEval" => Ok(json!(tree::format_eval(score(0)))),
        "winningChances" => Ok(json!(tree::winning_chances(score(0)))),
        "addMovePlan" => add_move_plan(arg(0)),
        "lineEnd" => Ok(json!(tree::line_end(text(arg(0)), &texts(arg(1))))),
        "deleteAtPlan" => Ok(json!(tree::delete_at_plan(text(arg(0))))),
        "onMainline" => Ok(json!(tree::on_mainline(&indices(arg(0))))),
        "promotePlan" => Ok(json!(tree::promote_plan(&indices(arg(0))))),
        "commentText" => Ok(json!(tree::comment_text(text(arg(0))))),
        "withCommentText" => Ok(json!(tree::with_comment_text(text(arg(0)), text(arg(1))))),
        "commentClock" => Ok(json!(position::comment_clock(text(arg(0))))),
        _ => return None,
    };
    Some(result)
}
