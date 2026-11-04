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
    let variant = pgn
        .lines()
        .map(str::trim)
        .take_while(|line| line.is_empty() || line.starts_with('['))
        .find_map(|line| {
            let rest = line.strip_prefix("[Variant ")?;
            rest.trim_end_matches(']')
                .trim()
                .strip_prefix('"')?
                .strip_suffix('"')
        });
    kchess_domain::rules::pgn_variant(variant) == Some(shakmaty::variant::Variant::Chess)
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

/// A Lichess game link pasted in place of a PGN: the game's id, the side the link looks from
/// (`/black`), and the move it points at (`#23`).
#[derive(Debug, PartialEq)]
pub struct GameLink {
    pub id: String,
    pub orientation: shakmaty::Color,
    pub ply: Option<usize>,
}

/// Reads `lichess.org/abcdefgh`, `https://lichess.org/abcdefgh1234/black#12` and the like; `None`
/// for anything else, a study or a puzzle included.
pub fn game_link(text: &str) -> Option<GameLink> {
    const NOT_GAMES: [&str; 9] = [
        "analysis", "training", "practice", "insights", "streamer", "learning", "tutorial",
        "settings", "features",
    ];
    let text = text.trim();
    let rest = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))
        .unwrap_or(text);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let rest = rest.strip_prefix("lichess.org/")?;
    let (path, anchor) = rest.split_once('#').unwrap_or((rest, ""));
    let path = path.split('?').next().unwrap_or_default();
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let first = parts.next()?;
    let side = parts.next();
    if parts.next().is_some()
        || !matches!(first.len(), 8 | 12)
        || !first.bytes().all(|byte| byte.is_ascii_alphanumeric())
        || NOT_GAMES.contains(&first)
    {
        return None;
    }
    let orientation = match side {
        None | Some("white") => shakmaty::Color::White,
        Some("black") => shakmaty::Color::Black,
        Some(_) => return None,
    };
    Some(GameLink {
        id: first[..8].to_string(),
        orientation,
        ply: anchor.parse().ok(),
    })
}

#[cfg(test)]
mod link_tests {
    use super::{GameLink, game_link};
    use shakmaty::Color;

    #[test]
    fn a_lichess_game_link_names_the_game_side_and_move() {
        let link = |id: &str, orientation, ply| {
            Some(GameLink {
                id: id.to_string(),
                orientation,
                ply,
            })
        };
        assert_eq!(
            game_link("https://lichess.org/q7ZvsdUF"),
            link("q7ZvsdUF", Color::White, None)
        );
        assert_eq!(
            game_link(" lichess.org/q7ZvsdUFab12/black#23 "),
            link("q7ZvsdUF", Color::Black, Some(23))
        );
        assert_eq!(
            game_link("http://www.lichess.org/q7ZvsdUF/white?x=1"),
            link("q7ZvsdUF", Color::White, None)
        );
        for other in [
            "q7ZvsdUF",
            "https://lichess.org/study/abcdefgh",
            "https://lichess.org/training/abcde",
            "https://lichess.org/analysis",
            "https://lichess.org/q7ZvsdUF/red",
            "https://example.org/q7ZvsdUF",
            "https://lichess.org/q7Zv-dUF",
        ] {
            assert_eq!(game_link(other), None, "{other}");
        }
    }
}
