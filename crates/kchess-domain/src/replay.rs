//! Replaying games (`replaySetup` in `variant.ts`, `replay` in `review.ts`).

use serde::Serialize;
use shakmaty::Position;
use shakmaty::variant::VariantPosition;

use crate::js;
use crate::rules;

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ReplayedMove {
    pub uci: String,
    pub san: String,
}

#[derive(Debug, Serialize)]
pub struct Replay {
    pub played: Vec<ReplayedMove>,
    /// The position after the last legal move.
    pub fen: String,
}

/// Replay UCI moves under a setup's rules, stopping at the first illegal move; `None` when the
/// setup is not a legal position. Castling may be king-to-rook or king-two-squares.
pub fn replay_setup<S: AsRef<str>>(variant: &str, fen: &str, moves: &[S]) -> Option<Replay> {
    let mut pos = rules::setup_start(variant, fen)?;
    let played = play_line(&mut pos, moves, true);
    Some(Replay {
        fen: rules::make_fen(&pos),
        played,
    })
}

/// How many moves `replaySetup` accepts, without building SAN (the validation decoders need).
pub fn replay_setup_count<S: AsRef<str>>(variant: &str, fen: &str, moves: &[S]) -> Option<usize> {
    let mut pos = rules::setup_start(variant, fen)?;
    Some(count_line(&mut pos, moves))
}

fn play_line<S: AsRef<str>>(
    pos: &mut VariantPosition,
    moves: &[S],
    trim: bool,
) -> Vec<ReplayedMove> {
    let mut played = Vec::with_capacity(moves.len());
    for uci in moves {
        let text = uci.as_ref();
        let parsed = if trim { js::trim(text) } else { text };
        let Some(m) = rules::parse_uci(parsed).and_then(|u| rules::legal_move(pos, u)) else {
            break;
        };
        let san = rules::san_and_play(pos, m);
        played.push(ReplayedMove {
            uci: text.to_string(),
            san,
        });
    }
    played
}

fn count_line<S: AsRef<str>>(pos: &mut VariantPosition, moves: &[S]) -> usize {
    for (count, uci) in moves.iter().enumerate() {
        let Some(m) =
            rules::parse_uci(js::trim(uci.as_ref())).and_then(|u| rules::legal_move(pos, u))
        else {
            return count;
        };
        pos.play_unchecked(m);
    }
    moves.len()
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ReplayedPosition {
    pub fen: String,
    pub turn: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<&'static str>,
}

/// `review.replay`'s `end`: checkmate, or a draw by stalemate or insufficient material.
fn end_of(pos: &VariantPosition) -> Option<&'static str> {
    let legal_empty = pos.legal_moves().is_empty();
    let checkmate = !pos.is_variant_end() && pos.is_check() && legal_empty;
    let stalemate = !pos.is_variant_end() && !pos.is_check() && legal_empty;
    if checkmate {
        Some("checkmate")
    } else if stalemate || pos.is_insufficient_material() {
        Some("draw")
    } else {
        None
    }
}

fn describe(pos: &VariantPosition) -> ReplayedPosition {
    ReplayedPosition {
        fen: rules::make_fen(pos),
        turn: rules::turn_name(pos.turn()),
        end: end_of(pos),
    }
}

/// `review.replay`: standard rules from `fen`, the start first, as far as the moves are legal.
pub fn replay_positions<S: AsRef<str>>(fen: &str, moves: &[S]) -> Vec<ReplayedPosition> {
    let Some(mut pos) = rules::standard_from_fen(fen) else {
        return Vec::new();
    };
    let mut positions = Vec::with_capacity(moves.len() + 1);
    positions.push(describe(&pos));
    for uci in moves {
        let Some(m) = rules::parse_uci(uci.as_ref()).and_then(|u| rules::legal_move(&pos, u))
        else {
            break;
        };
        pos.play_unchecked(m);
        positions.push(describe(&pos));
    }
    positions
}

/// Side to move and game end of each position `review.replay` returns, without their FENs.
pub fn replay_turns<S: AsRef<str>>(fen: &str, moves: &[S]) -> Vec<(bool, Option<&'static str>)> {
    let Some(mut pos) = rules::standard_from_fen(fen) else {
        return Vec::new();
    };
    let state = |pos: &VariantPosition| {
        let described = end_of(pos);
        (pos.turn().is_white(), described)
    };
    let mut turns = Vec::with_capacity(moves.len() + 1);
    turns.push(state(&pos));
    for uci in moves {
        let Some(m) = rules::parse_uci(uci.as_ref()).and_then(|u| rules::legal_move(&pos, u))
        else {
            break;
        };
        pos.play_unchecked(m);
        turns.push(state(&pos));
    }
    turns
}

/// How many positions `review.replay` returns.
pub fn replay_positions_count<S: AsRef<str>>(fen: &str, moves: &[S]) -> usize {
    let Some(mut pos) = rules::standard_from_fen(fen) else {
        return 0;
    };
    for (count, uci) in moves.iter().enumerate() {
        let Some(m) = rules::parse_uci(uci.as_ref()).and_then(|u| rules::legal_move(&pos, u))
        else {
            return count + 1;
        };
        pos.play_unchecked(m);
    }
    moves.len() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    #[test]
    fn replays_until_the_first_illegal_move() {
        let replay = replay_setup("standard", START, &["e2e4", "e7e5", "e1e3", "d2d4"]).unwrap();
        assert_eq!(
            replay
                .played
                .iter()
                .map(|m| m.san.as_str())
                .collect::<Vec<_>>(),
            ["e4", "e5"]
        );
        assert_eq!(
            replay_setup_count("standard", START, &["e2e4", "e7e5", "e1e3"]),
            Some(2)
        );
    }

    #[test]
    fn castling_is_accepted_both_ways() {
        let line = ["e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "g8f6", "e1g1"];
        assert_eq!(
            replay_setup("standard", START, &line).unwrap().played[6].san,
            "O-O"
        );
        let mut rook = line;
        rook[6] = "e1h1";
        assert_eq!(
            replay_setup("standard", START, &rook).unwrap().played[6].san,
            "O-O"
        );
    }

    #[test]
    fn positions_mark_checkmate() {
        let positions = replay_positions(START, &["f2f3", "e7e5", "g2g4", "d8h4"]);
        assert_eq!(positions.len(), 5);
        assert_eq!(positions[4].end, Some("checkmate"));
        assert_eq!(
            replay_positions_count(START, &["f2f3", "e7e5", "g2g4", "d8h4"]),
            5
        );
    }
}
