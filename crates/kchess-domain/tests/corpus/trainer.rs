//! The puzzle session's transitions and the analysis tree's helpers.

use serde_json::{Value, json};

use kchess_domain::position;
use kchess_domain::trainer::analysis_tree::{self as tree, MoveInput, Score};
use kchess_domain::trainer::puzzle_session as session;

use super::{Out, arg};

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    puzzle_session(method, args).or_else(|| analysis_tree(method, args))
}

fn puzzle_session(method: &str, args: &[Value]) -> Option<Out> {
    let transition: fn(&Value) -> Out = match method {
        "puzzleSessionInitial" => return Some(Ok(session::initial())),
        "isReconnect" => return Some(Ok(json!(args.first().is_some_and(session::is_reconnect)))),
        "puzzleSessionInvalidate" => session::invalidate,
        "puzzleSessionLoadBegin" => session::load_begin,
        "puzzleSessionLoadSettled" => session::load_settled,
        "puzzleSessionRetry" => session::retry,
        "puzzleSessionShow" => session::show,
        "puzzleSessionReportBegin" => session::report_begin,
        "puzzleSessionMove" => session::played,
        "puzzleSessionEnd" => session::end,
        "puzzleSessionReportSettled" => session::report_settled,
        "puzzleSessionResetSession" => session::reset_session,
        _ => return None,
    };
    Some(transition(args.first().unwrap_or(&Value::Null)))
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
        variant: field("variant"),
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
