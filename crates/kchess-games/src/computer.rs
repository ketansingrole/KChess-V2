//! A game against the engine as a frontend drives it: the rules
//! (`kchess_domain::games::computer`) decide everything; the page reports its clock, whether
//! assistance is allowed and the engine ready, and carries out the effects, in order.

use serde_json::Value;
use shakmaty::Color;

use kchess_domain::games::computer::{self as session, Saved, State};
use kchess_domain::games::{self, Clock, Setup};

pub use kchess_domain::games::computer::{Effect, Kind, SearchRequest, Verdict};

use crate::Outcome;

/// What the page reports to the game.
#[derive(Clone, Copy, Debug)]
pub struct Host {
    /// Monotonic milliseconds.
    pub now: f64,
    pub allowed: bool,
    pub ready: bool,
}

impl games::Host for Host {
    fn now(&mut self) -> f64 {
        self.now
    }
    fn allowed(&mut self) -> bool {
        self.allowed
    }
    fn ready(&mut self) -> bool {
        self.ready
    }
}

pub struct ComputerGame {
    game: session::ComputerGame,
    /// The game's start as `{ variant, fen }`, for the views.
    setup: Value,
}

/// The session a computer game is saved as, and the Game history kind it keeps its place by.
pub const SESSION: &str = "computer";
pub const HISTORY: &str = "computer";
/// Its PGN's `Event`.
pub const EVENT: &str = "Computer game (KChess)";

/// White's and Black's names in a game the player plays as `color` against `engine`.
pub fn players(color: Color, engine: &str) -> (&str, &str) {
    match color {
        Color::White => ("You", engine),
        Color::Black => (engine, "You"),
    }
}

fn read<T: serde::de::DeserializeOwned>(value: Value, what: &str) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("{what}: {error}"))
}

impl ComputerGame {
    /// A game from a saved session, or a new one.
    pub fn new(saved: Option<&Value>, host: &Host) -> Result<ComputerGame, String> {
        let saved: Saved = match saved {
            Some(saved) => read(saved.clone(), "the saved game")?,
            None => Saved::default(),
        };
        let runtime = session::Runtime::start(&mut { *host });
        let game = session::ComputerGame::new(State::from_saved(saved), runtime)?;
        let setup = serde_json::json!(game.state().setup);
        Ok(ComputerGame { game, setup })
    }

    /// The session document (the saved session plus `thinking` and `epoch`).
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

    /// The move shown: the moves before it are on the board.
    pub fn ply(&self) -> usize {
        self.game.state().ply.min(self.moves().len())
    }

    /// Show the position after `ply` moves (at most every move).
    pub fn show_ply(&mut self, ply: usize) {
        self.game.show_ply(ply);
    }

    /// The player's side.
    pub fn color(&self) -> Color {
        if self.game.state().color == "black" {
            Color::Black
        } else {
            Color::White
        }
    }

    /// Play `color` from the next game on.
    pub fn set_color(&mut self, color: Color) {
        self.game.set_color(crate::color_name(color));
    }

    /// The engine level's id.
    pub fn level(&self) -> &str {
        &self.game.state().level
    }

    /// Play at `level` from the next search on.
    pub fn set_level(&mut self, level: &str) {
        self.game.set_level(level);
    }

    /// The clock chosen, none when untimed.
    pub fn clock(&self) -> Option<Clock> {
        self.game.state().clock
    }

    /// Play on `clock` from the next game on.
    pub fn set_clock(&mut self, clock: Option<Clock>) {
        self.game.set_clock(clock);
    }

    /// The clocks run in this game.
    pub fn timed(&self) -> bool {
        self.game.state().times.is_some()
    }

    /// The engine is searching.
    pub fn thinking(&self) -> bool {
        self.game.state().thinking
    }

    /// A side's flag has fallen.
    pub fn flagged(&self) -> bool {
        self.game.state().flagged.is_some()
    }

    /// The result once the game has ended, from the player's win, loss or draw.
    pub fn outcome(&self) -> Option<Outcome> {
        let user = self.color();
        Some(match self.result()?.kind {
            Kind::Win => Outcome::Wins(user),
            Kind::Loss => Outcome::Wins(!user),
            Kind::Draw => Outcome::Draw,
        })
    }

    pub fn over(&self) -> bool {
        self.game.over()
    }

    /// The result from the player's side once the game has ended.
    pub fn result(&self) -> Option<Verdict> {
        self.game.result()
    }

    /// A side's clock in milliseconds.
    pub fn remaining(&self, color: Color, host: &Host) -> f64 {
        self.game.remaining(crate::color_name(color), host.now)
    }

    /// The session to save.
    pub fn snapshot(&self, host: &Host) -> Value {
        serde_json::to_value(self.game.snapshot(host.now)).unwrap_or(Value::Null)
    }

    /// The game as Game history keeps it.
    pub fn archive_snapshot(&self) -> Value {
        self.game
            .archive_snapshot()
            .ok()
            .and_then(|archived| serde_json::to_value(archived).ok())
            .unwrap_or(Value::Null)
    }

    pub fn availability_changed(&mut self, host: &Host) -> Vec<Effect> {
        self.game.availability_changed(&mut { *host })
    }

    pub fn expire(&mut self, host: &Host) -> Vec<Effect> {
        self.game.expire(host.now)
    }

    /// Play the player's move; false when it is not legal or not their turn.
    pub fn play(&mut self, uci: &str, host: &Host) -> (bool, Vec<Effect>) {
        self.game.play(&mut { *host }, uci)
    }

    /// The engine search to send now, if it is the engine's turn and nothing is searching.
    pub fn search_start(&mut self, host: &Host) -> Result<Option<SearchRequest>, String> {
        self.game.search_start(&mut { *host })
    }

    pub fn search_reply(
        &mut self,
        request: &SearchRequest,
        reply: &str,
        host: &Host,
    ) -> Vec<Effect> {
        let effects = self
            .game
            .search_reply(&mut { *host }, request.epoch, request.count, reply);
        self.game.search_end(request.epoch);
        effects
    }

    /// The search failed: `Failed` unless it was cancelled meanwhile.
    pub fn search_failed(&mut self, request: &SearchRequest) -> Vec<Effect> {
        let effect = self.game.search_failed(request.epoch);
        self.game.search_end(request.epoch);
        vec![effect]
    }

    /// A new game; `setup` and `clock` default to the current game's.
    pub fn start(
        &mut self,
        setup: Option<Setup>,
        clock: Option<Option<Clock>>,
        host: &Host,
    ) -> Result<Vec<Effect>, String> {
        let state = self.game.state();
        let setup = setup.unwrap_or_else(|| state.setup.clone());
        let clock = clock.unwrap_or(state.clock);
        let effects = self.game.start(&mut { *host }, setup, clock)?;
        self.setup = serde_json::json!(self.game.state().setup);
        Ok(effects)
    }

    pub fn takeback(&mut self, host: &Host) -> Vec<Effect> {
        self.game.takeback(&mut { *host })
    }

    pub fn resign(&mut self) -> Vec<Effect> {
        self.game.resign()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn host(now: f64) -> Host {
        Host {
            now,
            allowed: true,
            ready: true,
        }
    }

    fn one_minute() -> Option<Option<Clock>> {
        Some(Some(Clock {
            minutes: 1.,
            increment: 0.,
        }))
    }

    #[test]
    fn a_move_hands_the_turn_to_the_engine_and_its_reply_is_played() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        let (played, effects) = game.play("e2e4", &host(10.));
        assert!(played);
        assert!(effects.contains(&Effect::ComputerTurn), "{effects:?}");
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Moved { san, computer: false } if san == "e4"))
        );
        let request = game.search_start(&host(20.)).unwrap().expect("a search");
        assert_eq!(request.moves, vec!["e2e4"]);
        assert!(game.thinking());
        // A second start while it searches sends nothing.
        assert!(game.search_start(&host(21.)).unwrap().is_none());
        let effects = game.search_reply(&request, "e7e5", &host(30.));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Moved { san, computer: true } if san == "e5"))
        );
        assert_eq!(game.moves(), ["e2e4", "e7e5"]);
        assert!(!game.thinking());
    }

    /// Taking back after the player's flag fell left their clock at 0 ms: the game went on and
    /// they lost on time again with their next move. As on Lichess, a time-out is final.
    #[test]
    fn a_time_out_is_not_taken_back() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        game.start(None, one_minute(), &host(0.)).unwrap();
        game.play("e2e4", &host(1.));
        let request = game.search_start(&host(2.)).unwrap().expect("a search");
        game.search_reply(&request, "e7e5", &host(3.));
        game.expire(&host(200_000.));
        assert!(game.flagged());
        game.takeback(&host(200_001.));
        assert!(game.flagged());
        assert_eq!(game.moves(), ["e2e4", "e7e5"]);
    }

    #[test]
    fn a_reply_to_a_game_taken_back_is_ignored() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        game.play("e2e4", &host(1.));
        let request = game.search_start(&host(2.)).unwrap().unwrap();
        game.takeback(&host(3.));
        game.search_reply(&request, "e7e5", &host(4.));
        assert!(game.moves().is_empty());
    }

    /// Another variant's game is played by its rules and searched in it: a drop is a move, and
    /// the engine is told the variant (Fairy-Stockfish plays it) and the start.
    #[test]
    fn a_crazyhouse_game_searches_in_its_variant() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
        let setup = Setup {
            variant: "crazyhouse".into(),
            fen: start.into(),
        };
        game.start(Some(setup), None, &host(0.)).unwrap();
        for (ply, mv) in ["e2e4", "d7d5", "e4d5", "d8d5"].iter().enumerate() {
            if ply % 2 == 0 {
                assert!(game.play(mv, &host(1.)).0, "{mv}");
            } else {
                let request = game.search_start(&host(2.)).unwrap().expect("a search");
                assert_eq!(request.options.variant.as_deref(), Some("crazyhouse"));
                assert_eq!(request.options.fen.as_deref(), Some(start));
                game.search_reply(&request, mv, &host(3.));
            }
        }
        assert!(game.play("P@e3", &host(4.)).0);
        let request = game.search_start(&host(5.)).unwrap().expect("a search");
        assert_eq!(request.moves, ["e2e4", "d7d5", "e4d5", "d8d5", "P@e3"]);
        assert_eq!(json!(request.options)["chess960"], Value::Null);
    }

    /// A saved game on a setup that is not a legal position is refused: it used to be played on
    /// from the standard start, refusing every move.
    #[test]
    fn a_saved_game_on_an_illegal_setup_is_refused() {
        let saved =
            json!({ "setup": { "variant": "standard", "fen": "8/8/8/8/8/8/8/8 w - - 0 1" } });
        let error = ComputerGame::new(Some(&saved), &host(0.)).err();
        assert_eq!(error.as_deref(), Some(games::ILLEGAL_SETUP));
    }

    /// The engine's clock runs out on its turn: the engine stops, its flag falls and the player
    /// wins; the views report it all.
    #[test]
    fn the_engines_flag_falls_on_its_turn() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        game.start(None, one_minute(), &host(0.)).unwrap();
        // Each side's first move is free.
        assert!(game.play("e2e4", &host(0.)).0);
        let request = game.search_start(&host(0.)).unwrap().unwrap();
        game.search_reply(&request, "e7e5", &host(500.));
        assert_eq!(game.remaining(Color::White, &host(500.)), 60_000.);
        assert!(game.play("d2d4", &host(1_000.)).0);
        assert_eq!(game.remaining(Color::White, &host(1_000.)), 59_500.);
        assert_eq!(game.remaining(Color::Black, &host(31_000.)), 30_000.);
        assert!(game.expire(&host(30_000.)).is_empty());
        assert!(!game.over());
        assert_eq!(game.result(), None);

        let effects = game.expire(&host(61_000.));
        assert_eq!(effects, [Effect::StopEngine, Effect::Flagged]);
        assert!(game.over());
        assert_eq!(game.result().unwrap().kind, Kind::Win);
        let snapshot = game.snapshot(&host(61_000.));
        assert_eq!(snapshot["flagged"], "black");
        assert_eq!(snapshot["moves"], json!(["e2e4", "e7e5", "d2d4"]));
        let archived = game.archive_snapshot();
        assert_eq!(archived["result"], "1-0");
        assert_eq!(archived["timeControl"], "60+0");
    }

    /// A finished computer game is written with its winner, not as a draw: its result view is
    /// the player's win or loss and names no winner. The tag matches its history entry's.
    #[test]
    fn a_resigned_computer_game_is_a_loss_for_the_player() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        assert_eq!(Outcome::tag(game.outcome()), "*");
        assert!(game.play("e2e4", &host(1.)).0);
        game.resign();
        assert_eq!(game.outcome(), Some(Outcome::Wins(Color::Black)));
        assert_eq!(
            Outcome::tag(game.outcome()),
            game.archive_snapshot()["result"]
        );
        game.set_color(Color::Black);
        game.start(None, None, &host(4.)).unwrap();
        let request = game
            .search_start(&host(5.))
            .unwrap()
            .expect("White's search");
        game.search_reply(&request, "e2e4", &host(6.));
        game.resign();
        assert_eq!(Outcome::tag(game.outcome()), "1-0");
    }

    #[test]
    fn a_resignation_stops_the_engine_and_loses() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        game.play("e2e4", &host(1.));
        assert_eq!(game.resign(), [Effect::StopEngine]);
        assert!(game.over());
        assert_eq!(game.result().unwrap().kind, Kind::Loss);
    }

    /// Assistance stopping mid-search cancels the engine.
    #[test]
    fn losing_permission_stops_the_engine() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        game.play("e2e4", &host(1.));
        game.search_start(&host(2.)).unwrap().unwrap();
        let effects = game.availability_changed(&Host {
            now: 3.,
            allowed: false,
            ready: true,
        });
        assert_eq!(effects, [Effect::StopEngine]);
        assert!(!game.thinking());
    }

    /// A failed search is reported, and one cancelled meanwhile is not.
    #[test]
    fn a_failed_search_is_reported_unless_it_was_cancelled() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        game.play("e2e4", &host(1.));
        let request = game.search_start(&host(2.)).unwrap().unwrap();
        assert_eq!(game.search_failed(&request), [Effect::Failed]);
        assert!(!game.thinking());
        let request = game.search_start(&host(4.)).unwrap().unwrap();
        game.takeback(&host(5.));
        assert_eq!(game.search_failed(&request), [Effect::Cancelled]);
    }

    #[test]
    fn an_illegal_move_is_refused() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        assert!(!game.play("e2e5", &host(1.)).0);
        assert!(!game.over());
    }
}
