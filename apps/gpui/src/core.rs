//! The Rust core (`kchess-core`) as the GPUI app holds it: the profile's shared core service
//! (`docs/shared-core.md`), which every KChess window uses, or a core embedded in the app. Either
//! runs on a Tokio runtime; the UI awaits its answers through channels, and its events reach
//! views through `CoreEvents`, an entity they subscribe to. Host requests (`host:request`) are
//! answered on the UI thread, which owns the window and the browser (`answer_host`).

use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::{App, AppContext as _, Entity, EventEmitter, Global, PromptLevel};
use kchess_core::{Config, Core, CoreError, Host, Level};
use serde_json::{Value, json};

/// A core event, by its `CoreEvents` name.
#[derive(Clone, Debug)]
pub struct CoreEvent {
    pub name: String,
    pub payload: Value,
}

/// What views subscribe to for core events.
pub struct CoreEvents;

impl EventEmitter<CoreEvent> for CoreEvents {}

struct ChannelHost {
    events: mpsc::UnboundedSender<CoreEvent>,
}

impl Host for ChannelHost {
    fn log(&self, level: Level, scope: &str, message: &str) {
        crate::log::write(level, scope, message);
    }

    fn emit(&self, event: &str, payload: Value) {
        // The UI has gone when the receiver is dropped; late events have nowhere to go.
        let _ = self.events.unbounded_send(CoreEvent {
            name: event.to_string(),
            payload,
        });
    }
}

/// Where the app's core runs.
enum Backend {
    /// In this process, for this app alone.
    Embedded(Arc<Core>),
    /// The profile's shared core service; the client is replaced when it reconnects.
    Service(Arc<std::sync::Mutex<kchess_service::Client>>),
}

/// The core and the runtime it runs on.
pub struct CoreBridge {
    runtime: tokio::runtime::Runtime,
    backend: Backend,
    /// The window's focus as last reported (0 unknown, 1 blurred, 2 focused), sent again on each
    /// new connection to the shared core.
    focused: Arc<std::sync::atomic::AtomicU8>,
}

/// What the app hears when it reached the shared core again (`core:reconnected`): what it had
/// started there ended with the old connection, and its owners start it again.
pub const RECONNECTED: &str = "core:reconnected";

/// How the app reaches its core: `KCHESS_CORE_MODE=service` or `embedded`; otherwise the shared
/// service when its binary ships beside the app.
fn use_service() -> bool {
    match std::env::var("KCHESS_CORE_MODE").as_deref() {
        Ok("service") => true,
        Ok("embedded") => false,
        _ => service_binary().is_some_and(|binary| binary.exists()),
    }
}

/// The `kchess-service` binary beside the app's executable.
fn service_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(
        exe.parent()?
            .join(format!("kchess-service{}", std::env::consts::EXE_SUFFIX)),
    )
}

/// What the GPUI window can do for the core when it asks.
fn hello() -> kchess_service::Hello {
    kchess_service::Hello {
        kind: "gpui".into(),
        capabilities: vec![
            kchess_service::Capability::Browser,
            kchess_service::Capability::Focus,
        ],
    }
}

/// A release as numbers, to tell which of two is newer.
fn release_parts(release: &str) -> Vec<u64> {
    release
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// What the person reads when another KChess release has this profile open
/// (`docs/shared-core.md`, "Version check").
fn refusal(error: &kchess_service::ConnectError) -> String {
    let kchess_service::ConnectError::Refused { clients, .. } = error else {
        return error.to_string();
    };
    let ours = kchess_service::protocol::release();
    let Some(other) = clients.first() else {
        return error.to_string();
    };
    let app = match other.kind.as_str() {
        "electron" => "Electron",
        "gpui" => "GPUI",
        kind => kind,
    };
    if release_parts(&other.release) > release_parts(&ours) {
        format!(
            "KChess {} ({app}) is open on this profile. Update this app to {} to use them together.",
            other.release, other.release
        )
    } else {
        format!(
            "KChess {} ({app}) is open on this profile. Update it to {ours}, or quit it, to open this app.",
            other.release
        )
    }
}

impl CoreBridge {
    fn runtime() -> Result<tokio::runtime::Runtime, String> {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("kchess-core")
            .build()
            .map_err(|e| format!("The core could not start: {e}"))
    }

    /// Opens a core in this process over `config`'s profile; fails when another KChess app has
    /// it open.
    fn open(config: Config) -> Result<(CoreBridge, mpsc::UnboundedReceiver<CoreEvent>), String> {
        let runtime = Self::runtime()?;
        let (events, receiver) = mpsc::unbounded();
        let core = {
            let _entered = runtime.enter();
            Core::open(config, Arc::new(ChannelHost { events })).map_err(|e| e.message)?
        };
        Ok((
            CoreBridge {
                runtime,
                backend: Backend::Embedded(Arc::new(core)),
                focused: Arc::default(),
            },
            receiver,
        ))
    }

    /// Connects to the profile's shared core, starting it when none runs. If it stops, the app
    /// starts it again and carries on with the new connection.
    fn connect(config: Config) -> Result<(CoreBridge, mpsc::UnboundedReceiver<CoreEvent>), String> {
        let runtime = Self::runtime()?;
        let binary = service_binary().ok_or("The KChess core service is missing.")?;
        let profile = config.data_dir.clone();
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
        let (client, mut received) = runtime
            .block_on(kchess_service::connect_or_start(&profile, &hello(), &start))
            .map_err(|error| refusal(&error))?;
        let shared = Arc::new(std::sync::Mutex::new(client));
        let focused: Arc<std::sync::atomic::AtomicU8> = Arc::default();
        let (events, receiver) = mpsc::unbounded();
        {
            let shared = Arc::clone(&shared);
            let focused = Arc::clone(&focused);
            runtime.spawn(async move {
                loop {
                    while let Some(event) = received.recv().await {
                        match event {
                            kchess_service::ClientEvent::Event { name, payload } => {
                                let _ = events.unbounded_send(CoreEvent { name, payload });
                            }
                            kchess_service::ClientEvent::Closed => break,
                        }
                    }
                    crate::log::write(Level::Warn, "core", "The core stopped; reconnecting.");
                    let mut delay = std::time::Duration::from_millis(200);
                    loop {
                        match kchess_service::connect_or_start(&profile, &hello(), &start).await {
                            Ok((client, reconnected)) => {
                                match focused.load(std::sync::atomic::Ordering::SeqCst) {
                                    0 => {}
                                    known => client.focus(known == 2),
                                }
                                if let Ok(mut current) = shared.lock() {
                                    *current = client;
                                }
                                received = reconnected;
                                let _ = events.unbounded_send(CoreEvent {
                                    name: RECONNECTED.into(),
                                    payload: Value::Null,
                                });
                                break;
                            }
                            Err(error) => {
                                crate::log::write(
                                    Level::Warn,
                                    "core",
                                    &format!("The core could not restart: {}", refusal(&error)),
                                );
                                tokio::time::sleep(delay).await;
                                delay = (delay * 2).min(std::time::Duration::from_secs(5));
                            }
                        }
                    }
                }
            });
        }
        Ok((
            CoreBridge {
                runtime,
                backend: Backend::Service(shared),
                focused,
            },
            receiver,
        ))
    }

    /// This app's connection on the core: `origin` of its own writes in `profile:changed`.
    pub fn connection(&self) -> u64 {
        match &self.backend {
            Backend::Embedded(_) => kchess_core::connection::APP,
            Backend::Service(client) => client
                .lock()
                .map_or(kchess_core::connection::APP, |client| client.connection()),
        }
    }

    /// Tells the shared core whether this app's window is focused: the window focused last
    /// raises system notifications.
    pub fn focus(&self, focused: bool) {
        self.focused.store(
            if focused { 2 } else { 1 },
            std::sync::atomic::Ordering::SeqCst,
        );
        if let Backend::Service(client) = &self.backend
            && let Ok(client) = client.lock()
        {
            client.focus(focused);
        }
    }

    /// Run a core method; the future resolves on any executor.
    pub fn call(
        &self,
        method: &str,
        args: Vec<Value>,
    ) -> impl Future<Output = Result<Value, CoreError>> + 'static + use<> {
        let method = method.to_string();
        let task = match &self.backend {
            Backend::Embedded(core) => {
                let core = Arc::clone(core);
                self.runtime
                    .spawn(async move { core.call(&method, args).await })
            }
            Backend::Service(client) => {
                let answer = client
                    .lock()
                    .map(|client| client.call(&method, args))
                    .map_err(|_| CoreError::new("The KChess core stopped."));
                self.runtime
                    .spawn(async move { answer?.await.map_err(CoreError::new) })
            }
        };
        async move {
            task.await
                .unwrap_or_else(|_| Err(CoreError::new("The KChess core stopped.")))
        }
    }

    /// Run other work on the core's runtime (the voice model's download uses its HTTP client).
    pub fn spawn<T: Send + 'static, W: Future<Output = T> + Send + 'static>(
        &self,
        work: W,
    ) -> impl Future<Output = Option<T>> + 'static + use<T, W> {
        let task = self.runtime.spawn(work);
        async move { task.await.ok() }
    }

    /// Run a core method and wait for it: only before the window opens, as the Electron renderer
    /// waits for its library and preferences before it mounts.
    pub fn call_now(&self, method: &str, args: Vec<Value>) -> Result<Value, CoreError> {
        let answer = self.call(method, args);
        self.runtime.block_on(answer)
    }

    /// Cancel the core's work and release the profile, waiting at most a few seconds.
    pub fn close(&self) {
        // The shared core closes on its own once no window uses it.
        let Backend::Embedded(core) = &self.backend else {
            return;
        };
        let core = Arc::clone(core);
        let closing = self.runtime.spawn(async move {
            // The facade's own close (`close`) stops the services; then the files are released.
            let _ = core.call("close", Vec::new()).await;
            core.close().await;
        });
        let _ = self.runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), closing).await
        });
    }
}

/// Saves to finish before the core closes (`on_quit`): each returns the work still to wait for.
#[derive(Default)]
pub struct QuitFlushes(Vec<Flush>);

type Flush = Box<dyn Fn(&mut App) -> Option<BoxedSave>>;

/// Whether a save reached the profile, or why it did not.
pub type Saved = Result<(), String>;

pub type BoxedSave = std::pin::Pin<Box<dyn Future<Output = Saved>>>;

impl Global for QuitFlushes {}

/// Run `flush` when the app quits, before the core closes, so unsaved work reaches the profile.
/// The Electron app does the same through its prepare-quit handshake.
pub fn on_quit(cx: &mut App, flush: impl Fn(&mut App) -> Option<BoxedSave> + 'static) {
    cx.default_global::<QuitFlushes>().0.push(Box::new(flush));
}

/// Wait for every save, and report each that failed.
pub fn all_saved(saves: Vec<BoxedSave>) -> BoxedSave {
    Box::pin(async move {
        let failures: Vec<String> = futures::future::join_all(saves)
            .await
            .into_iter()
            .filter_map(Result::err)
            .collect();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("\n"))
        }
    })
}

/// Work for once the saves are done and the core has closed (`after_quit`).
#[derive(Default)]
pub struct QuitLast(Vec<Last>);

type Last = Box<dyn Fn(&mut App) -> Option<Box<dyn FnOnce()>>>;

impl Global for QuitLast {}

/// Run the work `last` returns (decided as the app starts quitting) only after every `on_quit`
/// save has finished and the core has closed: work that must not overlap them, such as an
/// installer that closes the app. Saves that fail or do not finish in time never run it, unless
/// the person chose to go on without them (`prepare_quit`).
pub fn after_quit(cx: &mut App, last: impl Fn(&mut App) -> Option<Box<dyn FnOnce()>> + 'static) {
    cx.default_global::<QuitLast>().0.push(Box::new(last));
}

/// How far preparing to quit has got (`QuitPreparation` in the Electron app).
#[derive(Default)]
struct Quitting {
    /// Saving with the window open: a second request waits for this one.
    preparing: bool,
    /// The person chose to quit without the saves that failed.
    discarded: bool,
}

impl Global for Quitting {}

/// Every registered save, started now.
fn flush_all(cx: &mut App) -> Vec<BoxedSave> {
    let flushes = std::mem::take(&mut cx.default_global::<QuitFlushes>().0);
    let pending = flushes.iter().filter_map(|flush| flush(cx)).collect();
    // Surfaces opened while saving registered theirs meanwhile: keep both.
    let mut kept = flushes;
    kept.append(&mut cx.default_global::<QuitFlushes>().0);
    cx.default_global::<QuitFlushes>().0 = kept;
    pending
}

/// How long saving before quitting may take before the person is asked what to do, as the
/// Electron app's `QuitPreparation` waits for its renderer.
const SAVE_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// What became of saves waited for until a deadline.
enum Waited {
    Saved(Saved),
    /// Still unanswered at the deadline: kept, so a retry waits for the same saves rather than
    /// starting them again over writes that may still land.
    Unanswered(BoxedSave),
}

/// Wait for `save` until `deadline` fires.
async fn wait_saved(save: BoxedSave, deadline: impl Future<Output = ()>) -> Waited {
    let deadline = std::pin::pin!(deadline);
    match futures::future::select(save, deadline).await {
        futures::future::Either::Left((saved, _)) => Waited::Saved(saved),
        futures::future::Either::Right(((), save)) => Waited::Unanswered(save),
    }
}

/// Save everything while the window is still open, then run `then` (quit, or install and
/// restart). A save that fails, or does not finish within `SAVE_WAIT`, asks first: retry, go on
/// without it (`without`, the button's label), or cancel and keep working (`cancelled`).
pub fn prepare_quit(
    cx: &mut App,
    without: &'static str,
    then: impl FnOnce(&mut App) + 'static,
    cancelled: impl FnOnce(&mut App) + 'static,
) {
    if std::mem::replace(&mut cx.default_global::<Quitting>().preparing, true) {
        return cx.defer(cancelled);
    }
    cx.spawn(async move |cx| {
        let mut unanswered: Option<BoxedSave> = None;
        loop {
            let save = match unanswered.take() {
                Some(save) => save,
                None => all_saved(cx.update(flush_all)),
            };
            let deadline = cx.background_executor().timer(SAVE_WAIT);
            let saved = match wait_saved(save, deadline).await {
                Waited::Saved(saved) => saved,
                Waited::Unanswered(save) => {
                    unanswered = Some(save);
                    Err("Saving is taking longer than expected.".to_string())
                }
            };
            let Err(cause) = saved else {
                cx.update(|cx| {
                    cx.default_global::<Quitting>().preparing = false;
                    then(cx);
                });
                return;
            };
            crate::log::write(
                Level::Warn,
                "app",
                &format!("Saving before quitting failed: {cause}"),
            );
            let answer = cx.update(|cx| {
                let window = cx
                    .active_window()
                    .or_else(|| cx.windows().first().copied())?;
                window
                    .update(cx, |_, window, cx| {
                        window.prompt(
                            PromptLevel::Warning,
                            "Some changes could not be saved.",
                            Some(&cause),
                            &["Retry", without, "Cancel"],
                            cx,
                        )
                    })
                    .ok()
            });
            // No window to ask in: quit, but nothing installs over the unsaved work.
            let asked = answer.is_some();
            let choice = match answer {
                Some(answer) => answer.await.unwrap_or(2),
                None => 1,
            };
            match choice {
                0 => continue,
                1 => {
                    cx.update(|cx| {
                        let quitting = cx.default_global::<Quitting>();
                        quitting.preparing = false;
                        quitting.discarded = asked;
                        then(cx);
                    });
                    return;
                }
                _ => {
                    cx.update(|cx| {
                        cx.default_global::<Quitting>().preparing = false;
                        cancelled(cx);
                    });
                    return;
                }
            }
        }
    })
    .detach();
}

/// Every registered flush, then the core's close, then the work for after both.
pub fn quit(cx: &mut App) -> impl Future<Output = ()> + 'static + use<> {
    let pending = flush_all(cx);
    let discarded = cx.default_global::<Quitting>().discarded;
    let last = std::mem::take(&mut cx.default_global::<QuitLast>().0);
    let last: Vec<Box<dyn FnOnce()>> = last.iter().filter_map(|last| last(cx)).collect();
    let bridge = Arc::clone(&cx.global::<AppCore>().bridge);
    finish(pending, move || bridge.close(), last, discarded)
}

/// The saves, then `close`, then the work for after both: never before every save has finished,
/// and only when every save succeeded or the person chose to go on without them (`discarded`).
async fn finish(
    pending: Vec<BoxedSave>,
    close: impl FnOnce(),
    last: Vec<Box<dyn FnOnce()>>,
    discarded: bool,
) {
    let saved = all_saved(pending).await;
    close();
    if let Err(cause) = saved
        && !discarded
    {
        crate::log::write(
            Level::Warn,
            "app",
            &format!("Unsaved work kept the update from installing: {cause}"),
        );
        return;
    }
    for work in last {
        work();
    }
}

/// Orders a surface's reads of the profile (`RequestScope` in the Electron app): only the newest
/// read is shown, and one started before a local change is read again rather than shown over it.
#[derive(Default)]
pub struct Reads {
    latest: u64,
    edits: u64,
}

/// A read in flight: which one it was, and the local changes made before it started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Read {
    id: u64,
    edits: u64,
}

/// What to do with a read's answer.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    Show,
    /// A newer read was started: drop this one.
    Superseded,
    /// A local change came after the read started: read again.
    Again,
}

impl Reads {
    pub fn start(&mut self) -> Read {
        self.latest += 1;
        Read {
            id: self.latest,
            edits: self.edits,
        }
    }

    /// A local change was made (sent to the core, or shown before it is).
    pub fn edited(&mut self) {
        self.edits += 1;
    }

    pub fn answer(&self, read: Read) -> Answer {
        if read.id != self.latest {
            Answer::Superseded
        } else if read.edits != self.edits {
            Answer::Again
        } else {
            Answer::Show
        }
    }
}

/// Runs `reload` when another window changes one of `areas` of the profile (`profile:changed`,
/// `docs/shared-core.md`), and after reconnecting to the core, which did not report what changed
/// meanwhile; this app's own writes are ignored.
pub fn on_profile_change(
    cx: &mut App,
    areas: &'static [&'static str],
    reload: impl Fn(&mut App) + 'static,
) {
    let events = cx.global::<AppCore>().events.clone();
    cx.subscribe(&events, move |_, event: &CoreEvent, cx| {
        if event.name == RECONNECTED {
            return reload(cx);
        }
        if event.name != "profile:changed" {
            return;
        }
        let own = cx.global::<AppCore>().bridge.connection();
        if event.payload["origin"].as_u64() == Some(own) {
            return;
        }
        let changed = event.payload["areas"].as_array().is_some_and(|changed| {
            changed
                .iter()
                .any(|area| area.as_str().is_some_and(|area| areas.contains(&area)))
        });
        if changed {
            reload(cx);
        }
    })
    .detach();
}

/// Which connection raises system notifications now (`alerts:owner`); `None` until the core says.
#[derive(Default)]
pub struct AlertOwner(pub Option<u64>);

impl Global for AlertOwner {}

/// Whether this app may raise a system notification: it is the alert owner, or nobody is.
pub fn alerts_here(cx: &App) -> bool {
    let own = cx.global::<AppCore>().bridge.connection();
    cx.try_global::<AlertOwner>()
        .and_then(|owner| owner.0)
        .is_none_or(|owner| owner == own)
}

/// The app's core, reachable from any view: `cx.global::<AppCore>()`.
#[derive(Clone)]
pub struct AppCore {
    pub bridge: Arc<CoreBridge>,
    pub events: Entity<CoreEvents>,
}

impl Global for AppCore {}

impl AppCore {
    pub fn call(
        &self,
        method: &str,
        args: Vec<Value>,
    ) -> impl Future<Output = Result<Value, CoreError>> + 'static + use<> {
        self.bridge.call(method, args)
    }
}

/// Opens the core for `data_dir` and delivers its events to `CoreEvents` on the UI thread.
pub fn start(data_dir: PathBuf, cx: &mut App) -> Result<(), String> {
    let config = Config {
        legacy_database_path: crate::profile::legacy_database(&data_dir),
        managed_engine_dir: Some(crate::profile::managed_engine_dir()),
        // A native executable, so no runtime runs it (`runtime_path: None`).
        bundled_engine_path: crate::profile::bundled_engine(),
        voice_helper_path: crate::profile::voice_helper(),
        system_credentials: true,
        ..Config::new(data_dir.clone())
    };
    let (bridge, mut receiver) = if use_service() {
        CoreBridge::connect(config)?
    } else {
        CoreBridge::open(config)?
    };
    let core = AppCore {
        bridge: Arc::new(bridge),
        events: cx.new(|_| CoreEvents),
    };
    cx.set_global(core.clone());
    cx.spawn(async move |cx| {
        while let Some(event) = receiver.next().await {
            if event.name == "host:request" {
                cx.update(|cx| answer_host(&event.payload, cx));
                continue;
            }
            if event.name == "alerts:owner" {
                let owner = event.payload["connection"].as_u64();
                cx.update(|cx| cx.set_global(AlertOwner(owner)));
            }
            core.events.update(cx, |_, cx| cx.emit(event));
        }
    })
    .detach();
    Ok(())
}

/// The core serves Lichess from a local stand-in (`KCHESS_TEST_LICHESS_BASE` under the test
/// switch, as the Electron end-to-end suite runs it).
fn test_lichess() -> bool {
    kchess_core::store::debug::enabled()
        && std::env::var("KCHESS_TEST_LICHESS_BASE").is_ok_and(|base| !base.is_empty())
}

/// Answer a `host:request` (`crates/kchess-core/src/capabilities.rs`). Tokens are sealed by the
/// core (`system_credentials`); the host is only asked to open ones Electron's `safeStorage`
/// sealed, which this app cannot read.
fn answer_host(request: &Value, cx: &mut App) {
    let Some(id) = request["id"].as_u64() else {
        return;
    };
    let payload = &request["payload"];
    let reply = match request["kind"].as_str().unwrap_or_default() {
        "secrets.available" => json!({ "value": false }),
        "secrets.encrypt" | "secrets.decrypt" => json!({
            "error": "This login was saved by an earlier KChess release. Reconnect your account."
        }),
        "openExternal" => match payload.as_str() {
            // With a local stand-in for Lichess (the test switch), the page a sign-in opens is
            // written to the profile for the test to follow, instead of the person's browser.
            Some(url) if test_lichess() => {
                let line = format!("{url}\n");
                let written = crate::profile::data_dir().and_then(|dir| {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(dir.join("opened-urls.txt"))
                        .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()))
                });
                match written {
                    Ok(()) => json!({ "value": null }),
                    Err(cause) => json!({ "error": cause.to_string() }),
                }
            }
            Some(url) if url.starts_with("https://") || url.starts_with("http://127.0.0.1") => {
                cx.open_url(url);
                json!({ "value": null })
            }
            _ => json!({ "error": "Only web addresses open in the browser." }),
        },
        "focus" => {
            cx.activate(true);
            json!({ "value": null })
        }
        "onBattery" => json!({ "value": crate::power::on_battery() }),
        kind => json!({ "error": format!("Unknown host capability {kind}.") }),
    };
    let bridge = Arc::clone(&cx.global::<AppCore>().bridge);
    let delivery = bridge.call("host.reply", vec![json!(id), reply]);
    cx.background_executor()
        .spawn(async move {
            if let Err(cause) = delivery.await {
                crate::log::write(
                    Level::Debug,
                    "native-core",
                    &format!("Host reply was not delivered: {}", cause.message),
                );
            }
        })
        .detach();
}

#[cfg(test)]
mod tests {
    use super::{Answer, BoxedSave, Reads, Waited, all_saved, finish, wait_saved};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::task::{Context, Poll, Waker};

    #[test]
    fn an_unanswered_save_is_reported_at_the_deadline_and_kept_for_a_retry() {
        let mut cx = Context::from_waker(Waker::noop());
        let (answered, answer) = futures::channel::oneshot::channel::<()>();
        // The core received the save and never replies, with its connection still open.
        let save: BoxedSave = Box::pin(async move {
            let _ = answer.await;
            Ok(())
        });
        let mut waiting = Box::pin(wait_saved(save, std::future::ready(())));
        let Poll::Ready(Waited::Unanswered(save)) = waiting.as_mut().poll(&mut cx) else {
            panic!("an unanswered save must end the wait at the deadline");
        };
        // Retrying waits for the same save, which may still land, rather than sending it again.
        let mut retry = Box::pin(wait_saved(save, std::future::pending()));
        assert!(retry.as_mut().poll(&mut cx).is_pending());
        let _ = answered.send(());
        assert!(matches!(
            retry.as_mut().poll(&mut cx),
            Poll::Ready(Waited::Saved(Ok(())))
        ));
    }

    #[test]
    fn work_after_quitting_waits_for_every_save_and_the_core() {
        let order = Rc::new(RefCell::new(Vec::new()));
        let (saved, save) = futures::channel::oneshot::channel::<()>();
        let pending: Vec<BoxedSave> = vec![Box::pin(async move {
            let _ = save.await;
            Ok(())
        })];
        let closed = Rc::clone(&order);
        let installed = Rc::clone(&order);
        let mut quitting = Box::pin(finish(
            pending,
            move || closed.borrow_mut().push("close"),
            vec![Box::new(move || installed.borrow_mut().push("install"))],
            false,
        ));
        let mut cx = Context::from_waker(Waker::noop());
        // A slow save: nothing installs while it is still being written.
        assert!(quitting.as_mut().poll(&mut cx).is_pending());
        assert!(order.borrow().is_empty());
        let _ = saved.send(());
        assert_eq!(quitting.as_mut().poll(&mut cx), Poll::Ready(()));
        assert_eq!(*order.borrow(), ["close", "install"]);
    }

    /// A study and a session that both fail to save: the core still closes, but no installer
    /// runs over the unsaved work unless the person chose to go on without it.
    #[test]
    fn a_failed_save_keeps_the_update_from_installing() {
        for (discarded, expected) in [(false, vec!["close"]), (true, vec!["close", "install"])] {
            let order = Rc::new(RefCell::new(Vec::new()));
            let pending: Vec<BoxedSave> = vec![
                Box::pin(async { Err("study: disk full".to_string()) }),
                Box::pin(async { Err("session: disk full".to_string()) }),
            ];
            let closed = Rc::clone(&order);
            let installed = Rc::clone(&order);
            let mut quitting = Box::pin(finish(
                pending,
                move || closed.borrow_mut().push("close"),
                vec![Box::new(move || installed.borrow_mut().push("install"))],
                discarded,
            ));
            let mut cx = Context::from_waker(Waker::noop());
            assert_eq!(quitting.as_mut().poll(&mut cx), Poll::Ready(()));
            assert_eq!(*order.borrow(), expected);
        }
    }

    #[test]
    fn every_failure_is_reported() {
        let saves: Vec<BoxedSave> = vec![
            Box::pin(async { Ok(()) }),
            Box::pin(async { Err("study".to_string()) }),
            Box::pin(async { Err("session".to_string()) }),
        ];
        let mut all = all_saved(saves);
        let mut cx = Context::from_waker(Waker::noop());
        assert_eq!(
            all.as_mut().poll(&mut cx),
            Poll::Ready(Err("study\nsession".to_string()))
        );
    }

    #[test]
    fn only_the_newest_read_without_later_changes_is_shown() {
        let mut reads = Reads::default();
        let older = reads.start();
        let newer = reads.start();
        // Answers in reverse order: the older one never replaces the newer.
        assert_eq!(reads.answer(newer), Answer::Show);
        assert_eq!(reads.answer(older), Answer::Superseded);
        // A local change after a read started: that read may predate it.
        let read = reads.start();
        reads.edited();
        assert_eq!(reads.answer(read), Answer::Again);
        let again = reads.start();
        assert_eq!(reads.answer(again), Answer::Show);
    }
}
