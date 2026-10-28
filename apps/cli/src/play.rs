//! `play` and `local`: a game against Stockfish, or two people at one terminal, with UCI moves.
//! The rules run both (`kchess-games`); the session and Game history entry are saved in the
//! core's library as the app saves them, so either can carry the game on.

use std::future::Future;
use std::time::Instant;

use kchess_games::archive::{self, GameArchive};
use kchess_games::computer::{self, ComputerGame, SearchRequest};
use kchess_games::local::LocalGame;
use serde_json::{Value, json};
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::core::{CoreClient, Event};

/// Awaits `work` while answering the core's host requests; other events are dropped.
async fn answering<T>(
    core: &CoreClient,
    events: &mut UnboundedReceiver<Event>,
    work: impl Future<Output = T>,
) -> T {
    tokio::pin!(work);
    loop {
        tokio::select! {
            done = &mut work => return done,
            Some(event) = events.recv() => {
                if event.name == "host:request" {
                    core.answer(&event.payload).await;
                }
            }
        }
    }
}

enum Game {
    Computer(ComputerGame),
    Local(LocalGame),
}

struct Session<'a> {
    core: &'a CoreClient,
    game: Game,
    history: GameArchive,
    games: Vec<Value>,
    started: Instant,
    failed: bool,
}

impl Session<'_> {
    /// Monotonic milliseconds, as `performance.now()`.
    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    fn host(&self) -> computer::Host {
        computer::Host {
            now: self.now(),
            allowed: true,
            ready: true,
        }
    }

    fn kind(&self) -> &'static str {
        match self.game {
            Game::Computer(_) => "computer",
            Game::Local(_) => "local",
        }
    }

    fn archive_kind(&self) -> &'static str {
        match self.game {
            Game::Computer(_) => "archive:computer",
            Game::Local(_) => "archive:board",
        }
    }

    fn state(&self) -> &Value {
        match &self.game {
            Game::Computer(game) => &game.state,
            Game::Local(game) => &game.state,
        }
    }

    fn moves(&self) -> Vec<String> {
        serde_json::from_value(self.state()["moves"].clone()).unwrap_or_default()
    }

    fn result(&self) -> Option<Value> {
        match &self.game {
            Game::Computer(game) => game.result(&self.host()),
            Game::Local(game) => game.result(self.now()),
        }
    }

    fn over(&self) -> bool {
        match &self.game {
            Game::Computer(game) => game.over(&self.host()),
            Game::Local(game) => game.over(self.now()),
        }
    }

    fn fen(&self) -> String {
        let setup = &self.state()["setup"];
        let variant = setup["variant"].as_str().unwrap_or("standard");
        let fen = setup["fen"].as_str().unwrap_or_default();
        kchess_domain::replay::replay_setup(variant, fen, &self.moves())
            .map(|replay| replay.fen)
            .unwrap_or_else(|| fen.to_string())
    }

    fn show(&self) {
        println!(
            "{}",
            json!({ "fen": self.fen(), "moves": self.moves(), "result": self.result() })
        );
    }

    async fn save(&mut self, method: &str, args: Vec<Value>) {
        if let Err(cause) = self.core.call(method, args).await {
            eprintln!("[cli] Saving failed: {cause}");
            self.failed = true;
        }
    }

    async fn written(&mut self, effects: Vec<archive::Effect>) {
        for effect in effects {
            match effect {
                archive::Effect::Save(game) => {
                    let id = game["id"].clone();
                    self.games.retain(|kept| kept["id"] != id);
                    self.games.push(game.clone());
                    self.save("saveArchivedGame", vec![game]).await;
                }
                archive::Effect::Remove(id) => {
                    self.games.retain(|kept| kept["id"] != json!(id));
                    self.save("removeArchivedGame", vec![json!(id)]).await;
                }
            }
        }
    }

    fn archive_host(&self) -> (Value, bool) {
        let source = match &self.game {
            Game::Computer(game) => game.archive_snapshot(&self.host()),
            Game::Local(game) => game.archive_snapshot(self.now()),
        };
        let has_play = !self.moves().is_empty() || self.result().is_some();
        (source, has_play)
    }

    /// The session and its Game history entry, saved as the app saves them.
    async fn persist(&mut self) {
        let snapshot = match &self.game {
            Game::Computer(game) => game.snapshot(&self.host()),
            Game::Local(game) => game.snapshot(self.now()),
        };
        self.save("saveSession", vec![json!(self.kind()), snapshot])
            .await;
        let (source, has_play) = self.archive_host();
        let finished = self.over();
        let games = self.games.clone();
        let host = archive::Host {
            source,
            has_play,
            games: &games,
            now: wall_now(),
        };
        match self.history.save(finished, &host) {
            Ok(effects) => self.written(effects).await,
            Err(cause) => eprintln!("[cli] Game history: {cause}"),
        }
        let identity = self.history.identity();
        self.save("saveSession", vec![json!(self.archive_kind()), identity])
            .await;
    }

    /// A new game gets its own Game history entry.
    async fn reset_history(&mut self) {
        let (source, has_play) = self.archive_host();
        let games = self.games.clone();
        let host = archive::Host {
            source,
            has_play,
            games: &games,
            now: wall_now(),
        };
        match self.history.reset(&host) {
            Ok(effects) => self.written(effects).await,
            Err(cause) => eprintln!("[cli] Game history: {cause}"),
        }
    }

    /// Carry out a computer game's effects; true when the engine should search next.
    async fn computer_effects(&mut self, effects: Vec<computer::Effect>) -> bool {
        let mut turn = false;
        for effect in effects {
            match effect {
                computer::Effect::StopEngine => {
                    let _ = self.core.call("stopEngine", vec![]).await;
                }
                computer::Effect::Moved { san, computer } => {
                    if computer {
                        println!("Stockfish: {san}");
                    }
                    self.persist().await;
                }
                computer::Effect::Flagged => self.persist().await,
                computer::Effect::ComputerTurn => turn = true,
                computer::Effect::Failed => eprintln!("Engine: the last engine operation failed."),
            }
        }
        turn
    }

    /// The engine's move, when it is its turn; repeated while the rules ask for another search.
    async fn computer_turn(&mut self, events: &mut UnboundedReceiver<Event>) {
        loop {
            let host = self.host();
            let Game::Computer(game) = &mut self.game else {
                return;
            };
            let request: SearchRequest = match game.search_start(&host) {
                Ok(Some(request)) => request,
                Ok(None) => return,
                Err(cause) => {
                    eprintln!("Engine: {cause}");
                    return;
                }
            };
            let search = self.core.call(
                "bestMove",
                vec![
                    json!(request.moves),
                    json!(request.level),
                    request.options.clone(),
                ],
            );
            let reply = answering(self.core, events, search).await;
            let host = self.host();
            let Game::Computer(game) = &mut self.game else {
                return;
            };
            let effects = match reply {
                Ok(reply) => game.search_reply(&request, reply.as_str().unwrap_or_default(), &host),
                Err(cause) => {
                    eprintln!("Engine: {cause}");
                    game.search_failed(&request, &host)
                }
            };
            match effects {
                Ok(effects) => {
                    if !self.computer_effects(effects).await {
                        return;
                    }
                }
                Err(cause) => {
                    eprintln!("Engine: {cause}");
                    return;
                }
            }
        }
    }

    /// One typed command; false when it was not understood or not legal.
    async fn command(&mut self, command: &str) -> Result<bool, String> {
        let host = self.host();
        let now = self.now();
        match &mut self.game {
            Game::Computer(game) => {
                let effects = match command {
                    "undo" => game.takeback(&host)?,
                    "resign" => game.resign(&host)?,
                    "new" => {
                        self.reset_history().await;
                        let Game::Computer(game) = &mut self.game else {
                            return Ok(false);
                        };
                        game.start(None, None, &host)?
                    }
                    uci => {
                        let (legal, effects) = game.play(uci, &host)?;
                        if !legal {
                            return Ok(false);
                        }
                        effects
                    }
                };
                self.computer_effects(effects).await;
            }
            Game::Local(game) => match command {
                "undo" => {
                    game.takeback(now)?;
                }
                "resign" => {
                    let fen = self.fen();
                    let turn = if fen.split_whitespace().nth(1) == Some("b") {
                        "black"
                    } else {
                        "white"
                    };
                    let Game::Local(game) = &mut self.game else {
                        return Ok(false);
                    };
                    game.resign(turn, now)?;
                }
                "draw" => {
                    game.agree_draw(now)?;
                }
                "pause" => {
                    game.toggle_pause(now)?;
                }
                "new" => {
                    self.reset_history().await;
                    let Game::Local(game) = &mut self.game else {
                        return Ok(false);
                    };
                    let (setup, clock) = (game.state["setup"].clone(), game.state["clock"].clone());
                    game.start(setup, clock, now)?;
                }
                uci => {
                    let (san, _) = game.play(uci, now)?;
                    if san.is_none() {
                        return Ok(false);
                    }
                }
            },
        }
        Ok(true)
    }

    /// The clock ran out: true when a flag fell.
    async fn expire(&mut self) -> bool {
        let host = self.host();
        let now = self.now();
        match &mut self.game {
            Game::Computer(game) => match game.expire(&host) {
                Ok(effects) => {
                    let fell = effects.contains(&computer::Effect::Flagged);
                    self.computer_effects(effects).await;
                    fell
                }
                Err(_) => false,
            },
            Game::Local(game) => game.expire(now).is_ok_and(|flagged| flagged.0),
        }
    }
}

/// Milliseconds since the epoch (`Date.now()`).
fn wall_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0)
}

pub async fn interactive(
    core: &CoreClient,
    events: &mut UnboundedReceiver<Event>,
    computer: bool,
    level: Option<String>,
) -> Result<(), String> {
    let library = core.call("library", vec![]).await?;
    let sessions = &library["sessions"];
    let started = Instant::now();
    let game = if computer {
        let mut saved = sessions["computer"].clone();
        if let Some(level) = level {
            let checked =
                kchess_domain::misc::validate_arguments("bestMove", &[json!([]), json!(level)]);
            if !matches!(checked, Some(Ok(()))) {
                return Err(format!("Unknown level {level}."));
            }
            if !saved.is_object() {
                saved = json!({});
            }
            saved["level"] = json!(level);
        }
        // The core still refuses engine help during a live game on every search.
        core.call("resumeOnline", vec![]).await?;
        let engine = core.call("engineStatus", vec![]).await?;
        if engine["ready"] != json!(true) {
            return Err("Stockfish is unavailable. Install an engine or pass --engine.".into());
        }
        let host = computer::Host {
            now: 0.0,
            allowed: true,
            ready: true,
        };
        Game::Computer(ComputerGame::new(
            Some(&saved).filter(|s| s.is_object()),
            &host,
        )?)
    } else {
        Game::Local(LocalGame::new(
            Some(&sessions["local"]).filter(|s| s.is_object()),
            0.0,
        )?)
    };
    let archive_kind = if computer {
        "archive:computer"
    } else {
        "archive:board"
    };
    let games: Vec<Value> = library["games"].as_array().cloned().unwrap_or_default();
    let mut session = Session {
        core,
        game,
        history: GameArchive::new(
            Some(&sessions[archive_kind]).filter(|s| s.is_object()),
            true,
            &archive::Host {
                source: Value::Null,
                has_play: false,
                games: &games,
                now: wall_now(),
            },
        )?
        .0,
        games,
        started,
        failed: false,
    };
    session.show();
    session.computer_turn(events).await;

    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let mut ticks = tokio::time::interval(std::time::Duration::from_millis(100));
    let interrupted = tokio::signal::ctrl_c();
    tokio::pin!(interrupted);
    loop {
        tokio::select! {
            _ = &mut interrupted => break,
            _ = ticks.tick() => {
                if session.expire().await {
                    session.persist().await;
                    session.show();
                }
            }
            Some(event) = events.recv() => {
                if event.name == "host:request" {
                    core.answer(&event.payload).await;
                } else if event.name == "core:closed" {
                    break;
                }
            }
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { break };
                let command = line.trim();
                match command {
                    "" => continue,
                    "quit" => break,
                    "status" => {}
                    command => match session.command(command).await {
                        Ok(true) => {}
                        Ok(false) => eprintln!("Illegal move or game unavailable."),
                        Err(cause) => eprintln!("{cause}"),
                    },
                }
                session.computer_turn(events).await;
                session.persist().await;
                session.show();
            }
        }
    }
    if let Game::Computer(_) = session.game {
        let _ = core.call("stopEngine", vec![]).await;
    }
    session.persist().await;
    if session.failed {
        return Err("Some changes could not be saved.".into());
    }
    Ok(())
}
