//! `play` and `local`: a game against Stockfish, or two people at one terminal, with moves in SAN
//! or UCI.
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

    /// A typed move as UCI: SAN ("Nf3", "O-O", "exd8=Q") is read in the position on the board;
    /// UCI, and anything that is not a legal move, are passed on as typed for the rules to judge.
    fn typed_uci(&self, text: &str) -> String {
        let bytes = text.as_bytes();
        let is_uci = bytes.len() >= 4
            && bytes[0].is_ascii_lowercase()
            && bytes[1].is_ascii_digit()
            && bytes[2].is_ascii_lowercase()
            && bytes[3].is_ascii_digit();
        if is_uci {
            return text.to_lowercase();
        }
        let variant = self.state()["setup"]["variant"]
            .as_str()
            .unwrap_or("standard");
        kchess_domain::rules::setup_start(variant, &self.fen())
            .and_then(|position| kchess_domain::position::play_san(&position, text))
            .map_or_else(|| text.to_string(), |played| played.uci)
    }

    /// The game as PGN, with its result, as the app's Copy PGN writes it.
    fn pgn(&self) -> Result<String, String> {
        let result = match self.result() {
            None => "*",
            Some(result) => match result["winner"].as_str() {
                Some("white") => "1-0",
                Some("black") => "0-1",
                _ => "1/2-1/2",
            },
        };
        let (white, black) = match &self.game {
            Game::Local(_) => ("White".to_string(), "Black".to_string()),
            Game::Computer(_) => {
                let player = if self.state()["color"] == json!("black") {
                    "black"
                } else {
                    "white"
                };
                let engine = "Stockfish".to_string();
                if player == "white" {
                    ("You".to_string(), engine)
                } else {
                    (engine, "You".to_string())
                }
            }
        };
        let headers = json!({
            "Event": if self.kind() == "computer" { "Computer game (KChess)" } else { "Over the board (KChess)" },
            "White": white,
            "Black": black,
            "Result": result,
        });
        kchess_games::rules(
            "setupPgn",
            &[self.state()["setup"].clone(), json!(self.moves()), headers],
        )
        .map_err(|cause| cause.to_string())?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "The game could not be written as PGN.".to_string())
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

    /// Play as `color` (`white`, `black` or `random`): a new game, as the app's New game starts
    /// one, unless the game on the board is still being played.
    async fn choose_color(&mut self, color: &str) -> Result<(), String> {
        let color = match color {
            "random" if kchess_games::random_unit() < 0.5 => "white",
            "random" => "black",
            chosen => chosen,
        };
        // The game on the board is already that: kept, unless it is over.
        if self.state()["color"].as_str().unwrap_or("white") == color && !self.over() {
            return Ok(());
        }
        if !self.moves().is_empty() && !self.over() {
            return Err(
                "A game is in progress: finish it, or type new, before choosing a colour.".into(),
            );
        }
        self.reset_history().await;
        let host = self.host();
        let Game::Computer(game) = &mut self.game else {
            return Ok(());
        };
        game.state["color"] = json!(color);
        let effects = game.start(None, None, &host)?;
        self.computer_effects(effects).await;
        self.persist().await;
        Ok(())
    }

    /// Start a game over the board in `variant`, as the app's New game does: Chess960 from a
    /// random position, the others from their usual start, the clock as it was.
    async fn choose_variant(&mut self, variant: &str) -> Result<(), String> {
        let now = self.now();
        if self.state()["setup"]["variant"]
            .as_str()
            .unwrap_or("standard")
            == variant
            && !self.over()
        {
            return Ok(());
        }
        if !self.moves().is_empty() && !self.over() {
            return Err(
                "A game is in progress: finish it, or type new, before choosing a variant.".into(),
            );
        }
        let fen = if variant == "chess960" {
            json!(kchess_domain::rules::chess960_fen(
                (kchess_games::random_unit() * 960.0) as i64
            ))
        } else {
            kchess_games::rules("defaultFen", &[json!(variant)])
                .map_err(|cause| cause.to_string())?
        };
        self.reset_history().await;
        let Game::Local(game) = &mut self.game else {
            return Ok(());
        };
        let clock = game.state["clock"].clone();
        game.start(json!({ "variant": variant, "fen": fen }), clock, now)?;
        self.persist().await;
        Ok(())
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
        let typed = self.typed_uci(command);
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
                    _ => {
                        let (legal, effects) = game.play(&typed, &host)?;
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
                _ => {
                    let (san, _) = game.play(&typed, now)?;
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

/// `play`'s level and colour, in either order: `play 5 black`, `play black`.
pub fn choices(words: &[String]) -> Result<(Option<String>, Option<String>), String> {
    let (mut level, mut color) = (None, None);
    for word in words {
        match word.as_str() {
            "white" | "black" | "random" if color.is_none() => color = Some(word.clone()),
            _ if level.is_none() => level = Some(word.clone()),
            other => return Err(format!("Unexpected argument {other}.")),
        }
    }
    Ok((level, color))
}

/// `local [variant]`: a variant as Lichess keys it, in any case (`crazyhouse`, `kingOfTheHill`).
pub fn local_variant(words: &[String]) -> Result<Option<String>, String> {
    match words {
        [] => Ok(None),
        [word] => kchess_domain::misc::constants::VARIANTS
            .iter()
            .find(|key| key.eq_ignore_ascii_case(word))
            .map(|key| Some((*key).to_string()))
            .ok_or_else(|| {
                format!(
                    "Unknown variant {word}. Choose one of {}.",
                    kchess_domain::misc::constants::VARIANTS.join(", ")
                )
            }),
        [_, other, ..] => Err(format!("Unexpected argument {other}.")),
    }
}

pub async fn interactive(
    core: &CoreClient,
    events: &mut UnboundedReceiver<Event>,
    computer: bool,
    level: Option<String>,
    color: Option<String>,
    variant: Option<String>,
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
    if let Some(color) = color.filter(|_| computer) {
        session.choose_color(&color).await?;
    }
    if let Some(variant) = variant.filter(|_| !computer) {
        session.choose_variant(&variant).await?;
    }
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
                    "pgn" => {
                        match session.pgn() {
                            Ok(pgn) => println!("{}", json!({ "pgn": pgn })),
                            Err(cause) => eprintln!("{cause}"),
                        }
                        continue;
                    }
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

#[cfg(test)]
mod tests {
    use super::choices;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_string()).collect()
    }

    #[test]
    fn local_takes_a_variant_in_any_case() {
        use super::local_variant;
        assert_eq!(local_variant(&[]), Ok(None));
        assert_eq!(
            local_variant(&words(&["CrazyHouse"])),
            Ok(Some("crazyhouse".into()))
        );
        assert_eq!(
            local_variant(&words(&["kingofthehill"])),
            Ok(Some("kingOfTheHill".into()))
        );
        assert!(local_variant(&words(&["bughouse"])).is_err());
        assert!(local_variant(&words(&["atomic", "horde"])).is_err());
    }

    #[test]
    fn play_takes_a_level_and_a_colour_in_either_order() {
        let some = |text: &str| Some(text.to_string());
        assert_eq!(choices(&words(&[])), Ok((None, None)));
        assert_eq!(choices(&words(&["club"])), Ok((some("club"), None)));
        assert_eq!(choices(&words(&["black"])), Ok((None, some("black"))));
        assert_eq!(
            choices(&words(&["random", "club"])),
            Ok((some("club"), some("random")))
        );
        assert!(choices(&words(&["club", "black", "extra"])).is_err());
    }
}
