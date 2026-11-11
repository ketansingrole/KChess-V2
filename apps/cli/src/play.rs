//! `play` and `local`: a game against Stockfish, or two people at one terminal, with moves in SAN
//! or UCI.
//! The rules run both (`kchess-games`); the session and Game history entry are saved in the
//! core's library as the app saves them, so either can carry the game on.

use kchess_contracts::calls::{self, CoreCall};
use kchess_contracts::check::{ArchivedGame, LibraryId, Moves, SessionObject};
use kchess_contracts::types::{
    ComputerSession, EngineLevel, HistoryKind, PlayedGame, SessionDocument, SessionKind,
};
use std::time::{Duration, Instant};

use kchess_domain::games::Outcome;
use kchess_domain::games::archive::Snapshot;
use kchess_domain::games::computer::{self, ComputerGame, SearchRequest};
use kchess_domain::games::local::{self, LocalGame};
use kchess_domain::games::{Host, Setup};
use kchess_domain::variant::Variant;
use kchess_games::history::{self, HistoryPlace, Write};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::core::{ClientEvent, CoreClient};
use crate::wait::{Next, Wait};

enum Game {
    Computer(ComputerGame),
    Local(LocalGame),
}

struct Session<'a> {
    core: &'a CoreClient,
    game: Game,
    history: HistoryPlace,
    games: Vec<PlayedGame>,
    started: Instant,
    failed: bool,
}

impl Session<'_> {
    /// The games' clock: milliseconds since the session started.
    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    fn host(&self) -> Host {
        Host {
            now: self.now(),
            allowed: true,
            ready: true,
        }
    }

    fn setup(&self) -> &Setup {
        match &self.game {
            Game::Computer(game) => &game.state().setup,
            Game::Local(game) => &game.state().setup,
        }
    }

    fn moves(&self) -> &[String] {
        match &self.game {
            Game::Computer(game) => game.moves(),
            Game::Local(game) => game.moves(),
        }
    }

    /// The result once the game has ended, as the status line shows it.
    fn result(&self) -> Option<Value> {
        match &self.game {
            Game::Computer(game) => game.result().map(|result| json!(result)),
            Game::Local(game) => game.result().map(|result| json!(result)),
        }
    }

    fn over(&self) -> bool {
        match &self.game {
            Game::Computer(game) => game.over(),
            Game::Local(game) => game.over(),
        }
    }

    fn fen(&self) -> String {
        let setup = self.setup();
        kchess_domain::replay::replay_setup(setup.variant, &setup.fen, self.moves())
            .map(|replay| replay.fen)
            .unwrap_or_else(|| setup.fen.clone())
    }

    /// A typed move as UCI: SAN ("Nf3", "O-O", "exd8=Q") or UCI read in the position on the
    /// board; anything that is not a legal move there is passed on as typed for the rules to
    /// judge.
    fn typed_uci(&self, text: &str) -> String {
        kchess_games::typed_uci(self.setup().variant, &self.fen(), text)
            .unwrap_or_else(|| text.to_string())
    }

    fn outcome(&self) -> Option<Outcome> {
        match &self.game {
            Game::Computer(game) => game.outcome(),
            Game::Local(game) => game.outcome(),
        }
    }

    /// The game as PGN, with its result, as the app's Copy PGN writes it.
    fn pgn(&self) -> Result<String, String> {
        let result = Outcome::tag(self.outcome());
        let (white, black, event) = match &self.game {
            Game::Local(_) => ("White", "Black", local::EVENT),
            Game::Computer(game) => {
                let (white, black) = computer::players(game.color(), "Stockfish");
                (white, black, computer::EVENT)
            }
        };
        let setup = self.setup();
        let headers = [
            ("Black", black),
            ("Event", event),
            ("Result", result),
            ("White", white),
        ];
        kchess_domain::pgn::setup_pgn(setup.variant, &setup.fen, self.moves(), &headers)
    }

    fn show(&self) {
        crate::print(json!({ "fen": self.fen(), "moves": self.moves(), "result": self.result() }));
    }

    async fn save<C: CoreCall>(&mut self, call: C) {
        if let Err(cause) = self.core.request(call).await {
            eprintln!("[cli] Saving failed: {cause}");
            self.failed = true;
        }
    }

    async fn save_session(&mut self, kind: SessionKind, session: SessionDocument) {
        self.save(calls::SaveSession {
            kind,
            session: SessionObject::of(&session),
        })
        .await;
    }

    /// Carry out what Game history asks, keeping the played games as they leave them.
    async fn written(&mut self, writes: Vec<Write>) {
        history::apply_to(&mut self.games, &writes);
        for write in writes {
            match write {
                Write::SaveGame(game) => {
                    self.save(calls::SaveArchivedGame {
                        game: ArchivedGame::of(&game),
                    })
                    .await
                }
                Write::RemoveGame(id) => {
                    self.save(calls::RemoveArchivedGame { id: LibraryId(id) })
                        .await
                }
                Write::SaveSession { kind, identity } => {
                    self.save_session(kind, SessionDocument::Archive(identity))
                        .await
                }
            }
        }
    }

    /// The game as Game history keeps it and whether it has play; none, said, when it cannot be
    /// described.
    fn archive_host(&self) -> Option<(Snapshot, bool)> {
        let source = match &self.game {
            Game::Computer(game) => game.archive_snapshot(),
            Game::Local(game) => Ok(game.archive_snapshot()),
        };
        let has_play = !self.moves().is_empty() || self.result().is_some();
        match source {
            Ok(source) => Some((source, has_play)),
            Err(cause) => {
                eprintln!("[cli] Game history: {cause}");
                None
            }
        }
    }

    /// The session and its Game history entry, saved as the app saves them.
    async fn persist(&mut self) {
        let (kind, session) = match &self.game {
            Game::Computer(game) => (
                SessionKind::Computer,
                SessionDocument::Computer(game.session(self.host().now)),
            ),
            Game::Local(game) => (
                SessionKind::Local,
                SessionDocument::Local(game.session(self.now())),
            ),
        };
        self.save_session(kind, session).await;
        let Some((source, has_play)) = self.archive_host() else {
            return;
        };
        let finished = self.over();
        let writes = self
            .history
            .save(finished, Some(source), has_play, &self.games);
        self.written(writes).await;
    }

    /// Play as `color` (`white`, `black` or `random`): a new game, as the app's New game starts
    /// one, unless the game on the board is still being played.
    async fn choose_color(&mut self, color: &str) -> Result<(), String> {
        let color = kchess_games::draw_color(color)?;
        let Game::Computer(game) = &self.game else {
            return Ok(());
        };
        // The game on the board is already that: kept, unless it is over.
        if game.color() == color && !self.over() {
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
        game.set_color(color);
        let effects = game.start(None, None, &host)?;
        self.computer_effects(effects).await;
        self.persist().await;
        Ok(())
    }

    /// Start a game in `variant` from `fen`, else as the app's New game does: Chess960 from a
    /// random position, the others from their usual start, the clock as it was.
    async fn choose_setup(&mut self, variant: Variant, fen: Option<&str>) -> Result<(), String> {
        let now = self.now();
        if fen.is_none() && self.setup().variant == variant && !self.over() {
            return Ok(());
        }
        if !self.moves().is_empty() && !self.over() {
            return Err(
                "A game is in progress: finish it, or type new, before choosing a variant.".into(),
            );
        }
        let fen = if let Some(fen) = fen {
            playable_fen(variant, fen)?
        } else if variant == Variant::Chess960 {
            kchess_domain::rules::chess960_fen(kchess_games::random_chess960()?)
        } else {
            variant.default_fen()
        };
        self.reset_history().await;
        let setup = Setup { variant, fen };
        let host = self.host();
        match &mut self.game {
            Game::Local(game) => {
                let clock = game.clock();
                game.start(setup, clock, now)?;
            }
            Game::Computer(game) => {
                game.start(Some(setup), None, &host)?;
            }
        }
        self.persist().await;
        Ok(())
    }

    /// A new game gets its own Game history entry.
    async fn reset_history(&mut self) {
        let Some((source, has_play)) = self.archive_host() else {
            return;
        };
        let writes = self.history.reset(Some(source), has_play, &self.games);
        self.written(writes).await;
    }

    /// Carry out a computer game's effects; true when the engine should search next.
    async fn computer_effects(&mut self, effects: Vec<computer::Effect>) -> bool {
        let mut turn = false;
        for effect in effects {
            match effect {
                computer::Effect::StopEngine => {
                    crate::told_if_failed(
                        "Stopping the engine",
                        self.core.request(calls::StopEngine {}).await,
                    );
                }
                computer::Effect::Moved { san, computer } => {
                    if computer {
                        crate::print(format_args!("Stockfish: {san}"));
                    }
                    self.persist().await;
                }
                computer::Effect::Flagged => self.persist().await,
                computer::Effect::ComputerTurn => turn = true,
                computer::Effect::Failed => eprintln!("Engine: the last engine operation failed."),
                computer::Effect::Cancelled => {}
            }
        }
        turn
    }

    /// The engine's move, when it is its turn; repeated while the rules ask for another search.
    async fn computer_turn(&mut self) {
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
            let Some(level) = EngineLevel::from_key(&request.level) else {
                eprintln!("Engine: Invalid engine level.");
                return;
            };
            let search = self.core.request(calls::BestMove {
                moves: Moves(request.moves.clone()),
                level,
                options: Some((&request.options).into()),
            });
            let reply = search.await;
            let host = self.host();
            let Game::Computer(game) = &mut self.game else {
                return;
            };
            let effects = match reply {
                Ok(reply) => game.search_reply(&request, &reply, &host),
                Err(cause) => {
                    eprintln!("Engine: {cause}");
                    game.search_failed(&request)
                }
            };
            if !self.computer_effects(effects).await {
                return;
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
                    "undo" => game.takeback(&host),
                    "resign" => game.resign(),
                    "new" => {
                        self.reset_history().await;
                        let Game::Computer(game) = &mut self.game else {
                            return Ok(false);
                        };
                        game.start(None, None, &host)?
                    }
                    _ => {
                        let (legal, effects) = game.play(&typed, &host);
                        if !legal {
                            return Ok(false);
                        }
                        effects
                    }
                };
                self.computer_effects(effects).await;
            }
            Game::Local(game) => match command {
                "undo" => game.takeback(now),
                "resign" => {
                    let fen = self.fen();
                    let turn = if fen.split_whitespace().nth(1) == Some("b") {
                        kchess_domain::color::Color::Black
                    } else {
                        kchess_domain::color::Color::White
                    };
                    let Game::Local(game) = &mut self.game else {
                        return Ok(false);
                    };
                    game.resign(turn);
                }
                "draw" => game.agree_draw(),
                "pause" => {
                    game.toggle_pause(now);
                }
                "new" => {
                    self.reset_history().await;
                    let Game::Local(game) = &mut self.game else {
                        return Ok(false);
                    };
                    let (setup, clock) = (game.state().setup.clone(), game.clock());
                    game.start(setup, clock, now)?;
                }
                _ => {
                    let (san, _) = game.play(&typed, now);
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
            Game::Computer(game) => {
                let effects = game.expire(host.now);
                let fell = effects.contains(&computer::Effect::Flagged);
                self.computer_effects(effects).await;
                fell
            }
            Game::Local(game) => game.expire(now),
        }
    }
}

/// What `play` was given: a level, a colour and a variant.
pub type Choices = (Option<String>, Option<String>, Option<Variant>);

/// The variant a word names by its Lichess key, in any case (`crazyhouse`, `KingOfTheHill`).
fn variant_named(word: &str) -> Option<Variant> {
    Variant::ALL
        .into_iter()
        .find(|variant| variant.key().eq_ignore_ascii_case(word))
}

/// `play`'s level, colour and variant, in any order: `play club black`, `play black crazyhouse`.
pub fn choices(words: &[String]) -> Result<Choices, String> {
    let (mut level, mut color, mut variant) = (None, None, None);
    for word in words {
        match (word.as_str(), variant_named(word)) {
            ("white" | "black" | "random", _) if color.is_none() => color = Some(word.clone()),
            (_, Some(named)) if variant.is_none() => variant = Some(named),
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
fn playable_fen(variant: Variant, fen: &str) -> Result<String, String> {
    let fen = fen.trim();
    let playable = if variant.standard_rules() {
        kchess_domain::position::fen_problem(fen).is_none()
    } else {
        kchess_domain::rules::setup_start(variant, fen).is_some()
    };
    if playable {
        Ok(fen.to_string())
    } else {
        Err(format!("Not a playable position: {fen}"))
    }
}

/// `local [variant]`.
pub fn local_variant(words: &[String]) -> Result<Option<Variant>, String> {
    match words {
        [] => Ok(None),
        [word] => variant_named(word).map(Some).ok_or_else(|| {
            format!(
                "Unknown variant {word}. Choose one of {}.",
                kchess_domain::variant::VARIANTS.join(", ")
            )
        }),
        [_, other, ..] => Err(format!("Unexpected argument {other}.")),
    }
}

/// A game against the computer, from its saved session and the level and variant asked for.
async fn computer_game(
    core: &CoreClient,
    saved: Option<ComputerSession>,
    level: Option<String>,
    variant: Option<Variant>,
) -> Result<Game, String> {
    let mut saved: Option<computer::Saved> = saved.map(computer::Saved::from);
    if let Some(level) = level {
        if EngineLevel::from_key(&level).is_none() {
            return Err(format!("Unknown level {level}."));
        }
        saved.get_or_insert_default().level = Some(level);
    }

    // The core still refuses engine help during a live game on every search.
    core.request(calls::ResumeOnline {}).await?;
    // Fairy-Stockfish plays the other variants; the core says so if it is missing.
    let fairy = variant.is_some_and(|variant| !variant.standard_rules());
    let engine = core.request(calls::EngineStatus { variant: None }).await?;
    if !fairy && !engine.ready {
        return Err("Stockfish is unavailable. Install an engine or pass --engine.".into());
    }
    let host = Host {
        now: 0.0,
        allowed: true,
        ready: true,
    };
    Ok(Game::Computer(ComputerGame::resume(saved, &host)?))
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
                    Ok(pgn) => crate::print(json!({ "pgn": pgn })),
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
    async fn play(&mut self, events: &mut UnboundedReceiver<ClientEvent>) {
        let mut wait = Wait::new(events)
            .typed()
            .ticking(Duration::from_millis(100));
        loop {
            match wait.next().await {
                Next::Tick => {
                    if self.expire().await {
                        self.persist().await;
                        self.show();
                    }
                }
                Next::Line(line) => {
                    match self.typed(&line).await {
                        Typed::Quit => break,
                        Typed::Skip => continue,
                        Typed::Refresh => {}
                    }
                    self.computer_turn().await;
                    self.persist().await;
                    self.show();
                }
                Next::Event(_) => {}
                Next::Interrupted | Next::Closed | Next::InputEnded => break,
            }
        }
    }
}

pub async fn interactive(
    core: &CoreClient,
    events: &mut UnboundedReceiver<ClientEvent>,
    computer: bool,
    level: Option<String>,
    color: Option<String>,
    variant: Option<Variant>,
    fen: Option<String>,
) -> Result<(), String> {
    let library = core.request(calls::Library {}).await?;
    let mut sessions = library.sessions;
    let started = Instant::now();
    let game = if computer {
        computer_game(core, sessions.computer.take(), level, variant).await?
    } else {
        Game::Local(LocalGame::resume(
            sessions.local.take().map(local::Saved::from),
            0.0,
        )?)
    };
    let kind = if computer {
        HistoryKind::Computer
    } else {
        HistoryKind::Board
    };
    let identity = sessions.take_identity(kind);
    let games = library.games;
    let (history, writes) = HistoryPlace::new(kind, identity, true, None, false, &games);
    let mut session = Session {
        core,
        game,
        history,
        games,
        started,
        failed: false,
    };
    session.written(writes).await;
    if let Some(color) = color.filter(|_| computer) {
        session.choose_color(&color).await?;
    }
    if variant.is_some() || fen.is_some() {
        let variant = variant.unwrap_or(Variant::Standard);
        session.choose_setup(variant, fen.as_deref()).await?;
    }
    session.show();
    session.computer_turn().await;

    session.play(events).await;
    if let Game::Computer(_) = session.game {
        crate::told_if_failed(
            "Stopping the engine",
            core.request(calls::StopEngine {}).await,
        );
    }
    session.persist().await;
    if session.failed {
        return Err("Some changes could not be saved.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Variant, choices};

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_string()).collect()
    }

    #[test]
    fn local_takes_a_variant_in_any_case() {
        use super::local_variant;
        assert_eq!(local_variant(&[]), Ok(None));
        assert_eq!(
            local_variant(&words(&["CrazyHouse"])),
            Ok(Some(Variant::Crazyhouse))
        );
        assert_eq!(
            local_variant(&words(&["kingofthehill"])),
            Ok(Some(Variant::KingOfTheHill))
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
        assert_eq!(playable_fen(Variant::Standard, fen).as_deref(), Ok(fen));
        assert!(playable_fen(Variant::Standard, "8/8/8/8/8/8/8/8 w - - 0 1").is_err());
        let zh = "rnb1kbnr/ppp1pppp/8/3q4/8/8/PPPP1PPP/RNBQKBNR[Pp] w KQkq - 0 3";
        assert!(playable_fen(Variant::Crazyhouse, zh).is_ok());
        assert!(playable_fen(Variant::Horde, zh).is_err());
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
            Ok((some("club"), some("black"), Some(Variant::Crazyhouse)))
        );
        assert!(choices(&words(&["club", "black", "extra"])).is_err());
        // A second colour is no colour: it is read as the level, which `play` then refuses.
        assert_eq!(
            choices(&words(&["white", "black"])),
            Ok((some("black"), some("white"), None))
        );
        assert!(choices(&words(&["atomic", "horde"])).is_err());
    }
}
