//! The Rust core (`kchess-core`) as the GPUI app holds it. The core runs on its own Tokio
//! runtime; the UI awaits its answers through channels, and its events reach views through
//! `CoreEvents`, an entity they subscribe to. Host requests (`host:request`) are answered on the
//! UI thread, which owns the window and the browser (`answer_host`).

use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::{App, AppContext as _, Entity, EventEmitter, Global};
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

/// The core and the runtime it runs on.
pub struct CoreBridge {
    runtime: tokio::runtime::Runtime,
    core: Arc<Core>,
}

impl CoreBridge {
    /// Opens the core over `config`'s profile; fails when another KChess app has it open.
    fn open(config: Config) -> Result<(CoreBridge, mpsc::UnboundedReceiver<CoreEvent>), String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("kchess-core")
            .build()
            .map_err(|e| format!("The core could not start: {e}"))?;
        let (events, receiver) = mpsc::unbounded();
        let core = {
            let _entered = runtime.enter();
            Core::open(config, Arc::new(ChannelHost { events })).map_err(|e| e.message)?
        };
        Ok((
            CoreBridge {
                runtime,
                core: Arc::new(core),
            },
            receiver,
        ))
    }

    /// Run a core method; the future resolves on any executor.
    pub fn call(
        &self,
        method: &str,
        args: Vec<Value>,
    ) -> impl Future<Output = Result<Value, CoreError>> + 'static {
        let core = Arc::clone(&self.core);
        let method = method.to_string();
        let task = self
            .runtime
            .spawn(async move { core.call(&method, args).await });
        async move {
            task.await
                .unwrap_or_else(|_| Err(CoreError::new("The KChess core stopped.")))
        }
    }

    /// Cancel the core's work and release the profile, waiting at most a few seconds.
    pub fn close(&self) {
        let core = Arc::clone(&self.core);
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
    ) -> impl Future<Output = Result<Value, CoreError>> + 'static {
        self.bridge.call(method, args)
    }
}

/// Opens the core for `data_dir` and delivers its events to `CoreEvents` on the UI thread.
pub fn start(data_dir: PathBuf, cx: &mut App) -> Result<(), String> {
    let config = Config {
        legacy_database_path: crate::profile::legacy_database(&data_dir),
        managed_engine_dir: Some(crate::profile::managed_engine_dir()),
        system_credentials: true,
        ..Config::new(data_dir.clone())
    };
    let (bridge, mut receiver) = CoreBridge::open(config)?;
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
            core.events.update(cx, |_, cx| cx.emit(event));
        }
    })
    .detach();
    Ok(())
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
