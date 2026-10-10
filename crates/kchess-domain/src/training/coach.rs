//! Review coaching (`crates/kchess-wasm/js/coach.ts`): a sentence about one reviewed move. Every claim
//! comes from a scored, legally replayed position.

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Out, text_of, to_value, value_arg};
use crate::{api, replay};

#[derive(Debug, Deserialize)]
struct Eval {
    cp: Option<f64>,
    mate: Option<f64>,
    best: Option<String>,
    pv: Option<Vec<String>>,
    depth: Option<f64>,
}

/// The parts of a `StoredReview` the explanation reads.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Review {
    fen: String,
    moves: Vec<String>,
    #[serde(default)]
    evals: Vec<Option<Eval>>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    complete: bool,
}

#[derive(Debug, Deserialize)]
struct PvMove {
    san: String,
    label: String,
}

/// `toFixed(digits)` as JavaScript computes it: the exact value, rounded to the nearest
/// multiple of 10^-digits, with a tie going to the larger number.
fn to_fixed(x: f64, digits: usize) -> String {
    if !x.is_finite() || x.abs() >= 1e21 {
        return text_of(x);
    }
    let exact = format!("{:.1100}", x.abs());
    let (whole, fraction) = exact.split_once('.').unwrap_or((exact.as_str(), ""));
    let mut kept: Vec<u8> = whole
        .bytes()
        .chain(fraction.bytes().take(digits))
        .map(|b| b - b'0')
        .collect();
    if fraction.as_bytes().get(digits).is_some_and(|b| *b >= b'5') {
        // Add one at the last kept digit, carrying leftwards.
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
    let body = if digits == 0 {
        text
    } else {
        format!("{}.{}", &text[..split], &text[split..])
    };
    if x < 0.0 { format!("-{body}") } else { body }
}

/// `formatEval`: a mate as `#n`, otherwise the score in pawns with one decimal, from White's side.
fn format_eval(cp: Option<f64>, mate: Option<f64>) -> String {
    if let Some(mate) = mate {
        return format!(
            "#{}{}",
            if mate < 0.0 { "−" } else { "" },
            text_of(mate.abs())
        );
    }
    let pawns = cp.unwrap_or(0.0) / 100.0;
    let sign = if pawns > 0.0 {
        "+"
    } else if pawns < 0.0 {
        "−"
    } else {
        ""
    };
    format!("{sign}{}", to_fixed(pawns.abs(), 1))
}

/// `pvSan`, through the rules' own implementation.
fn pv_san(fen: &str, pv: &[String], max: usize) -> Vec<PvMove> {
    let Ok(args) = serde_json::to_string(&json!([fen, pv, max])) else {
        return Vec::new();
    };
    api::call("pvSan", &args)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// `explainReviewedMove`: what the move did to the evaluation, and the line that was preferred.
fn explain(review: &Review, index: f64) -> String {
    if index < 0.0 || index.fract() != 0.0 || index >= review.moves.len() as f64 {
        return String::new();
    }
    let index = index as usize;
    let positions = replay::replay_positions(&review.fen, &review.moves);
    let before = review.evals.get(index).and_then(Option::as_ref);
    let after = review.evals.get(index + 1).and_then(Option::as_ref);
    let fen = positions.get(index).map(|position| position.fen.as_str());
    let (Some(before), Some(after), Some(fen)) = (before, after, fen) else {
        return String::new();
    };
    if fen.is_empty() {
        return String::new();
    }
    let Some(played) = pv_san(fen, std::slice::from_ref(&review.moves[index]), 1)
        .into_iter()
        .next()
    else {
        return String::new();
    };
    // The preferred line is the stored PV when it starts with the engine's best move.
    let pv = before.pv.clone().unwrap_or_default();
    let preferred: Vec<String> =
        if before.pv.as_ref().and_then(|p| p.first()) == before.best.as_ref() {
            pv
        } else {
            before
                .best
                .iter()
                .filter(|best| !best.is_empty())
                .cloned()
                .collect()
        };
    let suggested = pv_san(fen, &preferred, 6);
    let source = if review.source.as_deref() == Some("lichess") {
        "Lichess analysis".to_string()
    } else {
        match before.depth.filter(|d| *d != 0.0) {
            Some(depth) => format!("Stockfish at depth {}", text_of(depth)),
            None => "Stockfish".to_string(),
        }
    };
    let score = |value: &Eval| {
        if value.cp.is_none() && value.mate.is_none() {
            "unknown".to_string()
        } else {
            format_eval(value.cp, value.mate)
        }
    };
    let preferred_line = if suggested.is_empty() {
        String::new()
    } else {
        let labels: Vec<&str> = suggested.iter().map(|m| m.label.as_str()).collect();
        format!(" Preferred line: {}.", labels.join(" "))
    };
    let provisional = if review.complete {
        ""
    } else {
        " This review is still provisional."
    };
    format!(
        "{source}: after {}, the evaluation changed from {} to {} (White's perspective).{preferred_line}{provisional}",
        played.san,
        score(before),
        score(after),
    )
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    let result = match method {
        "explainReviewedMove" => value_arg::<Review>(args, 0, "review").and_then(|review| {
            let index = super::number_arg(args, 1);
            to_value(explain(&review, index))
        }),
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_point_rounds_ties_up_like_javascript() {
        assert_eq!(to_fixed(0.25, 1), "0.3");
        assert_eq!(to_fixed(0.35, 1), "0.3");
        assert_eq!(to_fixed(9.96, 1), "10.0");
        assert_eq!(to_fixed(0.0, 1), "0.0");
        assert_eq!(to_fixed(12.5, 0), "13");
        assert_eq!(format_eval(Some(25.0), None), "+0.3");
        assert_eq!(format_eval(Some(-25.0), None), "−0.3");
        assert_eq!(format_eval(None, Some(-2.0)), "#−2");
    }
}
