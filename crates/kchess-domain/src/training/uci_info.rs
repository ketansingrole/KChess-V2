//! UCI `info` lines: the scored principal variation a search
//! reports, turned to White's point of view.
// Engine output is outside input: read it without indexing that can panic.
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]

use serde_json::{Map, Value, json};

use super::{Out, num_out, text_arg, to_value};
use crate::{js, rules};

/// `/^[a-h][1-8][a-h][1-8][qrbnk]?$/` (`k` promotes to a king in Antichess), or a Crazyhouse
/// drop such as `P@e4`.
fn is_uci_move(token: &str) -> bool {
    rules::is_uci_text(token)
        || rules::is_drop_text(token)
        || token
            .strip_suffix('k')
            .is_some_and(|from_to| from_to.len() == 4 && rules::is_uci_text(from_to))
}

/// `Number(v)` of an optional token: a missing one is NaN.
fn number_of(token: Option<&str>) -> f64 {
    token.map_or(f64::NAN, js::string_to_number)
}

/// `parseInfo(text, whiteToMove)`: the scored line of one `info` message, or None for progress
/// lines (`currmove`, `hashfull`…), bound scores and anything without a score and a PV.
pub fn parse_info(text: &str, white_to_move: bool) -> Option<Value> {
    // A fast reject for lines that cannot start with `info` (the TypeScript hot path); indented
    // lines fall through to the trimmed read, and any other leading character is rejected.
    if !text.starts_with("info") && !matches!(text.chars().next(), Some(' ' | '\t' | '\n' | '\r')) {
        return None;
    }
    let trimmed = js::trim(text);
    let tokens: Vec<&str> = trimmed
        .split(js::is_space)
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.first() != Some(&"info") {
        return None;
    }
    let mut depth: Option<f64> = None;
    let mut rank = 1.0;
    let mut cp: Option<f64> = None;
    let mut mate: Option<f64> = None;
    let mut nps: Option<f64> = None;
    let mut pv: Option<Vec<&str>> = None;
    let mut i = 1;
    while let Some(&token) = tokens.get(i) {
        let value = tokens.get(i + 1).copied();
        match token {
            "depth" => depth = Some(number_of(value)),
            "multipv" => rank = number_of(value),
            "nps" => nps = Some(number_of(value)),
            "score" => {
                let amount = number_of(tokens.get(i + 2).copied());
                match value {
                    Some("cp") => cp = Some(amount),
                    Some("mate") => mate = Some(amount),
                    _ => {}
                }
                i += 2;
                // A lower or upper bound is a provisional score from a fail-high or fail-low.
                if matches!(
                    tokens.get(i + 1).copied(),
                    Some("lowerbound" | "upperbound")
                ) {
                    return None;
                }
                i += 1;
                continue;
            }
            "pv" => {
                pv = Some(
                    tokens
                        .get(i + 1..)
                        .unwrap_or_default()
                        .iter()
                        .copied()
                        .filter(|move_| is_uci_move(move_))
                        .collect(),
                );
                break;
            }
            _ => {}
        }
        // `string` is free text with no PV; `wdl` takes three values; every other key takes one.
        if token == "string" {
            return None;
        }
        i += if token == "wdl" { 3 } else { 1 };
        i += 1;
    }
    let depth = depth.filter(|d| d.is_finite())?;
    let pv = pv.filter(|moves| !moves.is_empty())?;
    if cp.is_none() && mate.is_none() {
        return None;
    }
    let sign = if white_to_move { 1.0 } else { -1.0 };
    let mut line = Map::new();
    line.insert("rank".into(), num_out(rank));
    line.insert("depth".into(), num_out(depth));
    line.insert("pv".into(), json!(pv));
    if let Some(mate) = mate.filter(|m| m.is_finite()) {
        line.insert("mate".into(), num_out(mate * sign));
    } else {
        let cp = cp.filter(|c| c.is_finite())?;
        line.insert("cp".into(), num_out(cp * sign));
    }
    let mut out = Map::new();
    out.insert("line".into(), Value::Object(line));
    if let Some(nps) = nps.filter(|n| n.is_finite()) {
        out.insert("nps".into(), num_out(nps));
    }
    Some(Value::Object(out))
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    let result = match method {
        "parseInfo" => text_arg(args, 0, "text").and_then(|text| {
            let white_to_move = super::arg(args, 1)
                .and_then(Value::as_bool)
                .unwrap_or(false);
            to_value(parse_info(text, white_to_move))
        }),
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_turn_to_white_and_bounds_are_skipped() {
        let line = parse_info("info depth 12 score cp 31 nodes 9 pv e2e4 e7e5", false).unwrap();
        assert_eq!(line["line"]["cp"], json!(-31.0));
        assert!(parse_info("info depth 12 score cp 40 lowerbound nodes 1 pv e2e4", true).is_none());
        assert!(parse_info("info string NNUE evaluation using nn.nnue", true).is_none());
        assert!(parse_info("", true).is_none());
    }

    #[test]
    fn variant_lines_keep_their_drops_and_king_promotions() {
        let line = parse_info("info depth 9 score cp 50 pv b1c3 d5e6 P@b4 e7e8k", true).unwrap();
        assert_eq!(line["line"]["pv"], json!(["b1c3", "d5e6", "P@b4", "e7e8k"]));
        let line = parse_info("info depth 9 score cp 50 pv e2e4 K@e5 p@e4", true).unwrap();
        assert_eq!(line["line"]["pv"], json!(["e2e4"]));
    }
}
