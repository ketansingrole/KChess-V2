//! What an endgame drill and a lesson's step share as you play them out against Stockfish: taking
//! back to your previous turn, and the board they draw.

use gpui_kit::*;
use shakmaty::Color;

use crate::board::{Arrow, BoardState};

/// A move's squares as the board marks them (castling king to its square).
pub fn move_squares(fen: &str, uci: &str) -> Option<(shakmaty::Square, shakmaty::Square)> {
    let squares = kchess_domain::training::puzzle::move_squares(fen, uci)?;
    Some((
        squares.first()?.parse().ok()?,
        squares.get(1)?.parse().ok()?,
    ))
}

/// The moves left once taken back to your previous turn: the last move, then each reply before it
/// until `yours` says you are to move after what is left.
pub(crate) fn taken_back(moves: &[String], yours: impl Fn(&[String]) -> bool) -> Vec<String> {
    let mut kept = moves.to_vec();
    kept.pop();
    while !kept.is_empty() && !yours(&kept) {
        kept.pop();
    }
    kept
}

/// The board as you play a position out.
pub(crate) struct Shown<'a> {
    pub fen: &'a str,
    /// The position before the last move, and that move.
    pub last: Option<(&'a str, &'a str)>,
    /// Your side: at the bottom unless `flipped`, and movable on your turn.
    pub player: Color,
    pub flipped: bool,
    pub your_turn: bool,
    pub hint: Option<&'a str>,
    /// The pieces show in blindfold play too.
    pub reveal: bool,
}

impl Shown<'_> {
    /// The board's state, with `marks` drawn under the hint's arrow; none for a position that
    /// cannot be read.
    pub fn state(&self, marks: impl FnOnce(&mut BoardState), cx: &App) -> Option<BoardState> {
        let position =
            kchess_domain::rules::setup_start(kchess_domain::variant::Variant::Standard, self.fen)?;
        let last = self
            .last
            .and_then(|(before, uci)| move_squares(before, uci));
        let orientation = if self.flipped {
            !self.player
        } else {
            self.player
        };
        let movable = self.your_turn.then_some(self.player);
        let mut state = BoardState::of(&position, orientation, movable, last);
        marks(&mut state);
        if let Some((from, to)) = self.hint.and_then(|hint| move_squares(self.fen, hint)) {
            state.arrows.push(Arrow::hint(from, to));
        }
        state.blindfold = crate::settings::frontend(cx).blindfold && !self.reveal;
        Some(state)
    }
}

#[cfg(test)]
mod tests {
    use super::taken_back;

    /// Taking back undoes your last move and the reply after it, so it is your turn again.
    #[test]
    fn a_takeback_goes_back_to_your_previous_turn() {
        let moves: Vec<String> = ["e2e4", "e7e5", "g1f3", "b8c6"].map(String::from).to_vec();
        let yours = |kept: &[String]| kept.len().is_multiple_of(2);
        assert_eq!(taken_back(&moves, yours), ["e2e4", "e7e5"]);
        assert_eq!(taken_back(&moves[..3], yours), ["e2e4", "e7e5"]);
        assert!(taken_back(&moves[..1], yours).is_empty());
        assert!(taken_back(&[], yours).is_empty());
    }
}
