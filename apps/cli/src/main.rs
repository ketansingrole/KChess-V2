//! `kchess`: the KChess command line, over the same profile and core as the app.

mod core;
mod play;

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::core::{Browser, CoreClient, Options};

const HELP: &str = "KChess CLI

Usage: kchess [options] <command>

Commands:
  library                          Print the library as JSON
  status                           Print account and engine status
  login                            Connect a Lichess account in your browser
  call <method> '[arguments]'      Invoke any CoreApi method (JSON argument array)
  call <method> --args-file <path> Read arguments from a JSON file
  analyze <fen>                    Stream engine analysis as JSON lines until Ctrl-C
  play [level]                     Play Stockfish with UCI moves
  local                            Play a local game with UCI moves
  methods                          List available CoreApi methods

Options:
  --data-dir <path>                Use another profile (default: the app's)
  --browser open|print             Open the login page or print its URL
  --no-credentials                 Disable OS credential storage
  --engine <path>                  Use a native Stockfish executable
  --embedded                       Run the core in this process, not the shared one

Interactive game commands: e2e4, undo, resign, new, status, quit.
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
                let fen = args.get(1).ok_or("analyze requires a FEN.")?;
                core.call("resumeOnline", vec![]).await?;
                core.call("startAnalysis", vec![json!({ "fen": fen, "lines": 3 })])
                    .await?;
                until_interrupted(&core, &mut events, |event| {
                    if event.name == "engine:analysis" {
                        print(&event.payload);
                    }
                })
                .await;
                let _ = core.call("stopAnalysis", vec![]).await;
            }
            "play" | "local" => {
                play::interactive(&core, &mut events, command == "play", args.get(1).cloned())
                    .await?;
            }
            other => return Err(format!("Unknown command: {other}. Run --help.")),
        }
        Ok(())
    }
    .await;
    core.close().await;
    result
}

#[tokio::main]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(&argv).await {
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

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn reads_options_anywhere_and_keeps_the_rest_in_order() {
        let cli = parse(&strings(&[
            "--data-dir",
            "/tmp/profile",
            "call",
            "--",
            "--no-credentials",
            "library",
            "--browser",
            "print",
        ]))
        .unwrap();
        assert_eq!(cli.args, ["call", "library"]);
        assert_eq!(cli.options.data_dir, PathBuf::from("/tmp/profile"));
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
