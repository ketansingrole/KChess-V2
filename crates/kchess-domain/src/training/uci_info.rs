//! UCI `info` lines (`crates/kchess-wasm/js/uciInfo.ts`): the scored principal variation a search
//! reports, turned to White's point of view.

use serde_json::{Map, Value, json};

use super::{Out, num_out, text_arg, to_value};
use crate::js;

/// `/^[a-h][1-8][a-h][1-8][qrbn]?$/`
fn is_uci_move(token: &str) -> bool {
    let b = token.as_bytes();
    let square =
        |file: u8, rank: u8| (b'a'..=b'h').contains(&file) && (b'1'..=b'8').contains(&rank);
    (b.len() == 4 || b.len() == 5)
        && square(b[0], b[1])
        && square(b[2], b[3])
        && (b.len() == 4 || b"qrbn".contains(&b[4]))
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
    while i < tokens.len() {
        let token = tokens[i];
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
                    tokens[i + 1..]
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
    } else if let Some(cp) = cp.filter(|c| c.is_finite()) {
        line.insert("cp".into(), num_out(cp * sign));
    } else {
        return None;
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
}
