//! The arguments of the engine methods: the computer's level and options, and analysis requests.

use kchess_domain::misc::value::J;
use kchess_domain::replay;
use kchess_domain::variant::{VARIANTS, Variant};

use super::*;
use crate::types::{AnalysisRequest, BestMoveOptions, ENGINE_LEVELS, EngineLevel};

/// The largest integer a JSON number holds exactly.
const MAX_SAFE: u64 = 9_007_199_254_740_991;
const LINES: Msg = Some("Invalid number of engine lines.");
const THINK_TIME: Msg = Some("Invalid think time.");

impl Validate for EngineLevel {
    fn validate(value: J) -> Result<Self, Refusal> {
        picked(value, ENGINE_LEVELS, Some("Invalid engine level."))
    }
}

/// A variant KChess plays, by its Lichess key.
impl Validate for Variant {
    fn validate(value: J) -> Result<Self, Refusal> {
        picked(value, VARIANTS, Some("Invalid variant."))
    }
}

/// Another variant's start must be legal by its rules; a standard one is a FEN the engine takes.
/// Absent options are the defaults. A variant with standard rules is dropped.
impl Validate for BestMoveOptions {
    fn validate(value: J) -> Result<Self, Refusal> {
        if matches!(value, J::Undef) {
            return Ok(BestMoveOptions::default());
        }
        let mut options: BestMoveOptions = decode(value, |keys| match keys {
            ["fen"] => Some("Invalid position."),
            ["movetime"] => THINK_TIME,
            ["variant"] => Some("Invalid variant."),
            _ => Some("Invalid engine options."),
        })?;
        if let Some(variant) = other_variant(options.variant.as_deref())? {
            let position = options.fen.as_deref().ok_or("Invalid position.")?;
            max_chars(position, 120, Some("Invalid position."), "fen")?;
            if kchess_domain::rules::setup_start(variant, position).is_none() {
                return Err("Invalid position.".into());
            }
        } else {
            options.variant = None;
            if let Some(position) = &options.fen {
                fen(position, None, Some("Invalid position."), "fen")?;
                engine_position(position)?;
            }
        }
        if let Some(movetime) = options.movetime {
            within(movetime, 50, 5000, THINK_TIME, "")?;
        }
        Ok(options)
    }
}

/// A variant game's position for analysis: its root under the variant's rules, and its moves
/// played there to the position asked about. Its FEN may carry what the variant keeps
/// (Crazyhouse's pockets, Three-check's checks) and Horde's 36 pawns.
fn variant_analysis_position(request: &AnalysisRequest, variant: Variant) -> Result<(), Refusal> {
    for text in std::iter::once(&request.fen).chain(request.root_fen.as_ref()) {
        max_chars(text, 120, None, "fen")?;
    }
    let root = request.root_fen.as_deref().unwrap_or(&request.fen);
    let moves = request.moves.as_deref().unwrap_or_default();
    if moves.len() > 600 {
        return Err("Invalid move list.".into());
    }
    let replay = replay::replay_setup(variant, root, moves).ok_or("Invalid position.")?;
    let asked = replay::replay_setup(variant, &request.fen, &Vec::<&str>::new())
        .ok_or("Invalid position.")?;
    if replay.played.len() != moves.len() || replay.fen != asked.fen {
        return Err("Invalid analysis position or move history.".into());
    }
    Ok(())
}

/// The request's shape, and a position its moves reach.
impl Validate for AnalysisRequest {
    fn validate(value: J) -> Result<Self, Refusal> {
        let request: AnalysisRequest = decode(value, |keys| match keys {
            [] | ["infinite"] => Some("Invalid engine options."),
            ["fen"] => Some("Invalid position."),
            ["lines"] => LINES,
            ["moves", rest @ ..] => move_list_messages(rest),
            ["clientId"] => Some("Invalid analysis request."),
            ["variant"] => Some("Invalid variant."),
            _ => None,
        })?;
        let client = |request: &AnalysisRequest| match request.client_id {
            Some(client) => within(client, 1, MAX_SAFE, Some("Invalid analysis request."), ""),
            None => Ok(()),
        };
        // Standard chess and Chess960 are checked as they always were, below.
        if let Some(variant) = other_variant(request.variant.as_deref())? {
            within(request.lines, 1, 5, LINES, "")?;
            client(&request)?;
            variant_analysis_position(&request, variant)?;
            return Ok(request);
        }
        fen(&request.fen, None, Some("Invalid position."), "fen")?;
        engine_position(&request.fen)?;
        within(request.lines, 1, 5, LINES, "")?;
        if let Some(root) = &request.root_fen {
            fen(root, None, None, "rootFen")?;
        }
        if let Some(moves) = &request.moves {
            move_list(moves)?;
        }
        client(&request)?;
        let root = request.root_fen.as_deref().unwrap_or(&request.fen);
        let moves = request.moves.as_deref().unwrap_or_default();
        // Only the final position matters: replay without a FEN and end check for every ply.
        let last = replay::fen_after(root, moves, moves.len());
        let normalized = replay::fen_after(&request.fen, &[] as &[&str], 0);
        if normalized.is_none() || last != normalized {
            return Err("Invalid analysis position or move history.".into());
        }
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_and_a_variant_are_named() {
        assert_eq!(
            check::<EngineLevel>(J::Str("strong-club".into())),
            Ok(EngineLevel::StrongClub)
        );
        assert_eq!(
            check::<EngineLevel>(J::Str("grandmaster".into())),
            Err("Invalid engine level.".into())
        );
        assert_eq!(
            check::<Variant>(J::Str("crazyhouse".into())),
            Ok(Variant::Crazyhouse)
        );
        assert_eq!(
            check::<Variant>(J::Str("chess".into())),
            Err("Invalid variant.".into())
        );
    }

    /// A value of the wrong type keeps the message of its field.
    #[test]
    fn each_option_keeps_its_message() {
        use serde_json::json;
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert_eq!(
            checked::<BestMoveOptions>(json!({ "movetime": "fast" })),
            Err(THINK_TIME.unwrap().into())
        );
        assert_eq!(
            checked::<BestMoveOptions>(json!({ "movetime": 10 })),
            Err(THINK_TIME.unwrap().into())
        );
        for (field, value, message) in [
            ("lines", json!("all"), LINES.unwrap()),
            ("moves", json!("e2e4"), "Invalid move list."),
            ("moves", json!([1]), "Invalid move."),
        ] {
            let mut request = json!({ "fen": start, "lines": 1 });
            request[field] = value;
            assert_eq!(
                checked::<AnalysisRequest>(request),
                Err(message.into()),
                "{field}"
            );
        }
    }

    /// A variant's moves must reach the position asked about from its root.
    #[test]
    fn a_variant_analysis_replays_its_moves() {
        use serde_json::json;
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
        let after = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR[] b KQkq - 0 1";
        let request = |fen: &str| {
            json!({
                "fen": fen, "rootFen": start, "moves": ["e2e4"], "lines": 1,
                "variant": "crazyhouse",
            })
        };
        assert!(checked::<AnalysisRequest>(request(after)).is_ok());
        assert_eq!(
            checked::<AnalysisRequest>(request(start)),
            Err("Invalid analysis position or move history.".into())
        );
    }

    fn obj(fields: Vec<(&str, J)>) -> J {
        J::Obj(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    #[test]
    fn absent_options_are_the_defaults() {
        assert_eq!(
            check::<BestMoveOptions>(J::Undef),
            Ok(BestMoveOptions::default())
        );
        // A list is an object of its indices.
        assert_eq!(
            check::<BestMoveOptions>(J::Arr(vec![])),
            Ok(BestMoveOptions::default())
        );
    }

    #[test]
    fn the_engine_is_not_given_positions_stockfish_refuses() {
        // Black has a seventeenth piece: Stockfish quit on it ("More than 32 pieces").
        let crowded = "rnbqkbnr/pppppppp/8/8/3q3N/4K3/1PPPPPPP/RNBQ1BNR w kq - 0 1";
        let refused = "Each side can have at most 16 pieces.".to_string();
        let analysis = obj(vec![
            ("fen", J::Str(crowded.into())),
            ("lines", J::Num(3.0)),
        ]);
        assert_eq!(check::<AnalysisRequest>(analysis), Err(refused.clone()));
        let search = obj(vec![("fen", J::Str(crowded.into()))]);
        assert_eq!(check::<BestMoveOptions>(search), Err(refused));
        // Horde's thirty-six pawns are Horde's: Fairy-Stockfish plays them.
        let horde = "rnbqkbnr/pppppppp/8/1PP2PP1/PPPPPPPP/PPPPPPPP/PPPPPPPP/PPPPPPPP w kq - 0 1";
        let search = obj(vec![
            ("fen", J::Str(horde.into())),
            ("variant", J::Str("horde".into())),
        ]);
        assert_eq!(
            check::<BestMoveOptions>(search),
            Ok(BestMoveOptions {
                fen: Some(horde.into()),
                variant: Some("horde".into()),
                ..BestMoveOptions::default()
            })
        );
        let search = obj(vec![
            ("fen", J::Str(horde.into())),
            ("variant", J::Str("crazyhouse".into())),
        ]);
        assert_eq!(
            check::<BestMoveOptions>(search),
            Err("Invalid position.".into())
        );
        let search = obj(vec![("variant", J::Str("bughouse".into()))]);
        assert_eq!(
            check::<BestMoveOptions>(search),
            Err("Invalid variant.".into())
        );
    }
}
