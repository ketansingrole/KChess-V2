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

/// "Nov 4", or "Nov 4, 2025" outside this year: a game's day where space is short.
pub fn short_game_date(millis: f64) -> String {
    use chrono::{Datelike as _, TimeZone as _};
    chrono::Local
        .timestamp_millis_opt(millis as i64)
        .single()
        .map(|date| {
            if date.year() == chrono::Local::now().year() {
                date.format("%b %-d").to_string()
            } else {
                date.format("%b %-d, %Y").to_string()
            }
        })
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
        "fen" => "That is not a valid FEN",
        "empty" => "The board is empty",
        "kings" => "Each side needs exactly one king",
        "pawnsOnBackrank" => "Pawns can’t stand on the first or last rank",
        "tooManyPieces" => "Each side can have at most 16 pieces",
        "oppositeCheck" => "The side not to move is in check",
        _ => "This position is not legal",
    })
}

/// Whether the analysis board, its reviews and the computer can take a game in this variant:
/// standard chess, and Chess960, which they play with king-takes-rook castling.
pub fn analysable(variant: &str) -> bool {
    matches!(variant, "standard" | "chess960")
}

/// A position's setup for a game started from it: Chess960 when it can still castle with a king
/// or rook off their usual squares, which an engine reads only with `UCI_Chess960` on, and
/// standard otherwise.
pub fn setup_from_fen(fen: &str) -> Value {
    json!({ "variant": if chess960_castling(fen) { "chess960" } else { "standard" }, "fen": fen })
}

fn chess960_castling(fen: &str) -> bool {
    use shakmaty::{CastlingSide, Color, Position as _, Square};
    let Some(position) = kchess_domain::rules::standard_from_fen(fen) else {
        return false;
    };
    let castles = position.castles();
    [Color::White, Color::Black].into_iter().any(|color| {
        let (king, a, h) = match color {
            Color::White => (Square::E1, Square::A1, Square::H1),
            Color::Black => (Square::E8, Square::A8, Square::H8),
        };
        let rooks = [(CastlingSide::QueenSide, a), (CastlingSide::KingSide, h)];
        let any = rooks
            .iter()
            .any(|(side, _)| castles.rook(color, *side).is_some());
        any && (position.board().king_of(color) != Some(king)
            || rooks.iter().any(|(side, usual)| {
                castles
                    .rook(color, *side)
                    .is_some_and(|rook| rook != *usual)
            }))
    })
}

#[cfg(test)]
mod setup_tests {
    use super::setup_from_fen;

    #[test]
    fn a_position_castling_from_unusual_squares_is_chess960() {
        let variant = |fen: &str| setup_from_fen(fen)["variant"].clone();
        assert_eq!(variant(super::STANDARD_FEN), "standard");
        assert_eq!(variant("4k3/8/8/8/8/8/8/4K3 w - - 0 1"), "standard");
        assert_eq!(variant(&super::chess960(0)), "chess960");
        assert_eq!(variant("1rk4r/8/8/8/8/8/8/1RK4R w HBhb - 0 1"), "chess960");
        // Its king and rooks moved, so the position no longer castles at all.
        assert_eq!(variant("1r5r/2k5/8/8/8/8/2K5/1R5R w - - 0 1"), "standard");
    }
}
