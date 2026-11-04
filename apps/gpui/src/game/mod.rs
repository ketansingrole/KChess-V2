//! The app's game helpers: the shared session drivers (`kchess-games`), and the replay, tree,
//! puzzle and view helpers the pages use.

pub mod history;
pub mod puzzle;
pub mod tree;
pub mod view;

/// The usual starting position.
pub const STANDARD_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

use serde_json::{Value, json};

pub use kchess_domain::links::{
    BroadcastLink, GameLink, broadcast_link, game_link, is_username, pgn_orientation, profile_link,
    study_link,
};
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

/// A Chess960 start's number picked at random, or 518 (the standard start) without randomness.
pub fn random_chess960() -> i64 {
    let mut bytes = [0u8; 2];
    if getrandom::fill(&mut bytes).is_err() {
        return 518;
    }
    i64::from(u16::from_le_bytes(bytes) % 960)
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

/// Whether a PGN's `Variant` header names a game the analysis board can play: standard chess or
/// Chess960 (both read as standard rules with Chess960 castling), or none at all. An Atomic or
/// Crazyhouse game read that way would show positions the game never had.
pub fn pgn_analysable(pgn: &str) -> bool {
    pgn_variant_key(pgn).is_some_and(analysable)
}

/// The Lichess key of the variant a PGN's `Variant` header names ("atomic", "chess960"), standard
/// without one; `None` for a variant KChess does not know.
pub fn pgn_variant_key(pgn: &str) -> Option<&'static str> {
    use shakmaty::variant::Variant;
    // The tags before the moves, one per line or (pasted into a one-line field) side by side.
    let mut name = None;
    let mut rest = pgn.trim_start();
    while let Some(tag) = rest.strip_prefix('[') {
        let Some(end) = tag.find(']') else { break };
        if let Some(value) = tag[..end].strip_prefix("Variant ") {
            name = value
                .trim()
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'));
        }
        rest = tag[end + 1..].trim_start();
    }
    Some(match kchess_domain::rules::pgn_variant(name)? {
        Variant::Chess => {
            let name = name.unwrap_or_default().to_lowercase();
            if name.contains("960") || name.contains("fischer") {
                "chess960"
            } else {
                "standard"
            }
        }
        Variant::Crazyhouse => "crazyhouse",
        Variant::KingOfTheHill => "kingOfTheHill",
        Variant::ThreeCheck => "threeCheck",
        Variant::Antichess => "antichess",
        Variant::Atomic => "atomic",
        Variant::Horde => "horde",
        Variant::RacingKings => "racingKings",
    })
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
mod pgn_tests {
    use super::pgn_analysable;

    #[test]
    fn only_standard_and_chess960_games_are_analysable() {
        assert!(pgn_analysable("1. e4 e5 *"));
        assert!(pgn_analysable("[Event \"x\"]\n\n1. e4 *"));
        assert!(pgn_analysable("[Variant \"Standard\"]\n\n1. e4 *"));
        assert!(pgn_analysable(
            "[Variant \"Chess960\"]\n[FEN \"x\"]\n\n1. e4 *"
        ));
        assert!(!pgn_analysable("[Variant \"Atomic\"]\n\n1. e4 *"));
        assert!(!pgn_analysable(
            "[Event \"x\"]\n[Variant \"Crazyhouse\"]\n\n1. e4 *"
        ));
        assert!(!pgn_analysable("[Variant \"Something else\"]\n\n1. e4 *"));
        // Pasted into a one-line field, the tags sit side by side.
        assert!(!pgn_analysable(
            "[Event \"x\"] [Variant \"Crazyhouse\"] 1. e4 *"
        ));
        assert_eq!(super::pgn_variant_key("1. e4 *"), Some("standard"));
        assert_eq!(
            super::pgn_variant_key("[Variant \"Chess960\"]\n\n*"),
            Some("chess960")
        );
        assert_eq!(
            super::pgn_variant_key("[Variant \"King of the Hill\"]\n\n*"),
            Some("kingOfTheHill")
        );
        assert_eq!(
            super::pgn_variant_key("[Variant \"Something else\"]\n\n*"),
            None
        );
    }
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

/// A variant's Lichess key, its name and its one-line hint (`data/variants.json`).
#[derive(serde::Deserialize, Clone, Debug)]
pub struct VariantText {
    pub key: String,
    pub label: String,
    pub hint: String,
}

/// Every variant KChess plays, standard first, in the order the variant pickers list them.
pub fn variants() -> &'static [VariantText] {
    static VARIANTS: std::sync::OnceLock<Vec<VariantText>> = std::sync::OnceLock::new();
    VARIANTS.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../../crates/kchess-domain/data/variants.json"
        ))
        .unwrap_or_default()
    })
}

/// A variant's name ("King of the Hill") from its key ("kingOfTheHill"); the key itself for one
/// KChess does not know.
pub fn variant_label(key: &str) -> String {
    variants()
        .iter()
        .find(|text| text.key == key)
        .map_or_else(|| key.to_string(), |text| text.label.clone())
}

#[cfg(test)]
mod variant_tests {
    #[test]
    fn every_variant_has_a_name_and_a_hint() {
        let all = super::variants();
        assert_eq!(
            all.first().map(|text| text.label.as_str()),
            Some("Standard")
        );
        assert!(
            all.iter()
                .all(|text| !text.label.is_empty() && !text.hint.is_empty())
        );
        assert_eq!(super::variant_label("kingOfTheHill"), "King of the Hill");
        assert_eq!(super::variant_label("racingKings"), "Racing Kings");
        assert_eq!(super::variant_label("fromPosition"), "fromPosition");
    }
}
