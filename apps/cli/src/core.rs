//! The profile's core, as the CLI reaches it: the shared core service the app also uses (started
//! when none runs), or, with `--embedded`, a core in this process. Either way calls go by
//! `CoreApi` name and events arrive on one channel; the core's host requests are answered here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kchess_core::{Config, Core, Host, Level};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// One core event (`CoreEvents` names, and `host:request`).
pub struct Event {
    pub name: String,
    pub payload: Value,
}

/// How the login page reaches the person.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Browser {
    Open,
    Print,
}

pub struct Options {
    pub data_dir: PathBuf,
    pub browser: Browser,
    pub credentials: bool,
    pub embedded: bool,
}

enum Backend {
    Embedded(Arc<Core>),
    Service(kchess_service::Client),
}

pub struct CoreClient {
    backend: Backend,
    browser: Browser,
}

/// Diagnostics go to stderr, so JSON on stdout stays usable in pipelines.
struct StderrHost {
    events: mpsc::UnboundedSender<Event>,
}

impl Host for StderrHost {
    fn log(&self, level: Level, scope: &str, message: &str) {
        if matches!(level, Level::Warn | Level::Error) {
            eprintln!("[{scope}] {message}");
        }
    }

    fn emit(&self, event: &str, payload: Value) {
        let _ = self.events.send(Event {
            name: event.to_string(),
            payload,
        });
    }
}

/// The app's profile, which the CLI shares: `KCHESS_USER_DATA_DIR`, else the app's own folder.
pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("KCHESS_USER_DATA_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/KChess")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
            .join("KChess")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("KChess")
    }
}

/// A file shipped beside this executable (the app's package keeps the CLI with its helpers).
fn beside(name: &str) -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?.parent()?.join(name);
    path.exists().then_some(path)
}

fn executable(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn config(options: &Options) -> Config {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_default();
    Config {
        managed_engine_dir: Some(home.join(".kchess/engines/stockfish/current")),
        bundled_engine_path: std::env::var_os("KCHESS_ENGINE_PATH")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| beside(&executable("stockfish"))),
        voice_helper_path: beside(&format!("voice/{}", executable("kchess-voice"))),
        system_credentials: options.credentials,
        ..Config::new(options.data_dir.clone())
    }
}

impl CoreClient {
    /// Opens the profile's core and starts delivering its events.
    pub async fn open(
        options: &Options,
    ) -> Result<(CoreClient, mpsc::UnboundedReceiver<Event>), String> {
        std::fs::create_dir_all(&options.data_dir)
            .map_err(|cause| format!("The profile folder is unavailable: {cause}"))?;
        let config = config(options);
        let (events, received) = mpsc::unbounded_channel();
        let service = std::env::var_os("KCHESS_SERVICE_BINARY")
            .map(PathBuf::from)
            .or_else(|| beside(&executable("kchess-service")));
        let backend = match service {
            Some(binary) if !options.embedded => {
                let start = kchess_service::Start {
                    binary,
                    config: kchess_service::ServiceConfig {
                        legacy_database_path: config.legacy_database_path,
                        bundled_engine_path: config.bundled_engine_path,
                        runtime_path: config.runtime_path,
                        runtime_env: config.runtime_env,
                        managed_engine_dir: config.managed_engine_dir,
                        voice_helper_path: config.voice_helper_path,
                        system_credentials: config.system_credentials,
                    },
                };
                let hello = kchess_service::Hello {
                    kind: "cli".into(),
                    capabilities: vec![kchess_service::Capability::Browser],
                };
                let (client, mut incoming) =
                    kchess_service::connect_or_start(&options.data_dir, &hello, &start)
                        .await
                        .map_err(|error| error.to_string())?;
                tokio::spawn(async move {
                    while let Some(event) = incoming.recv().await {
                        match event {
                            kchess_service::ClientEvent::Event { name, payload } => {
                                let _ = events.send(Event { name, payload });
                            }
                            kchess_service::ClientEvent::Closed => {
                                let _ = events.send(Event {
                                    name: "core:closed".into(),
                                    payload: Value::Null,
                                });
                                break;
                            }
                        }
                    }
                });
                Backend::Service(client)
            }
            _ => {
                let core = Core::open(config, Arc::new(StderrHost { events }))
                    .map_err(|cause| cause.message)?;
                Backend::Embedded(Arc::new(core))
            }
        };
        Ok((
            CoreClient {
                backend,
                browser: options.browser,
            },
            received,
        ))
    }

    /// A `CoreApi` method by name.
    pub async fn call(&self, method: &str, args: Vec<Value>) -> Result<Value, String> {
        match &self.backend {
            Backend::Embedded(core) => core.call(method, args).await.map_err(|cause| cause.message),
            Backend::Service(client) => client.call(method, args).await,
        }
    }

    /// Answers a `host:request` the core sent; other events are left to the caller.
    pub async fn answer(&self, request: &Value) {
        let Some(id) = request["id"].as_u64() else {
            return;
        };
        let reply = match request["kind"].as_str().unwrap_or_default() {
            "openExternal" => match request["payload"].as_str() {
                Some(url) if url.starts_with("https://") || url.starts_with("http://127.0.0.1") => {
                    if self.browser == Browser::Open && open_browser(url).is_ok() {
                        eprintln!("Opened {url}");
                    } else {
                        eprintln!("Open this page to continue: {url}");
                    }
                    json!({ "value": null })
                }
                _ => json!({ "error": "Only web addresses open in the browser." }),
            },
            "onBattery" => json!({ "value": false }),
            kind => json!({ "error": format!("The command line cannot answer {kind}.") }),
        };
        if let Err(cause) = self.call("host.reply", vec![json!(id), reply]).await {
            eprintln!("[cli] A host reply was not delivered: {cause}");
        }
    }

    pub async fn close(&self) {
        if let Backend::Embedded(core) = &self.backend {
            core.close().await;
        }
    }
}

fn open_browser(url: &str) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut command = std::process::Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    } else {
        std::process::Command::new("xdg-open")
    };
    command
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// A `--data-dir` as given, made absolute.
pub fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    }
}
