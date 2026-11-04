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
  analyze <fen|startpos|moves>     Stream engine analysis as JSON lines until Ctrl-C
  play [level] [white|black|random] Play Stockfish; type moves in SAN or UCI
  local [variant]                  Play over the board (chess960, crazyhouse, atomic…)
  puzzle [id|link]                 Solve the daily puzzle, or that one; solution shows it
  pgn <link>                       Print a Lichess game's PGN, or a public study chapter's
  review <link|file|moves>         Review a game with Stockfish: accuracy and faulted moves
  explore [masters|opening|tablebase] <fen|startpos|moves>
                                   Look a position up (the tablebase for 7 pieces or fewer)
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
/// with its history so repetitions count.
fn analysis_request(words: &[String]) -> Result<Value, String> {
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
