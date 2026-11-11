//! The app's game helpers: the shared session drivers (`kchess-games`), and the replay, tree,
//! puzzle and view helpers the pages use.

pub mod history;
pub mod low_time;
pub mod played;
pub mod puzzle;
pub mod session;
pub mod surface;
pub mod tree;
pub mod view;

/// The clocks both new-game dialogs offer (Play with computer, Over the board), as minutes and
/// increment.
pub const CLOCKS: &[(&str, Option<(f64, f64)>)] = &[
    ("No clock", None),
    ("3+2", Some((3., 2.))),
    ("5+0", Some((5., 0.))),
    ("5+3", Some((5., 3.))),
    ("10+0", Some((10., 0.))),
    ("10+5", Some((10., 5.))),
    ("15+10", Some((15., 10.))),
    ("30+0", Some((30., 0.))),
    ("60+30", Some((60., 30.))),
    ("90+30", Some((90., 30.))),
];

/// The usual starting position.
pub const STANDARD_FEN: &str = kchess_domain::rules::INITIAL_FEN;

use kchess_domain::variant::Variant;
use serde_json::{Value, json};

pub use kchess_domain::links::{
    BroadcastLink, GameLink, broadcast_link, game_link, is_username, pgn_orientation, profile_link,
    study_link,
};
pub use kchess_games::{computer, draw_color, local, random_chess960, random_unit};

/// A random draw, or none when the system had no randomness to give, which is logged: the
/// caller then leaves things as they are.
pub fn drawn<T>(draw: Result<T, String>) -> Option<T> {
    draw.map_err(|cause| crate::log::write(kchess_contracts::Level::Warn, "random", &cause))
        .ok()
}

/// A `{ variant, fen }` setup's UCI moves as PGN, as far as they are legal, after `headers` (a
/// JSON object of strings). `None` when the setup is not a legal one or a header is not one.
pub fn setup_pgn(setup: &Value, moves: &[String], headers: &Value) -> Option<String> {
    let (variant, fen) = (setup["variant"].as_str()?, setup["fen"].as_str()?);
    let variant = Variant::from_key(variant)?;
    let headers: Vec<(&str, &str)> = match headers {
        Value::Object(headers) => headers
            .iter()
            .map(|(name, value)| value.as_str().map(|value| (name.as_str(), value)))
            .collect::<Option<_>>()?,
        _ => Vec::new(),
    };
    kchess_domain::pgn::setup_pgn(variant, fen, moves, &headers).ok()
}

use kchess_contracts::Level;
use kchess_domain::training::engine_levels::{self, EngineLevelInfo};
use kchess_domain::training::openings::{self, OpeningName};

/// The opening a game set up as `setup` reached after `ply` of `moves`.
pub fn opening_at<S: AsRef<str>>(setup: &Value, moves: &[S], ply: usize) -> Option<OpeningName> {
    openings::opening_at(
        setup["variant"].as_str().unwrap_or_default(),
        setup["fen"].as_str().unwrap_or_default(),
        moves,
        ply as f64,
    )
}

/// A computer level's search profile; none for a level the ladder does not know.
pub fn engine_level(level: &str) -> Option<&'static EngineLevelInfo> {
    engine_levels::info(level).unwrap_or_else(|cause| {
        crate::log::write(Level::Warn, "engine-levels", &cause);
        None
    })
}

/// "Grandmaster · ~2550"; empty for a level the ladder does not know.
pub fn engine_level_label(level: &str) -> String {
    engine_levels::label(level).unwrap_or_else(|cause| {
        crate::log::write(Level::Warn, "engine-levels", &cause);
        String::new()
    })
}

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
/// The FEN of a board editor setup (`editor_setup`'s shape): castling and en passant only where
/// the pieces allow them. None, logged, for anything else.
pub fn setup_fen(setup: &Value) -> Option<String> {
    match serde_json::from_value::<kchess_domain::voice::EditorSetup>(setup.clone()) {
        Ok(setup) => Some(kchess_domain::voice::setup_fen(&setup)),
        Err(cause) => {
            crate::log::write(
                kchess_contracts::Level::Warn,
                "editor",
                &format!("not an editor setup: {cause}"),
            );
            None
        }
    }
}

pub fn editor_setup(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    let parsed = kchess_domain::position::fen_setup(trimmed)?;
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    let rights = castling_letters(&parsed.board, fields.get(2).copied().unwrap_or("-"));
    let ep = fields
        .get(3)
        .copied()
        .filter(|ep| {
            let bytes = ep.as_bytes();
            bytes.len() == 2 && (b'a'..=b'h').contains(&bytes[0]) && matches!(bytes[1], b'3' | b'6')
        })
        .unwrap_or_default();
    Some(json!({
        "board": parsed.board,
        "turn": parsed.turn,
        "castling": {
            "K": rights.contains('K'),
            "Q": rights.contains('Q'),
            "k": rights.contains('k'),
            "q": rights.contains('q'),
        },
        "ep": ep,
    }))
}

/// Castling rights as `KQkq`: Shredder-FEN and X-FEN name a rook by its file (`HAha`), which is
/// the short side when it stands right of its king.
fn castling_letters(board: &str, rights: &str) -> String {
    let king_file = |rank: usize, king: char| {
        let mut file = 0;
        for c in board.split('/').nth(rank).unwrap_or_default().chars() {
            match c.to_digit(10) {
                Some(skip) => file += skip as usize,
                None if c == king => return Some(file),
                None => file += 1,
            }
        }
        None
    };
    rights
        .chars()
        .filter_map(|c| match c {
            'K' | 'Q' | 'k' | 'q' => Some(c),
            'A'..='H' | 'a'..='h' => {
                let white = c.is_ascii_uppercase();
                let king = king_file(if white { 7 } else { 0 }, if white { 'K' } else { 'k' })?;
                let file = (c.to_ascii_lowercase() as u8 - b'a') as usize;
                let short = file > king;
                Some(match (white, short) {
                    (true, true) => 'K',
                    (true, false) => 'Q',
                    (false, true) => 'k',
                    (false, false) => 'q',
                })
            }
            _ => None,
        })
        .collect()
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

/// Whether the analysis board can take a game in this variant: every variant KChess plays. A
/// variant other than standard chess and Chess960 is analysed by Fairy-Stockfish.
pub fn analysable(variant: &str) -> bool {
    Variant::from_key(variant).is_some()
}

/// A FEN as a position of the variant a Lichess key names; none for a variant KChess does not
/// play or a position its rules refuse.
pub fn setup_position(variant: &str, fen: &str) -> Option<shakmaty::variant::VariantPosition> {
    kchess_domain::rules::setup_start(Variant::from_key(variant)?, fen)
}

/// The usual start of the variant a Lichess key names; none for one KChess does not play.
pub fn default_fen(variant: &str) -> Option<String> {
    Variant::from_key(variant).map(Variant::default_fen)
}

/// Whether a game in this variant is played by standard rules: standard chess, and Chess960, which
/// plays with king-takes-rook castling. Stockfish reviews them and their mistakes become drills;
/// Fairy-Stockfish reviews the other variants.
pub fn standard_rules(variant: &str) -> bool {
    Variant::from_key(variant).is_some_and(Variant::standard_rules)
}

/// Whether a PGN's `Variant` header names a game the analysis board can play (any variant KChess
/// knows, or none at all).
pub fn pgn_analysable(pgn: &str) -> bool {
    pgn_variant_key(pgn).is_some_and(analysable)
}

/// The Lichess key of the variant a PGN's `Variant` header names ("atomic", "chess960"), standard
/// without one; `None` for a variant KChess does not know.
pub fn pgn_variant_key(pgn: &str) -> Option<&'static str> {
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
    Variant::from_pgn_name(name).map(Variant::key)
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
    use super::{pgn_analysable, pgn_variant_key, standard_rules};

    #[test]
    fn known_variants_are_analysable_and_standard_ones_follow_standard_rules() {
        for pgn in [
            "1. e4 e5 *",
            "[Event \"x\"]\n\n1. e4 *",
            "[Variant \"Standard\"]\n\n1. e4 *",
            "[Variant \"Chess960\"]\n[FEN \"x\"]\n\n1. e4 *",
        ] {
            assert!(
                pgn_analysable(pgn) && pgn_variant_key(pgn).is_some_and(standard_rules),
                "{pgn}"
            );
        }
        for pgn in [
            "[Variant \"Atomic\"]\n\n1. e4 *",
            "[Event \"x\"]\n[Variant \"Crazyhouse\"]\n\n1. e4 *",
            // Pasted into a one-line field, the tags sit side by side.
            "[Event \"x\"] [Variant \"Crazyhouse\"] 1. e4 *",
        ] {
            assert!(
                pgn_analysable(pgn) && !pgn_variant_key(pgn).is_some_and(standard_rules),
                "{pgn}"
            );
        }
        assert!(!pgn_analysable("[Variant \"Something else\"]\n\n1. e4 *"));
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

    /// A Chess960 position pasted into the board editor keeps its castling, written as `KQkq`,
    /// and plays as Chess960 from there.
    #[test]
    fn the_editor_keeps_a_chess960_positions_castling() {
        for pasted in [
            "bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w HFhf - 0 1",
            "bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w KQkq - 0 1",
        ] {
            let setup = super::editor_setup(pasted).expect("a position");
            assert_eq!(
                setup["castling"],
                serde_json::json!({ "K": true, "Q": true, "k": true, "q": true }),
                "{pasted}"
            );
            let fen = super::setup_fen(&setup).expect("its FEN");
            assert_eq!(
                fen,
                "bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w KQkq - 0 1"
            );
            assert_eq!(setup_from_fen(&fen)["variant"], "chess960");
        }
        // Only the rights written: the long side's rook (F) on its own.
        let setup = super::editor_setup("bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w F - 0 1");
        assert_eq!(
            setup.expect("a position")["castling"],
            serde_json::json!({ "K": false, "Q": true, "k": false, "q": false })
        );
    }
}

/// A variant's Lichess key, its name and its one-line hint.
#[derive(Clone, Debug)]
pub struct VariantText {
    pub key: String,
    pub label: String,
    pub hint: String,
}

/// Every variant KChess plays, standard first, in the order the variant pickers list them.
pub fn variants() -> &'static [VariantText] {
    static VARIANTS: std::sync::OnceLock<Vec<VariantText>> = std::sync::OnceLock::new();
    VARIANTS.get_or_init(|| {
        Variant::ALL
            .into_iter()
            .map(|variant| VariantText {
                key: variant.key().into(),
                label: variant.label().into(),
                hint: variant.hint().into(),
            })
            .collect()
    })
}

/// A variant's name ("King of the Hill") from its key ("kingOfTheHill"); the key itself for one
/// KChess does not know.
pub fn variant_label(key: &str) -> String {
    Variant::from_key(key)
        .map_or(key, |variant| variant.label())
        .to_string()
}

/// What a game page names its start by: the variant ("Crazyhouse"), or "From position" for
/// standard chess from another position; nothing for the ordinary game.
pub fn setup_label(setup: &Value) -> Option<String> {
    let variant = setup["variant"].as_str().unwrap_or("standard");
    if variant == "standard" {
        (setup["fen"].as_str() != Some(STANDARD_FEN)).then(|| "From position".to_string())
    } else {
        Some(variant_label(variant))
    }
}

#[cfg(test)]
mod variant_tests {
    use serde_json::json;

    /// The game pages name a variant, or a standard game's other start, and not the usual game.
    #[test]
    fn a_setup_is_named_by_its_variant_or_its_start() {
        let label = |variant: &str, fen: &str| {
            super::setup_label(&json!({ "variant": variant, "fen": fen }))
        };
        assert_eq!(label("standard", super::STANDARD_FEN), None);
        assert_eq!(
            label("standard", "4k3/8/8/8/8/8/8/4K2R w K - 0 1").as_deref(),
            Some("From position")
        );
        assert_eq!(
            label("kingOfTheHill", super::STANDARD_FEN).as_deref(),
            Some("King of the Hill")
        );
        assert_eq!(
            label("chess960", super::STANDARD_FEN).as_deref(),
            Some("Chess960")
        );
    }

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
