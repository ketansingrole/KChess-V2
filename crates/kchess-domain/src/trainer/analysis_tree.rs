//! The move tree's edits and helpers.
//!
//! The caller holds the tree and walks it (a child is found by its
//! `uci`); it sends the path and the few values a decision needs (the child ucis or indices along
//! the path, the parent's position and fen), and applies what comes back in place. No whole tree
//! crosses the boundary.

use serde::Serialize;
use serde_json::Value;

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

/// A comment split into what a reader wrote and the commands PGN tools embed in it
/// (`[%clk 0:15:06]`, `[%eval 0.3]`, `[%cal Ge2e4]`), each command whole.
fn comment_parts(comment: &str) -> (String, Vec<&str>) {
    let mut text = String::new();
    let mut commands = Vec::new();
    let mut rest = comment;
    while let Some(at) = rest.find("[%") {
        let Some(end) = rest[at..].find(']') else {
            break;
        };
        text.push_str(&rest[..at]);
        text.push(' ');
        commands.push(&rest[at..at + end + 1]);
        rest = &rest[at + end + 1..];
    }
    text.push_str(rest);
    (text, commands)
}

/// The comment as a reader sees it, without its commands.
pub fn comment_text(comment: &str) -> String {
    comment_parts(comment)
        .0
        .split(js::is_space)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `text` exactly as written, then the comment's commands, so a game's clock times outlive an
/// edit to its notes. Empty when neither is left.
pub fn with_comment_text(comment: &str, text: &str) -> String {
    let commands = comment_parts(comment).1.join(" ");
    match (
        text.trim_matches(js::is_space).is_empty(),
        commands.is_empty(),
    ) {
        (true, _) => commands,
        (false, true) => text.to_string(),
        (false, false) => format!("{text} {commands}"),
    }
}

/// A move's annotation glyph ("!?") and its name.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Glyph {
    pub nag: f64,
    pub glyph: &'static str,
    pub label: &'static str,
}

/// The first table glyph whose NAG a move has.
pub fn move_glyph(nags: &[f64]) -> Option<Glyph> {
    GLYPHS
        .iter()
        .find(|(nag, _, _)| nags.contains(nag))
        .map(|&(nag, glyph, label)| Glyph { nag, glyph, label })
}

/// A move's NAG list with glyph `nag` set (the glyph first, the other NAGs kept), or none when
/// none remain. A zero or missing `nag` removes the glyph.
pub fn set_move_glyph(nags: &[f64], nag: Option<f64>) -> Option<Vec<f64>> {
    let others: Vec<f64> = nags
        .iter()
        .copied()
        .filter(|n| *n < 1.0 || *n > 6.0)
        .collect();
    let next: Vec<f64> = match nag.filter(|n| *n != 0.0 && !n.is_nan()) {
        Some(glyph) => std::iter::once(glyph).chain(others).collect(),
        None => others,
    };
    (!next.is_empty()).then_some(next)
}

/// A node's path: the UCI moves from the root, joined with spaces.
pub fn path_of(moves: &[String]) -> String {
    moves.join(" ")
}

/// The UCI moves of a path.
pub fn moves_of(path: &str) -> Vec<String> {
    if path.is_empty() {
        Vec::new()
    } else {
        path.split(' ').map(str::to_string).collect()
    }
}

/// “1.”, “1…”, or nothing: the number to print before a move at `ply`.
pub fn move_number(ply: f64, always: bool) -> String {
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

/// An engine line's score: centipawns, or mate in so many moves, from White's side.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Score {
    pub cp: Option<f64>,
    pub mate: Option<f64>,
}

impl Score {
    /// The score of an engine line (`{ cp, mate, … }`); none when it is not an object.
    pub fn of_line(line: &Value) -> Option<Score> {
        let line = line.as_object()?;
        Some(Score {
            cp: line.get("cp").and_then(Value::as_f64),
            mate: line.get("mate").and_then(Value::as_f64),
        })
    }
}

/// “+0.34”, “−1.20”, “#3”, “#−2”, or “…” without a line.
pub fn format_eval(line: Option<Score>) -> String {
    let Some(line) = line else {
        return "…".into();
    };
    if let Some(mate) = line.mate {
        let sign = if mate < 0.0 { "−" } else { "" };
        return format!("#{sign}{}", js::number_to_string(mate.abs()));
    }
    let pawns = line.cp.unwrap_or(0.0) / 100.0;
    let sign = if pawns > 0.0 {
        "+"
    } else if pawns < 0.0 {
        "−"
    } else {
        ""
    };
    format!("{sign}{}", to_fixed(pawns.abs(), 1))
}

/// White's chances in −1…1 on Lichess's curve.
pub fn winning_chances(line: Option<Score>) -> f64 {
    let Some(line) = line else {
        return 0.0;
    };
    if let Some(mate) = line.mate {
        return if mate > 0.0 {
            1.0
        } else if mate < 0.0 {
            -1.0
        } else {
            0.0
        };
    }
    let cp = line.cp.unwrap_or(0.0).clamp(-1000.0, 1000.0);
    2.0 / (1.0 + js_exp(-0.00368208 * cp)) - 1.0
}

/// A move to add below a node: the node's path, its position (`fen`, `ply`, and `variant`, a
/// Lichess key, empty for standard chess) and the moves already tried from it.
#[derive(Clone, Debug, Default)]
pub struct MoveInput<'a> {
    pub path: &'a str,
    pub uci: &'a str,
    pub fen: &'a str,
    pub ply: f64,
    pub child_ucis: &'a [String],
    pub variant: &'a str,
}

/// A legal move's place in the tree: the child's path and, unless the move was already tried,
/// the node to append.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedMove {
    pub child_path: String,
    pub child: Option<NewNode>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NewNode {
    pub uci: String,
    pub san: String,
    pub fen: String,
    pub ply: f64,
}

/// Where a move goes below a node; none when it is illegal there.
pub fn add_move_plan(input: &MoveInput) -> Option<AddedMove> {
    let mut pos = match input.variant {
        "" => rules::standard_from_fen(input.fen),
        variant => rules::setup_start(variant, input.fen),
    }?;
    let played = rules::parse_uci(input.uci).and_then(|u| rules::legal_move(&pos, u))?;
    // Castling comes back as king-takes-rook, so a move has one spelling in the tree.
    let played_uci = rules::make_uci(&played);
    let san = rules::san_and_play(&mut pos, played);
    let fen = rules::make_fen(&pos);
    let mut child_path = moves_of(input.path);
    child_path.push(played_uci.clone());
    let child = (!input.child_ucis.contains(&played_uci)).then_some(NewNode {
        uci: played_uci,
        san,
        fen,
        ply: input.ply + 1.0,
    });
    Some(AddedMove {
        child_path: path_of(&child_path),
        child,
    })
}

/// The path, then the first-child chain from the node it reaches.
pub fn line_end(path: &str, chain: &[String]) -> String {
    let mut moves = moves_of(path);
    moves.extend(chain.iter().cloned());
    path_of(&moves)
}

/// The node to delete: its parent's path and its move (none at the root).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePlan {
    pub parent_path: String,
    pub uci: Option<String>,
}

pub fn delete_at_plan(path: &str) -> DeletePlan {
    let mut moves = moves_of(path);
    match moves.pop() {
        Some(uci) if !uci.is_empty() => DeletePlan {
            parent_path: path_of(&moves),
            uci: Some(uci),
        },
        _ => DeletePlan {
            parent_path: String::new(),
            uci: None,
        },
    }
}

/// Every step of the path (child indices) is a first child.
pub fn on_mainline(indices: &[u64]) -> bool {
    indices.iter().all(|index| *index == 0)
}

/// `(depth, index)` for each parent along a path whose child must move to the front to make it
/// the main line.
pub fn promote_plan(indices: &[u64]) -> Vec<(usize, u64)> {
    indices
        .iter()
        .enumerate()
        .filter(|(_, index)| **index != 0)
        .map(|(depth, index)| (depth, *index))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_is_played_by_its_trees_variant() {
        // White holds a pawn: legal to drop in Crazyhouse, unreadable in standard chess.
        let fen = "rnb1kbnr/ppp1pppp/8/q7/8/2N5/PPPP1PPP/R1BQKBNR[P] w KQkq - 2 4";
        let input = |variant| MoveInput {
            uci: "P@d4",
            fen,
            ply: 6.0,
            variant,
            ..MoveInput::default()
        };
        let plan = add_move_plan(&input("crazyhouse")).unwrap();
        assert_eq!(plan.child.unwrap().san, "P@d4");
        assert_eq!(plan.child_path, "P@d4");
        assert!(add_move_plan(&input("")).is_none());
    }

    #[test]
    fn a_comment_reads_without_its_commands() {
        assert_eq!(comment_text(" [%clk 0:15:06] "), "");
        assert_eq!(
            comment_text("Good idea [%eval 0.3] here [%clk 0:01:00]"),
            "Good idea here"
        );
        assert_eq!(comment_text("no commands"), "no commands");
        // An unclosed bracket is text, as a reader wrote it.
        assert_eq!(comment_text("see [%clk"), "see [%clk");
    }

    #[test]
    fn an_edit_keeps_the_commands_and_the_text_as_typed() {
        let clock = " [%clk 0:15:06] ";
        assert_eq!(
            with_comment_text(clock, "Strong "),
            "Strong  [%clk 0:15:06]"
        );
        assert_eq!(with_comment_text(clock, ""), "[%clk 0:15:06]");
        assert_eq!(
            with_comment_text("old [%eval 1] [%clk 0:00:05]", "new"),
            "new [%eval 1] [%clk 0:00:05]"
        );
        assert_eq!(with_comment_text("old", "  "), "");
        assert_eq!(with_comment_text("old", "new"), "new");
        assert_eq!(comment_text(&with_comment_text(clock, "Strong ")), "Strong");
    }
}
