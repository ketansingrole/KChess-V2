//! Game sessions: the computer game, the board game with clocks, the board clock and the archive
//! identity. Each is a typed state machine: the frontend holds it, calls its methods with what it
//! knows now (its clock, whether the engine may play), and carries out the effects it returns,
//! in order.

pub mod archive;
pub mod computer;
pub mod local;
pub mod otb_clock;

use serde::{Deserialize, Serialize};
use shakmaty::Color;
use shakmaty::variant::VariantPosition;

use crate::position::{self, Info};
use crate::records::games::{self as results, GameEnd};
use crate::replay;
use crate::rules;

pub use crate::records::games::Outcome;
pub use crate::rules::INITIAL_FEN;
pub use crate::variant::Setup;

pub const ILLEGAL_SETUP: &str = "the setup is not a legal position";

/// What the frontend knows when it calls a game.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Host {
    /// Monotonic milliseconds.
    pub now: f64,
    /// Assistance is allowed: the engine may play.
    pub allowed: bool,
    /// The engine is ready.
    pub ready: bool,
}

/// A clock for one side: minutes and the increment in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clock {
    pub minutes: f64,
    pub increment: f64,
}

impl Clock {
    /// Whether a saved session may keep it: 0 to 180 minutes and seconds.
    pub fn fits(self) -> bool {
        let fits = |value: f64| (0.0..=180.0).contains(&value);
        fits(self.minutes) && fits(self.increment)
    }

    /// A clock a game may start with: what a saved session keeps. `1e306` minutes made the times
    /// infinite, which are stored as `null` and failed every later move, so the game could only
    /// be reset.
    pub fn checked(self) -> Result<Clock, String> {
        if self.fits() {
            Ok(self)
        } else {
            Err("A clock is 0 to 180 minutes with an increment of 0 to 180 seconds.".into())
        }
    }
}

/// Each side's remaining time in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Times {
    pub white: f64,
    pub black: f64,
}

impl Times {
    pub fn get(&self, color: Color) -> f64 {
        match color {
            Color::White => self.white,
            Color::Black => self.black,
        }
    }

    pub fn set(&mut self, color: Color, value: f64) {
        match color {
            Color::White => self.white = value,
            Color::Black => self.black = value,
        }
    }
}

/// A setup, its moves and what they reach: the position, how it ended, and any draw by
/// repetition or the fifty-move rule. Worked out once per change of the moves.
#[derive(Clone, Debug)]
pub struct Board {
    setup: Setup,
    moves: Vec<String>,
    position: VariantPosition,
    info: Info,
    draw: Option<String>,
    end: Option<GameEnd>,
}

impl Board {
    /// The board of a setup and its moves; moves past the first illegal one are kept but not
    /// played. An illegal setup is refused.
    pub fn new(setup: Setup, moves: Vec<String>) -> Result<Board, String> {
        let replayed =
            replay::replay_setup(setup.variant, &setup.fen, &moves).ok_or(ILLEGAL_SETUP)?;
        let position = rules::setup_start(setup.variant, &replayed.fen).ok_or(ILLEGAL_SETUP)?;
        let texts: Vec<&str> = moves.iter().map(String::as_str).collect();
        let draw =
            results::setup_draw_reason(setup.variant, &setup.fen, &texts).map(str::to_string);
        let end = results::board_result(&position, setup.variant, draw.as_deref());
        Ok(Board {
            info: position::info(&position),
            setup,
            moves,
            position,
            draw,
            end,
        })
    }

    pub fn setup(&self) -> &Setup {
        &self.setup
    }

    pub fn moves(&self) -> &[String] {
        &self.moves
    }

    pub fn position(&self) -> &VariantPosition {
        &self.position
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    /// The side to move.
    pub fn turn(&self) -> Color {
        shakmaty::Position::turn(&self.position)
    }

    /// The repetition or fifty-move draw reached, if any.
    pub fn draw(&self) -> Option<&str> {
        self.draw.as_deref()
    }

    /// How the position ended the game (or the draw reached); none while it goes on.
    pub fn end(&self) -> Option<&GameEnd> {
        self.end.as_ref()
    }

    /// The winner the position itself names, if any.
    pub fn outcome_winner(&self) -> Option<Color> {
        self.end.as_ref().and_then(|end| end.winner)
    }

    /// Who wins when `flagged` runs out of time here: the other side, unless it cannot mate.
    pub fn timeout_winner(&self, flagged: Color) -> Option<Color> {
        results::timeout_winner(&self.position, flagged)
    }

    /// The board once `uci` is played, with its SAN; none when it is not legal here.
    pub fn with_move(&self, uci: &str) -> Option<(Board, String)> {
        let mut next = self.moves.clone();
        next.push(uci.to_string());
        let replayed = replay::replay_setup(self.setup.variant, &self.setup.fen, &next)?;
        if replayed.played.len() != next.len() {
            return None;
        }
        let san = replayed.played.last()?.san.clone();
        Some((Board::new(self.setup.clone(), next).ok()?, san))
    }

    /// The side to move after the first `count` moves.
    pub fn turn_after(&self, count: usize) -> Color {
        let moves = &self.moves[..count.min(self.moves.len())];
        replay::replay_setup_turn(self.setup.variant, &self.setup.fen, moves)
            .map_or(Color::White, |(_, turn)| turn)
    }
}

/// The clock both board games share: each side's time, charged to the side to move while the
/// game says its clock runs, from `turn_started` on the host's clock.
pub(crate) struct Clocking {
    pub times: Option<Times>,
    pub turn_started: f64,
    pub running: bool,
    pub turn: Color,
}

impl Clocking {
    pub fn remaining(&self, color: Color, at: f64) -> f64 {
        let left = self.times.map_or(0.0, |times| times.get(color));
        if self.running && self.turn == color {
            (left - (at - self.turn_started)).max(0.0)
        } else {
            left
        }
    }

    /// Both sides' time at `at`: what a clock keeps when it stops.
    pub fn times_at(&self, at: f64) -> Times {
        Times {
            white: self.remaining(Color::White, at),
            black: self.remaining(Color::Black, at),
        }
    }

    /// The side to move, when its time has run out at `at` on a running clock.
    pub fn out_of_time(&self, at: f64) -> Option<Color> {
        (self.running && self.remaining(self.turn, at) <= 0.0).then_some(self.turn)
    }

    /// The clock once `mover` moves at `at` and gains `increment` milliseconds; none without one.
    pub fn charged(&self, mover: Color, at: f64, increment: f64) -> Option<Times> {
        let mut times = self.times?;
        let spent = if self.running {
            at - self.turn_started
        } else {
            0.0
        };
        let left = (times.get(mover) - spent).max(0.0);
        times.set(mover, left + increment);
        Some(times)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An illegal setup was replaced by the standard start without a word: the game showed the
    /// usual position and refused every move.
    #[test]
    fn an_illegal_setup_is_refused() {
        let illegal = Setup {
            variant: crate::variant::Variant::Standard,
            fen: "8/8/8/8/8/8/8/8 w - - 0 1".into(),
        };
        assert_eq!(Board::new(illegal, Vec::new()).unwrap_err(), ILLEGAL_SETUP);
        let board = Board::new(Setup::standard(), vec!["e2e4".into()]).unwrap();
        assert_eq!(board.turn(), Color::Black);
        assert_eq!(board.turn_after(0), Color::White);
    }

    #[test]
    fn outcomes_read_and_write_as_pgn_tags() {
        for outcome in [
            None,
            Some(Outcome::Wins(Color::White)),
            Some(Outcome::Wins(Color::Black)),
            Some(Outcome::Draw),
        ] {
            assert_eq!(Outcome::from_tag(Outcome::tag(outcome)), outcome);
        }
        assert_eq!(Outcome::tag(Outcome::of(true, None)), "1/2-1/2");
        assert_eq!(Outcome::tag(Outcome::of(false, Some(Color::White))), "*");
    }
}
