//! `ComputerGame` : a game against the engine. The rules decide everything
//! (`crates/kchess-domain/src/games/computer.rs`); the page answers the reads (`now`, `allowed`,
//! `ready`) and carries out the effects, in order.

use serde_json::{Value, json};

use kchess_domain::games::computer as session;

use shakmaty::Color;

use crate::driver::{self, Driver};
use crate::{Outcome, Transition};

/// What a transition asks the page to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    StopEngine,
    Flagged,
    Moved {
        san: String,
        computer: bool,
    },
    /// It is the engine's turn: start a search (`search_start`).
    ComputerTurn,
    /// The last engine operation failed.
    Failed,
}

/// An engine search to send; its reply must still match the epoch and move count.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchRequest {
    pub epoch: u64,
    pub count: u64,
    pub moves: Vec<String>,
    pub level: String,
    pub options: Value,
}

/// What the page reports when a transition reads the host.
pub struct Host {
    /// Monotonic milliseconds, as `performance.now()`.
    pub now: f64,
    pub allowed: bool,
    pub ready: bool,
}

pub struct ComputerGame {
    driver: Driver,
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

struct Done {
    value: Value,
    effects: Vec<Effect>,
}

fn effects(done: &Value) -> Vec<Effect> {
    done["effects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|effect| match effect["type"].as_str()? {
            "stopEngine" => Some(Effect::StopEngine),
            "flagged" => Some(Effect::Flagged),
            "moved" => Some(Effect::Moved {
                san: effect["san"].as_str()?.to_string(),
                computer: effect["computer"].as_bool()?,
            }),
            "computerTurn" => Some(Effect::ComputerTurn),
            "failed" => Some(Effect::Failed),
            // `debug`: an expected cancellation, silent by design.
            _ => None,
        })
        .collect()
}

impl ComputerGame {
    /// A game from a saved session (`computerState(saved)`), or a new one.
    pub fn new(saved: Option<&Value>, host: &Host) -> Result<ComputerGame, String> {
        let state = session::state(&[saved.cloned().unwrap_or(Value::Null)])?;
        let driver = Driver::start(
            state,
            session::init,
            |kind| read(host, kind),
            "computer game",
        )?;
        Ok(ComputerGame { driver })
    }

    /// The `ComputerState` document (the saved session plus `thinking` and `epoch`).
    pub fn state(&self) -> &Value {
        &self.driver.state
    }

    /// The game's start: its `{ variant, fen }`.
    pub fn setup(&self) -> &Value {
        &self.driver.state["setup"]
    }

    /// The game's moves, in UCI.
    pub fn moves(&self) -> Vec<String> {
        crate::moves_of(&self.driver.state)
    }

    /// The move shown: the moves before it are on the board.
    pub fn ply(&self) -> usize {
        (crate::whole(&self.driver.state["ply"]) as usize).min(self.moves().len())
    }

    /// Show the position after `ply` moves (at most every move).
    pub fn show_ply(&mut self, ply: usize) {
        self.driver.state["ply"] = json!(ply.min(self.moves().len()));
    }

    /// The player's side.
    pub fn color(&self) -> Color {
        if self.driver.state["color"] == "black" {
            Color::Black
        } else {
            Color::White
        }
    }

    /// Play `color` from the next game on.
    pub fn set_color(&mut self, color: Color) {
        self.driver.state["color"] = json!(crate::color_name(color));
    }

    /// The engine level's id.
    pub fn level(&self) -> &str {
        self.driver.state["level"].as_str().unwrap_or("club")
    }

    /// Play at `level` from the next search on.
    pub fn set_level(&mut self, level: &str) {
        self.driver.state["level"] = json!(level);
    }

    /// The clock chosen (`{ minutes, increment }`), null when untimed.
    pub fn clock(&self) -> &Value {
        &self.driver.state["clock"]
    }

    /// Play on `clock` from the next game on.
    pub fn set_clock(&mut self, clock: Value) {
        self.driver.state["clock"] = clock;
    }

    /// The clocks run in this game.
    pub fn timed(&self) -> bool {
        !self.driver.state["times"].is_null()
    }

    /// The engine is searching.
    pub fn thinking(&self) -> bool {
        self.driver.state["thinking"].as_bool() == Some(true)
    }

    /// A side's flag has fallen.
    pub fn flagged(&self) -> bool {
        !self.driver.state["flagged"].is_null()
    }

    /// The result once the game has ended, from the player's win, loss or draw.
    pub fn outcome(&self, host: &Host) -> Option<Outcome> {
        let result = self.result(host)?;
        let user = self.color();
        Some(match result["kind"].as_str() {
            Some("win") => Outcome::Wins(user),
            Some("loss") => Outcome::Wins(!user),
            _ => Outcome::Draw,
        })
    }

    fn apply(&mut self, step: Transition, input: &[Value], host: &Host) -> Result<Done, String> {
        let done = self.driver.apply(step, input, |kind| read(host, kind))?;
        Ok(Done {
            value: driver::value(&done),
            effects: effects(&done),
        })
    }

    /// A getter of the game; nothing changes.
    fn view(&self, query: Value, host: &Host) -> Value {
        self.driver
            .view(session::view, query, |kind| read(host, kind))
    }

    pub fn over(&self, host: &Host) -> bool {
        self.view(driver::named("over"), host).as_bool() == Some(true)
    }

    /// `{ kind, title, detail }` once the game has ended.
    pub fn result(&self, host: &Host) -> Option<Value> {
        Some(self.view(driver::named("result"), host)).filter(|value| !value.is_null())
    }

    /// A side's clock in milliseconds.
    pub fn remaining(&self, color: Color, host: &Host) -> f64 {
        self.view(
            json!({ "view": "remaining", "color": crate::color_name(color), "at": host.now }),
            host,
        )
        .as_f64()
        .unwrap_or(0.)
    }

    /// The session to save.
    pub fn snapshot(&self, host: &Host) -> Value {
        self.view(driver::named("snapshot"), host)
    }

    /// The finished game for the history (`saveArchivedGame`).
    pub fn archive_snapshot(&self, host: &Host) -> Value {
        self.view(driver::named("archiveSnapshot"), host)
    }

    pub fn availability_changed(&mut self, host: &Host) -> Result<Vec<Effect>, String> {
        Ok(self.apply(session::availability, &[], host)?.effects)
    }

    pub fn expire(&mut self, host: &Host) -> Result<Vec<Effect>, String> {
        Ok(self
            .apply(session::expire, &[json!(host.now)], host)?
            .effects)
    }

    /// Play the player's move; false when it is not legal or not their turn.
    pub fn play(&mut self, uci: &str, host: &Host) -> Result<(bool, Vec<Effect>), String> {
        let done = self.apply(session::play, &[json!(uci)], host)?;
        Ok((done.value.as_bool() == Some(true), done.effects))
    }

    /// The engine search to send now, if it is the engine's turn and nothing is searching.
    pub fn search_start(&mut self, host: &Host) -> Result<Option<SearchRequest>, String> {
        if self.thinking() {
            return Ok(None);
        }
        let request = self.apply(session::search_start, &[], host)?.value;
        if request.is_null() {
            return Ok(None);
        }
        Ok(Some(SearchRequest {
            epoch: crate::whole(&request["epoch"]),
            count: crate::whole(&request["count"]),
            moves: serde_json::from_value(request["moves"].clone()).unwrap_or_default(),
            level: request["level"].as_str().unwrap_or("beginner").to_string(),
            options: request["options"].clone(),
        }))
    }

    pub fn search_reply(
        &mut self,
        request: &SearchRequest,
        reply: &str,
        host: &Host,
    ) -> Result<Vec<Effect>, String> {
        let input = json!({ "epoch": request.epoch, "count": request.count, "move": reply });
        let effects = self.apply(session::search_reply, &[input], host)?.effects;
        self.apply(
            session::search_end,
            &[json!({ "epoch": request.epoch })],
            host,
        )?;
        Ok(effects)
    }

    pub fn search_failed(
        &mut self,
        request: &SearchRequest,
        host: &Host,
    ) -> Result<Vec<Effect>, String> {
        let effects = self
            .apply(
                session::search_failed,
                &[json!({ "epoch": request.epoch })],
                host,
            )?
            .effects;
        self.apply(
            session::search_end,
            &[json!({ "epoch": request.epoch })],
            host,
        )?;
        Ok(effects)
    }

    /// A new game; `setup` and `clock` default to the current game's.
    pub fn start(
        &mut self,
        setup: Option<Value>,
        clock: Option<Value>,
        host: &Host,
    ) -> Result<Vec<Effect>, String> {
        let setup = setup.unwrap_or_else(|| self.state()["setup"].clone());
        let clock = clock.unwrap_or_else(|| self.state()["clock"].clone());
        Ok(self.apply(session::start, &[setup, clock], host)?.effects)
    }

    pub fn takeback(&mut self, host: &Host) -> Result<Vec<Effect>, String> {
        Ok(self.apply(session::takeback, &[], host)?.effects)
    }

    pub fn resign(&mut self, host: &Host) -> Result<Vec<Effect>, String> {
        Ok(self.apply(session::resign, &[], host)?.effects)
    }
}

fn read(host: &Host, kind: &str) -> Result<Value, String> {
    match kind {
        "now" => Ok(json!(host.now)),
        "allowed" => Ok(json!(host.allowed)),
        "ready" => Ok(json!(host.ready)),
        other => Err(format!("A computer game has no host read {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(now: f64) -> Host {
        Host {
            now,
            allowed: true,
            ready: true,
        }
    }

    #[test]
    fn a_move_hands_the_turn_to_the_engine_and_its_reply_is_played() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        let (played, effects) = game.play("e2e4", &host(10.)).unwrap();
        assert!(played);
        assert!(effects.contains(&Effect::ComputerTurn), "{effects:?}");
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Moved { san, computer: false } if san == "e4"))
        );
        let request = game.search_start(&host(20.)).unwrap().expect("a search");
        assert_eq!(request.moves, vec!["e2e4"]);
        assert_eq!(game.state()["thinking"], true);
        // A second start while it searches sends nothing.
        assert!(game.search_start(&host(21.)).unwrap().is_none());
        let effects = game.search_reply(&request, "e7e5", &host(30.)).unwrap();
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Moved { san, computer: true } if san == "e5"))
        );
        assert_eq!(game.state()["moves"], json!(["e2e4", "e7e5"]));
        assert_eq!(game.state()["thinking"], false);
    }

    /// Taking back after the player's flag fell left their clock at 0 ms: the game went on and
    /// they lost on time again with their next move. As on Lichess, a time-out is final.
    #[test]
    fn a_time_out_is_not_taken_back() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        game.start(
            None,
            Some(json!({ "minutes": 1, "increment": 0 })),
            &host(0.),
        )
        .unwrap();
        game.play("e2e4", &host(1.)).unwrap();
        let request = game.search_start(&host(2.)).unwrap().expect("a search");
        game.search_reply(&request, "e7e5", &host(3.)).unwrap();
        game.expire(&host(200_000.)).unwrap();
        assert!(!game.state()["flagged"].is_null());
        game.takeback(&host(200_001.)).unwrap();
        assert!(!game.state()["flagged"].is_null());
        assert_eq!(game.state()["moves"], json!(["e2e4", "e7e5"]));
    }

    #[test]
    fn a_reply_to_a_game_taken_back_is_ignored() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        game.play("e2e4", &host(1.)).unwrap();
        let request = game.search_start(&host(2.)).unwrap().unwrap();
        game.takeback(&host(3.)).unwrap();
        game.search_reply(&request, "e7e5", &host(4.)).unwrap();
        assert_eq!(game.state()["moves"], json!([]));
    }

    /// Another variant's game is played by its rules and searched in it: a drop is a move, and
    /// the engine is told the variant (Fairy-Stockfish plays it) and the start.
    #[test]
    fn a_crazyhouse_game_searches_in_its_variant() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
        game.start(
            Some(json!({ "variant": "crazyhouse", "fen": start })),
            None,
            &host(0.),
        )
        .unwrap();
        for (ply, mv) in ["e2e4", "d7d5", "e4d5", "d8d5"].iter().enumerate() {
            if ply % 2 == 0 {
                assert!(game.play(mv, &host(1.)).unwrap().0, "{mv}");
            } else {
                let request = game.search_start(&host(2.)).unwrap().expect("a search");
                assert_eq!(request.options["variant"], "crazyhouse");
                assert_eq!(request.options["fen"], start);
                game.search_reply(&request, mv, &host(3.)).unwrap();
            }
        }
        let (played, _) = game.play("P@e3", &host(4.)).unwrap();
        assert!(played);
        let request = game.search_start(&host(5.)).unwrap().expect("a search");
        assert_eq!(request.moves, ["e2e4", "d7d5", "e4d5", "d8d5", "P@e3"]);
        assert!(request.options.get("chess960").is_none());
    }

    fn host_at(now: f64, allowed: bool) -> Host {
        Host {
            now,
            allowed,
            ready: true,
        }
    }

    /// The engine's clock runs out on its turn: the engine stops, its flag falls and the player
    /// wins; the views report it all.
    #[test]
    fn the_engines_flag_falls_on_its_turn() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        let clock = json!({ "minutes": 1, "increment": 0 });
        game.start(None, Some(clock), &host(0.)).unwrap();
        // Each side's first move is free.
        assert!(game.play("e2e4", &host(0.)).unwrap().0);
        let request = game.search_start(&host(0.)).unwrap().unwrap();
        game.search_reply(&request, "e7e5", &host(500.)).unwrap();
        assert_eq!(game.remaining(Color::White, &host(500.)), 60_000.);
        assert!(game.play("d2d4", &host(1_000.)).unwrap().0);
        assert_eq!(game.remaining(Color::White, &host(1_000.)), 59_500.);
        assert_eq!(game.remaining(Color::Black, &host(31_000.)), 30_000.);
        assert!(game.expire(&host(30_000.)).unwrap().is_empty());
        assert!(!game.over(&host(30_000.)));
        assert_eq!(game.result(&host(30_000.)), None);

        let effects = game.expire(&host(61_000.)).unwrap();
        assert_eq!(effects, [Effect::StopEngine, Effect::Flagged]);
        assert!(game.over(&host(61_000.)));
        assert_eq!(game.result(&host(61_000.)).unwrap()["kind"], "win");
        let snapshot = game.snapshot(&host(61_000.));
        assert_eq!(snapshot["flagged"], "black");
        assert_eq!(snapshot["moves"], json!(["e2e4", "e7e5", "d2d4"]));
        let archived = game.archive_snapshot(&host(61_000.));
        assert_eq!(archived["result"], "1-0");
        assert_eq!(archived["timeControl"], "60+0");
    }

    /// A finished computer game is written with its winner, not as a draw: its result view is
    /// the player's win or loss and names no winner. The tag matches its history entry's.
    #[test]
    fn a_resigned_computer_game_is_a_loss_for_the_player() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        assert_eq!(Outcome::tag(game.outcome(&host(0.))), "*");
        assert!(game.play("e2e4", &host(1.)).unwrap().0);
        game.resign(&host(2.)).unwrap();
        assert_eq!(game.outcome(&host(3.)), Some(Outcome::Wins(Color::Black)));
        assert_eq!(
            Outcome::tag(game.outcome(&host(3.))),
            game.archive_snapshot(&host(3.))["result"]
        );
        game.set_color(Color::Black);
        game.start(None, None, &host(4.)).unwrap();
        let request = game
            .search_start(&host(5.))
            .unwrap()
            .expect("White's search");
        game.search_reply(&request, "e2e4", &host(6.)).unwrap();
        game.resign(&host(7.)).unwrap();
        assert_eq!(Outcome::tag(game.outcome(&host(8.))), "1-0");
    }

    #[test]
    fn a_resignation_stops_the_engine_and_loses() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        game.play("e2e4", &host(1.)).unwrap();
        assert_eq!(game.resign(&host(2.)).unwrap(), [Effect::StopEngine]);
        assert!(game.over(&host(3.)));
        assert_eq!(game.result(&host(3.)).unwrap()["kind"], "loss");
    }

    /// Assistance stopping mid-search cancels the engine.
    #[test]
    fn losing_permission_stops_the_engine() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        game.play("e2e4", &host(1.)).unwrap();
        game.search_start(&host(2.)).unwrap().unwrap();
        let effects = game.availability_changed(&host_at(3., false)).unwrap();
        assert_eq!(effects, [Effect::StopEngine]);
        assert_eq!(game.state()["thinking"], false);
    }

    /// A failed search is reported, and one cancelled meanwhile is not.
    #[test]
    fn a_failed_search_is_reported_unless_it_was_cancelled() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        game.play("e2e4", &host(1.)).unwrap();
        let request = game.search_start(&host(2.)).unwrap().unwrap();
        assert_eq!(
            game.search_failed(&request, &host(3.)).unwrap(),
            [Effect::Failed]
        );
        assert_eq!(game.state()["thinking"], false);
        let request = game.search_start(&host(4.)).unwrap().unwrap();
        game.takeback(&host(5.)).unwrap();
        assert!(game.search_failed(&request, &host(6.)).unwrap().is_empty());
    }

    #[test]
    fn an_illegal_move_is_refused() {
        let mut game = ComputerGame::new(None, &host(0.)).unwrap();
        let (played, _) = game.play("e2e5", &host(1.)).unwrap();
        assert!(!played);
        assert!(!game.over(&host(1.)));
    }
}
