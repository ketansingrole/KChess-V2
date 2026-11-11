//! `kchess-cli`: the KChess command line, over the same profile and core as the app.

mod core;
mod play;
mod puzzle;
mod review;
mod wait;

use kchess_contracts::calls;
use kchess_contracts::types::PositionLookupKind;
use kchess_domain::variant::Variant;
use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::core::{Browser, CoreClient, CoreEvent, Options};
use crate::wait::{Next, Wait};

/// Declares [`Command`] and [`COMMANDS`] together: each command's name, its other spellings,
/// and its forms with what each does. The help, the
/// suggestion for a mistyped command and `run`'s exhaustive match all read them.
macro_rules! commands {
    ($($variant:ident $name:literal [$($alias:literal),*] {
        $($usage:literal => $about:literal,)+
    })+) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Command { $($variant,)+ }

        const COMMANDS: &[Spec] = &[$(Spec {
            command: Command::$variant,
            name: $name,
            aliases: &[$($alias),*],
            forms: &[$(($usage, $about)),+],
        },)+];
    };
}

struct Spec {
    command: Command,
    name: &'static str,
    aliases: &'static [&'static str],
    forms: &'static [(&'static str, &'static str)],
}

commands! {
    Library "library" [] {
        "" => "Print the library as JSON",
    }
    Status "status" [] {
        "" => "Print account and engine status",
    }
    Login "login" [] {
        "" => "Connect a Lichess account in your browser",
    }
    Call "call" [] {
        "<method> '[arguments]'" => "Invoke any CoreApi method (JSON argument array)",
        "<method> --args-file <path>" => "Read arguments from a JSON file",
    }
    Analyze "analyze" [] {
        "[--variant name] <fen|startpos|moves>" => "Stream engine analysis as JSON lines until Ctrl-C",
    }
    Play "play" [] {
        "[level] [white|black|random] [variant] [--fen FEN]" => "Play Stockfish; type moves in SAN or UCI",
    }
    Local "local" [] {
        "[variant] [--fen FEN]" => "Play over the board (chess960, crazyhouse, atomic…)",
    }
    Puzzle "puzzle" [] {
        "[id|link]" => "Solve the daily puzzle, or that one (hint, solution)",
    }
    Pgn "pgn" [] {
        "<link>" => "Print a Lichess game's PGN, or a public study chapter's",
    }
    Review "review" [] {
        "<link|file|moves>" => "Review a game with the engine: accuracy and faulted moves",
    }
    Explore "explore" [] {
        "[masters|opening|tablebase] [--variant name] <fen|startpos|moves>" => "Look a position up (the tablebase for 7 pieces or fewer)",
    }
    Opening "opening" [] {
        "<fen|startpos|moves>" => "Name the opening a position or line reaches (ECO)",
    }
    Position "position" [] {
        "[--variant name] <fen|startpos|moves>" => "Print whose turn, check or mate, and the legal moves",
    }
    Watch "watch" [] {
        "[channel|link]" => "Stream Lichess TV, a game or a broadcast as JSON lines",
    }
    Methods "methods" [] {
        "" => "List available CoreApi methods",
    }
    Version "version" ["--version", "-V"] {
        "" => "Print the CLI's version (also --version)",
    }
    Help "help" ["--help", "-h"] {
        "" => "Print this help",
    }
}

/// Where a command's description starts in the help.
const COLUMN: usize = 35;

fn help() -> String {
    let mut out =
        String::from("KChess CLI\n\nUsage: kchess-cli [options] <command>\n\nCommands:\n");
    for spec in COMMANDS {
        for (usage, about) in spec.forms {
            let call = format!("  {} {usage}", spec.name);
            let call = call.trim_end();
            if call.chars().count() < COLUMN {
                out.push_str(&format!("{call:COLUMN$}{about}\n"));
            } else {
                out.push_str(&format!("{call}\n{:COLUMN$}{about}\n", ""));
            }
        }
    }
    out.push_str(
        "
Options:
  --data-dir <path>                Use another profile (default: the app's)
  --browser open|print             Open the login page or print its URL
  --no-credentials                 Disable OS credential storage
  --engine <path>                  Use a native Stockfish executable
  --embedded                       Run the core in this process, not the shared one

Interactive game commands: e4 or e2e4, undo, resign, new, status, pgn, quit.
Local games also accept draw and pause. Sessions are saved in the core library.
",
    );
    out
}

fn command(word: &str) -> Option<Command> {
    COMMANDS
        .iter()
        .find(|spec| spec.name == word || spec.aliases.contains(&word))
        .map(|spec| spec.command)
}

/// The most argument text `call` reads.
const MAX_ARGUMENTS: u64 = 2_000_000;

// Compared and printed only by the tests.
#[cfg_attr(test, derive(Debug, PartialEq))]
struct Cli {
    options: Options,
    args: Vec<String>,
    engine: Option<String>,
}

fn parse(argv: &[String]) -> Result<Cli, String> {
    let mut options = Options {
        data_dir: crate::core::default_data_dir(),
        browser: Browser::Open,
        credentials: true,
        embedded: false,
    };
    let mut args = Vec::new();
    let mut engine = None;
    let mut values = argv.iter();
    let value_of = |flag: &str, next: Option<&String>| -> Result<String, String> {
        next.filter(|value| !value.starts_with("--"))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "{flag} requires a {}.",
                    if flag == "--browser" { "mode" } else { "path" }
                )
            })
    };
    while let Some(value) = values.next() {
        match value.as_str() {
            "--" => {}
            "--engine" => engine = Some(value_of("--engine", values.next())?),
            "--data-dir" => {
                options.data_dir =
                    crate::core::absolute(&PathBuf::from(value_of("--data-dir", values.next())?));
            }
            "--browser" => {
                options.browser = match values.next().map(String::as_str) {
                    Some("open") => Browser::Open,
                    Some("print") => Browser::Print,
                    _ => return Err("--browser must be open or print.".into()),
                }
            }
            "--no-credentials" => options.credentials = false,
            "--embedded" => options.embedded = true,
            _ => args.push(value.clone()),
        }
    }
    Ok(Cli {
        options,
        args,
        engine,
    })
}

/// A result on stdout. `println!` panics when the reader has gone (`kchess-cli … | head -1`):
/// a closed pipe ends the output quietly instead.
fn print(value: impl std::fmt::Display) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stdout().lock(), "{value}");
}

/// A step a command takes on its way out, whose failure is told but does not fail the command.
fn told_if_failed<T>(what: &str, result: Result<T, String>) {
    if let Err(cause) = result {
        eprintln!("[cli] {what} failed: {cause}");
    }
}

fn method_names() -> Vec<&'static str> {
    kchess_contracts::core_api::METHODS
        .iter()
        .map(|method| method.name)
        .collect()
}

async fn arguments(args: &[String]) -> Result<Vec<Value>, String> {
    let text = if args.first().map(String::as_str) == Some("--args-file") {
        let path = args.get(1).ok_or("--args-file requires a path.")?;
        small_file(path, "Arguments are too large.").await?
    } else {
        args.first().cloned().unwrap_or_else(|| "[]".into())
    };
    if text.len() as u64 > MAX_ARGUMENTS {
        return Err("Arguments are too large.".into());
    }
    match serde_json::from_str(&text) {
        Ok(Value::Array(values)) => Ok(values),
        Ok(_) => Err("Arguments must be a JSON array.".into()),
        Err(cause) => Err(format!("Arguments are not JSON: {cause}")),
    }
}

/// An unknown command's message, naming the command it was probably meant to be.
fn unknown_command(command: &str) -> String {
    let nearest = COMMANDS
        .iter()
        .map(|known| (edit_distance(command, known.name), known.name))
        .min()
        .filter(|(distance, known)| *distance <= 2.min(known.len() / 2));
    match nearest {
        Some((_, known)) => format!("Unknown command: {command}. Did you mean {known}?"),
        None => format!("Unknown command: {command}. Run --help."),
    }
}

/// The Levenshtein distance between `a` and `b`, by characters.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(x != *y));
            diagonal = above;
        }
    }
    row[b.len()]
}

async fn run(argv: &[String]) -> Result<(), String> {
    let Cli {
        options,
        args,
        engine,
    } = parse(argv)?;
    let word = args.first().map_or("help", String::as_str);
    // An unknown command is refused before the core opens, which takes a moment.
    let command = command(word).ok_or_else(|| unknown_command(word))?;
    let rest = args.get(1..).unwrap_or_default();
    match command {
        Command::Help => print!("{}", help()),
        Command::Version => print(format_args!("kchess-cli {}", env!("CARGO_PKG_VERSION"))),
        Command::Methods => print(json!(method_names())),
        Command::Opening => print(&opening_name(rest)?),
        Command::Position => print(&position_info(rest)?),
        _ => return with_core(command, rest, &options, engine).await,
    }
    Ok(())
}

/// A command that needs the core, run over the profile's.
async fn with_core(
    command: Command,
    rest: &[String],
    options: &Options,
    engine: Option<String>,
) -> Result<(), String> {
    let (core, mut events) = CoreClient::open(options).await?;
    let result = async {
        if let Some(engine) = engine {
            // An explicit operator path is trusted and kept without a fingerprint prompt, as the
            // app's own "your own Stockfish" setting; the core checks it on use.
            core.request(calls::TrustEnginePath {
                path: json!(engine),
            })
            .await?;
            let mut settings = core.request(calls::StoredSettings {}).await?;
            settings["enginePath"] = json!(engine);
            core.request(calls::SaveSettings { settings }).await?;
        }
        match command {
            Command::Library => print(&core.request(calls::Library {}).await?),
            Command::Status => {
                let data = core.request(calls::LoadData {}).await?;
                let engine = core.request(calls::EngineStatus { variant: None }).await?;
                print(json!({ "data": data, "engine": engine }));
            }
            Command::Login => {
                // The sign-in page is opened (or printed) while the core waits for it.
                print(&core.request(calls::ConnectLichess { look: None }).await?);
            }
            Command::Call => {
                let method = rest.first().map(String::as_str).unwrap_or_default();
                if !method_names().contains(&method) {
                    return Err("Unknown CoreApi method. Run methods to list them.".into());
                }
                let parameters = arguments(rest.get(1..).unwrap_or_default()).await?;
                let assists = kchess_contracts::core_api::method(method)
                    .is_some_and(|method| method.assist.is_some());
                if assists {
                    core.request(calls::ResumeOnline {}).await?;
                }
                print(&core.call_by_name(method, parameters).await?);
            }
            Command::Analyze => {
                let request = analysis_request("analyze", rest)?;
                core.request(calls::ResumeOnline {}).await?;
                core.request(calls::StartAnalysis { request }).await?;
                let mut wait = Wait::new(&mut events);
                loop {
                    match wait.next().await {
                        Next::Event(CoreEvent::Analysis(update)) => print(&update),
                        Next::Event(_) | Next::Line(_) | Next::Tick => {}
                        Next::Interrupted | Next::Closed | Next::InputEnded => break,
                    }
                }
                told_if_failed(
                    "Stopping the analysis",
                    core.request(calls::StopAnalysis { kill: None }).await,
                );
            }
            Command::Play | Command::Local => {
                let computer = command == Command::Play;
                let (fen, words) = play::take_fen(rest)?;
                let (level, color, variant) = if computer {
                    play::choices(&words)?
                } else {
                    (None, None, play::local_variant(&words)?)
                };
                play::interactive(&core, &mut events, computer, level, color, variant, fen).await?;
            }
            Command::Puzzle => {
                let id = rest.first().map(String::as_str);
                puzzle::interactive(&core, &mut events, id).await?;
            }
            Command::Pgn => {
                let link = rest.first().map(String::as_str).unwrap_or_default();
                print(linked_pgn(&core, link).await?.trim_end());
            }
            Command::Review => {
                let pgn = review_input(&core, rest).await?;
                print(&review::run(&core, &mut events, &pgn).await?);
            }
            Command::Watch => {
                let target = watch_target(rest)?;
                core.request(calls::ResumeOnline {}).await?;
                match target {
                    Watched::Feed(target) => core.request(calls::Watch { target }).await?,
                    Watched::Round(round) => {
                        core.request(calls::WatchBroadcast {
                            round_id: json!(round),
                        })
                        .await?
                    }
                    Watched::Tour(id) => {
                        let tour = core.request(calls::BroadcastTour { id: json!(id) }).await?;
                        let round = kchess_games::broadcast::opening_round(&tour)
                            .ok_or("That broadcast has no rounds yet.")?;
                        core.request(calls::WatchBroadcast {
                            round_id: round["id"].clone(),
                        })
                        .await?
                    }
                };
                let watched = watch_frames(&mut events).await;
                told_if_failed(
                    "Stopping watching",
                    core.request(calls::StopWatching { session: None }).await,
                );
                watched?;
            }
            Command::Explore => {
                let (kind, fen, variant) = explore_request(rest)?;
                let options = match variant {
                    Some(variant) => json!({ "variant": variant.key() }),
                    None => json!({}),
                };
                core.request(calls::ResumeOnline {}).await?;
                print(
                    &core
                        .request(calls::PositionLookup {
                            kind: json!(kind),
                            fen: json!(fen),
                            options: Some(options),
                        })
                        .await?,
                );
            }
            Command::Help
            | Command::Version
            | Command::Methods
            | Command::Opening
            | Command::Position => {} // Answered by `run`.
        }
        Ok(())
    }
    .await;
    core.close().await;
    result
}

/// `pgn <link>`: the PGN of a Lichess game, or of a public study's chapter (its first for a
/// study's own link).
async fn linked_pgn(core: &CoreClient, link: &str) -> Result<String, String> {
    use kchess_domain::links::{game_link, study_link};
    let pgn = if let Some(game) = game_link(link) {
        core.request(calls::ExportGame { id: json!(game.id) })
            .await?
    } else if let Some((study, chapter)) = study_link(link) {
        core.request(calls::LichessStudyChapter {
            id: json!(study),
            chapter: json!(chapter),
        })
        .await?["pgn"]
            .take()
    } else {
        return Err("pgn takes a lichess.org game or study link.".into());
    };
    pgn.as_str()
        .map(str::to_string)
        .ok_or_else(|| "Lichess sent no PGN.".into())
}

/// What `watch` follows.
#[derive(Debug, PartialEq)]
enum Watched {
    /// A TV channel or a game: `{ channel }` or `{ gameId }`.
    Feed(Value),
    /// A broadcast round, by id.
    Round(String),
    /// A broadcast tournament, at its live round.
    Tour(String),
}

/// What `watch` was given: a Lichess TV channel (the best game when none is named), a game by its
/// link or id, or a broadcast's link.
fn watch_target(words: &[String]) -> Result<Watched, String> {
    use kchess_domain::links::{BroadcastLink, broadcast_link, game_link};
    use kchess_domain::misc::constants::TV_CHANNELS;
    let word = words.first().map_or("best", String::as_str);
    if let Some((channel, _)) = TV_CHANNELS
        .iter()
        .find(|(channel, _)| channel.eq_ignore_ascii_case(word))
    {
        return Ok(Watched::Feed(json!({ "channel": channel })));
    }
    match broadcast_link(word) {
        Some(BroadcastLink::Tour(tour)) => return Ok(Watched::Tour(tour)),
        Some(BroadcastLink::Round { round, .. }) => return Ok(Watched::Round(round)),
        None => {}
    }
    let link = if word.contains('/') {
        word.to_string()
    } else {
        format!("https://lichess.org/{word}")
    };
    game_link(&link)
        .map(|game| Watched::Feed(json!({ "gameId": game.id })))
        .ok_or_else(|| {
            format!(
                "watch takes a TV channel ({}), or a Lichess game or broadcast link.",
                TV_CHANNELS.map(|(channel, _)| channel).join(", ")
            )
        })
}

/// Print each frame of the watched game, or each update of the broadcast round, as a JSON line
/// until Ctrl-C; an error, or the round's end, ends it.
async fn watch_frames(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<core::ClientEvent>,
) -> Result<(), String> {
    let mut wait = Wait::new(events);
    loop {
        match wait.next().await {
            Next::Interrupted | Next::InputEnded => return Ok(()),
            Next::Closed => return Err("The core closed.".into()),
            Next::Event(CoreEvent::WatchFrame(frame)) => print(&frame),
            Next::Event(CoreEvent::WatchBroadcast(update)) => {
                if let Some(error) = update["error"].as_str() {
                    return Err(error.to_string());
                }
                print(&update);
                if update["ended"] == json!(true) {
                    return Ok(());
                }
            }
            Next::Event(CoreEvent::WatchState(state)) if state["phase"] == "error" => {
                let message = state["message"].as_str().unwrap_or("Watching failed.");
                return Err(message.to_string());
            }
            Next::Event(_) | Next::Line(_) | Next::Tick => {}
        }
    }
}

/// The game `review` was given: a Lichess link, a PGN file, or the PGN or moves themselves.
async fn review_input(core: &CoreClient, words: &[String]) -> Result<String, String> {
    let text = words.join(" ");
    let text = text.trim();
    if text.is_empty() {
        return Err("review requires a game: a Lichess link, a PGN file or moves.".into());
    }
    if kchess_domain::links::game_link(text).is_some()
        || kchess_domain::links::study_link(text).is_some()
    {
        return linked_pgn(core, text).await;
    }
    let path = std::path::Path::new(text);
    if path.is_file() {
        return small_file(text, &format!("{text} is too large.")).await;
    }
    Ok(text.to_string())
}

/// A file of up to `MAX_ARGUMENTS` bytes, refused as `too_large` by its size before it is read.
async fn small_file(path: &str, too_large: &str) -> Result<String, String> {
    let size = tokio::fs::metadata(path)
        .await
        .map_err(|cause| format!("Cannot read {path}: {cause}"))?
        .len();
    if size > MAX_ARGUMENTS {
        return Err(too_large.into());
    }
    tokio::fs::read_to_string(path)
        .await
        .map_err(|cause| format!("Cannot read {path}: {cause}"))
}

/// What `analyze` searches: a position, or the one a game's moves (SAN or a PGN) reach, sent
/// with its history so repetitions count. `--variant crazyhouse` (or a PGN's `Variant` header)
/// reads them in that variant, which Fairy-Stockfish analyses. `command` names the command in its
/// errors.
fn analysis_request(command: &str, words: &[String]) -> Result<Value, String> {
    let (variant, words) = match words {
        [flag, name, rest @ ..] if flag == "--variant" => {
            (play::local_variant(std::slice::from_ref(name))?, rest)
        }
        [flag] if flag == "--variant" => return Err("--variant needs a variant.".into()),
        words => (None, words),
    };
    let text = words.join(" ");
    let header = kchess_domain::pgn::parse_pgn(&text)
        .first()
        .and_then(|game| game.header("Variant").map(str::to_string));
    let variant = match (variant, header) {
        (Some(variant), _) => Some(variant),
        (None, Some(name)) => Some(review::pgn_variant(&name)?),
        (None, None) => None,
    };
    match variant {
        Some(variant) if !variant.standard_rules() => {
            variant_analysis_request(command, variant, &text)
        }
        _ => standard_analysis_request(command, words),
    }
}

/// A position in another variant, or the one its moves reach, read by that variant's rules.
fn variant_analysis_request(command: &str, variant: Variant, text: &str) -> Result<Value, String> {
    use kchess_domain::rules;
    let text = text.trim();
    let fen = match text {
        "" => return Err(format!("{command} requires a FEN or moves.")),
        "startpos" => Some(rules::make_fen(&rules::default_position(variant.rules()))),
        text => rules::setup_start(variant.key(), text).map(|position| rules::make_fen(&position)),
    };
    let variant = variant.key();
    if let Some(fen) = fen {
        return Ok(json!({ "fen": fen, "lines": 3, "variant": variant }));
    }
    let pgn = if text.starts_with('[') {
        text.to_string()
    } else {
        format!("[Variant \"{variant}\"]\n\n{text}")
    };
    let tree = kchess_domain::tree::tree_from_pgn(&pgn)
        .ok_or_else(|| format!("Not a legal {variant} position or moves: {text}"))?;
    let mut moves = Vec::new();
    let mut node = &tree;
    while let Some(next) = node.children.first() {
        moves.push(next.uci.clone());
        node = next;
    }
    Ok(json!({
        "fen": node.fen,
        "rootFen": tree.fen,
        "moves": moves,
        "lines": 3,
        "variant": variant,
    }))
}

/// A standard (or Chess960) position, or the one a game's moves reach.
fn standard_analysis_request(command: &str, words: &[String]) -> Result<Value, String> {
    if let Ok(fen) = analysis_fen(words) {
        return Ok(json!({ "fen": fen, "lines": 3 }));
    }
    let text = words.join(" ");
    let line = review::main_line(&text).map_err(|cause| match text.trim() {
        "" => format!("{command} requires a FEN or moves."),
        _ if cause.starts_with("Move ") => cause,
        text => format!("Not a legal position or moves: {text}"),
    })?;
    let fen = kchess_domain::replay::replay_positions(&line.fen, &line.moves)
        .pop()
        .map(|position| position.fen)
        .ok_or("Not a legal position.")?;
    Ok(json!({ "fen": fen, "rootFen": line.fen, "moves": line.moves, "lines": 3 }))
}

/// The position `analyze` was given: a FEN, quoted or as separate words, or `startpos`.
fn analysis_fen(words: &[String]) -> Result<String, String> {
    let fen = words.join(" ");
    let fen = match fen.trim() {
        "" => return Err("analyze requires a FEN.".into()),
        "startpos" => kchess_domain::rules::INITIAL_FEN.to_string(),
        fen => fen.to_string(),
    };
    if kchess_domain::position::fen_problem(&fen).is_some() {
        return Err(format!("Not a legal position: {fen}"));
    }
    Ok(fen)
}

/// `opening`: the named opening a position or line reaches, from the openings the app names, as
/// Lichess does (the last named position on the way). Offline: no core is needed.
fn opening_name(words: &[String]) -> Result<Value, String> {
    let request = analysis_request("opening", words)?;
    if request.get("variant").is_some() {
        return Err("Opening names are for standard chess only.".into());
    }
    let root = request
        .get("rootFen")
        .unwrap_or(&request["fen"])
        .as_str()
        .unwrap_or_default();
    let moves: Vec<String> = match request.get("moves") {
        Some(moves) => serde_json::from_value(moves.clone()).map_err(|e| format!("moves: {e}"))?,
        None => Vec::new(),
    };
    let ply = moves.len() as f64;
    let named = kchess_domain::training::openings::opening_at("standard", root, &moves, ply)
        .ok_or("No named opening there.")?;
    Ok(json!({ "eco": named.eco, "name": named.name, "fen": request["fen"] }))
}

/// The position a FEN or line reaches: whose turn, check and how the game stands, and every legal
/// move in SAN.
fn position_info(words: &[String]) -> Result<Value, String> {
    let request = analysis_request("position", words)?;
    let variant = request["variant"].as_str().unwrap_or("standard");
    let fen = request["fen"].as_str().unwrap_or_default();
    let position = kchess_domain::rules::setup_start(variant, fen);
    let info = position.as_ref().map(kchess_domain::position::info);
    let legal = position
        .as_ref()
        .map(kchess_domain::position::legal_sans)
        .unwrap_or_default();
    Ok(json!({
        "fen": info.as_ref().map(|info| &info.fen),
        "turn": info.as_ref().map(|info| info.turn),
        "check": info.as_ref().map(|info| info.check),
        "checkmate": info.as_ref().map(|info| info.checkmate),
        "stalemate": info.as_ref().map(|info| info.stalemate),
        "insufficient": info.as_ref().map(|info| info.insufficient),
        "legal": legal,
    }))
}

/// What `explore` looks up: the database named first, else the tablebase when it covers the
/// position (seven pieces or fewer, no castling rights) and the masters' games when not. A
/// variant's position (`--variant`, or a PGN's header) is looked up in Lichess's games of it.
fn explore_request(
    words: &[String],
) -> Result<(PositionLookupKind, String, Option<Variant>), String> {
    // A player's games need the player, which `explore` does not take.
    let named = words
        .first()
        .and_then(|word| serde_json::from_value(json!(word)).ok())
        .filter(|kind| *kind != PositionLookupKind::Player);
    let words = if named.is_some() { &words[1..] } else { words };
    let request = analysis_request("explore", words)?;
    let fen = request["fen"].as_str().unwrap_or_default().to_string();
    if let Some(variant) = request["variant"].as_str().and_then(Variant::from_key) {
        return match named {
            None | Some(PositionLookupKind::Opening) => {
                Ok((PositionLookupKind::Opening, fen, Some(variant)))
            }
            Some(_) => Err("Masters games and tablebases are standard chess only.".into()),
        };
    }
    let kind = named.unwrap_or_else(|| {
        let mut fields = fen.split(' ');
        let pieces = fields
            .next()
            .unwrap_or_default()
            .chars()
            .filter(char::is_ascii_alphabetic)
            .count();
        let castling = fields.nth(1).is_some_and(|rights| rights != "-");
        if pieces <= 7 && !castling {
            PositionLookupKind::Tablebase
        } else {
            PositionLookupKind::Masters
        }
    });
    Ok((kind, fen, None))
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let cli = std::thread::Builder::new()
        .name("kchess-cli".into())
        .stack_size(kchess_service::CORE_STACK)
        .spawn(move || {
            let runtime = kchess_service::core_runtime()
                .map_err(|cause| format!("Could not start: {cause}"))?;
            runtime.block_on(run(&argv))
        });
    let result = match cli.map(std::thread::JoinHandle::join) {
        Ok(Ok(result)) => result,
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(cause) => Err(format!("Could not start: {cause}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(cause) => {
            eprintln!("[cli] Command failed: {cause}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_in_the_help_and_named_once() {
        let help = help();
        let mut words = Vec::new();
        for spec in COMMANDS {
            assert!(
                help.contains(&format!("\n  {} ", spec.name)),
                "help misses {}",
                spec.name
            );
            assert_eq!(command(spec.name), Some(spec.command));
            words.push(spec.name);
            words.extend(spec.aliases);
        }
        let count = words.len();
        words.sort_unstable();
        words.dedup();
        assert_eq!(words.len(), count, "a word names two commands");
        assert_eq!(command("--help"), Some(Command::Help));
        assert_eq!(command("-V"), Some(Command::Version));
        assert_eq!(command("bogus"), None);
    }

    #[test]
    fn a_mistyped_command_names_the_nearest_one() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "pgn"), 3);
        assert_eq!(
            unknown_command("anaylze"),
            "Unknown command: anaylze. Did you mean analyze?"
        );
        assert_eq!(
            unknown_command("pgm"),
            "Unknown command: pgm. Did you mean pgn?"
        );
        // Too far from every command to guess.
        assert_eq!(unknown_command("xyz"), "Unknown command: xyz. Run --help.");
        assert_eq!(
            unknown_command("bogus"),
            "Unknown command: bogus. Run --help."
        );
        // A three-letter command allows one typo, not two.
        assert_eq!(unknown_command("pzz"), "Unknown command: pzz. Run --help.");
        assert_eq!(edit_distance("pgn", ""), 3);
        assert_eq!(edit_distance("ab", "b"), 1);
    }

    #[test]
    fn watch_follows_a_channel_a_game_or_a_broadcast() {
        use super::Watched::{Feed, Round, Tour};
        use serde_json::json;
        let words = |text: &str| {
            text.split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        let target = |text: &str| super::watch_target(&words(text));
        assert_eq!(target(""), Ok(Feed(json!({ "channel": "best" }))));
        assert_eq!(target("Blitz"), Ok(Feed(json!({ "channel": "blitz" }))));
        assert_eq!(
            target("ultrabullet"),
            Ok(Feed(json!({ "channel": "ultraBullet" })))
        );
        let game = Ok(Feed(json!({ "gameId": "q7ZvsdUF" })));
        assert_eq!(target("https://lichess.org/q7ZvsdUF/black"), game);
        assert_eq!(target("q7ZvsdUF"), game);
        assert_eq!(
            target("https://lichess.org/broadcast/kchess-open/round-1/kcRound1"),
            Ok(Round("kcRound1".into()))
        );
        assert_eq!(
            target("lichess.org/broadcast/kchess-open/kcTour01"),
            Ok(Tour("kcTour01".into()))
        );
        assert!(target("lichess.org/training").is_err_and(|cause| cause.contains("best, bullet")));
    }

    #[test]
    fn a_position_names_its_turn_mate_and_legal_moves() {
        let words = |text: &str| text.split(' ').map(str::to_string).collect::<Vec<_>>();
        let start = super::position_info(&words("startpos")).expect("the start");
        assert_eq!(start["turn"], "white");
        assert_eq!(start["legal"].as_array().map(Vec::len), Some(20));
        // Fool's mate: Black has mated, White has no moves.
        let mated = super::position_info(&words("1. f3 e5 2. g4 Qh4#")).expect("the mate");
        assert_eq!(mated["checkmate"], true);
        assert_eq!(mated["turn"], "white");
        assert_eq!(mated["legal"], serde_json::json!([]));
        assert!(
            super::position_info(&words("1. e4 e4")).is_err_and(|cause| cause.starts_with("Move "))
        );
        // A variant's position is read by its rules: a Crazyhouse pocket's pawn drops, and King
        // of the Hill is won on the centre.
        let zh = super::position_info(&words("--variant crazyhouse 1. e4 d5 2. exd5 Qxd5"))
            .expect("a Crazyhouse position");
        assert!(
            zh["legal"]
                .as_array()
                .is_some_and(|legal| legal.contains(&serde_json::json!("P@e3"))),
            "{zh}"
        );
        let hill = super::position_info(&words(
            "--variant kingOfTheHill 1. d4 e5 2. Kd2 Ke7 3. Kd3 Ke6 4. Ke4",
        ))
        .expect("a King of the Hill position");
        assert_eq!(hill["legal"], serde_json::json!([]), "{hill}");
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn reads_options_anywhere_and_keeps_the_rest_in_order() {
        // An absolute path on every platform (`/tmp/…` is relative on Windows).
        let profile = std::env::temp_dir().join("profile");
        let profile_text = profile.to_string_lossy().to_string();
        let cli = parse(&strings(&[
            "--data-dir",
            &profile_text,
            "call",
            "--",
            "--no-credentials",
            "library",
            "--browser",
            "print",
        ]))
        .unwrap();
        assert_eq!(cli.args, ["call", "library"]);
        assert_eq!(cli.options.data_dir, profile);
        assert!(!cli.options.credentials);
        assert_eq!(cli.options.browser, Browser::Print);
        assert!(!cli.options.embedded);
    }

    #[test]
    fn refuses_options_without_their_values() {
        assert_eq!(
            parse(&strings(&["--engine"])).unwrap_err(),
            "--engine requires a path."
        );
        assert_eq!(
            parse(&strings(&["--data-dir", "--embedded"])).unwrap_err(),
            "--data-dir requires a path."
        );
        assert_eq!(
            parse(&strings(&["--browser", "never"])).unwrap_err(),
            "--browser must be open or print."
        );
    }

    /// Without --data-dir, the CLI uses the app's own profile.
    #[test]
    fn the_profile_defaults_to_the_apps() {
        let options = parse(&[]).unwrap().options;
        assert_eq!(options.data_dir, crate::core::default_data_dir());
        assert!(!options.data_dir.as_os_str().is_empty());
        assert_eq!(options.browser, Browser::Open);
        assert!(options.credentials && !options.embedded);
    }

    #[test]
    fn the_last_browser_mode_wins() {
        let mode = |args: &[&str]| parse(&strings(args)).unwrap().options.browser;
        assert_eq!(mode(&["--browser", "print"]), Browser::Print);
        assert_eq!(
            mode(&["--browser", "print", "--browser", "open"]),
            Browser::Open
        );
    }

    #[test]
    fn analyze_reads_a_fen_quoted_or_in_words() {
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert_eq!(
            super::analysis_fen(&strings(&[start])),
            Ok(start.to_string())
        );
        assert_eq!(
            super::analysis_fen(&strings(&start.split(' ').collect::<Vec<_>>())),
            Ok(start.to_string())
        );
        assert_eq!(
            super::analysis_fen(&strings(&["startpos"])),
            Ok(start.to_string())
        );
        assert!(super::analysis_fen(&[]).is_err());
        assert!(super::analysis_fen(&strings(&["8/8/8/8/8/8/8/8 w - - 0 1"])).is_err());
    }

    #[test]
    fn an_opening_is_named_from_moves_or_a_position() {
        let sicilian =
            opening_name(&strings(&["1.", "e4", "c5", "2.", "Nf3", "d6"])).expect("named");
        assert_eq!(sicilian["eco"], json!("B50"));
        assert_eq!(
            sicilian["name"],
            json!("Sicilian Defense: Modern Variations")
        );
        // A position is named the same, by itself.
        let fen = sicilian["fen"].as_str().expect("a FEN").to_string();
        assert_eq!(opening_name(&strings(&[&fen])), Ok(sicilian));
        assert_eq!(
            opening_name(&strings(&["8/8/8/8/8/3k4/8/K6Q w - - 0 1"])),
            Err("No named opening there.".into())
        );
        assert_eq!(
            opening_name(&[]),
            Err("opening requires a FEN or moves.".into())
        );
        // Opening names are standard chess's, as the game pages show them.
        assert_eq!(
            opening_name(&strings(&["--variant", "crazyhouse", "1.", "e4", "c5"])),
            Err("Opening names are for standard chess only.".into())
        );
    }

    #[test]
    fn explore_picks_the_tablebase_for_few_pieces() {
        use PositionLookupKind::{Masters, Opening, Tablebase};
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert_eq!(
            explore_request(&strings(&["startpos"])),
            Ok((Masters, start.to_string(), None))
        );
        assert_eq!(
            explore_request(&strings(&["opening", "startpos"])),
            Ok((Opening, start.to_string(), None))
        );
        let ending = "8/8/8/8/8/3k4/8/K6Q w - - 0 1";
        assert_eq!(
            explore_request(&strings(&ending.split(' ').collect::<Vec<_>>())),
            Ok((Tablebase, ending.to_string(), None))
        );
        // Castling rights keep a few-piece position out of the tablebase.
        let castling = "4k3/8/8/8/8/8/8/4K2R w K - 0 1";
        assert_eq!(
            explore_request(&strings(&[castling])),
            Ok((Masters, castling.to_string(), None))
        );
        assert_eq!(
            explore_request(&strings(&["opening", "1.", "e4", "c5"])),
            Ok((
                Opening,
                "rnbqkbnr/pp1ppppp/8/2p5/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2".to_string(),
                None
            ))
        );
        assert_eq!(
            explore_request(&strings(&["tablebase"])),
            Err("explore requires a FEN or moves.".into())
        );
        assert_eq!(
            explore_request(&strings(&["8/8/8/8/8/3k4/8/K4Q2 w - - 0 1"])),
            Err("Not a legal position or moves: 8/8/8/8/8/3k4/8/K4Q2 w - - 0 1".into())
        );
        // A variant's position is looked up in Lichess's games of it, which only that database holds.
        let zh = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
        assert_eq!(
            explore_request(&strings(&["--variant", "crazyhouse", "startpos"])),
            Ok((Opening, zh.to_string(), Some(Variant::Crazyhouse)))
        );
        assert_eq!(
            explore_request(&strings(&["masters", "--variant", "atomic", "startpos"])),
            Err("Masters games and tablebases are standard chess only.".into())
        );
    }

    #[test]
    fn analyze_reads_a_variant_by_its_own_rules() {
        // A drop in Crazyhouse, which standard chess cannot read.
        let request = analysis_request(
            "analyze",
            &strings(&[
                "--variant",
                "crazyhouse",
                "1.",
                "e4",
                "d5",
                "2.",
                "exd5",
                "Qxd5",
                "3.",
                "P@e3",
            ]),
        )
        .unwrap();
        assert_eq!(request["variant"], "crazyhouse");
        assert_eq!(
            request["moves"],
            json!(["e2e4", "d7d5", "e4d5", "d8d5", "P@e3"])
        );
        assert!(
            request["fen"].as_str().unwrap().contains("[p]"),
            "{request}"
        );
        // A PGN names its own variant: the Atomic capture explodes.
        let request = analysis_request(
            "analyze",
            &strings(&["[Variant \"Atomic\"]", "1.", "e4", "d5", "2.", "exd5"]),
        )
        .unwrap();
        assert_eq!(request["variant"], "atomic");
        assert_eq!(
            request["fen"],
            "rnbqkbnr/ppp1pppp/8/8/8/8/PPPP1PPP/RNBQKBNR b KQkq - 0 2"
        );
        let start =
            analysis_request("analyze", &strings(&["--variant", "horde", "startpos"])).unwrap();
        assert_eq!(start["variant"], "horde");
        assert!(start["fen"].as_str().unwrap().contains("PPPPPPPP/PPPPPPPP"));
        assert!(
            analysis_request("analyze", &strings(&["--variant", "bughouse", "startpos"])).is_err()
        );
        // Standard chess and Chess960 stay with Stockfish.
        let request =
            analysis_request("analyze", &strings(&["--variant", "chess960", "e4"])).unwrap();
        assert!(request.get("variant").is_none());
    }

    #[test]
    fn analyze_reads_moves_and_keeps_their_history() {
        let request =
            analysis_request("analyze", &strings(&["1.", "e4", "e5", "2.", "Nf3"])).unwrap();
        assert_eq!(
            request["fen"],
            "rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2"
        );
        assert_eq!(request["moves"], json!(["e2e4", "e7e5", "g1f3"]));
        assert_eq!(
            request["rootFen"],
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        );
        assert_eq!(
            analysis_request("analyze", &strings(&["e4", "Ke7"])).unwrap_err(),
            "Move 1 (Ke7) is not legal there."
        );
        assert_eq!(
            analysis_request("analyze", &[]).unwrap_err(),
            "analyze requires a FEN or moves."
        );
        // A FEN still goes as it is, without a history.
        let start = analysis_request("analyze", &strings(&["startpos"])).unwrap();
        assert!(start.get("moves").is_none());
    }

    #[tokio::test]
    async fn call_arguments_must_be_a_json_array() {
        assert_eq!(arguments(&[]).await.unwrap(), Vec::<Value>::new());
        assert_eq!(
            arguments(&strings(&["[1, \"a\"]"])).await.unwrap(),
            vec![json!(1), json!("a")]
        );
        assert_eq!(
            arguments(&strings(&["{}"])).await.unwrap_err(),
            "Arguments must be a JSON array."
        );
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "[true]").unwrap();
        let path = file.path().to_string_lossy().to_string();
        assert_eq!(
            arguments(&strings(&["--args-file", &path])).await.unwrap(),
            vec![json!(true)]
        );
    }

    /// A file up to `MAX_ARGUMENTS` bytes is read; a larger one is refused unread.
    #[tokio::test]
    async fn a_file_is_read_up_to_its_limit() {
        let limit = usize::try_from(MAX_ARGUMENTS).unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_string_lossy().to_string();
        let game_file = || small_file(&path, "too large");
        std::fs::write(file.path(), "e".repeat(limit)).unwrap();
        assert_eq!(game_file().await.unwrap().len(), limit);
        std::fs::write(file.path(), vec![0xff; limit + 1]).unwrap();
        assert_eq!(game_file().await.unwrap_err(), "too large");
        std::fs::write(file.path(), [0xff]).unwrap();
        assert!(game_file().await.unwrap_err().starts_with("Cannot read"));
    }

    #[test]
    fn a_variant_flag_needs_its_variant() {
        assert_eq!(
            analysis_request("analyze", &strings(&["--variant"])).unwrap_err(),
            "--variant needs a variant."
        );
    }

    /// Arguments up to `MAX_ARGUMENTS` bytes are read, inline or from a file; a larger file is
    /// refused by its size, before it is read.
    #[tokio::test]
    async fn arguments_are_read_up_to_their_limit() {
        let limit = usize::try_from(MAX_ARGUMENTS).unwrap();
        let padded = |length: usize| format!("[{}]", " ".repeat(length - 2));
        assert_eq!(
            arguments(&[padded(limit)]).await.unwrap(),
            Vec::<Value>::new()
        );
        assert_eq!(
            arguments(&[padded(limit + 1)]).await.unwrap_err(),
            "Arguments are too large."
        );
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_string_lossy().to_string();
        std::fs::write(file.path(), padded(limit)).unwrap();
        assert_eq!(
            arguments(&strings(&["--args-file", &path])).await.unwrap(),
            Vec::<Value>::new()
        );
        // Not even text: only the size check can name the problem.
        std::fs::write(file.path(), vec![0xff; limit + 1]).unwrap();
        assert_eq!(
            arguments(&strings(&["--args-file", &path]))
                .await
                .unwrap_err(),
            "Arguments are too large."
        );
    }
}
