//! What the game pages show of a game: the position at a move, the moves in SAN, the last move,
//! a typed move and the board's state, from a setup (`{ variant, fen }`) and its UCI moves.

use serde_json::Value;
use shakmaty::variant::VariantPosition;
use shakmaty::{Chess, Color, Position as _, Square};

use crate::board::BoardState;

/// The setup's variant and FEN.
fn setup_parts(setup: &Value) -> (&str, &str) {
    (
        setup["variant"].as_str().unwrap_or("standard"),
        setup["fen"].as_str().unwrap_or_default(),
    )
}

/// The position after the first `ply` moves.
pub fn position_at(setup: &Value, moves: &[String], ply: usize) -> Option<VariantPosition> {
    let (variant, fen) = setup_parts(setup);
    let replay = kchess_domain::replay::replay_setup(variant, fen, &moves[..ply.min(moves.len())])?;
    kchess_domain::rules::setup_start(variant, &replay.fen)
}

/// Each move in SAN.
pub fn sans(setup: &Value, moves: &[String]) -> Vec<String> {
    let (variant, fen) = setup_parts(setup);
    kchess_domain::replay::replay_setup(variant, fen, moves)
        .map(|replay| replay.played.into_iter().map(|played| played.san).collect())
        .unwrap_or_default()
}

/// The squares of the move before `ply`.
pub fn last_move(moves: &[String], ply: usize) -> Option<(Square, Square)> {
    crate::board::last_squares(moves.get(ply.checked_sub(1)?)?)
}

/// The game's first move: its number and whether Black plays it (the move list then starts
/// with "30. …" for a game set up at Black's 30th move).
pub fn first_move(setup: &Value) -> crate::widgets::FirstMove {
    let fields: Vec<&str> = setup_parts(setup).1.split(' ').collect();
    crate::widgets::FirstMove {
        number: fields
            .get(5)
            .and_then(|number| number.parse().ok())
            .filter(|number| *number > 0)
            .unwrap_or(1),
        black: fields.get(1) == Some(&"b"),
    }
}

/// A move typed as SAN or UCI, as UCI when it is legal in `position` (`kchess_games::typed_uci`).
pub fn typed_uci(setup: &Value, position: &VariantPosition, text: &str) -> Option<String> {
    let (variant, _) = setup_parts(setup);
    kchess_games::typed_uci(variant, &kchess_domain::rules::make_fen(position), text)
}

/// The board for the game at `ply`, with `movable` the side that may move now.
pub fn board_state(
    setup: &Value,
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
    use super::{Way, Wheel, navigate};
    use gpui_kit::{point, px};

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
