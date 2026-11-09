//! The profile's core, as the CLI reaches it: the shared core service the app also uses (started
//! when none runs), or, with `--embedded`, a core in this process. Either way calls go by
//! `CoreApi` name and events arrive on one channel; the core's host requests are answered here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kchess_contracts::Level;
use kchess_contracts::calls::CoreCall;
use kchess_core::{Config, Core, Host};
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
    data_dir_from(|name| std::env::var_os(name))
}

/// `default_data_dir` with the environment read through `var`.
fn data_dir_from(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> PathBuf {
    if let Some(dir) = var("KCHESS_USER_DATA_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = var(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/KChess")
    } else if cfg!(windows) {
        var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
            .join("KChess")
    } else {
        var("XDG_CONFIG_HOME")
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
    config_from(options, &|name| kchess_core::env::var_os(name))
}

/// `config` with the environment read through `var`.
fn config_from(options: &Options, var: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Config {
    let home = var(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_default();
    // A development override, as `kchess_core::dev_override` reads one.
    let engine =
        var("KCHESS_ENGINE_PATH").filter(|path| kchess_core::dev_overrides() && !path.is_empty());
    Config {
        managed_engine_dir: Some(home.join(".kchess/engines/stockfish/current")),
        bundled_engine_path: engine
            .map(PathBuf::from)
            .or_else(|| beside(&executable("stockfish"))),
        // Fairy-Stockfish for variants: beside the executable, or (a build in this repository)
        // where `cargo xtask fairy-stockfish` builds it.
        variant_engine_path: var("KCHESS_VARIANT_ENGINE_PATH")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| beside(&executable("fairy-stockfish")))
            .or_else(|| {
                let built = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/fairy-stockfish/bin")
                    .join(executable("fairy-stockfish"));
                built.exists().then_some(built)
            }),
        // Beside the executable, or in a Mac bundle's Resources, as the app finds it: the first
        // client to start the profile's core chooses it for every client.
        voice_helper_path: beside(&format!("voice/{}", executable("kchess-voice"))).or_else(|| {
            beside(&format!(
                "../Resources/voice/{}",
                executable("kchess-voice")
            ))
        }),
        // Earlier macOS releases kept their database outside the profile; only the app's own
        // profile imports it, as the app does (`apps/gpui/src/profile.rs`).
        legacy_database_path: (cfg!(target_os = "macos")
            && var("KCHESS_USER_DATA_DIR").is_none()
            && options.data_dir == data_dir_from(var))
        .then(|| home.join(".kchess/kchess.db")),
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
                        variant_engine_path: config.variant_engine_path,
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
            Backend::Service(client) => client
                .call(method, args)
                .await
                .map_err(|cause| cause.message),
        }
    }

    /// A typed core call (`kchess_contracts::calls`): its method with its arguments, and its
    /// answer read in its shape.
    pub async fn request<C: CoreCall>(&self, call: C) -> Result<C::Answer, String> {
        let answer = self.call(C::METHOD, call.args()).await?;
        kchess_contracts::calls::decode::<C>(answer).map_err(|cause| cause.message)
    }

    /// Answers a `host:request` the core sent; other events are left to the caller.
    pub async fn answer(&self, request: &Value) {
        let Some((id, reply)) = reply(request, self.browser, open_browser) else {
            return;
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

/// The reply to a `host:request`, opening a web page with `open` when `browser` says so and
/// telling the person on stderr. `None` for a request without an id.
fn reply(
    request: &Value,
    browser: Browser,
    open: impl FnOnce(&str) -> std::io::Result<()>,
) -> Option<(u64, Value)> {
    let id = request["id"].as_u64()?;
    let reply = match request["kind"].as_str().unwrap_or_default() {
        "openExternal" => match request["payload"].as_str() {
            Some(url) if kchess_core::capabilities::opens_in_browser(url) => {
                if browser == Browser::Open && open(url).is_ok() {
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
    Some((id, reply))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let pairs: Vec<(String, OsString)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), OsString::from(value)))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    /// `KCHESS_USER_DATA_DIR` names the profile; set but empty, it is ignored.
    #[test]
    fn the_profile_is_the_variable_else_the_apps_own() {
        let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let own = data_dir_from(environment(&[(home, "/home/someone")]));
        assert!(own.ends_with("KChess"), "{own:?}");
        assert!(own.starts_with("/home/someone"), "{own:?}");
        assert_eq!(
            data_dir_from(environment(&[
                ("KCHESS_USER_DATA_DIR", "/scratch"),
                (home, "/h")
            ])),
            PathBuf::from("/scratch")
        );
        assert_eq!(
            data_dir_from(environment(&[
                ("KCHESS_USER_DATA_DIR", ""),
                (home, "/home/someone")
            ])),
            own
        );
    }

    fn options(data_dir: &str, credentials: bool) -> Options {
        Options {
            data_dir: PathBuf::from(data_dir),
            browser: Browser::Print,
            credentials,
            embedded: true,
        }
    }

    /// The paths the core is given: the overrides this build honours, else the app's own.
    #[test]
    fn the_core_is_configured_from_the_options_and_overrides() {
        let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let config = config_from(
            &options("/profile", false),
            &environment(&[
                (home, "/home/someone"),
                ("KCHESS_ENGINE_PATH", "/engines/stockfish"),
                ("KCHESS_VARIANT_ENGINE_PATH", "/engines/fairy"),
            ]),
        );
        assert_eq!(config.data_dir, PathBuf::from("/profile"));
        assert_eq!(
            config.managed_engine_dir,
            Some(PathBuf::from(
                "/home/someone/.kchess/engines/stockfish/current"
            ))
        );
        assert_eq!(
            config.bundled_engine_path,
            Some(PathBuf::from("/engines/stockfish"))
        );
        assert_eq!(
            config.variant_engine_path,
            Some(PathBuf::from("/engines/fairy"))
        );
        assert!(!config.system_credentials);
        assert!(config_from(&options("/profile", true), &environment(&[])).system_credentials);
        // Empty overrides count as unset.
        let unset = config_from(
            &options("/profile", true),
            &environment(&[
                ("KCHESS_ENGINE_PATH", ""),
                ("KCHESS_VARIANT_ENGINE_PATH", ""),
            ]),
        );
        assert_ne!(unset.bundled_engine_path, Some(PathBuf::new()));
        assert_ne!(unset.variant_engine_path, Some(PathBuf::new()));
    }

    /// Only the app's own profile on a Mac imports the database earlier releases kept outside it.
    #[test]
    fn only_the_apps_own_mac_profile_imports_the_old_database() {
        let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let environment = environment(&[(home, "/home/someone")]);
        let own = data_dir_from(&environment);
        let own = own.to_string_lossy();
        let legacy = Some(PathBuf::from("/home/someone/.kchess/kchess.db"));
        assert_eq!(
            config_from(&options(&own, true), &environment).legacy_database_path,
            if cfg!(target_os = "macos") {
                legacy
            } else {
                None
            }
        );
        assert_eq!(
            config_from(&options("/elsewhere", true), &environment).legacy_database_path,
            None
        );
    }

    /// The voice helper is taken from beside the executable, as the app ships it.
    #[test]
    fn the_voice_helper_beside_the_executable_is_used() {
        let exe = std::env::current_exe().unwrap();
        let voice = exe.parent().unwrap().join("voice");
        std::fs::create_dir_all(&voice).unwrap();
        let helper = voice.join(executable("kchess-voice"));
        std::fs::write(&helper, b"").unwrap();
        let config = config_from(&options("/profile", true), &environment(&[]));
        let _ = std::fs::remove_file(&helper);
        assert_eq!(config.voice_helper_path, Some(helper));
    }

    /// A web page opens in the browser only when asked to, and nothing else opens at all.
    #[test]
    fn host_requests_are_answered() {
        let page = json!({ "id": 3, "kind": "openExternal", "payload": "https://lichess.org/" });
        let opened = std::cell::RefCell::new(Vec::new());
        let open = |url: &str| {
            opened.borrow_mut().push(url.to_string());
            Ok(())
        };
        assert_eq!(
            reply(&page, Browser::Print, open),
            Some((3, json!({ "value": null })))
        );
        assert!(opened.borrow().is_empty());
        assert_eq!(
            reply(&page, Browser::Open, open),
            Some((3, json!({ "value": null })))
        );
        assert_eq!(*opened.borrow(), ["https://lichess.org/"]);
        let file = json!({ "id": 4, "kind": "openExternal", "payload": "file:///etc/passwd" });
        assert_eq!(
            reply(&file, Browser::Open, open),
            Some((
                4,
                json!({ "error": "Only web addresses open in the browser." })
            ))
        );
        assert_eq!(opened.borrow().len(), 1);
        assert_eq!(
            reply(
                &json!({ "id": 5, "kind": "onBattery" }),
                Browser::Open,
                open
            ),
            Some((5, json!({ "value": false })))
        );
        assert_eq!(
            reply(&json!({ "id": 6, "kind": "focus" }), Browser::Open, open),
            Some((
                6,
                json!({ "error": "The command line cannot answer focus." })
            ))
        );
        assert_eq!(
            reply(&json!({ "kind": "focus" }), Browser::Open, open),
            None
        );
    }

    #[test]
    fn helpers_are_found_beside_the_executable() {
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(beside(&name), Some(exe));
        assert_eq!(beside("no-such-helper"), None);
        assert_eq!(
            executable("kchess-voice"),
            format!("kchess-voice{}", std::env::consts::EXE_SUFFIX)
        );
    }
}
