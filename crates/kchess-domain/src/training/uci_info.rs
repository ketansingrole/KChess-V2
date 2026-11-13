//! UCI `info` lines: the scored principal variation a search
//! reports, turned to White's point of view.
// Engine output is outside input: read it without indexing that can panic.
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]

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

/// One scored principal variation, from White's side.
#[derive(Debug, Clone, PartialEq)]
pub struct InfoLine {
    /// `multipv`: 1 for the best line. Not a number when the engine wrote something else.
    pub rank: f64,
    pub depth: f64,
    /// The score in centipawns, unless it is a mate.
    pub cp: Option<f64>,
    /// Moves to mate: positive when White mates.
    pub mate: Option<f64>,
    pub pv: Vec<String>,
}

/// One `info` message's scored line, and the engine's speed when it gave it.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub line: InfoLine,
    pub nps: Option<f64>,
}

/// The scored line of one `info` message, or None for progress lines (`currmove`, `hashfull`…),
/// bound scores and anything without a score and a PV.
pub fn parse_info(text: &str, white_to_move: bool) -> Option<Info> {
    // A fast reject for lines that cannot start with `info` (the hot path); indented
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
    let white = |score: f64| if white_to_move { score } else { -score };
    let (cp, mate) = match mate.filter(|m| m.is_finite()) {
        Some(mate) => (None, Some(white(mate))),
        None => (Some(white(cp.filter(|c| c.is_finite())?)), None),
    };
    Some(Info {
        line: InfoLine {
            rank,
            depth,
            cp,
            mate,
            pv: pv.into_iter().map(str::to_string).collect(),
        },
        nps: nps.filter(|n| n.is_finite()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_turn_to_white_and_bounds_are_skipped() {
        let line = parse_info("info depth 12 score cp 31 nodes 9 pv e2e4 e7e5", false).unwrap();
        assert_eq!(line.line.cp, Some(-31.0));
        assert!(parse_info("info depth 12 score cp 40 lowerbound nodes 1 pv e2e4", true).is_none());
        assert!(parse_info("info string NNUE evaluation using nn.nnue", true).is_none());
        assert!(parse_info("", true).is_none());
    }

    #[test]
    fn variant_lines_keep_their_drops_and_king_promotions() {
        let line = parse_info("info depth 9 score cp 50 pv b1c3 d5e6 P@b4 e7e8k", true).unwrap();
        assert_eq!(line.line.pv, ["b1c3", "d5e6", "P@b4", "e7e8k"]);
        let line = parse_info("info depth 9 score cp 50 pv e2e4 K@e5 p@e4", true).unwrap();
        assert_eq!(line.line.pv, ["e2e4"]);
        // A king promotion is four squares and a `k`; nothing else ending in `k` is a move.
        let line = parse_info("info depth 9 score cp 5 pv e7e8k zzzzk e7e8qk", true).unwrap();
        assert_eq!(line.line.pv, ["e7e8k"]);
    }

    /// A mate's moves count from White's side: positive when White mates.
    #[test]
    fn a_mate_turns_to_white() {
        let mate = |white_to_move| {
            parse_info("info depth 5 score mate 3 pv e2e4", white_to_move)
                .unwrap()
                .line
        };
        assert_eq!((mate(true).mate, mate(true).cp), (Some(3.0), None));
        assert_eq!(mate(false).mate, Some(-3.0));
        let cp = parse_info("info depth 5 score cp 20 pv e2e4", true).unwrap();
        assert_eq!(cp.line.cp, Some(20.0));
    }
}
