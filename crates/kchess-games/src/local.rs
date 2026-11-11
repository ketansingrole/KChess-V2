//! Two people at one board as a frontend drives it: the rules
//! (`kchess_domain::games::local`) run it; the page reports its clock and sounds a falling flag.

use serde_json::Value;
use shakmaty::Color;

use kchess_domain::games::Setup;
use kchess_domain::games::local::{self as session, Saved, State};
use kchess_domain::records::games::GameEnd;

pub use kchess_domain::games::local::LocalClock;

use crate::Outcome;

/// The session a board game is saved as, and the Game history kinds of the board game and of
/// the chess clock.
pub const SESSION: &str = "local";
pub const HISTORY: &str = "board";
pub const CLOCK_HISTORY: &str = "clock";
/// Its PGN's `Event`.
pub const EVENT: &str = "Over the board (KChess)";

pub struct LocalGame {
    game: session::LocalGame,
    /// The game's start as `{ variant, fen }`, for the views.
    setup: Value,
}

impl LocalGame {
    /// A game from a saved session, or a new one.
    pub fn new(saved: Option<&Value>, now: f64) -> Result<LocalGame, String> {
        let saved: Saved = match saved {
            Some(saved) => serde_json::from_value(saved.clone())
                .map_err(|error| format!("the saved game: {error}"))?,
            None => Saved::default(),
        };
        let runtime = session::Runtime { turn_started: now };
        let game = session::LocalGame::new(State::from_saved(saved), runtime)?;
        let setup = serde_json::json!(game.state().setup);
        Ok(LocalGame { game, setup })
    }

    /// The session document: the saved session and its pause flag.
    pub fn state(&self) -> &State {
        self.game.state()
    }

    /// The game's start, as `{ variant, fen }`.
    pub fn setup(&self) -> &Value {
        &self.setup
    }

    /// The game's moves, in UCI.
    pub fn moves(&self) -> &[String] {
        &self.game.state().moves
    }

    /// The clocks chosen, none when untimed.
    pub fn clock(&self) -> Option<LocalClock> {
        self.game.state().clock
    }

    /// The clocks run in this game.
    pub fn timed(&self) -> bool {
        self.game.state().times.is_some()
    }

    /// The clocks are paused.
    pub fn paused(&self) -> bool {
        self.game.state().paused
    }

    /// The winner, if any, and why, once the game has ended.
    pub fn result(&self) -> Option<&GameEnd> {
        self.game.result()
    }

    /// The result once the game has ended, from the winner it names.
    pub fn outcome(&self) -> Option<Outcome> {
        let result = self.result()?;
        Some(match result.winner.as_deref() {
            Some("white") => Outcome::Wins(Color::White),
            Some("black") => Outcome::Wins(Color::Black),
            _ => Outcome::Draw,
        })
    }

    pub fn over(&self) -> bool {
        self.game.over()
    }

    /// A side's clock in milliseconds.
    pub fn remaining(&self, color: Color, now: f64) -> f64 {
        self.game.remaining(crate::color_name(color), now)
    }

    /// The session to save.
    pub fn snapshot(&self, now: f64) -> Value {
        serde_json::to_value(self.game.snapshot(now)).unwrap_or(Value::Null)
    }

    /// The game as Game history keeps it.
    pub fn archive_snapshot(&self) -> Value {
        serde_json::to_value(self.game.archive_snapshot()).unwrap_or(Value::Null)
    }

    /// Whether a flag fell now.
    pub fn expire(&mut self, now: f64) -> bool {
        self.game.expire(now)
    }

    /// Play a move: its SAN, or `None` when it is not legal; and whether a flag fell instead.
    pub fn play(&mut self, uci: &str, now: f64) -> (Option<String>, bool) {
        self.game.play(uci, || now)
    }

    pub fn takeback(&mut self, now: f64) {
        self.game.takeback(|| now);
    }

    /// A new game from `setup` with `clock`.
    pub fn start(
        &mut self,
        setup: Setup,
        clock: Option<LocalClock>,
        now: f64,
    ) -> Result<(), String> {
        self.game.start(setup, clock, || now)?;
        self.setup = serde_json::json!(self.game.state().setup);
        Ok(())
    }

    pub fn resign(&mut self, color: Color) {
        self.game.resign(crate::color_name(color));
    }

    pub fn agree_draw(&mut self) {
        self.game.agree_draw();
    }

    /// Whether a flag fell instead of the clocks pausing or resuming.
    pub fn toggle_pause(&mut self, now: f64) -> bool {
        self.game.toggle_pause(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kchess_domain::games::Clock;
    use serde_json::json;

    fn clock(minutes: f64, increment: f64) -> Clock {
        Clock { minutes, increment }
    }

    fn one_minute() -> Option<LocalClock> {
        Some(LocalClock {
            white: clock(1., 0.),
            black: clock(1., 0.),
        })
    }

    /// Taking back after a flag fell left that side on 0 ms: it lost on time again as soon as it
    /// was back on move. As on Lichess, a time-out is final.
    #[test]
    fn a_time_out_is_not_taken_back() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        game.start(Setup::standard(), one_minute(), 0.).unwrap();
        game.play("e2e4", 1.);
        game.play("e7e5", 2.);
        assert!(game.expire(200_000.));
        game.takeback(200_001.);
        assert!(game.over());
        assert_eq!(game.moves(), ["e2e4", "e7e5"]);
    }

    #[test]
    fn both_sides_move_and_a_resignation_ends_the_game() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        assert_eq!(game.play("e2e4", 1.).0.as_deref(), Some("e4"));
        assert_eq!(game.play("e7e5", 2.).0.as_deref(), Some("e5"));
        assert_eq!(game.play("e2e4", 3.).0, None);
        game.resign(Color::White);
        assert!(game.over());
        assert_eq!(game.result().unwrap().winner.as_deref(), Some("black"));
        assert_eq!(Outcome::tag(game.outcome()), "0-1");
        assert_eq!(
            Outcome::tag(game.outcome()),
            game.archive_snapshot()["result"]
        );
    }

    /// A flag falls only when the time is up, and the snapshots describe the finished game.
    #[test]
    fn a_flag_falls_when_the_time_is_up() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        assert!(!game.over());
        game.start(Setup::standard(), one_minute(), 0.).unwrap();
        let (_, flagged) = game.play("e2e4", 1_000.);
        assert!(!flagged);
        assert!(!game.expire(30_000.));
        assert!(game.expire(100_000.));
        assert!(game.over());
        assert_eq!(game.snapshot(100_000.)["moves"], json!(["e2e4"]));
        let archived = game.archive_snapshot();
        assert_eq!(archived["source"], "board");
        assert_eq!(archived["result"], "1-0");
    }

    #[test]
    fn a_clock_runs_from_the_first_move_and_pauses() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        game.start(Setup::standard(), one_minute(), 0.).unwrap();
        assert_eq!(game.remaining(Color::White, 10_000.), 60_000.);
        game.play("e2e4", 1_000.);
        assert!(game.remaining(Color::Black, 11_000.) < 60_000.);
        game.toggle_pause(11_000.);
        let paused = game.remaining(Color::Black, 11_000.);
        assert_eq!(game.remaining(Color::Black, 40_000.), paused);
    }

    #[test]
    fn a_flag_falls_once_the_side_to_move_runs_out() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        game.start(Setup::standard(), one_minute(), 0.).unwrap();
        game.play("e2e4", 1_000.);
        assert!(!game.expire(30_000.));
        assert!(!game.over());
        assert!(game.expire(62_000.));
        assert!(game.over());
        assert_eq!(game.result().unwrap().winner.as_deref(), Some("white"));
    }

    #[test]
    fn an_agreed_draw_ends_the_game_without_a_winner() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        game.play("e2e4", 1.);
        game.agree_draw();
        assert!(game.over());
        assert_eq!(game.result().unwrap().winner, None);
    }

    #[test]
    fn a_takeback_undoes_the_last_move() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        game.play("e2e4", 1.);
        game.takeback(2.);
        // White is to move again, so e4 is legal once more and e5 is not.
        assert_eq!(game.play("e7e5", 3.).0, None);
        assert_eq!(game.play("e2e4", 4.).0.as_deref(), Some("e4"));
    }

    #[test]
    fn checkmate_ends_the_game() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        for (ply, uci) in ["f2f3", "e7e5", "g2g4", "d8h4"].iter().enumerate() {
            assert!(!game.over());
            game.play(uci, ply as f64);
        }
        assert!(game.over());
        assert_eq!(game.result().unwrap().winner.as_deref(), Some("black"));
    }

    #[test]
    fn a_saved_game_resumes_where_it_was_left() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        game.play("e2e4", 1.);
        game.play("e7e5", 2.);
        let saved = game.snapshot(3.);
        let mut resumed = LocalGame::new(Some(&saved), 3.).unwrap();
        assert_eq!(resumed.play("e2e4", 4.).0, None);
        assert_eq!(resumed.play("g1f3", 5.).0.as_deref(), Some("Nf3"));
    }

    /// `1e306` minutes made both times infinite, stored as `null`: every later move failed to
    /// read the state, and the game could only be reset. Negative minutes flagged at once.
    #[test]
    fn a_clock_no_game_can_have_is_refused_at_the_start() {
        let mut game = LocalGame::new(None, 0.).unwrap();
        for (minutes, increment) in [(1e306, 0.), (-5., 0.), (5., -1.), (181., 0.), (5., 1e9)] {
            let clock = LocalClock {
                white: clock(minutes, increment),
                black: clock(5., 0.),
            };
            assert!(
                game.start(Setup::standard(), Some(clock), 0.).is_err(),
                "{minutes} {increment}"
            );
            assert_eq!(game.play("e2e4", 1.).0.as_deref(), Some("e4"));
            game.takeback(2.);
        }
        let clock = LocalClock {
            white: clock(180., 180.),
            black: clock(0., 0.),
        };
        game.start(Setup::standard(), Some(clock), 0.)
            .expect("the longest and shortest clocks");
    }
}
