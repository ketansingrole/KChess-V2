//! A game review as stored, the key it is stored under, and which games get one.

use serde::{Deserialize, Serialize};

use crate::variant::Variant;

crate::union!(#[doc = "Severity of a move mistake in review."] Judgment {
    Inaccuracy = "inaccuracy",
    Mistake = "mistake",
    Blunder = "blunder",
});
crate::union!(#[doc = "Where a game review came from."] ReviewSource { Lichess = "lichess", Local = "local" });

/// One position of a reviewed game: its score from White's side and the engine's choice there.
/// A checkmated position is `mate: 0` (the side to move has lost).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewEval {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cp: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mate: Option<f64>,
    /// The engine's move in this position (UCI), and the line it expects after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pv: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<f64>,
}

/// Lichess's accuracy figures, as Lichess sent them (`87`, `64.5`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewAccuracy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub white: Option<serde_json::Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub black: Option<serde_json::Number>,
}

/// A game's review as stored: the raw scores; labels and accuracy are worked out from them.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StoredReview {
    /// The same moves from the same start share one review.
    pub key: String,
    pub fen: String,
    /// UCI, from `fen`.
    pub moves: Vec<String>,
    pub source: ReviewSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// Index 0 is the starting position, index i the position after move i; null until analysed.
    pub evals: Vec<Option<ReviewEval>>,
    /// Depth the local analysis aims for; 0 for Lichess's analysis.
    pub depth: u32,
    pub complete: bool,
    pub updated_at: i64,
    /// The Lichess game it belongs to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    /// Lichess's own label for each move (index i is move i+1), when Lichess did the analysis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judgments: Option<Vec<Option<Judgment>>>,
    /// Lichess's own accuracy figures, when Lichess did the analysis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accuracy: Option<ReviewAccuracy>,
    /// The Lichess key of a variant game's variant, which Fairy-Stockfish reviews.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// What another release stored beside these, kept as it was.
    #[serde(flatten)]
    pub rest: serde_json::Map<String, serde_json::Value>,
}

/// One player's side of a review. Figures keep the form they were stored in (`100` when read
/// back, `100.0` when just worked out).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSide {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accuracy: Option<serde_json::Number>,
    /// Average centipawn loss.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acpl: Option<serde_json::Number>,
    pub inaccuracy: u32,
    pub mistake: u32,
    pub blunder: u32,
}

/// What the game list shows for a reviewed game.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSummary {
    pub key: String,
    pub source: ReviewSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    pub complete: bool,
    pub white: ReviewSide,
    pub black: ReviewSide,
}

/// Lichess speeds of standard chess; variants are not reviewed.
const STANDARD_PERFS: &[&str] = &crate::perf::SPEED_KEYS;

/// The key a review of `moves` (UCI) from `fen` is stored under. A game of a variant not played
/// by standard rules has a key that names it: the same moves are another game in Atomic.
pub fn review_key<S: AsRef<str>>(fen: &str, moves: &[S], variant: Option<Variant>) -> String {
    let moves: Vec<&str> = moves.iter().map(AsRef::as_ref).collect();
    match variant {
        Some(variant) if !variant.standard_rules() => {
            cyrb53(&format!("{}|{fen}", variant.key()), &moves.join(" "))
        }
        _ => cyrb53(fen, &moves.join(" ")),
    }
}

/// Whether a review is made for games of this Lichess perf: the standard speeds only.
pub fn is_reviewable_perf(perf: &str) -> bool {
    STANDARD_PERFS.contains(&perf)
}

/// cyrb53 of `fen|moves`, over UTF-16 code units, as 16 hex digits.
fn cyrb53(fen: &str, moves: &str) -> String {
    let text = format!("{fen}|{moves}");
    let mut h1 = 0xdeadbeefu32 as i32;
    let mut h2 = 0x41c6ce57u32 as i32;
    for unit in text.encode_utf16() {
        let code = i32::from(unit);
        h1 = (h1 ^ code).wrapping_mul(2_654_435_761u32 as i32);
        h2 = (h2 ^ code).wrapping_mul(1_597_334_677u32 as i32);
    }
    h1 = (h1 ^ shr(h1, 16)).wrapping_mul(2_246_822_507u32 as i32)
        ^ (h2 ^ shr(h2, 13)).wrapping_mul(3_266_489_909u32 as i32);
    h2 = (h2 ^ shr(h2, 16)).wrapping_mul(2_246_822_507u32 as i32)
        ^ (h1 ^ shr(h1, 13)).wrapping_mul(3_266_489_909u32 as i32);
    format!("{:08x}{:08x}", h2 as u32, h1 as u32)
}

/// JavaScript's `x >>> n` read back as the signed 32-bit value `^` works on.
fn shr(x: i32, n: u32) -> i32 {
    ((x as u32) >> n) as i32
}

/// A score: centipawns or mate in moves, from White's side.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Score {
    Cp(f64),
    Mate(f64),
}

/// The score of a finished game's last position: checkmate is lost for the side to move, a
/// variant won by the side to move (`won`, Antichess) is mate for it, the rest drawn.
pub fn end_eval(end: &str, turn: &str) -> Score {
    match end {
        "won" => Score::Mate(if turn == "white" { 1.0 } else { -1.0 }),
        "checkmate" => Score::Mate(0.0),
        _ => Score::Cp(0.0),
    }
}
