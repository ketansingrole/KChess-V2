//! Replaying games (`replaySetup`, `replay`).

use serde::Serialize;
use shakmaty::variant::VariantPosition;
use shakmaty::{CastlingMode, Position};

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

/// The side to move once `replaySetup` has played what it accepts, and how many moves that is,
/// without building SAN or a FEN.
pub fn replay_setup_turn<S: AsRef<str>>(
    variant: &str,
    fen: &str,
    moves: &[S],
) -> Option<(usize, shakmaty::Color)> {
    let mut pos = rules::setup_start(variant, fen)?;
    let count = count_line(&mut pos, moves);
    Some((count, pos.turn()))
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

/// `review.replay`'s `end`: checkmate, or a draw by stalemate or insufficient material. A
/// variant's own ending is `checkmate` when the side to move has lost (its king exploded, the
/// other king reached the hill…), `won` when it has won (Antichess), or a `draw`.
fn end_of(pos: &VariantPosition) -> Option<&'static str> {
    if let Some(end) = variant_end_of(pos) {
        return Some(end);
    }
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

fn variant_end_of(pos: &VariantPosition) -> Option<&'static str> {
    if matches!(
        pos,
        VariantPosition::Chess(_) | VariantPosition::Crazyhouse(_)
    ) {
        return None;
    }
    let ended = match pos {
        // Antichess ends when the side to move has no pieces or no moves: it has won.
        VariantPosition::Antichess(_) => pos.us().is_empty() || pos.legal_moves().is_empty(),
        _ => pos.is_variant_end(),
    };
    if !ended {
        return None;
    }
    if let VariantPosition::Antichess(_) = pos {
        return Some("won");
    }
    Some(match pos.variant_outcome() {
        shakmaty::Outcome::Known(shakmaty::KnownOutcome::Decisive { winner })
            if winner == pos.turn() =>
        {
            "won"
        }
        shakmaty::Outcome::Known(shakmaty::KnownOutcome::Decisive { .. }) => "checkmate",
        _ => "draw",
    })
}

/// The start of a replay: `variant` is a Lichess key, or empty for standard rules.
pub(crate) fn start_of(variant: &str, fen: &str) -> Option<VariantPosition> {
    match variant {
        "" | "standard" | "chess960" => rules::standard_from_fen(fen),
        variant => rules::setup_start(variant, fen),
    }
}

/// `review.replay`: standard rules from `fen`, the start first, as far as the moves are legal.
pub fn replay_positions<S: AsRef<str>>(fen: &str, moves: &[S]) -> Vec<ReplayedPosition> {
    replay_positions_in("", fen, moves)
}

/// `replay_positions` by a variant's rules (a Lichess key; empty for standard chess).
pub fn replay_positions_in<S: AsRef<str>>(
    variant: &str,
    fen: &str,
    moves: &[S],
) -> Vec<ReplayedPosition> {
    let Some(mut pos) = start_of(variant, fen) else {
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
    replay_turns_with_end(fen, moves, true)
}

/// The same walk, skipping the end-of-game check when the caller has scores for every position
/// and only needs whose turn it is. The check generates every legal move at every ply.
pub fn replay_turns_with_end<S: AsRef<str>>(
    fen: &str,
    moves: &[S],
    with_end: bool,
) -> Vec<(bool, Option<&'static str>)> {
    replay_turns_in("", fen, moves, with_end)
}

/// `replay_turns_with_end` by a variant's rules (a Lichess key; empty for standard chess).
pub fn replay_turns_in<S: AsRef<str>>(
    variant: &str,
    fen: &str,
    moves: &[S],
    with_end: bool,
) -> Vec<(bool, Option<&'static str>)> {
    let Some(mut pos) = start_of(variant, fen) else {
        return Vec::new();
    };
    let state = |pos: &VariantPosition| {
        let described = if with_end { end_of(pos) } else { None };
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

/// The FEN after `count` legal moves from `fen` (standard rules), or None when one is illegal.
pub fn fen_after<S: AsRef<str>>(fen: &str, moves: &[S], count: usize) -> Option<String> {
    let mut pos = rules::standard_from_fen(fen)?;
    for uci in moves.iter().take(count) {
        let m = rules::parse_uci(uci.as_ref()).and_then(|u| rules::legal_move(&pos, u))?;
        pos.play_unchecked(m);
    }
    Some(rules::make_fen(&pos))
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

/// The moves from `fen` (standard rules) spelled as a UCI engine reads them: castling is
/// king-takes-rook with `UCI_Chess960` on and king-two-squares with it off. `None` when a move
/// is not legal.
pub fn engine_line<S: AsRef<str>>(fen: &str, moves: &[S], chess960: bool) -> Option<Vec<String>> {
    let mut pos = rules::standard_from_fen(fen)?;
    let mode = if chess960 {
        CastlingMode::Chess960
    } else {
        CastlingMode::Standard
    };
    moves
        .iter()
        .map(|uci| {
            let m = rules::parse_uci(uci.as_ref()).and_then(|u| rules::legal_move(&pos, u))?;
            let text = m.to_uci(mode).to_string();
            pos.play_unchecked(m);
            Some(text)
        })
        .collect()
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
    fn engine_line_spells_castling_for_the_engine_mode() {
        let line = [
            "e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "g8f6", "e1h1", "f8c5", "e1g1",
        ];
        let standard = engine_line(START, &line[..8], false).unwrap();
        assert_eq!(standard[6], "e1g1");
        let chess960 = engine_line(START, &line[..8], true).unwrap();
        assert_eq!(chess960[6], "e1h1");
        let mut typed = line;
        typed[6] = "e1g1";
        assert_eq!(engine_line(START, &typed[..8], true).unwrap()[6], "e1h1");
        assert_eq!(engine_line(START, &line, false), None);
    }

    /// Stalemate and bare kings are draws; a position with play left has no end.
    #[test]
    fn positions_mark_draws() {
        let stalemate = replay_positions("k7/8/1Q6/8/8/8/8/K7 w - - 0 1", &["b6c7"]);
        assert_eq!(stalemate[0].end, None);
        assert_eq!(stalemate[1].end, Some("draw"));
        let bare = replay_positions("k7/8/8/8/8/8/1q6/K7 w - - 0 1", &["a1b2"]);
        assert_eq!(bare[1].end, Some("draw"));
    }

    /// Each position's side to move and end, the end only when asked for.
    #[test]
    fn turns_follow_the_replay() {
        let mate = ["f2f3", "e7e5", "g2g4", "d8h4"];
        let turns = replay_turns(START, &mate);
        assert_eq!(
            turns,
            [
                (true, None),
                (false, None),
                (true, None),
                (false, None),
                (true, Some("checkmate")),
            ]
        );
        let unchecked = replay_turns_with_end(START, &mate, false);
        assert_eq!(unchecked.last(), Some(&(true, None)));
        assert_eq!(replay_turns(START, &["e2e4", "e2e4"]).len(), 2);
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
        // The start and each move played before the first illegal one.
        assert_eq!(replay_positions_count(START, &["e2e4", "e7e5", "e1e3"]), 3);
        assert_eq!(replay_positions_count("not a fen", &["e2e4"]), 0);
    }
}

#[cfg(test)]
mod variant_tests {
    use super::*;

    #[test]
    fn a_variant_replay_ends_where_its_rules_do() {
        // Atomic: Qxf7 explodes Black's king, so Black, to move, has lost.
        let fen = "rnbqkbnr/pppp1ppp/8/4p2Q/8/4P3/PPPP1PPP/RNB1KBNR w KQkq - 0 3";
        let atomic = replay_positions_in("atomic", fen, &["h5f7"]);
        assert_eq!(atomic.len(), 2);
        assert_eq!(atomic[1].end, Some("checkmate"));
        // Standard rules see an ordinary capture.
        assert_eq!(replay_positions(fen, &["h5f7"])[1].end, None);
        // Antichess: White must take Black's last piece, and Black, left with none, has won.
        let last = "8/8/8/8/8/1p6/P7/8 w - - 0 1";
        let antichess = replay_positions_in("antichess", last, &["a2b3"]);
        assert_eq!(antichess[1].end, Some("won"));
        assert_eq!(antichess[1].turn, "black");
        // Left with pieces but no moves, the side to move has won too; the other side having
        // no pieces does not end it.
        let blocked = "8/p7/P7/8/8/8/8/8 b - - 0 1";
        assert_eq!(
            replay_positions_in("antichess", blocked, &[] as &[&str])[0].end,
            Some("won")
        );
        let bare = "8/8/8/8/8/8/8/k7 b - - 0 1";
        assert_eq!(
            replay_positions_in("antichess", bare, &[] as &[&str])[0].end,
            None
        );
        // King of the Hill: a king already on the hill has won for the side to move.
        let hill = "8/8/8/8/4K3/8/8/k7 w - - 0 1";
        assert_eq!(
            replay_positions_in("kingOfTheHill", hill, &[] as &[&str])[0].end,
            Some("won")
        );
        let score = crate::api::call("endEval", r#"["won", "black"]"#).unwrap();
        assert_eq!(score, r#"{"mate":-1}"#);
        // A drop replays in Crazyhouse and not in standard chess.
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[P] w KQkq - 0 1";
        assert_eq!(replay_positions_in("crazyhouse", start, &["P@e4"]).len(), 2);
        assert_eq!(replay_positions(start, &["P@e4"]).len(), 1);
    }
}
