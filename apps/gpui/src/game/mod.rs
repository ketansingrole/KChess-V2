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

use kchess_domain::variant::{Setup, Variant};
use serde_json::{Value, json};

pub use kchess_domain::links::{
    BroadcastLink, GameLink, broadcast_link, game_link, is_username, pgn_orientation, profile_link,
    study_link,
};

/// A random draw, or none when the system had no randomness to give, which is logged: the
/// caller then leaves things as they are.
pub fn drawn<T>(draw: Result<T, String>) -> Option<T> {
    draw.map_err(|cause| crate::log::write(kchess_contracts::Level::Warn, "random", &cause))
        .ok()
}

/// The setup a stored document or a Lichess payload names (`{ variant, fen }`): standard chess
/// when it names no variant; none for a variant KChess does not play.
pub fn setup_of(value: &Value) -> Option<Setup> {
    let variant = match value["variant"].as_str() {
        None => Variant::Standard,
        Some(key) => Variant::from_key(key)?,
    };
    Some(Setup {
        variant,
        fen: value["fen"].as_str().unwrap_or_default().to_string(),
    })
}

/// A setup's UCI moves as PGN, as far as they are legal, after `headers` (a JSON object of
/// strings). `None` when the setup is not a legal one or a header is not one.
pub fn setup_pgn(setup: &Setup, moves: &[String], headers: &Value) -> Option<String> {
    let headers: Vec<(&str, &str)> = match headers {
        Value::Object(headers) => headers
            .iter()
            .map(|(name, value)| value.as_str().map(|value| (name.as_str(), value)))
            .collect::<Option<_>>()?,
        _ => Vec::new(),
    };
    kchess_domain::pgn::setup_pgn(setup.variant, &setup.fen, moves, &headers).ok()
}

use kchess_contracts::Level;
use kchess_domain::training::engine_levels::{self, EngineLevelInfo};
use kchess_domain::training::openings::{self, OpeningName};

/// The opening a game set up as `setup` reached after `ply` of `moves`.
pub fn opening_at<S: AsRef<str>>(setup: &Setup, moves: &[S], ply: usize) -> Option<OpeningName> {
    openings::opening_at(setup.variant.key(), &setup.fen, moves, ply as f64)
}

/// A computer level's search profile; none for a level the ladder does not know.
pub fn engine_level(level: &str) -> Option<&'static EngineLevelInfo> {
    engine_levels::info(level)
}

/// "Grandmaster · ~2550"; empty for a level the ladder does not know.
pub fn engine_level_label(level: &str) -> String {
    engine_levels::label(level).unwrap_or_else(|cause| {
        crate::log::write(Level::Warn, "engine-levels", &cause);
        String::new()
    })
}

/// "Oct 22, 2026" in local time.
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

/// The Chess960 start numbered `n`.
pub fn chess960(n: i64) -> String {
    kchess_domain::rules::chess960_fen(n)
}

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

/// Why a position cannot be played, in the words the board editor uses.
pub fn position_problem(fen: &str) -> Option<&'static str> {
    kchess_domain::position::fen_problem(fen).map(kchess_domain::position::FenProblem::message)
}

/// Why a typed FEN cannot start a new game of `variant`: what is wrong with a standard position,
/// or that another variant's rules refuse it.
pub fn setup_problem(variant: Variant, fen: &str) -> Option<&'static str> {
    if variant == Variant::Standard
        && let Some(problem) = position_problem(fen)
    {
        return Some(problem);
    }
    kchess_domain::rules::setup_start(variant, fen)
        .is_none()
        .then_some("That FEN is not a legal position")
}

/// Whether a PGN's `Variant` header names a game the analysis board can play (any variant KChess
/// knows, or none at all).
pub fn pgn_analysable(pgn: &str) -> bool {
    pgn_variant(pgn).is_some()
}

/// The variant a PGN's `Variant` header names, standard without one; `None` for a variant KChess
/// does not know.
pub fn pgn_variant(pgn: &str) -> Option<Variant> {
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
    Variant::from_pgn_name(name)
}

/// A position's setup for a game started from it: Chess960 when it can still castle with a king
/// or rook off their usual squares, which an engine reads only with `UCI_Chess960` on, and
/// standard otherwise.
pub fn setup_from_fen(fen: &str) -> Setup {
    Setup {
        variant: if chess960_castling(fen) {
            Variant::Chess960
        } else {
            Variant::Standard
        },
        fen: fen.to_string(),
    }
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
    use super::{pgn_analysable, pgn_variant};
    use kchess_domain::variant::Variant;

    #[test]
    fn known_variants_are_analysable_and_standard_ones_follow_standard_rules() {
        for pgn in [
            "1. e4 e5 *",
            "[Event \"x\"]\n\n1. e4 *",
            "[Variant \"Standard\"]\n\n1. e4 *",
            "[Variant \"Chess960\"]\n[FEN \"x\"]\n\n1. e4 *",
        ] {
            assert!(
                pgn_analysable(pgn) && pgn_variant(pgn).is_some_and(Variant::standard_rules),
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
                pgn_analysable(pgn) && !pgn_variant(pgn).is_some_and(Variant::standard_rules),
                "{pgn}"
            );
        }
        assert!(!pgn_analysable("[Variant \"Something else\"]\n\n1. e4 *"));
        assert_eq!(pgn_variant("1. e4 *"), Some(Variant::Standard));
        assert_eq!(
            pgn_variant("[Variant \"Chess960\"]\n\n*"),
            Some(Variant::Chess960)
        );
        assert_eq!(
            pgn_variant("[Variant \"King of the Hill\"]\n\n*"),
            Some(Variant::KingOfTheHill)
        );
        assert_eq!(pgn_variant("[Variant \"Something else\"]\n\n*"), None);
    }
}

#[cfg(test)]
mod setup_tests {
    use super::setup_from_fen;

    #[test]
    fn a_position_castling_from_unusual_squares_is_chess960() {
        use kchess_domain::variant::Variant;
        let variant = |fen: &str| setup_from_fen(fen).variant;
        assert_eq!(variant(super::STANDARD_FEN), Variant::Standard);
        assert_eq!(variant("4k3/8/8/8/8/8/8/4K3 w - - 0 1"), Variant::Standard);
        assert_eq!(variant(&super::chess960(0)), Variant::Chess960);
        assert_eq!(
            variant("1rk4r/8/8/8/8/8/8/1RK4R w HBhb - 0 1"),
            Variant::Chess960
        );
        // Its king and rooks moved, so the position no longer castles at all.
        assert_eq!(
            variant("1r5r/2k5/8/8/8/8/2K5/1R5R w - - 0 1"),
            Variant::Standard
        );
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
            assert_eq!(
                setup_from_fen(&fen).variant,
                kchess_domain::variant::Variant::Chess960
            );
        }
        // Only the rights written: the long side's rook (F) on its own.
        let setup = super::editor_setup("bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w F - 0 1");
        assert_eq!(
            setup.expect("a position")["castling"],
            serde_json::json!({ "K": false, "Q": true, "k": false, "q": false })
        );
    }
}

/// A variant's name ("King of the Hill") from its Lichess key ("kingOfTheHill"); the key itself
/// for one KChess does not know.
pub fn variant_label(key: &str) -> &str {
    Variant::from_key(key).map_or(key, |variant| variant.label())
}

/// What a game page names its start by: the variant ("Crazyhouse"), or "From position" for
/// standard chess from another position; nothing for the ordinary game.
pub fn setup_label(setup: &Setup) -> Option<&'static str> {
    if setup.variant == Variant::Standard {
        (setup.fen != STANDARD_FEN).then_some("From position")
    } else {
        Some(setup.variant.label())
    }
}

#[cfg(test)]
mod variant_tests {
    use kchess_domain::variant::{Setup, Variant};

    /// The game pages name a variant, or a standard game's other start, and not the usual game.
    #[test]
    fn a_setup_is_named_by_its_variant_or_its_start() {
        let label = |variant: Variant, fen: &str| {
            super::setup_label(&Setup {
                variant,
                fen: fen.into(),
            })
        };
        assert_eq!(label(Variant::Standard, super::STANDARD_FEN), None);
        assert_eq!(
            label(Variant::Standard, "4k3/8/8/8/8/8/8/4K2R w K - 0 1"),
            Some("From position")
        );
        assert_eq!(
            label(Variant::KingOfTheHill, super::STANDARD_FEN),
            Some("King of the Hill")
        );
        assert_eq!(
            label(Variant::Chess960, super::STANDARD_FEN),
            Some("Chess960")
        );
    }

    #[test]
    fn a_variant_is_named_by_its_key() {
        assert_eq!(super::variant_label("kingOfTheHill"), "King of the Hill");
        assert_eq!(super::variant_label("fromPosition"), "fromPosition");
    }

    /// A stored setup reads as it was written; one without a variant is standard chess, and one
    /// in a variant KChess does not play is none.
    #[test]
    fn a_stored_setup_reads_back() {
        let setup = Setup::of(Variant::Crazyhouse);
        assert_eq!(
            super::setup_of(&serde_json::json!(setup)).as_ref(),
            Some(&setup)
        );
        assert_eq!(
            serde_json::json!(setup),
            serde_json::json!({ "variant": "crazyhouse", "fen": setup.fen })
        );
        assert_eq!(
            super::setup_of(&serde_json::json!({ "fen": super::STANDARD_FEN })),
            Some(Setup::standard())
        );
        assert_eq!(
            super::setup_of(&serde_json::json!({ "variant": "bughouse", "fen": "" })),
            None
        );
    }

    /// Both new-game dialogs name a standard position's problem and refuse what another
    /// variant's rules do.
    #[test]
    fn a_typed_start_is_checked_by_its_variant() {
        assert_eq!(
            super::setup_problem(Variant::Standard, "8/8/8/8/8/8/8/8 w - - 0 1"),
            Some("The board is empty")
        );
        assert_eq!(
            super::setup_problem(Variant::Standard, super::STANDARD_FEN),
            None
        );
        assert_eq!(
            super::setup_problem(Variant::Horde, &Variant::Horde.default_fen()),
            None
        );
        assert_eq!(
            super::setup_problem(Variant::Horde, "8/8/8/8/8/8/8/8 w - - 0 1"),
            Some("That FEN is not a legal position")
        );
    }
}
