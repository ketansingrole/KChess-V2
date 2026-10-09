//! `core/src/domain/analysisTree.ts`: the move tree's edits and helpers.
//!
//! The TypeScript side holds the tree as mutable objects and walks it (a child is found by its
//! `uci`); it sends the path and the few values a decision needs (the child ucis or indices along
//! the path, the parent's position and fen), and applies what comes back in place. No whole tree
//! crosses the boundary.

use serde_json::{Value, json};

use super::Out;
use crate::js;
use crate::review::js_exp;
use crate::rules;

/// `MOVE_GLYPHS`, in table order: a move's glyph is the first of these its NAGs contain.
const GLYPHS: [(f64, &str, &str); 6] = [
    (1.0, "!", "Good move"),
    (2.0, "?", "Mistake"),
    (3.0, "!!", "Brilliant move"),
    (4.0, "??", "Blunder"),
    (5.0, "!?", "Interesting move"),
    (6.0, "?!", "Dubious move"),
];

/// This module's methods for `api::call`; None when the method is not one of them.
pub(super) fn call(method: &str, args: &[Value]) -> Option<Out> {
    let arg = |i: usize| args.get(i).filter(|value| !value.is_null());
    let result = match method {
        "moveGlyph" => Ok(move_glyph(&numbers(arg(0)))),
        "setMoveGlyph" => Ok(set_move_glyph(&numbers(arg(0)), number(arg(1)))),
        "pathOf" => Ok(json!(path_of(&texts(arg(0))))),
        "movesOf" => Ok(json!(moves_of(
            arg(0).and_then(Value::as_str).unwrap_or("")
        ))),
        "moveNumber" => Ok(json!(move_number(
            number(arg(0)).unwrap_or(f64::NAN),
            arg(1).and_then(Value::as_bool).unwrap_or(false)
        ))),
        "formatEval" => Ok(json!(format_eval(arg(0)))),
        "winningChances" => Ok(json!(winning_chances(arg(0)))),
        "addMovePlan" => add_move_plan(arg(0)),
        "lineEnd" => Ok(json!(line_end(
            arg(0).and_then(Value::as_str).unwrap_or(""),
            &texts(arg(1))
        ))),
        "deleteAtPlan" => Ok(delete_at_plan(arg(0).and_then(Value::as_str).unwrap_or(""))),
        "onMainline" => Ok(json!(on_mainline(&indices(arg(0))))),
        "promotePlan" => Ok(promote_plan(&indices(arg(0)))),
        _ => return None,
    };
    Some(result)
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

/// `moveGlyph(node)`: the first table glyph whose NAG the node has, or null.
fn move_glyph(nags: &[f64]) -> Value {
    GLYPHS.iter().find(|(nag, _, _)| nags.contains(nag)).map_or(
        Value::Null,
        |(nag, glyph, label)| json!({ "nag": nag, "glyph": glyph, "label": label }),
    )
}

/// `setMoveGlyph(node, nag)`: the new NAG list (the glyph first, the other NAGs kept), or null
/// when none remain. A zero or missing `nag` removes the glyph.
fn set_move_glyph(nags: &[f64], nag: Option<f64>) -> Value {
    let others: Vec<f64> = nags
        .iter()
        .copied()
        .filter(|n| *n < 1.0 || *n > 6.0)
        .collect();
    let next: Vec<f64> = match nag.filter(|n| *n != 0.0 && !n.is_nan()) {
        Some(glyph) => std::iter::once(glyph).chain(others).collect(),
        None => others,
    };
    if next.is_empty() {
        Value::Null
    } else {
        json!(next)
    }
}

fn path_of(moves: &[String]) -> String {
    moves.join(" ")
}

fn moves_of(path: &str) -> Vec<String> {
    if path.is_empty() {
        Vec::new()
    } else {
        path.split(' ').map(str::to_string).collect()
    }
}

/// `moveNumber(ply, always)`: “1.”, “1…”, or nothing.
fn move_number(ply: f64, always: bool) -> String {
    let number = js::number_to_string((ply / 2.0).floor() + 1.0);
    if ply % 2.0 == 0.0 {
        format!("{number}.")
    } else if always {
        format!("{number}…")
    } else {
        String::new()
    }
}

/// `toFixed(digits)` for a non-negative number: the exact value rounded to the nearest multiple
/// of 10^-digits, a tie going to the larger number.
fn to_fixed(x: f64, digits: usize) -> String {
    if !x.is_finite() || x >= 1e21 {
        return js::number_to_string(x);
    }
    let exact = format!("{x:.1100}");
    let (whole, fraction) = exact.split_once('.').unwrap_or((exact.as_str(), ""));
    let mut kept: Vec<u8> = whole
        .bytes()
        .chain(fraction.bytes().take(digits))
        .map(|b| b - b'0')
        .collect();
    if fraction.as_bytes().get(digits).is_some_and(|b| *b >= b'5') {
        let mut at = kept.len();
        loop {
            if at == 0 {
                kept.insert(0, 1);
                break;
            }
            at -= 1;
            if kept[at] == 9 {
                kept[at] = 0;
            } else {
                kept[at] += 1;
                break;
            }
        }
    }
    let split = kept.len() - digits;
    let text: String = kept.iter().map(|d| char::from(b'0' + d)).collect();
    let (whole, fraction) = text.split_at(split);
    if digits == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{fraction}")
    }
}

/// `formatEval(line)`: “+0.34”, “−1.20”, “#3”, “#−2”, or “…” without a line.
fn format_eval(line: Option<&Value>) -> String {
    let Some(line) = line.and_then(Value::as_object) else {
        return "…".into();
    };
    let mate = line
        .get("mate")
        .filter(|v| !v.is_null())
        .and_then(Value::as_f64);
    if let Some(mate) = mate {
        let sign = if mate < 0.0 { "−" } else { "" };
        return format!("#{sign}{}", js::number_to_string(mate.abs()));
    }
    let pawns = line.get("cp").and_then(Value::as_f64).unwrap_or(0.0) / 100.0;
    let sign = if pawns > 0.0 {
        "+"
    } else if pawns < 0.0 {
        "−"
    } else {
        ""
    };
    format!("{sign}{}", to_fixed(pawns.abs(), 1))
}

/// `winningChances(line)`: White's chances in −1…1 on Lichess's curve.
fn winning_chances(line: Option<&Value>) -> f64 {
    let Some(line) = line.and_then(Value::as_object) else {
        return 0.0;
    };
    if let Some(mate) = line
        .get("mate")
        .filter(|v| !v.is_null())
        .and_then(Value::as_f64)
    {
        return if mate > 0.0 {
            1.0
        } else if mate < 0.0 {
            -1.0
        } else {
            0.0
        };
    }
    let cp = line.get("cp").and_then(Value::as_f64).unwrap_or(0.0);
    let cp = cp.clamp(-1000.0, 1000.0);
    2.0 / (1.0 + js_exp(-0.00368208 * cp)) - 1.0
}

/// `addMove`'s rules. Input: the path, the move, the parent's fen, ply and child ucis. Output:
/// null when the move is illegal, else the child's path and the node to append (null when the
/// move was already tried, so the existing node is kept).
fn add_move_plan(input: Option<&Value>) -> Out {
    let input = input
        .and_then(Value::as_object)
        .ok_or("addMove input must be an object")?;
    let text = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or("");
    let path = text("path");
    let uci = text("uci");
    let Some(mut pos) = rules::standard_from_fen(text("fen")) else {
        return Ok(Value::Null);
    };
    let Some(played) = rules::parse_uci(uci).and_then(|u| rules::legal_move(&pos, u)) else {
        return Ok(Value::Null);
    };
    // Castling comes back as king-takes-rook, so a move has one spelling in the tree.
    let played_uci = rules::make_uci(&played);
    let san = rules::san_and_play(&mut pos, played);
    let fen = rules::make_fen(&pos);
    let mut child_path = moves_of(path);
    child_path.push(played_uci.clone());
    let child_path = path_of(&child_path);
    let duplicate = texts(input.get("childUcis")).contains(&played_uci);
    let child = if duplicate {
        Value::Null
    } else {
        let ply = number(input.get("ply")).unwrap_or(f64::NAN);
        json!({ "uci": played_uci, "san": san, "fen": fen, "ply": ply + 1.0 })
    };
    Ok(json!({ "childPath": child_path, "child": child }))
}

/// `lineEnd`: the path, then the first-child chain from the node it reaches.
fn line_end(path: &str, chain: &[String]) -> String {
    let mut moves = moves_of(path);
    moves.extend(chain.iter().cloned());
    path_of(&moves)
}

/// `deleteAt`: the parent's path and the move to remove (null when there is none).
fn delete_at_plan(path: &str) -> Value {
    let mut moves = moves_of(path);
    match moves.pop() {
        Some(uci) if !uci.is_empty() => json!({ "parentPath": path_of(&moves), "uci": uci }),
        _ => json!({ "parentPath": "", "uci": null }),
    }
}

/// `onMainline`: every step of the path is a first child.
fn on_mainline(indices: &[u64]) -> bool {
    indices.iter().all(|index| *index == 0)
}

/// `promoteToMainline`: `[depth, index]` for each parent whose child must move to the front.
fn promote_plan(indices: &[u64]) -> Value {
    let steps: Vec<Value> = indices
        .iter()
        .enumerate()
        .filter(|(_, index)| **index != 0)
        .map(|(depth, index)| json!([depth, index]))
        .collect();
    Value::Array(steps)
}
