//! The app's game helpers: the shared session drivers (`kchess-games`), and the replay, tree,
//! puzzle and view helpers the pages use.

pub mod history;
pub mod puzzle;
pub mod tree;
pub mod view;

/// The usual starting position.
pub const STANDARD_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

use serde_json::{Value, json};

pub use kchess_games::{archive, computer, local, random_unit, rules, whole};

/// `formatGameDate`: "Oct 22, 2026" in local time.
pub fn game_date(millis: f64) -> String {
    use chrono::TimeZone as _;
    chrono::Local
        .timestamp_millis_opt(millis as i64)
        .single()
        .map(|date| date.format("%b %-d, %Y").to_string())
        .unwrap_or_default()
}

/// The Chess960 start numbered `n` (`chess960Fen`).
pub fn chess960(n: i64) -> String {
    kchess_domain::rules::chess960_fen(n)
}

/// A FEN typed or pasted, as the board editor reads it (`setupFromFen`): the placement and side
/// from the rules, castling rights as written, and a plausible en passant square.
pub fn editor_setup(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    let parsed = rules("fenSetup", &[json!(trimmed)]).ok()?;
    if parsed.is_null() {
        return None;
    }
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    let rights = fields.get(2).copied().unwrap_or("-");
    let ep = fields
        .get(3)
        .copied()
        .filter(|ep| {
            let bytes = ep.as_bytes();
            bytes.len() == 2 && (b'a'..=b'h').contains(&bytes[0]) && matches!(bytes[1], b'3' | b'6')
        })
        .unwrap_or_default();
    Some(json!({
        "board": parsed["board"],
        "turn": parsed["turn"],
        "castling": {
            "K": rights.contains('K'),
            "Q": rights.contains('Q'),
            "k": rights.contains('k'),
            "q": rights.contains('q'),
        },
        "ep": ep,
    }))
}

/// Why a position cannot be played, in the words the board editor uses (`positionProblem`).
pub fn position_problem(fen: &str) -> Option<&'static str> {
    Some(match kchess_domain::position::fen_problem(fen)? {
        "fen" => "That is not a valid FEN.",
        "empty" => "The board is empty.",
        "kings" => "Each side needs exactly one king.",
        "pawnsOnBackrank" => "Pawns can’t stand on the first or last rank.",
        "tooManyPieces" => "Each side can have at most 16 pieces.",
        "oppositeCheck" => "The side not to move is in check.",
        _ => "This position is not legal.",
    })
}
