//! How a game's clock was used, read from its PGN: the time control (`TimeControl`), the clock
//! left after every move (`[%clk h:mm:ss]`, which Lichess and Chess.com both write) and where
//! the middlegame and the endgame began, so time can be told apart by phase.

use crate::pgn::{Game, parse_pgn};
use crate::position::comment_clock;
use crate::review::phase_starts;
use crate::rules;
use shakmaty::Position as _;

crate::union!(
    /// A part of the game the clock is told apart by.
    Phase {
        Opening = "opening",
        Middlegame = "middlegame",
        Endgame = "endgame",
    }
);

/// The opening, the middlegame and the endgame, in game order.
pub const PHASES: [Phase; 3] = [Phase::Opening, Phase::Middlegame, Phase::Endgame];

/// A game's clock, move by move.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeUse {
    /// Each side's starting time, in seconds.
    pub initial: f64,
    /// Seconds added after each move.
    pub increment: f64,
    /// The mover's clock after each ply of the main line, in seconds; None where the PGN has none.
    pub clocks: Vec<Option<f64>>,
    /// The plies where the middlegame and the endgame begin (`clocks.len()` when never reached).
    pub middlegame: usize,
    pub endgame: usize,
    /// Whether White played the first ply (false for a set-up position with Black to move).
    pub white_starts: bool,
}

impl TimeUse {
    /// The clock the side playing ply `ply` had before it.
    pub fn before(&self, ply: usize) -> Option<f64> {
        if ply < 2 {
            Some(self.initial)
        } else {
            self.clocks.get(ply - 2).copied().flatten()
        }
    }

    /// Seconds spent on ply `ply`. A side's first move gets no increment from either site.
    pub fn spent(&self, ply: usize) -> Option<f64> {
        let after = self.clocks.get(ply).copied().flatten()?;
        let increment = if ply < 2 { 0.0 } else { self.increment };
        Some((self.before(ply)? + increment - after).max(0.0))
    }

    /// Whether White played ply `ply`.
    pub fn white(&self, ply: usize) -> bool {
        self.white_starts == ply.is_multiple_of(2)
    }

    /// The phase ply `ply` was played in: an index of `PHASES`.
    pub fn phase(&self, ply: usize) -> usize {
        if ply >= self.endgame {
            2
        } else if ply >= self.middlegame {
            1
        } else {
            0
        }
    }

    /// Whether the side playing `ply` was short of time before it: under a tenth of its start.
    pub fn short_before(&self, ply: usize) -> Option<bool> {
        self.before(ply).map(|clock| clock < self.initial / 10.0)
    }
}

/// `TimeControl` as Lichess and Chess.com write it (`180+2`, or `600`): the start and increment
/// in seconds. None for correspondence (`-`, `1/259200`) or anything else.
pub fn time_control(text: &str) -> Option<(f64, f64)> {
    let (base, increment) = text.split_once('+').unwrap_or((text, "0"));
    let parse = |part: &str| {
        (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
            .then(|| part.parse::<f64>().ok())
            .flatten()
    };
    let (base, increment) = (parse(base)?, parse(increment)?);
    (base > 0.0).then_some((base, increment))
}

/// A game's clock use from its PGN; None without a real-time control, without a clock on any
/// move, or with a main-line move that is not legal.
pub fn time_use(pgn: &str) -> Option<TimeUse> {
    let game = parse_pgn(pgn).into_iter().next()?;
    let (initial, increment) = time_control(game.header("TimeControl")?)?;
    let (sans, clocks) = main_line(&game);
    if clocks.iter().all(Option::is_none) {
        return None;
    }
    let variant = rules::pgn_variant(game.header("Variant"))?;
    let start = match game.header("FEN") {
        Some(fen) => rules::setup_position(variant, rules::parse_fen(fen)?)?,
        None => rules::default_position(variant),
    };
    let white_starts = start.turn() == shakmaty::Color::White;
    let (middlegame, endgame) = phase_starts(start, sans.len(), |pos, ply| {
        rules::parse_san(pos, &sans[ply])
    })?;
    Some(TimeUse {
        initial,
        increment,
        clocks,
        middlegame,
        endgame,
        white_starts,
    })
}

/// The main line's moves and the clock after each.
fn main_line(game: &Game) -> (Vec<String>, Vec<Option<f64>>) {
    let (mut sans, mut clocks) = (Vec::new(), Vec::new());
    for next in game.main_line() {
        let played = game.node(next);
        sans.push(played.san.as_str().to_string());
        clocks.push(
            played
                .comments
                .iter()
                .flatten()
                .filter_map(|comment| comment_clock(comment))
                .next_back(),
        );
    }
    (sans, clocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_controls_are_real_time_only() {
        assert_eq!(time_control("180+2"), Some((180.0, 2.0)));
        assert_eq!(time_control("600"), Some((600.0, 0.0)));
        assert_eq!(time_control("-"), None);
        assert_eq!(time_control("1/259200"), None);
        assert_eq!(time_control("0+1"), None);
        assert_eq!(time_control("+2"), None);
    }

    #[test]
    fn clocks_give_the_time_spent_on_each_move() {
        let pgn = "[TimeControl \"180+2\"]\n\n\
            1. e4 { [%clk 0:03:00] } 1... e5 { [%clk 0:03:00] } \
            2. Nf3 { [%clk 0:02:52] } 2... Nc6 { [%clk 0:02:30] } 3. Bb5 { [%clk 0:00:10] } 1-0";
        let time = time_use(pgn).expect("a clocked game");
        assert_eq!((time.initial, time.increment), (180.0, 2.0));
        assert_eq!(time.clocks.len(), 5);
        assert_eq!(time.spent(0), Some(0.0));
        assert_eq!(time.spent(2), Some(10.0));
        assert_eq!(time.spent(3), Some(32.0));
        assert_eq!(time.spent(4), Some(164.0));
        assert_eq!(time.short_before(4), Some(false));
        assert_eq!(time.before(5), Some(150.0));
        assert_eq!(time.before(6), Some(10.0));
        assert_eq!(time.phase(4), 0, "the game never left the opening");
        assert_eq!(time.endgame, 5);
    }

    #[test]
    fn the_endgame_begins_where_the_review_phases_say() {
        // A position with a queen and a rook each: already the endgame.
        let pgn = "[TimeControl \"60\"]\n[FEN \"4k3/8/8/8/8/8/q6r/4K2R w - - 0 1\"]\n[SetUp \"1\"]\n\n\
            1. Rxh2 { [%clk 0:00:59] } 1-0";
        let time = time_use(pgn).expect("a clocked game");
        assert_eq!((time.middlegame, time.endgame), (0, 0));
        assert_eq!(time.phase(0), 2);
        assert!(time.white(0));
    }

    fn clocked(white_starts: bool) -> TimeUse {
        TimeUse {
            initial: 180.0,
            increment: 2.0,
            clocks: vec![Some(170.0), Some(160.0), Some(150.0)],
            middlegame: 3,
            endgame: 3,
            white_starts,
        }
    }

    #[test]
    fn a_side_second_move_starts_from_the_clock_after_its_first() {
        let time = clocked(true);
        assert_eq!(time.before(1), Some(180.0));
        assert_eq!(time.before(2), Some(170.0));
        assert_eq!(time.before(3), Some(160.0));
    }

    #[test]
    fn plies_alternate_from_the_side_that_started() {
        assert!(clocked(true).white(0) && !clocked(true).white(1) && clocked(true).white(2));
        assert!(!clocked(false).white(0) && clocked(false).white(1));
    }

    #[test]
    fn short_of_time_is_under_a_tenth_of_the_start() {
        let mut time = clocked(true);
        time.clocks = vec![Some(100.0), Some(10.0), Some(18.0), Some(17.9), None];
        assert_eq!(time.short_before(2), Some(false), "100 s of 180");
        assert_eq!(time.short_before(3), Some(true), "10 s of 180");
        assert_eq!(
            time.short_before(4),
            Some(false),
            "exactly a tenth is not under it"
        );
        assert_eq!(time.short_before(5), Some(true));
        assert_eq!(time.short_before(6), None);
    }

    #[test]
    fn a_time_control_is_whole_seconds() {
        for text in ["1e3", "60+1.5", "inf", "nan", "+", "60+", "6 0", "-5"] {
            assert_eq!(time_control(text), None, "{text}");
        }
    }
}
