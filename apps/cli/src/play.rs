//! `play` and `local`: a game against Stockfish, or two people at one terminal, with moves in SAN
//! or UCI.
//! The rules run both (`kchess-games`); the session and Game history entry are saved in the
//! core's library as the app saves them, so either can carry the game on.

use kchess_contracts::calls;
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
        let result = match &self.game {
            Game::Computer(game) => computer_result_tag(game, &self.host()),
            Game::Local(game) => local_result_tag(game, self.now()),
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
        let event = if self.kind() == "computer" {
            "Computer game (KChess)"
        } else {
            "Over the board (KChess)"
        };
        let setup = &self.state()["setup"];
        let (Some(variant), Some(fen)) = (setup["variant"].as_str(), setup["fen"].as_str()) else {
            return Err("The game could not be written as PGN.".to_string());
        };
        let headers = [
            ("Black", black.as_str()),
            ("Event", event),
            ("Result", &result),
            ("White", white.as_str()),
        ];
        Ok(kchess_domain::pgn::setup_pgn(
            variant,
            fen,
            &self.moves(),
            &headers,
        ))
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
    /// Start a game in `variant`, from its usual start or from `fen`.
    async fn choose_setup(&mut self, variant: &str, fen: Option<&str>) -> Result<(), String> {
        let now = self.now();
        if fen.is_none()
            && self.state()["setup"]["variant"]
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
        let fen = if let Some(fen) = fen {
            playable_fen(variant, fen)?
        } else if variant == "chess960" {
            json!(kchess_domain::rules::chess960_fen(
                (kchess_games::random_unit() * 960.0) as i64
            ))
        } else {
            json!(kchess_domain::rules::default_fen(variant).ok_or("variant")?)
        };
        self.reset_history().await;
        let setup = json!({ "variant": variant, "fen": fen });
        let host = self.host();
        match &mut self.game {
            Game::Local(game) => {
                let clock = game.state["clock"].clone();
                game.start(setup, clock, now)?;
            }
            // Fairy-Stockfish plays the other variants.
            Game::Computer(game) => {
                let clock = game.state["clock"].clone();
                game.start(Some(setup), Some(clock), &host)?;
            }
        }
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
                    let _ = self.core.request(calls::StopEngine {}).await;
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
            let search = self.core.request(calls::BestMove {
                moves: json!(request.moves),
                level: json!(request.level),
                options: Some(request.options.clone()),
            });
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

/// What `play` was given: a level, a colour and a variant.
pub type Choices = (Option<String>, Option<String>, Option<String>);

/// `play`'s level, colour and variant, in any order: `play 5 black`, `play black crazyhouse`.
pub fn choices(words: &[String]) -> Result<Choices, String> {
    let (mut level, mut color, mut variant) = (None, None, None);
    for word in words {
        let known = kchess_domain::misc::constants::VARIANTS
            .iter()
            .find(|key| key.eq_ignore_ascii_case(word));
        match (word.as_str(), known) {
            ("white" | "black" | "random", _) if color.is_none() => color = Some(word.clone()),
            (_, Some(key)) if variant.is_none() => variant = Some((*key).to_string()),
            (other, Some(_)) => return Err(format!("Unexpected argument {other}.")),
            _ if level.is_none() => level = Some(word.clone()),
            (other, _) => return Err(format!("Unexpected argument {other}.")),
        }
    }
    Ok((level, color, variant))
}

/// `--fen <FEN>` among a command's words, quoted or as its six fields: the position to start
/// from, and the other words.
pub fn take_fen(words: &[String]) -> Result<(Option<String>, Vec<String>), String> {
    let Some(at) = words.iter().position(|word| word == "--fen") else {
        return Ok((None, words.to_vec()));
    };
    let field = |index: usize, word: &str| match index {
        1 => matches!(word, "w" | "b"),
        2 => word
            .chars()
            .all(|c| c == '-' || "KQkqABCDEFGHabcdefgh".contains(c)),
        3 => word == "-" || (word.len() == 2 && word.as_bytes()[1].is_ascii_digit()),
        _ => word.parse::<u32>().is_ok(),
    };
    // The first word is the board (or the whole quoted FEN); each next one a further field.
    let mut fields: Vec<&str> = Vec::new();
    let mut taken = 1;
    for word in &words[at + 1..] {
        if fields.is_empty() {
            fields.extend(word.split_whitespace());
        } else if fields.len() < 6 && field(fields.len(), word) {
            fields.push(word);
        } else {
            break;
        }
        taken += 1;
    }
    if fields.is_empty() {
        return Err("--fen needs a position.".into());
    }
    let mut rest = words.to_vec();
    rest.drain(at..at + taken);
    Ok((Some(fields.join(" ")), rest))
}

/// A position a game can start from in `variant`, as the app's "Play from here" checks it.
fn playable_fen(variant: &str, fen: &str) -> Result<Value, String> {
    let fen = fen.trim();
    let playable = if matches!(variant, "standard" | "chess960") {
        kchess_domain::position::fen_problem(fen).is_none()
    } else {
        kchess_domain::rules::setup_start(variant, fen).is_some()
    };
    if playable {
        Ok(json!(fen))
    } else {
        Err(format!("Not a playable position: {fen}"))
    }
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

/// A game against the computer, from its saved session and the level and variant asked for.
async fn computer_game(
    core: &CoreClient,
    saved: &Value,
    level: Option<String>,
    variant: Option<&str>,
) -> Result<Game, String> {
    let mut saved = saved.clone();
    if let Some(level) = level {
        let checked = kchess_contracts::core_api::check_arguments(
            "bestMove",
            kchess_contracts::core_api::as_checked(vec![json!([]), json!(level)]),
        );
        if !matches!(checked, Some(Ok(_))) {
            return Err(format!("Unknown level {level}."));
        }
        if !saved.is_object() {
            saved = json!({});
        }
        saved["level"] = json!(level);
    }

    // The core still refuses engine help during a live game on every search.
    core.request(calls::ResumeOnline {}).await?;
    // Fairy-Stockfish plays the other variants; the core says so if it is missing.
    let fairy = variant.is_some_and(|variant| !matches!(variant, "standard" | "chess960"));
    let engine = core.request(calls::EngineStatus { variant: None }).await?;
    if !fairy && engine["ready"] != json!(true) {
        return Err("Stockfish is unavailable. Install an engine or pass --engine.".into());
    }
    let host = computer::Host {
        now: 0.0,
        allowed: true,
        ready: true,
    };
    Ok(Game::Computer(ComputerGame::new(
        Some(&saved).filter(|s| s.is_object()),
        &host,
    )?))
}

/// What a typed line asks of the game loop.
enum Typed {
    Quit,
    /// Nothing changed: wait for the next line.
    Skip,
    /// The game may have changed: let the computer reply, save and show it.
    Refresh,
}

impl Session<'_> {
    /// Answers one typed line.
    async fn typed(&mut self, command: &str) -> Typed {
        match command {
            "" => Typed::Skip,
            "quit" => Typed::Quit,
            "status" => Typed::Refresh,
            "pgn" => {
                match self.pgn() {
                    Ok(pgn) => println!("{}", json!({ "pgn": pgn })),
                    Err(cause) => eprintln!("{cause}"),
                }
                Typed::Skip
            }
            command => {
                match self.command(command).await {
                    Ok(true) => {}
                    Ok(false) => eprintln!("Illegal move or game unavailable."),
                    Err(cause) => eprintln!("{cause}"),
                }
                Typed::Refresh
            }
        }
    }

    /// Plays typed lines, clock ticks and core events until the player quits, stdin ends, the
    /// core closes or Ctrl-C.
    async fn play(&mut self, events: &mut UnboundedReceiver<Event>) {
        let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        let mut ticks = tokio::time::interval(std::time::Duration::from_millis(100));
        let interrupted = tokio::signal::ctrl_c();
        tokio::pin!(interrupted);
        loop {
            tokio::select! {
                _ = &mut interrupted => break,
                _ = ticks.tick() => {
                    if self.expire().await {
                        self.persist().await;
                        self.show();
                    }
                }
                Some(event) = events.recv() => {
                    if event.name == "host:request" {
                        self.core.answer(&event.payload).await;
                    } else if event.name == "core:closed" {
                        break;
                    }
                }
                line = lines.next_line() => {
                    let Ok(Some(line)) = line else { break };
                    match self.typed(line.trim()).await {
                        Typed::Quit => break,
                        Typed::Skip => continue,
                        Typed::Refresh => {}
                    }
                    self.computer_turn(events).await;
                    self.persist().await;
                    self.show();
                }
            }
        }
    }
}

pub async fn interactive(
    core: &CoreClient,
    events: &mut UnboundedReceiver<Event>,
    computer: bool,
    level: Option<String>,
    color: Option<String>,
    variant: Option<String>,
    fen: Option<String>,
) -> Result<(), String> {
    let library = core.request(calls::Library {}).await?;
    let sessions = &library["sessions"];
    let started = Instant::now();
    let game = if computer {
        computer_game(core, &sessions["computer"], level, variant.as_deref()).await?
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
    if variant.is_some() || fen.is_some() {
        let variant = variant.as_deref().unwrap_or("standard");
        session.choose_setup(variant, fen.as_deref()).await?;
    }
    session.show();
    session.computer_turn(events).await;

    session.play(events).await;
    if let Game::Computer(_) = session.game {
        let _ = core.request(calls::StopEngine {}).await;
    }
    session.persist().await;
    if session.failed {
        return Err("Some changes could not be saved.".into());
    }
    Ok(())
}

/// A computer game's PGN result, as its Game history entry records it: its result view is from
/// the player's side and names no winner.
fn computer_result_tag(game: &ComputerGame, host: &computer::Host) -> String {
    game.archive_snapshot(host)["result"]
        .as_str()
        .unwrap_or("*")
        .to_string()
}

/// A board game's PGN result, from the winner its result names.
fn local_result_tag(game: &LocalGame, now: f64) -> String {
    match game.result(now) {
        None => "*",
        Some(result) => match result["winner"].as_str() {
            Some("white") => "1-0",
            Some("black") => "0-1",
            _ => "1/2-1/2",
        },
    }
    .to_string()
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

    /// A game starts from a position given quoted or as its six words, among the other choices;
    /// the position must be playable in its variant.
    #[test]
    fn a_game_takes_a_starting_position() {
        use super::{playable_fen, take_fen};
        let fen = "4k3/8/8/8/8/8/4P3/4K3 b - - 0 40";
        assert_eq!(
            take_fen(&words(&["club", "--fen", fen, "black"])),
            Ok((Some(fen.to_string()), words(&["club", "black"])))
        );
        let mut split = vec!["--fen"];
        split.extend(fen.split(' '));
        split.push("crazyhouse");
        assert_eq!(
            take_fen(&words(&split)),
            Ok((Some(fen.to_string()), words(&["crazyhouse"])))
        );
        // A FEN with fewer fields stops at the first word that is not one.
        assert_eq!(
            take_fen(&words(&["--fen", "4k3/8/8/8/8/8/4P3/4K3", "w", "black"])),
            Ok((
                Some("4k3/8/8/8/8/8/4P3/4K3 w".to_string()),
                words(&["black"])
            ))
        );
        assert_eq!(take_fen(&words(&["club"])), Ok((None, words(&["club"]))));
        assert!(take_fen(&words(&["--fen"])).is_err());
        // An en passant square is a file and a rank; six fields are the most a FEN has.
        let board = "4k3/8/8/3pP3/8/8/8/4K3";
        assert_eq!(
            take_fen(&words(&["--fen", board, "w", "-", "d6", "0", "2", "3"])),
            Ok((Some(format!("{board} w - d6 0 2")), words(&["3"])))
        );
        assert_eq!(
            take_fen(&words(&["--fen", board, "w", "-", "d66"])),
            Ok((Some(format!("{board} w -")), words(&["d66"])))
        );
        assert_eq!(playable_fen("standard", fen), Ok(serde_json::json!(fen)));
        assert!(playable_fen("standard", "8/8/8/8/8/8/8/8 w - - 0 1").is_err());
        let zh = "rnb1kbnr/ppp1pppp/8/3q4/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3";
        assert!(playable_fen("crazyhouse", zh).is_ok());
        assert!(playable_fen("horde", zh).is_err());
    }

    #[test]
    fn play_takes_a_level_a_colour_and_a_variant_in_any_order() {
        let some = |text: &str| Some(text.to_string());
        assert_eq!(choices(&words(&[])), Ok((None, None, None)));
        assert_eq!(choices(&words(&["club"])), Ok((some("club"), None, None)));
        assert_eq!(choices(&words(&["black"])), Ok((None, some("black"), None)));
        assert_eq!(
            choices(&words(&["random", "club"])),
            Ok((some("club"), some("random"), None))
        );
        // A variant, in any case and anywhere among them, is played by Fairy-Stockfish.
        assert_eq!(
            choices(&words(&["CrazyHouse", "black", "club"])),
            Ok((some("club"), some("black"), some("crazyhouse")))
        );
        assert!(choices(&words(&["club", "black", "extra"])).is_err());
        // A second colour is no colour: it is read as the level, which `play` then refuses.
        assert_eq!(
            choices(&words(&["white", "black"])),
            Ok((some("black"), some("white"), None))
        );
        assert!(choices(&words(&["atomic", "horde"])).is_err());
    }

    /// A finished computer game is written with its winner, not as a draw: its result view has
    /// no `winner`, so the PGN takes the result its history entry records.
    #[test]
    fn a_resigned_computer_game_exports_as_a_loss() {
        use super::{computer_result_tag, local_result_tag};
        use kchess_games::computer::{ComputerGame, Host};
        use kchess_games::local::LocalGame;
        let host = Host {
            now: 0.,
            allowed: true,
            ready: true,
        };
        let mut game = ComputerGame::new(None, &host).unwrap();
        assert_eq!(computer_result_tag(&game, &host), "*");
        assert!(game.play("e2e4", &host).unwrap().0);
        game.resign(&host).unwrap();
        assert!(game.over(&host));
        assert_eq!(computer_result_tag(&game, &host), "0-1");

        let mut board = LocalGame::new(None, 0.).unwrap();
        assert_eq!(local_result_tag(&board, 0.), "*");
        board.resign("black", 0.).unwrap();
        assert_eq!(local_result_tag(&board, 0.), "1-0");
    }
}
