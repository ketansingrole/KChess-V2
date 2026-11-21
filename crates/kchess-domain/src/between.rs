//! The moves between two positions a few plies apart, when only one line joins them.
//!
//! Lichess's export of an ongoing game leaves out its last three plies, while its TV feed shows
//! the position now: the moves between them are found here, so a watched game's move list need
//! not wait for three more moves to be played.

use shakmaty::variant::VariantPosition;
use shakmaty::{Bitboard, Move, Position, Role, Setup};

use crate::rules;

/// How many positions a search may visit before it gives up: three plies of a middlegame visit
/// about 30 000, a Crazyhouse game with full pockets far more.
const BUDGET: usize = 400_000;

/// More lines than this reaching the position cannot be told apart by its clock either.
const MOST_LINES: usize = 16;

/// The one line of exactly `plies` moves, in UCI, that leads from `from` to the position `to`
/// describes: its pieces, side to move, and its pockets and checks left where `to` has them.
/// When several lines lead there (two moves of one side played in either order), the half-move
/// clock picks the one that matches it. None when no line, or more than one, does, or when the
/// search would visit more than its budget: a wrong move is worse than none.
pub fn line_between(from: &VariantPosition, to: &Setup, plies: u32) -> Option<Vec<String>> {
    let mut search = Search {
        to,
        // An explosion empties up to nine squares: the bound on changed squares does not hold.
        bounded: !matches!(from, VariantPosition::Atomic(_)),
        path: Vec::new(),
        found: Vec::new(),
        left: BUDGET,
    };
    search.visit(from, plies)?;
    let found = search.found;
    let line = match found.as_slice() {
        [(line, _)] => line,
        _ => {
            // Crazyhouse's clock after a drop is Lichess's own: only the board is trusted there.
            if matches!(from, VariantPosition::Crazyhouse(_)) {
                return None;
            }
            let mut clocked = found.iter().filter(|(_, clock)| *clock);
            match (clocked.next(), clocked.next()) {
                (Some((line, _)), None) => line,
                _ => return None,
            }
        }
    };
    Some(line.iter().map(rules::make_uci).collect())
}

struct Search<'a> {
    to: &'a Setup,
    bounded: bool,
    path: Vec<Move>,
    /// Each line that reaches `to`, and whether its half-move clock matches too.
    found: Vec<(Vec<Move>, bool)>,
    left: usize,
}

impl Search<'_> {
    /// Every line of `plies` moves from `pos`; None once the search is over its budget or has
    /// found more lines than can be told apart.
    fn visit(&mut self, pos: &VariantPosition, plies: u32) -> Option<()> {
        self.left = self.left.checked_sub(1)?;
        if plies == 0 {
            if self.reached(pos) {
                let clock = pos.halfmoves() == self.to.halfmoves;
                self.found.push((self.path.clone(), clock));
                if self.found.len() > MOST_LINES {
                    return None;
                }
            }
            return Some(());
        }
        // A move changes at most four squares (castling): further away, nothing gets there.
        if self.bounded && changed(pos, &self.to.board).count() > 4 * plies as usize {
            return Some(());
        }
        if ended(pos) {
            return Some(());
        }
        for m in pos.legal_moves() {
            let mut next = pos.clone();
            next.play_unchecked(m.clone());
            self.path.push(m);
            let visited = self.visit(&next, plies - 1);
            self.path.pop();
            visited?;
        }
        Some(())
    }

    fn reached(&self, pos: &VariantPosition) -> bool {
        let to = self.to;
        pos.board() == &to.board
            && pos.turn() == to.turn
            && match (pos.pockets(), to.pockets.as_ref()) {
                (Some(ours), Some(theirs)) => ours == theirs,
                _ => true,
            }
            && match (pos.remaining_checks(), to.remaining_checks.as_ref()) {
                (Some(ours), Some(theirs)) => ours == theirs,
                _ => true,
            }
    }
}

/// The squares whose piece differs between `pos` and `board`.
fn changed(pos: &VariantPosition, board: &shakmaty::Board) -> Bitboard {
    let ours = pos.board();
    let mut changed = ours.white() ^ board.white();
    for role in Role::ALL {
        changed |= ours.by_role(role) ^ board.by_role(role);
    }
    changed
}

/// Antichess ends only when the side to move has no pieces left; every other variant as it says.
fn ended(pos: &VariantPosition) -> bool {
    match pos {
        VariantPosition::Antichess(_) => pos.us().is_empty(),
        _ => pos.is_variant_end(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::variant::Variant;

    fn played(variant: Variant, fen: &str, uci: &[&str]) -> VariantPosition {
        let mut pos = rules::setup_start(variant, fen).unwrap();
        for text in uci {
            let m = rules::legal_move(&pos, rules::parse_uci(text).unwrap()).unwrap();
            pos.play_unchecked(m);
        }
        pos
    }

    fn setup_of(pos: &VariantPosition) -> Setup {
        rules::parse_fen(&rules::make_fen(pos)).unwrap()
    }

    fn between(from: &VariantPosition, to: &VariantPosition, plies: u32) -> Option<Vec<String>> {
        line_between(from, &setup_of(to), plies)
    }

    const START: &str = rules::INITIAL_FEN;

    #[test]
    fn the_one_line_between_two_positions_is_found() {
        let from = played(Variant::Standard, START, &["e2e4", "d7d5"]);
        let to = played(
            Variant::Standard,
            START,
            &["e2e4", "d7d5", "e4d5", "d8d5", "b1c3"],
        );
        assert_eq!(
            between(&from, &to, 3),
            Some(vec!["e4d5".into(), "d8d5".into(), "b1c3".into()])
        );
    }

    /// Two quiet moves of one side reach the same position in either order: nothing tells
    /// which came first, so no line is offered.
    #[test]
    fn moves_that_could_come_in_either_order_give_no_line() {
        let from = played(Variant::Standard, START, &["e2e4", "e7e5"]);
        let to = played(
            Variant::Standard,
            START,
            &["e2e4", "e7e5", "g1f3", "b8c6", "b1c3"],
        );
        assert_eq!(between(&from, &to, 3), None);
    }

    /// A pawn move resets the half-move clock, which tells when it was played.
    #[test]
    fn the_clock_tells_which_order_the_moves_came_in() {
        let from = played(Variant::Standard, START, &["e2e4", "e7e5"]);
        let to = played(
            Variant::Standard,
            START,
            &["e2e4", "e7e5", "g1f3", "b8c6", "d2d4"],
        );
        assert_eq!(
            between(&from, &to, 3),
            Some(vec!["g1f3".into(), "b8c6".into(), "d2d4".into()])
        );
        let to = played(
            Variant::Standard,
            START,
            &["e2e4", "e7e5", "d2d4", "b8c6", "g1f3"],
        );
        assert_eq!(
            between(&from, &to, 3),
            Some(vec!["d2d4".into(), "b8c6".into(), "g1f3".into()])
        );
    }

    #[test]
    fn a_position_out_of_reach_gives_no_line() {
        let from = played(Variant::Standard, START, &[]);
        let to = played(Variant::Standard, START, &["e2e4", "e7e5", "g1f3", "b8c6"]);
        assert_eq!(between(&from, &to, 3), None);
        assert_eq!(between(&from, &from, 0), Some(Vec::new()));
    }

    #[test]
    fn castling_captures_and_promotions_are_found() {
        let fen = "r3k2r/1P6/8/8/8/8/8/R3K2R w KQkq - 0 1";
        let from = played(Variant::Standard, fen, &[]);
        let to = played(Variant::Standard, fen, &["e1c1", "e8g8", "b7b8q"]);
        let line = between(&from, &to, 3).unwrap();
        assert_eq!(
            rules::make_fen(&played(
                Variant::Standard,
                fen,
                &line.iter().map(String::as_str).collect::<Vec<_>>()
            )),
            rules::make_fen(&to)
        );
    }

    #[test]
    fn variants_are_searched_by_their_own_rules() {
        let from = played(Variant::Atomic, START, &["e2e4"]);
        let to = played(Variant::Atomic, START, &["e2e4", "d7d5", "e4d5", "d8d5"]);
        // The explosion on d5 takes both pawns: the queen comes to an empty square.
        assert_eq!(
            between(&from, &to, 3),
            Some(vec!["d7d5".into(), "e4d5".into(), "d8d5".into()])
        );
        let to = played(Variant::Atomic, START, &["e2e4", "d7d5", "e4d5"]);
        assert_eq!(
            between(&from, &to, 2),
            Some(vec!["d7d5".into(), "e4d5".into()])
        );
        let house = Variant::Crazyhouse.default_fen();
        let from = played(Variant::Crazyhouse, &house, &["e2e4", "d7d5"]);
        let to = played(
            Variant::Crazyhouse,
            &house,
            &["e2e4", "d7d5", "e4d5", "d8d5", "P@e4"],
        );
        assert_eq!(
            between(&from, &to, 3),
            Some(vec!["e4d5".into(), "d8d5".into(), "P@e4".into()])
        );
    }
}
