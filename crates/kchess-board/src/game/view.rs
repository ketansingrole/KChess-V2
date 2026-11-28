//! What the game pages show of a game: the position at a move, the moves in SAN, the last move,
//! a typed move and the board's state, from a setup and its UCI moves.

use kchess_domain::games::Board;
use kchess_domain::variant::Setup;
use shakmaty::variant::VariantPosition;
use shakmaty::{Chess, Color, Position as _, Square};

use crate::board::BoardState;

/// The position after the first `ply` moves.
pub fn position_at(setup: &Setup, moves: &[String], ply: usize) -> Option<VariantPosition> {
    let replay = kchess_domain::replay::replay_setup(
        setup.variant,
        &setup.fen,
        &moves[..ply.min(moves.len())],
    )?;
    kchess_domain::rules::setup_start(setup.variant, &replay.fen)
}

/// The position after the first `ply` moves of `board`: the board's own from the latest move on,
/// so drawing the game as it stands replays nothing.
pub fn board_position_at(board: &Board, ply: usize) -> Option<VariantPosition> {
    if ply >= board.moves().len() {
        Some(board.position().clone())
    } else {
        position_at(board.setup(), board.moves(), ply)
    }
}

/// Each move in SAN.
pub fn sans(setup: &Setup, moves: &[String]) -> Vec<String> {
    kchess_domain::replay::replay_setup(setup.variant, &setup.fen, moves)
        .map(|replay| replay.played.into_iter().map(|played| played.san).collect())
        .unwrap_or_default()
}

/// The squares of the move before `ply`.
pub fn last_move(moves: &[String], ply: usize) -> Option<(Square, Square)> {
    crate::board::last_squares(moves.get(ply.checked_sub(1)?)?)
}

/// The game's first move: its number and whether Black plays it (the move list then starts
/// with "30. …" for a game set up at Black's 30th move).
pub fn first_move(setup: &Setup) -> crate::widgets::FirstMove {
    let fields: Vec<&str> = setup.fen.split(' ').collect();
    crate::widgets::FirstMove {
        number: fields
            .get(5)
            .and_then(|number| number.parse().ok())
            .filter(|number| *number > 0)
            .unwrap_or(1),
        black: fields.get(1) == Some(&"b"),
    }
}

/// The board for the game at `ply`, with `movable` the side that may move now.
pub fn board_state(
    setup: &Setup,
    moves: &[String],
    ply: usize,
    orientation: Color,
    movable: Option<Color>,
) -> BoardState {
    let position =
        position_at(setup, moves, ply).unwrap_or_else(|| VariantPosition::Chess(Chess::default()));
    BoardState::of(&position, orientation, movable, last_move(moves, ply))
}

/// Whose turn it is in a position.
pub fn turn(position: Option<&VariantPosition>) -> Color {
    position.map_or(Color::White, |position| position.turn())
}

/// One step through a game's moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    Back,
    On,
}

impl Way {
    /// The ply this step reaches from `ply`, of `max`.
    pub fn from(self, ply: usize, max: usize) -> usize {
        match self {
            Way::Back => ply.saturating_sub(1),
            Way::On => (ply + 1).min(max),
        }
    }
}

/// Where an arrow key moves a move list's position (←/→ one ply, ↑/↓ or Home/End first/last);
/// `None` for other keys.
pub fn navigate(key: &str, ply: usize, max: usize) -> Option<usize> {
    match key {
        "left" => Some(Way::Back.from(ply, max)),
        "right" => Some(Way::On.from(ply, max)),
        "up" | "home" => Some(0),
        "down" | "end" => Some(max),
        _ => None,
    }
}

/// The mouse wheel over a board, gathered until it has turned far enough for one move: Lichess
/// steps through the moves this way.
#[derive(Default)]
pub struct Wheel(f32);

impl Wheel {
    /// How far the wheel turns for one move, in pixels.
    const STEP: f32 = 40.;

    /// The step the turn so far stands for: a move on with the wheel towards the user (as
    /// `deltaY > 0` in the browser), a move back away from them. Sideways scrolls are left alone.
    pub fn turn(&mut self, delta: gpui_kit::Point<gpui_kit::Pixels>) -> Option<Way> {
        if delta.x.abs() > delta.y.abs() {
            return None;
        }
        self.0 -= delta.y.as_f32();
        if self.0.abs() < Self::STEP {
            return None;
        }
        let way = if self.0 > 0. { Way::On } else { Way::Back };
        self.0 = 0.;
        Some(way)
    }
}

#[cfg(test)]
mod tests {
    use super::{Way, Wheel, board_position_at, navigate, position_at, sans};
    use gpui_kit::{point, px};
    use kchess_domain::games::Board;
    use kchess_domain::variant::{Setup, Variant};

    /// A game page draws its moves and positions from its board, which replayed them once: the
    /// same as replaying them again at every ply, past an illegal move and in a variant too.
    #[test]
    fn a_boards_moves_and_positions_are_its_replays() {
        let games = [
            (
                Setup::standard(),
                vec!["e2e4", "e7e5", "g1f3", "b8c6", "f1b5"],
            ),
            (Setup::standard(), vec!["e2e4", "e7e5", "e1e3", "d7d5"]),
            (
                Setup {
                    variant: Variant::Crazyhouse,
                    fen: "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1".into(),
                },
                vec!["e2e4", "d7d5", "e4d5", "d8d5", "P@e4"],
            ),
        ];
        for (setup, moves) in games {
            let moves: Vec<String> = moves.into_iter().map(String::from).collect();
            let board = Board::new(setup.clone(), moves.clone()).expect("a legal setup");
            assert_eq!(board.sans(), sans(&setup, &moves), "{moves:?}");
            for ply in 0..=moves.len() + 1 {
                assert_eq!(
                    board_position_at(&board, ply),
                    position_at(&setup, &moves, ply),
                    "{moves:?} at {ply}"
                );
            }
        }
    }

    /// A wheel's small turns add up to one move; down goes on, up goes back, sideways neither.
    #[test]
    fn the_wheel_steps_one_move_at_a_time() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.turn(point(px(0.), px(-25.))), None);
        assert_eq!(wheel.turn(point(px(0.), px(-25.))), Some(Way::On));
        assert_eq!(wheel.turn(point(px(0.), px(-30.))), None);
        assert_eq!(wheel.turn(point(px(0.), px(80.))), Some(Way::Back));
        assert_eq!(wheel.turn(point(px(-100.), px(10.))), None);
    }

    /// The arrows, Home and End move through a move list on every board page that uses it.
    #[test]
    fn home_and_end_go_to_the_first_and_last_move() {
        assert_eq!(navigate("home", 5, 9), Some(0));
        assert_eq!(navigate("end", 5, 9), Some(9));
        assert_eq!(navigate("up", 5, 9), Some(0));
        assert_eq!(navigate("down", 5, 9), Some(9));
        assert_eq!(navigate("left", 0, 9), Some(0));
        assert_eq!(navigate("right", 9, 9), Some(9));
        assert_eq!(navigate("pageup", 5, 9), None);
    }
}
