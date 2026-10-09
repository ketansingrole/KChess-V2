//! Opening names (`core/src/domain/openings.ts`): Lichess's CC0 names keyed by the first four FEN
//! fields, parsed once from the table `tooling/make-openings.mjs` writes.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

use super::{Out, js_slice_end, to_value, value_arg};
use crate::{js, position, rules};

const OPENINGS_JSON: &str = include_str!("../../../../core/src/domain/data/openings.json");

/// `OpeningName`
#[derive(Debug, Serialize)]
pub struct OpeningName {
    pub eco: String,
    pub name: String,
}

fn table() -> &'static HashMap<String, String> {
    static TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    TABLE.get_or_init(|| serde_json::from_str(OPENINGS_JSON).unwrap_or_default())
}

/// `repetitionKey`: the first four FEN fields.
fn repetition_key(fen: &str) -> String {
    fen.split(' ').take(4).collect::<Vec<_>>().join(" ")
}

/// `openingAt`: the name of the opening reached after `ply` moves. As Lichess suggests, step back
/// from the position until a named one is found. Only games under standard rules have names.
pub fn opening_at(variant: &str, fen: &str, moves: &[String], ply: f64) -> Option<OpeningName> {
    if variant != "standard" {
        return None;
    }
    let pos = rules::setup_start("standard", fen)?;
    let mut seen = vec![repetition_key(&rules::make_fen(&pos))];
    let end = js_slice_end(moves.len(), ply);
    let (_, line) = position::line(pos, &moves[..end], false, false);
    seen.extend(line.iter().map(|step| repetition_key(&step.fen)));
    let books = table();
    for key in seen.iter().rev() {
        let Some(hit) = books.get(key).filter(|hit| !hit.is_empty()) else {
            continue;
        };
        // `indexOf('|')` and then `slice`: a missing bar leaves every character but the last as the code.
        let (eco, name) = match hit.find('|') {
            Some(split) => (&hit[..split], &hit[split + 1..]),
            None => (
                &hit[..hit.len() - hit.chars().last().map_or(0, char::len_utf8)],
                hit.as_str(),
            ),
        };
        return Some(OpeningName {
            eco: eco.to_string(),
            name: name.to_string(),
        });
    }
    None
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    let result = match method {
        "openingAt" => opening_at_call(args),
        _ => return None,
    };
    Some(result)
}

fn opening_at_call(args: &[Value]) -> Out {
    let setup: Value = value_arg(args, 0, "setup")?;
    let variant = setup.get("variant").and_then(Value::as_str).unwrap_or("");
    let fen = setup.get("fen").and_then(Value::as_str).unwrap_or("");
    let moves: Vec<String> = value_arg(args, 1, "moves")?;
    // `ply` defaults to the number of moves; a non-finite one arrives as its text.
    let ply = match args.get(2).filter(|v| !v.is_null()) {
        Some(_) => js::to_number(args.get(2)),
        None => moves.len() as f64,
    };
    to_value(opening_at(variant, fen, &moves, ply))
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    #[test]
    fn names_the_last_named_position() {
        let moves: Vec<String> = ["e2e4", "a7a6", "h2h3"]
            .iter()
            .map(|m| (*m).to_string())
            .collect();
        // After 1.e4 a6 2.h3 the nearest named position is 1.e4 a6 (St. George Defense).
        let name = opening_at("standard", START, &moves, 3.0).expect("a named opening");
        assert_eq!(
            (name.eco.as_str(), name.name.as_str()),
            ("B00", "St. George Defense")
        );
        let first = opening_at("standard", START, &moves, 1.0).expect("the first move is named");
        assert_eq!(first.name, "King's Pawn Game");
        assert!(opening_at("chess960", START, &moves, 1.0).is_none());
        assert!(opening_at("standard", START, &moves, 0.0).is_none());
    }
}
