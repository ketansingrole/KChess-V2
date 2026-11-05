//! `kchess-cli`: the KChess command line, over the same profile and core as the app.

mod core;
mod play;
mod puzzle;
mod review;

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::core::{Browser, CoreClient, Options};

const HELP: &str = "KChess CLI

Usage: kchess-cli [options] <command>

Commands:
  library                          Print the library as JSON
  status                           Print account and engine status
  login                            Connect a Lichess account in your browser
  call <method> '[arguments]'      Invoke any CoreApi method (JSON argument array)
  call <method> --args-file <path> Read arguments from a JSON file
  analyze [--variant name] <fen|startpos|moves>
                                   Stream engine analysis as JSON lines until Ctrl-C
  play [level] [white|black|random] Play Stockfish; type moves in SAN or UCI
  local [variant]                  Play over the board (chess960, crazyhouse, atomic…)
  puzzle [id|link]                 Solve the daily puzzle, or that one (hint, solution)
  pgn <link>                       Print a Lichess game's PGN, or a public study chapter's
  review <link|file|moves>         Review a game with the engine: accuracy and faulted moves
  explore [masters|opening|tablebase] <fen|startpos|moves>
                                   Look a position up (the tablebase for 7 pieces or fewer)
  opening <fen|startpos|moves>     Name the opening a position or line reaches (ECO)
  position <fen|startpos|moves>    Print whose turn, check or mate, and the legal moves
  watch [channel|link]             Stream Lichess TV, a game or a broadcast as JSON lines
  methods                          List available CoreApi methods

Options:
  --data-dir <path>                Use another profile (default: the app's)
  --browser open|print             Open the login page or print its URL
  --no-credentials                 Disable OS credential storage
  --engine <path>                  Use a native Stockfish executable
  --embedded                       Run the core in this process, not the shared one

Interactive game commands: e4 or e2e4, undo, resign, new, status, pgn, quit.
Local games also accept draw and pause. Sessions are saved in the core library.
";

/// The most argument text `call` reads.
const MAX_ARGUMENTS: u64 = 2_000_000;

#[derive(Debug, PartialEq)]
struct Cli {
    options: Options,
    args: Vec<String>,
    engine: Option<String>,
}

impl PartialEq for Options {
    fn eq(&self, other: &Options) -> bool {
        self.data_dir == other.data_dir
            && self.browser == other.browser
            && self.credentials == other.credentials
            && self.embedded == other.embedded
    }
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("data_dir", &self.data_dir)
            .field("browser", &self.browser)
            .field("credentials", &self.credentials)
            .field("embedded", &self.embedded)
            .finish()
    }
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

fn print(value: &Value) {
    println!("{value}");
}

fn method_names() -> Vec<&'static str> {
    kchess_contracts::core_api::methods()
        .iter()
        .map(|method| method.name)
        .collect()
}

async fn arguments(args: &[String]) -> Result<Vec<Value>, String> {
    let text = if args.first().map(String::as_str) == Some("--args-file") {
        let path = args.get(1).ok_or("--args-file requires a path.")?;
        let size = tokio::fs::metadata(path)
            .await
            .map_err(|cause| format!("Cannot read {path}: {cause}"))?
            .len();
        if size > MAX_ARGUMENTS {
            return Err("Arguments are too large.".into());
        }
        tokio::fs::read_to_string(path)
            .await
            .map_err(|cause| format!("Cannot read {path}: {cause}"))?
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

/// Until Ctrl-C (or the core closing), answering host requests and handing other events to `on`.
async fn until_interrupted(
    core: &CoreClient,
    events: &mut tokio::sync::mpsc::UnboundedReceiver<core::Event>,
    mut on: impl FnMut(&core::Event),
) {
    let interrupted = tokio::signal::ctrl_c();
    tokio::pin!(interrupted);
    loop {
        tokio::select! {
            _ = &mut interrupted => return,
            event = events.recv() => match event {
                None => return,
                Some(event) if event.name == "core:closed" => return,
                Some(event) if event.name == "host:request" => core.answer(&event.payload).await,
                Some(event) => on(&event),
            },
        }
    }
}

async fn run(argv: &[String]) -> Result<(), String> {
    let Cli {
        options,
        args,
        engine,
    } = parse(argv)?;
    let command = args.first().map_or("help", String::as_str);
    match command {
        "help" | "--help" | "-h" => {
            print!("{HELP}");
            return Ok(());
        }
        "methods" => {
            print(&json!(method_names()));
            return Ok(());
        }
        "opening" => {
            print(&opening_name(&args[1..])?);
            return Ok(());
        }
        "position" => {
            print(&position_info(&args[1..])?);
            return Ok(());
        }
        _ => {}
    }
    let (core, mut events) = CoreClient::open(&options).await?;
    let result = async {
        if let Some(engine) = engine {
            // An explicit operator path is trusted and kept without a fingerprint prompt, as the
            // app's own "your own Stockfish" setting; the core checks it on use.
            core.call("trustEnginePath", vec![json!(engine)]).await?;
            let mut settings = core.call("settings", vec![]).await?;
            settings["enginePath"] = json!(engine);
            core.call("saveSettings", vec![settings]).await?;
        }
        match command {
            "library" => print(&core.call("library", vec![]).await?),
            "status" => {
                let data = core.call("loadData", vec![]).await?;
                let engine = core.call("engineStatus", vec![]).await?;
                print(&json!({ "data": data, "engine": engine }));
            }
            "login" => {
                // The sign-in page is opened (or printed) while the core waits for it.
                let login = core.call("connectLichess", vec![]);
                tokio::pin!(login);
                let answer = loop {
                    tokio::select! {
                        answer = &mut login => break answer?,
                        Some(event) = events.recv() => {
                            if event.name == "host:request" {
                                core.answer(&event.payload).await;
                            }
                        }
                    }
                };
                print(&answer);
            }
            "call" => {
                let method = args.get(1).map(String::as_str).unwrap_or_default();
                if !method_names().contains(&method) {
                    return Err("Unknown CoreApi method. Run methods to list them.".into());
                }
                let parameters = arguments(&args[2..]).await?;
                if kchess_domain::misc::ASSISTANCE_METHODS.contains(&method) {
                    core.call("resumeOnline", vec![]).await?;
                }
                print(&core.call(method, parameters).await?);
            }
            "analyze" => {
                let request = analysis_request(&args[1..])?;
                core.call("resumeOnline", vec![]).await?;
                core.call("startAnalysis", vec![request]).await?;
                until_interrupted(&core, &mut events, |event| {
                    if event.name == "engine:analysis" {
                        print(&event.payload);
                    }
                })
                .await;
                let _ = core.call("stopAnalysis", vec![]).await;
            }
            "play" | "local" => {
                let computer = command == "play";
                let ((level, color), variant) = if computer {
                    (play::choices(&args[1..])?, None)
                } else {
                    ((None, None), play::local_variant(&args[1..])?)
                };
                play::interactive(&core, &mut events, computer, level, color, variant).await?;
            }
            "puzzle" => {
                let id = args.get(1).map(String::as_str);
                puzzle::interactive(&core, &mut events, id).await?;
            }
            "pgn" => {
                let link = args.get(1).map(String::as_str).unwrap_or_default();
                println!("{}", linked_pgn(&core, link).await?.trim_end());
            }
            "review" => {
                let pgn = review_input(&core, &args[1..]).await?;
                print(&review::run(&core, &mut events, &pgn).await?);
            }
            "watch" => {
                let target = watch_target(&args[1..])?;
                core.call("resumeOnline", vec![]).await?;
                match target {
                    Watched::Feed(target) => core.call("watch", vec![target]).await?,
                    Watched::Round(round) => {
                        core.call("watchBroadcast", vec![json!(round)]).await?
                    }
                    Watched::Tour(id) => {
                        let tour = core.call("broadcastTour", vec![json!(id)]).await?;
                        let round = live_round(&tour).ok_or("That broadcast has no rounds yet.")?;
                        core.call("watchBroadcast", vec![json!(round)]).await?
                    }
                };
                let watched = watch_frames(&core, &mut events).await;
                let _ = core.call("stopWatching", vec![]).await;
                watched?;
            }
            "explore" => {
                let (kind, fen) = explore_request(&args[1..])?;
                core.call("resumeOnline", vec![]).await?;
                print(
                    &core
                        .call("positionLookup", vec![json!(kind), json!(fen), json!({})])
                        .await?,
                );
            }
            other => return Err(format!("Unknown command: {other}. Run --help.")),
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
        core.call("exportGame", vec![json!(game.id)]).await?
    } else if let Some((study, chapter)) = study_link(link) {
        core.call("lichessStudyChapter", vec![json!(study), json!(chapter)])
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
    use kchess_domain::misc::TV_CHANNEL_KEYS;
    let word = words.first().map_or("best", String::as_str);
    if let Some(channel) = TV_CHANNEL_KEYS
        .iter()
        .find(|channel| channel.eq_ignore_ascii_case(word))
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
                TV_CHANNEL_KEYS.join(", ")
            )
        })
}

/// The round a broadcast opens at, as the app opens it: the one Lichess names, else the one being
/// played, else the last.
fn live_round(tour: &Value) -> Option<String> {
    let rounds = tour["rounds"].as_array()?;
    rounds
        .iter()
        .find(|round| round["id"] == tour["defaultRoundId"])
        .or_else(|| rounds.iter().find(|round| round["ongoing"] == json!(true)))
        .or(rounds.last())
        .and_then(|round| round["id"].as_str().map(str::to_string))
}

/// Print each frame of the watched game, or each update of the broadcast round, as a JSON line
/// until Ctrl-C; an error, or the round's end, ends it.
async fn watch_frames(
    core: &CoreClient,
    events: &mut tokio::sync::mpsc::UnboundedReceiver<core::Event>,
) -> Result<(), String> {
    let interrupted = tokio::signal::ctrl_c();
    tokio::pin!(interrupted);
    loop {
        tokio::select! {
            _ = &mut interrupted => return Ok(()),
            event = events.recv() => match event {
                None => return Err("The core closed.".into()),
                Some(event) if event.name == "core:closed" => return Err("The core closed.".into()),
                Some(event) if event.name == "host:request" => core.answer(&event.payload).await,
                Some(event) if event.name == "watch:frame" => print(&event.payload),
                Some(event) if event.name == "watch:broadcast" => {
                    if let Some(error) = event.payload["error"].as_str() {
                        return Err(error.to_string());
                    }
                    print(&event.payload);
                    if event.payload["ended"] == json!(true) {
                        return Ok(());
                    }
                }
                Some(event) if event.name == "watch:state" && event.payload["phase"] == "error" => {
                    let message = event.payload["message"].as_str().unwrap_or("Watching failed.");
                    return Err(message.to_string());
                }
                Some(_) => {}
            },
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
        let size = tokio::fs::metadata(path)
            .await
            .map_err(|cause| format!("Cannot read {text}: {cause}"))?
            .len();
        if size > MAX_ARGUMENTS {
            return Err(format!("{text} is too large."));
        }
        return tokio::fs::read_to_string(path)
            .await
            .map_err(|cause| format!("Cannot read {text}: {cause}"));
    }
    Ok(text.to_string())
}

/// What `analyze` searches: a position, or the one a game's moves (SAN or a PGN) reach, sent
/// with its history so repetitions count. `--variant crazyhouse` (or a PGN's `Variant` header)
/// reads them in that variant, which Fairy-Stockfish analyses.
fn analysis_request(words: &[String]) -> Result<Value, String> {
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
        (None, Some(name)) => Some(review::variant_key(&name)?.to_string()),
        (None, None) => None,
    };
    match variant.as_deref() {
        Some(variant) if !matches!(variant, "standard" | "chess960") => {
            variant_analysis_request(variant, &text)
        }
        _ => standard_analysis_request(words),
    }
}

/// A position in another variant, or the one its moves reach, read by that variant's rules.
fn variant_analysis_request(variant: &str, text: &str) -> Result<Value, String> {
    use kchess_domain::rules;
    let start = rules::lichess_variant(variant).ok_or("Unknown variant.")?;
    let text = text.trim();
    let fen = match text {
        "" => return Err("analyze requires a FEN or moves.".into()),
        "startpos" => Some(rules::make_fen(&rules::default_position(start))),
        text => rules::setup_start(variant, text).map(|position| rules::make_fen(&position)),
    };
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
fn standard_analysis_request(words: &[String]) -> Result<Value, String> {
    if let Ok(fen) = analysis_fen(words) {
        return Ok(json!({ "fen": fen, "lines": 3 }));
    }
    let text = words.join(" ");
    let line = review::main_line(&text).map_err(|cause| match text.trim() {
        "" => "analyze requires a FEN or moves.".to_string(),
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
        "startpos" => "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1".to_string(),
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
    let request = analysis_request(words).map_err(|cause| cause.replace("analyze", "opening"))?;
    let root = request.get("rootFen").unwrap_or(&request["fen"]).clone();
    let moves = request.get("moves").cloned().unwrap_or_else(|| json!([]));
    let named = kchess_domain::api::call_value(
        "openingAt",
        &[json!({ "variant": "standard", "fen": root }), moves],
    )
    .map_err(|cause| cause.to_string())?;
    if named.is_null() {
        return Err("No named opening there.".into());
    }
    Ok(json!({ "eco": named["eco"], "name": named["name"], "fen": request["fen"] }))
}

/// The position a FEN or line reaches: whose turn, check and how the game stands, and every legal
/// move in SAN.
fn position_info(words: &[String]) -> Result<Value, String> {
    let request = analysis_request(words).map_err(|cause| cause.replace("analyze", "position"))?;
    let setup = json!({ "variant": "standard", "fen": request["fen"] });
    let call = |method: &str| {
        kchess_domain::api::call_value(method, std::slice::from_ref(&setup))
            .map_err(|cause| cause.to_string())
    };
    let info = call("position")?;
    let legal: Vec<Value> = call("legalMoves")?
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| entry["san"].clone())
        .collect();
    Ok(json!({
        "fen": info["fen"],
        "turn": info["turn"],
        "check": info["check"],
        "checkmate": info["checkmate"],
        "stalemate": info["stalemate"],
        "insufficient": info["insufficient"],
        "legal": legal,
    }))
}

/// What `explore` looks up: the database named first, else the tablebase when it covers the
/// position (seven pieces or fewer, no castling rights) and the masters' games when not.
fn explore_request(words: &[String]) -> Result<(&'static str, String), String> {
    const KINDS: [&str; 3] = ["masters", "opening", "tablebase"];
    let named = words
        .first()
        .and_then(|word| KINDS.iter().find(|kind| **kind == word.as_str()));
    let words = if named.is_some() { &words[1..] } else { words };
    let fen = analysis_request(words).map_err(|cause| cause.replace("analyze", "explore"))?["fen"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let kind = named.copied().unwrap_or_else(|| {
        let mut fields = fen.split(' ');
        let pieces = fields
            .next()
            .unwrap_or_default()
            .chars()
            .filter(char::is_ascii_alphabetic)
            .count();
        let castling = fields.nth(1).is_some_and(|rights| rights != "-");
        if pieces <= 7 && !castling {
            "tablebase"
        } else {
            "masters"
        }
    });
    Ok((kind, fen))
}

/// The core's advice names the app's Settings; here the same fix is a command or an option.
fn for_the_cli(cause: &str) -> String {
    const LOGIN: &str = "with `kchess-cli login`";
    [
        ("Reconnect in Settings", format!("Reconnect {LOGIN}")),
        ("Reconnect it in Settings", format!("Reconnect it {LOGIN}")),
        ("Connect it in Settings", format!("Connect it {LOGIN}")),
        ("account in Settings", format!("account {LOGIN}")),
        (
            "Choose an engine in Settings",
            "Pass --engine <path>".to_string(),
        ),
    ]
    .iter()
    .fold(cause.to_string(), |text, (app, cli)| text.replace(app, cli))
}

#[tokio::main]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(&argv).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(cause) => {
            eprintln!("[cli] Command failed: {}", for_the_cli(&cause));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_broadcast_opens_at_its_live_round() {
        use serde_json::json;
        let rounds = json!([
            { "id": "r1", "finished": true },
            { "id": "r2", "ongoing": true },
            { "id": "r3" },
        ]);
        let named = json!({ "rounds": rounds, "defaultRoundId": "r1" });
        assert_eq!(super::live_round(&named).as_deref(), Some("r1"));
        let unnamed = json!({ "rounds": rounds });
        assert_eq!(super::live_round(&unnamed).as_deref(), Some("r2"));
        let none_live = json!({ "rounds": [{ "id": "r1" }, { "id": "r2" }] });
        assert_eq!(super::live_round(&none_live).as_deref(), Some("r2"));
        assert_eq!(super::live_round(&json!({ "rounds": [] })), None);
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
    }

    #[test]
    fn settings_advice_names_the_cli_fix() {
        assert_eq!(
            for_the_cli("Connect a Lichess account in Settings to use the opening explorer."),
            "Connect a Lichess account with `kchess-cli login` to use the opening explorer."
        );
        assert_eq!(
            for_the_cli("@ann's Lichess login is unavailable. Reconnect it in Settings."),
            "@ann's Lichess login is unavailable. Reconnect it with `kchess-cli login`."
        );
        assert_eq!(
            for_the_cli("Stockfish could not be found. Choose an engine in Settings."),
            "Stockfish could not be found. Pass --engine <path>."
        );
        assert_eq!(for_the_cli("Move 2 is not legal."), "Move 2 is not legal.");
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
    }

    #[test]
    fn explore_picks_the_tablebase_for_few_pieces() {
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert_eq!(
            explore_request(&strings(&["startpos"])),
            Ok(("masters", start.to_string()))
        );
        assert_eq!(
            explore_request(&strings(&["opening", "startpos"])),
            Ok(("opening", start.to_string()))
        );
        let ending = "8/8/8/8/8/3k4/8/K6Q w - - 0 1";
        assert_eq!(
            explore_request(&strings(&ending.split(' ').collect::<Vec<_>>())),
            Ok(("tablebase", ending.to_string()))
        );
        // Castling rights keep a few-piece position out of the tablebase.
        let castling = "4k3/8/8/8/8/8/8/4K2R w K - 0 1";
        assert_eq!(
            explore_request(&strings(&[castling])),
            Ok(("masters", castling.to_string()))
        );
        assert_eq!(
            explore_request(&strings(&["opening", "1.", "e4", "c5"])),
            Ok((
                "opening",
                "rnbqkbnr/pp1ppppp/8/2p5/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2".to_string()
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
    }

    #[test]
    fn analyze_reads_a_variant_by_its_own_rules() {
        // A drop in Crazyhouse, which standard chess cannot read.
        let request = analysis_request(&strings(&[
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
        ]))
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
        let request = analysis_request(&strings(&[
            "[Variant \"Atomic\"]",
            "1.",
            "e4",
            "d5",
            "2.",
            "exd5",
        ]))
        .unwrap();
        assert_eq!(request["variant"], "atomic");
        assert_eq!(
            request["fen"],
            "rnbqkbnr/ppp1pppp/8/8/8/8/PPPP1PPP/RNBQKBNR b KQkq - 0 2"
        );
        let start = analysis_request(&strings(&["--variant", "horde", "startpos"])).unwrap();
        assert_eq!(start["variant"], "horde");
        assert!(start["fen"].as_str().unwrap().contains("PPPPPPPP/PPPPPPPP"));
        assert!(analysis_request(&strings(&["--variant", "bughouse", "startpos"])).is_err());
        // Standard chess and Chess960 stay with Stockfish.
        let request = analysis_request(&strings(&["--variant", "chess960", "e4"])).unwrap();
        assert!(request.get("variant").is_none());
    }

    #[test]
    fn analyze_reads_moves_and_keeps_their_history() {
        let request = analysis_request(&strings(&["1.", "e4", "e5", "2.", "Nf3"])).unwrap();
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
            analysis_request(&strings(&["e4", "Ke7"])).unwrap_err(),
            "Move 1 (Ke7) is not legal there."
        );
        assert_eq!(
            analysis_request(&[]).unwrap_err(),
            "analyze requires a FEN or moves."
        );
        // A FEN still goes as it is, without a history.
        let start = analysis_request(&strings(&["startpos"])).unwrap();
        assert!(start.get("moves").is_none());
    }

    #[test]
    fn lists_the_core_methods() {
        let names = method_names();
        assert!(names.contains(&"library"));
        assert!(names.contains(&"bestMove"));
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
}
