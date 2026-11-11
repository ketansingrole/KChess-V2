//! A game against the engine: its clock, the engine's replies and the cancellation of work that
//! is no longer wanted.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use shakmaty::Color;

use super::{Board, Clock, Clocking, Host, Outcome, Setup, Times};
use crate::js;
use crate::training::engine_levels;
use crate::variant::Variant;

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

/// The session document and the engine turn's bookkeeping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub moves: Vec<String>,
    pub ply: usize,
    pub level: String,
    #[serde(with = "crate::color::by_name")]
    pub color: Color,
    pub resigned: bool,
    pub setup: Setup,
    pub clock: Option<Clock>,
    pub times: Option<Times>,
    #[serde(with = "crate::color::option_by_name")]
    pub flagged: Option<Color>,
    pub thinking: bool,
    pub epoch: u64,
}

/// What the game keeps beside the state: when the turn began, whether the engine may play, and
/// whether the game was closed (a closed game ignores every input).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    pub turn_started: f64,
    pub permitted: bool,
    pub disposed: bool,
}

impl Runtime {
    /// The runtime of a new game: the clock and availability as the host reports them now.
    pub fn start(host: &Host) -> Runtime {
        Runtime {
            turn_started: host.now,
            permitted: host.allowed,
            disposed: false,
        }
    }
}

/// A saved session: every field may be missing when it is read back.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub moves: Option<Vec<String>>,
    pub ply: Option<usize>,
    pub level: Option<String>,
    #[serde(default, with = "crate::color::option_by_name")]
    pub color: Option<Color>,
    pub resigned: Option<bool>,
    pub setup: Option<Setup>,
    pub clock: Option<Clock>,
    pub times: Option<Times>,
    #[serde(default, with = "crate::color::option_by_name")]
    pub flagged: Option<Color>,
}

impl State {
    /// A fresh state from a saved session.
    pub fn from_saved(saved: Saved) -> State {
        State {
            moves: saved.moves.unwrap_or_default(),
            ply: saved.ply.unwrap_or(0),
            level: saved.level.unwrap_or_else(|| "club".into()),
            color: saved.color.unwrap_or(Color::White),
            resigned: saved.resigned.unwrap_or(false),
            setup: saved.setup.unwrap_or_else(Setup::standard),
            clock: saved.clock,
            times: saved.times,
            flagged: saved.flagged,
            thinking: false,
            epoch: 0,
        }
    }
}

/// What the game asks its caller to do, in order.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Effect {
    StopEngine,
    Flagged,
    Moved {
        san: String,
        computer: bool,
    },
    /// It is the engine's turn: start a search (`search_start`).
    ComputerTurn,
    /// The engine's search failed.
    Failed,
    /// A search cancelled meanwhile failed: expected, and silent.
    #[serde(rename = "debug")]
    Cancelled,
}

/// The options of an engine search.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SearchOptions {
    /// The start, when it is not the usual one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fen: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub chess960: bool,
    /// A variant not played by standard rules: Fairy-Stockfish plays it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<Variant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub movetime: Option<f64>,
}

/// An engine search to send; its reply must still match the epoch and move count.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SearchRequest {
    pub epoch: u64,
    pub count: usize,
    pub moves: Vec<String>,
    pub level: String,
    pub options: SearchOptions,
}

/// Whether the player won, lost or drew.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Win,
    Loss,
    Draw,
}

/// The result from the player's side.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Verdict {
    pub kind: Kind,
    pub title: &'static str,
    pub detail: String,
}

/// The game as the archive stores it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Archived {
    pub source: &'static str,
    pub setup: Setup,
    pub moves: Vec<String>,
    pub white: String,
    pub black: String,
    pub result: &'static str,
    pub reason: String,
    pub time_control: String,
}

/// A game against the engine.
#[derive(Clone, Debug)]
pub struct ComputerGame {
    state: State,
    runtime: Runtime,
    board: Board,
}

impl ComputerGame {
    /// The game of a state and its runtime; an illegal setup is refused.
    pub fn new(state: State, runtime: Runtime) -> Result<ComputerGame, String> {
        let board = Board::new(state.setup.clone(), state.moves.clone())?;
        Ok(ComputerGame {
            state,
            runtime,
            board,
        })
    }

    /// A game from a saved session, or a new one; a session on an illegal setup is refused.
    pub fn resume(saved: Option<&Value>, host: &Host) -> Result<ComputerGame, String> {
        let saved: Saved = match saved {
            Some(saved) => {
                Saved::deserialize(saved).map_err(|error| format!("the saved game: {error}"))?
            }
            None => Saved::default(),
        };
        ComputerGame::new(State::from_saved(saved), Runtime::start(host))
    }

    /// The game's start.
    pub fn setup(&self) -> &Setup {
        &self.state.setup
    }

    /// The game's moves, in UCI.
    pub fn moves(&self) -> &[String] {
        &self.state.moves
    }

    /// The move shown: the moves before it are on the board.
    pub fn ply(&self) -> usize {
        self.state.ply.min(self.state.moves.len())
    }

    /// The player's side.
    pub fn color(&self) -> Color {
        self.state.color
    }

    /// The engine level's id.
    pub fn level(&self) -> &str {
        &self.state.level
    }

    /// The clock chosen, none when untimed.
    pub fn clock(&self) -> Option<Clock> {
        self.state.clock
    }

    /// The clocks run in this game.
    pub fn timed(&self) -> bool {
        self.state.times.is_some()
    }

    /// The engine is searching.
    pub fn thinking(&self) -> bool {
        self.state.thinking
    }

    /// A side's flag has fallen.
    pub fn flagged(&self) -> bool {
        self.state.flagged.is_some()
    }

    /// The result once the game has ended.
    pub fn outcome(&self) -> Option<Outcome> {
        Outcome::of(self.over(), self.winner())
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    /// Show the position after `ply` moves (at most every move).
    pub fn show_ply(&mut self, ply: usize) {
        self.state.ply = ply.min(self.state.moves.len());
    }

    /// Play `color` from the next game on.
    pub fn set_color(&mut self, color: Color) {
        self.state.color = color;
    }

    /// Play at `level` from the next search on.
    pub fn set_level(&mut self, level: &str) {
        self.state.level = level.to_string();
    }

    /// Play on `clock` from the next game on.
    pub fn set_clock(&mut self, clock: Option<Clock>) {
        self.state.clock = clock;
    }

    fn set_board(&mut self, board: Board) {
        self.state.moves = board.moves().to_vec();
        self.board = board;
    }

    /// Resigned, flagged, the position ended, or a draw reached.
    pub fn over(&self) -> bool {
        self.state.resigned
            || self.state.flagged.is_some()
            || self.board.info().end
            || self.board.draw().is_some()
    }

    /// A clock is set, both sides have moved and the game goes on.
    pub fn running(&self) -> bool {
        self.runtime.permitted
            && self.state.times.is_some()
            && !self.over()
            && self.state.moves.len() >= 2
    }

    fn clocking(&self) -> Clocking {
        Clocking {
            times: self.state.times,
            turn_started: self.runtime.turn_started,
            running: self.running(),
            turn: self.board.turn(),
        }
    }

    /// A side's clock in milliseconds at `at`.
    pub fn remaining(&self, color: Color, at: f64) -> f64 {
        self.clocking().remaining(color, at)
    }

    /// The repetition or fifty-move draw reached, if any.
    pub fn draw(&self) -> Option<&str> {
        self.board.draw()
    }

    /// The side that won, or none for a draw or a game going on.
    pub fn winner(&self) -> Option<Color> {
        if !self.over() {
            return None;
        }
        if self.state.resigned {
            return Some(!self.state.color);
        }
        match self.state.flagged {
            Some(flagged) => self.board.timeout_winner(flagged),
            None => self.board.outcome_winner(),
        }
    }

    /// The result from the player's side, once the game is over.
    pub fn result(&self) -> Option<Verdict> {
        if !self.over() {
            return None;
        }
        if self.state.resigned {
            return Some(Verdict {
                kind: Kind::Loss,
                title: "Stockfish won",
                detail: "You resigned".into(),
            });
        }
        let winner = self.winner();
        let won = winner == Some(self.state.color);
        let detail = if self.state.flagged.is_some() {
            if winner.is_none() {
                "Time ran out, but mate was impossible"
            } else if won {
                "Stockfish ran out of time"
            } else {
                "You ran out of time"
            }
            .to_string()
        } else {
            self.board
                .end()
                .map_or("Game over", |end| end.reason.as_str())
                .to_string()
        };
        let (kind, title) = if winner.is_none() {
            (Kind::Draw, "Draw")
        } else if won {
            (Kind::Win, "You won")
        } else {
            (Kind::Loss, "Stockfish won")
        };
        Some(Verdict {
            kind,
            title,
            detail,
        })
    }

    /// Stop the engine and invalidate any reply it may still send.
    fn cancel(&mut self, fx: &mut Vec<Effect>) {
        self.state.epoch += 1;
        self.state.thinking = false;
        fx.push(Effect::StopEngine);
    }

    fn expire_into(&mut self, at: f64, fx: &mut Vec<Effect>) -> bool {
        let Some(turn) = self.clocking().out_of_time(at) else {
            return false;
        };
        if let Some(times) = self.state.times.as_mut() {
            times.set(turn, 0.0);
        }
        self.state.flagged = Some(turn);
        self.cancel(fx);
        fx.push(Effect::Flagged);
        true
    }

    /// Flag the side on the clock when its time is up at `at`.
    pub fn expire(&mut self, at: f64) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.expire_into(at, &mut fx);
        fx
    }

    /// Play a legal move, charge the clock and report it.
    fn commit(&mut self, uci: &str, at: f64, computer: bool, fx: &mut Vec<Effect>) -> bool {
        let Some((board, san)) = self.board.with_move(uci) else {
            return false;
        };
        let mover = self.board.turn();
        let increment = if self.state.moves.len() >= 2 {
            self.state.clock.map_or(0.0, |clock| clock.increment) * 1000.0
        } else {
            0.0
        };
        if let Some(times) = self.clocking().charged(mover, at, increment) {
            self.state.times = Some(times);
        }
        self.runtime.turn_started = at;
        self.set_board(board);
        self.state.ply = self.state.moves.len();
        fx.push(Effect::Moved { san, computer });
        true
    }

    /// The player's move, when it is theirs to make: whether it was played.
    pub fn play(&mut self, uci: &str, host: &Host) -> (bool, Vec<Effect>) {
        let mut fx = Vec::new();
        if self.runtime.disposed || !host.allowed || !host.ready {
            return (false, fx);
        }
        if self.state.ply != self.state.moves.len()
            || self.over()
            || self.state.thinking
            || self.board.turn() != self.state.color
        {
            return (false, fx);
        }
        let at = host.now;
        if self.expire_into(at, &mut fx) || !self.commit(uci, at, false, &mut fx) {
            return (false, fx);
        }
        fx.push(Effect::ComputerTurn);
        (true, fx)
    }

    /// The start of an engine turn, when one is due: the request to send.
    pub fn search_start(&mut self, host: &Host) -> Result<Option<SearchRequest>, String> {
        if self.runtime.disposed || self.state.thinking {
            return Ok(None);
        }
        if self.board.turn() == self.state.color || self.over() {
            return Ok(None);
        }
        if !host.ready || !host.allowed {
            return Ok(None);
        }
        self.state.thinking = true;
        let left = self.remaining(self.board.turn(), host.now);
        let movetime = match self.state.times {
            Some(_) => {
                let time = level_info(&self.state.level)?.time;
                let increment = self.state.clock.map_or(0.0, |clock| clock.increment);
                let budget = (left / 30.0 + increment * 800.0).floor();
                Some(50.0_f64.max(5000.0_f64.min(time).min(budget)))
            }
            None => None,
        };
        let setup = &self.state.setup;
        let options = SearchOptions {
            fen: (!setup.is_standard_start()).then(|| setup.fen.clone()),
            chess960: setup.variant == Variant::Chess960,
            variant: (!setup.variant.standard_rules()).then_some(setup.variant),
            movetime,
        };
        Ok(Some(SearchRequest {
            epoch: self.state.epoch,
            count: self.state.moves.len(),
            moves: self.state.moves.clone(),
            level: self.state.level.clone(),
            options,
        }))
    }

    /// The engine's reply to the search of `epoch` at `count` moves, if it is still wanted: the
    /// move is played (or the clock flags). The search goes on until `search_end`.
    pub fn reply(&mut self, epoch: u64, count: usize, reply: &str, host: &Host) -> Vec<Effect> {
        let mut fx = Vec::new();
        if self.runtime.disposed
            || epoch != self.state.epoch
            || count != self.state.moves.len()
            || self.over()
            || !host.allowed
        {
            return fx;
        }
        let at = host.now;
        if !self.expire_into(at, &mut fx) {
            self.commit(reply, at, true, &mut fx);
        }
        fx
    }

    /// The engine's reply to `request`, which ends its search.
    pub fn search_reply(
        &mut self,
        request: &SearchRequest,
        reply: &str,
        host: &Host,
    ) -> Vec<Effect> {
        let effects = self.reply(request.epoch, request.count, reply, host);
        self.search_end(request.epoch);
        effects
    }

    /// The search of `epoch` failed: reported unless it was cancelled meanwhile.
    pub fn failure(&self, epoch: u64) -> Effect {
        if epoch == self.state.epoch && !self.runtime.disposed {
            Effect::Failed
        } else {
            Effect::Cancelled
        }
    }

    /// The search of `request` failed, which ends it: `Failed` unless it was cancelled
    /// meanwhile.
    pub fn search_failed(&mut self, request: &SearchRequest) -> Vec<Effect> {
        let effect = self.failure(request.epoch);
        self.search_end(request.epoch);
        vec![effect]
    }

    /// The end of the search of `epoch`, however it finished.
    pub fn search_end(&mut self, epoch: u64) {
        if epoch == self.state.epoch {
            self.state.thinking = false;
        }
    }

    /// A new game; `setup` and `clock` default to the current game's. An illegal setup or clock
    /// is refused.
    pub fn start(
        &mut self,
        setup: Option<Setup>,
        clock: Option<Option<Clock>>,
        host: &Host,
    ) -> Result<Vec<Effect>, String> {
        let setup = setup.unwrap_or_else(|| self.state.setup.clone());
        let clock = clock.unwrap_or(self.state.clock);
        let clock = clock.map(Clock::checked).transpose()?;
        let mut fx = Vec::new();
        if self.runtime.disposed {
            return Ok(fx);
        }
        let board = Board::new(setup.clone(), Vec::new())?;
        self.cancel(&mut fx);
        self.state.setup = setup;
        self.state.clock = clock;
        self.set_board(board);
        self.state.ply = 0;
        self.state.resigned = false;
        self.state.flagged = None;
        self.state.times = clock.map(|clock| {
            let ms = clock.minutes * 60000.0;
            Times {
                white: ms,
                black: ms,
            }
        });
        self.runtime.turn_started = host.now;
        fx.push(Effect::ComputerTurn);
        Ok(fx)
    }

    /// Undo back to the player's last turn. A time-out is final: undoing it left the flagged
    /// side on 0 ms, to lose on time again on its next move.
    pub fn takeback(&mut self, host: &Host) -> Vec<Effect> {
        let mut fx = Vec::new();
        if self.runtime.disposed || self.state.moves.is_empty() || self.state.flagged.is_some() {
            return fx;
        }
        self.cancel(&mut fx);
        self.state.resigned = false;
        self.state.flagged = None;
        let mut kept = self.state.moves.len() - 1;
        while kept > 0 && self.board.turn_after(kept) != self.state.color {
            kept -= 1;
        }
        let moves = self.state.moves[..kept].to_vec();
        if let Ok(board) = Board::new(self.state.setup.clone(), moves) {
            self.set_board(board);
        }
        self.state.ply = self.state.moves.len();
        self.runtime.turn_started = host.now;
        fx.push(Effect::ComputerTurn);
        fx
    }

    pub fn resign(&mut self) -> Vec<Effect> {
        let mut fx = Vec::new();
        if self.runtime.disposed || self.state.moves.is_empty() || self.over() {
            return fx;
        }
        self.cancel(&mut fx);
        self.state.resigned = true;
        self.state.ply = self.state.moves.len();
        fx
    }

    /// Assistance was allowed or stopped: freeze the clocks and cancel the engine when it stops.
    pub fn availability_changed(&mut self, host: &Host) -> Vec<Effect> {
        let mut fx = Vec::new();
        let permitted = host.allowed;
        let at = host.now;
        if self.runtime.permitted && !permitted {
            if self.state.times.is_some() {
                self.state.times = Some(self.clocking().times_at(at));
            }
            self.cancel(&mut fx);
        }
        if permitted != self.runtime.permitted {
            self.runtime.turn_started = at;
        }
        self.runtime.permitted = permitted;
        fx
    }

    /// The session to save, with each side's remaining time at `at`.
    pub fn snapshot(&self, at: f64) -> Saved {
        Saved {
            moves: Some(self.state.moves.clone()),
            ply: Some(self.state.ply),
            level: Some(self.state.level.clone()),
            color: Some(self.state.color),
            resigned: Some(self.state.resigned),
            setup: Some(self.state.setup.clone()),
            clock: self.state.clock,
            times: self.state.times.map(|_| self.clocking().times_at(at)),
            flagged: self.state.flagged,
        }
    }

    /// The game as the archive stores it.
    pub fn archive_snapshot(&self) -> Result<Archived, String> {
        let engine = format!("Stockfish ({})", level_info(&self.state.level)?.label);
        let (white, black) = players(self.state.color, &engine);
        let (white, black) = (white.to_string(), black.to_string());
        let reason = self
            .result()
            .map_or_else(|| "In progress".to_string(), |verdict| verdict.detail);
        let time_control = match self.state.clock {
            Some(clock) => format!(
                "{}+{}",
                js::number_to_string(clock.minutes * 60.0),
                js::number_to_string(clock.increment)
            ),
            None => "-".to_string(),
        };
        Ok(Archived {
            source: "computer",
            setup: self.state.setup.clone(),
            moves: self.state.moves.clone(),
            white,
            black,
            result: Outcome::tag(self.outcome()),
            reason,
            time_control,
        })
    }
}

/// The ladder entry for an engine level; an error for an unknown one.
fn level_info(level: &str) -> Result<&'static engine_levels::EngineLevelInfo, String> {
    engine_levels::info(level)?.ok_or_else(|| format!("unknown engine level {level}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::ILLEGAL_SETUP;
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
        let mut game = ComputerGame::resume(None, &host(0.)).unwrap();
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
        let mut game = ComputerGame::resume(None, &host(0.)).unwrap();
        game.start(None, one_minute(), &host(0.)).unwrap();
        game.play("e2e4", &host(1.));
        let request = game.search_start(&host(2.)).unwrap().expect("a search");
        game.search_reply(&request, "e7e5", &host(3.));
        game.expire(200_000.);
        assert!(game.flagged());
        game.takeback(&host(200_001.));
        assert!(game.flagged());
        assert_eq!(game.moves(), ["e2e4", "e7e5"]);
    }

    #[test]
    fn a_reply_to_a_game_taken_back_is_ignored() {
        let mut game = ComputerGame::resume(None, &host(0.)).unwrap();
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
        let mut game = ComputerGame::resume(None, &host(0.)).unwrap();
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
        let setup = Setup {
            variant: Variant::Crazyhouse,
            fen: start.into(),
        };
        game.start(Some(setup), None, &host(0.)).unwrap();
        for (ply, mv) in ["e2e4", "d7d5", "e4d5", "d8d5"].iter().enumerate() {
            if ply % 2 == 0 {
                assert!(game.play(mv, &host(1.)).0, "{mv}");
            } else {
                let request = game.search_start(&host(2.)).unwrap().expect("a search");
                assert_eq!(request.options.variant, Some(Variant::Crazyhouse));
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
        let error = ComputerGame::resume(Some(&saved), &host(0.)).err();
        assert_eq!(error.as_deref(), Some(ILLEGAL_SETUP));
    }

    /// The engine's clock runs out on its turn: the engine stops, its flag falls and the player
    /// wins; the views report it all.
    #[test]
    fn the_engines_flag_falls_on_its_turn() {
        let mut game = ComputerGame::resume(None, &host(0.)).unwrap();
        game.start(None, one_minute(), &host(0.)).unwrap();
        // Each side's first move is free.
        assert!(game.play("e2e4", &host(0.)).0);
        let request = game.search_start(&host(0.)).unwrap().unwrap();
        game.search_reply(&request, "e7e5", &host(500.));
        assert_eq!(game.remaining(Color::White, 500.), 60_000.);
        assert!(game.play("d2d4", &host(1_000.)).0);
        assert_eq!(game.remaining(Color::White, 1_000.), 59_500.);
        assert_eq!(game.remaining(Color::Black, 31_000.), 30_000.);
        assert!(game.expire(30_000.).is_empty());
        assert!(!game.over());
        assert_eq!(game.result(), None);

        let effects = game.expire(61_000.);
        assert_eq!(effects, [Effect::StopEngine, Effect::Flagged]);
        assert!(game.over());
        assert_eq!(game.result().unwrap().kind, Kind::Win);
        let snapshot = json!(game.snapshot(61_000.));
        assert_eq!(snapshot["flagged"], "black");
        assert_eq!(snapshot["moves"], json!(["e2e4", "e7e5", "d2d4"]));
        let archived = json!(game.archive_snapshot().unwrap());
        assert_eq!(archived["result"], "1-0");
        assert_eq!(archived["timeControl"], "60+0");
    }

    /// A finished computer game is written with its winner, not as a draw: its result view is
    /// the player's win or loss and names no winner. The tag matches its history entry's.
    #[test]
    fn a_resigned_computer_game_is_a_loss_for_the_player() {
        let mut game = ComputerGame::resume(None, &host(0.)).unwrap();
        assert_eq!(Outcome::tag(game.outcome()), "*");
        assert!(game.play("e2e4", &host(1.)).0);
        game.resign();
        assert_eq!(game.outcome(), Some(Outcome::Wins(Color::Black)));
        assert_eq!(
            Outcome::tag(game.outcome()),
            game.archive_snapshot().unwrap().result
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
        let mut game = ComputerGame::resume(None, &host(0.)).unwrap();
        game.play("e2e4", &host(1.));
        assert_eq!(game.resign(), [Effect::StopEngine]);
        assert!(game.over());
        assert_eq!(game.result().unwrap().kind, Kind::Loss);
    }

    /// Assistance stopping mid-search cancels the engine.
    #[test]
    fn losing_permission_stops_the_engine() {
        let mut game = ComputerGame::resume(None, &host(0.)).unwrap();
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
        let mut game = ComputerGame::resume(None, &host(0.)).unwrap();
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
        let mut game = ComputerGame::resume(None, &host(0.)).unwrap();
        assert!(!game.play("e2e5", &host(1.)).0);
        assert!(!game.over());
    }
}
