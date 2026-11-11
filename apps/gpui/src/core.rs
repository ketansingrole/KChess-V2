//! The core as the GPUI app holds it (`kchess_service::CoreHandle`): the profile's shared core
//! service (`docs/shared-core.md`), which every KChess window uses, or a core embedded in the app.
//! Either runs on a Tokio runtime; the UI awaits its answers through channels, and its events
//! reach views through `CoreEvents`, an entity they subscribe to. Host requests are answered on
//! the UI thread, which owns the window and the browser (`answer_host`).

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Entity, EventEmitter, Global};
use kchess_contracts::calls::CoreCall;
use kchess_contracts::types::{self, ProfileArea, ProfileChange};
use kchess_contracts::{Capability, CoreError, HostAsk, HostReply, HostRequest, Level};
use kchess_core::TestSwitches;
use kchess_service::{ConnectError, CoreHandle, Hello, Layout, Mode};
use serde_json::Value;

pub use kchess_contracts::CoreEvent;
pub use kchess_service::ClientEvent;

/// What views subscribe to for core events, and for `ClientEvent::Reconnected`.
pub struct CoreEvents;

impl EventEmitter<ClientEvent> for CoreEvents {}

/// The core and the runtime it runs on.
pub struct CoreBridge {
    runtime: tokio::runtime::Runtime,
    handle: CoreHandle,
}

/// What the person reads when another KChess release has this profile open
/// (`docs/shared-core.md`, "Version check").
fn refusal(error: &ConnectError) -> String {
    let ConnectError::Refused { clients, .. } = error else {
        return error.to_string();
    };
    let ours = kchess_service::protocol::release();
    let Some(other) = clients.first() else {
        return error.to_string();
    };
    let app = match other.kind.as_str() {
        "gpui" => "GPUI",
        kind => kind,
    };
    if kchess_delta::Version::parse(&other.release) > kchess_delta::Version::parse(&ours) {
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
    /// Opens the profile's core in `mode`; a shared one that stops is started again, and its
    /// users start again what they had started there on `ClientEvent::Reconnected`.
    fn open(
        config: kchess_core::Config,
        mode: Mode,
    ) -> Result<
        (
            CoreBridge,
            tokio::sync::mpsc::UnboundedReceiver<ClientEvent>,
        ),
        String,
    > {
        let runtime =
            kchess_service::core_runtime().map_err(|e| format!("The core could not start: {e}"))?;
        let options = kchess_service::Options {
            config,
            hello: Hello {
                kind: "gpui".into(),
                capabilities: vec![Capability::Browser, Capability::Focus],
            },
            mode,
            reconnect: true,
            log: Arc::new(crate::log::write),
            answer: None,
        };
        let (handle, events) = runtime
            .block_on(CoreHandle::open(options))
            .map_err(|error| refusal(&error))?;
        Ok((CoreBridge { runtime, handle }, events))
    }

    /// This app's connection on the core: `origin` of its own writes in `profile:changed`.
    pub fn connection(&self) -> u64 {
        self.handle.connection()
    }

    /// Tells the shared core whether this app's window is focused: the window focused last
    /// raises system notifications.
    pub fn focus(&self, focused: bool) {
        self.handle.focus(focused);
    }

    /// Runs `work` on the core's runtime; the future resolves on any executor.
    fn spawn<T: Send + 'static, W: Future<Output = Result<T, CoreError>> + Send + 'static>(
        &self,
        work: W,
    ) -> impl Future<Output = Result<T, CoreError>> + 'static + use<T, W> {
        let task = self.runtime.spawn(work);
        async move {
            task.await
                .unwrap_or_else(|_| Err(CoreError::new("The KChess core stopped")))
        }
    }

    /// A typed core call (`kchess_contracts::calls`); the future resolves on any executor.
    pub fn request<C: CoreCall + Send + 'static>(
        &self,
        call: C,
    ) -> impl Future<Output = Result<C::Answer, CoreError>> + 'static + use<C>
    where
        C::Answer: Send,
    {
        self.spawn(self.handle.request(call))
    }

    /// A typed core call waited for: only before the window opens, as the app needs its library
    /// and preferences before it draws.
    pub fn request_now<C: CoreCall + Send + 'static>(&self, call: C) -> Result<C::Answer, CoreError>
    where
        C::Answer: Send,
    {
        let answer = self.request(call);
        self.runtime.block_on(answer)
    }

    /// The answer to the host request `id`.
    fn reply(
        &self,
        id: u64,
        reply: HostReply,
    ) -> impl Future<Output = Result<(), CoreError>> + use<> {
        self.spawn(self.handle.reply(id, reply))
    }

    /// Done with the core: an embedded one cancels its work and releases the profile, waiting at
    /// most a few seconds; the shared core closes on its own once no window uses it.
    pub fn close(&self) {
        let handle = self.handle.clone();
        let closing = self.runtime.spawn(async move { handle.close().await });
        let _ = self.runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), closing).await
        });
    }
}

/// Orders a surface's reads of the profile : only the newest
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
    areas: &'static [ProfileArea],
    reload: impl Fn(&mut App) + 'static,
) {
    let events = cx.global::<AppCore>().events.clone();
    cx.subscribe(&events, move |_, event: &ClientEvent, cx| {
        let change = match event {
            ClientEvent::Reconnected => return reload(cx),
            ClientEvent::Event(CoreEvent::ProfileChanged(change)) => change,
            _ => return,
        };
        let Ok(change) = serde_json::from_value::<ProfileChange>(change.clone()) else {
            return;
        };
        if change.origin == cx.global::<AppCore>().bridge.connection() {
            return;
        }
        if change.areas.iter().any(|area| areas.contains(area)) {
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
    /// The test switches the app was started with (`Config::test`).
    pub test: Option<Arc<TestSwitches>>,
}

impl Global for AppCore {}

impl AppCore {
    /// Run a typed core call (`kchess_contracts::calls`): its method with its arguments, and its
    /// answer read in its shape.
    pub fn request<C: CoreCall + Send + 'static>(
        &self,
        call: C,
    ) -> impl Future<Output = Result<C::Answer, CoreError>> + 'static + use<C>
    where
        C::Answer: Send,
    {
        self.bridge.request(call)
    }
}

/// Opens the core for `data_dir` and delivers its events to `CoreEvents` on the UI thread.
pub fn start(data_dir: PathBuf, cx: &mut App) -> Result<(), String> {
    let layout = Layout::current();
    // Tokens are sealed by the core, so this app is never asked to seal or open one.
    let config = layout.config(&data_dir, true);
    let test = config.test.clone().map(Arc::new);
    let (bridge, mut receiver) = CoreBridge::open(config, layout.mode(false)?)?;
    let core = AppCore {
        bridge: Arc::new(bridge),
        events: cx.new(|_| CoreEvents),
        test,
    };
    cx.set_global(core.clone());
    cx.spawn(async move |cx| {
        while let Some(event) = receiver.recv().await {
            if let ClientEvent::Event(CoreEvent::HostRequest(payload)) = &event {
                if let Ok(ask) = serde_json::from_value::<HostAsk>(payload.clone()) {
                    cx.update(|cx| answer_host(ask, cx));
                }
                continue;
            }
            if let ClientEvent::Event(CoreEvent::AlertsOwner(owner)) = &event {
                let owner = serde_json::from_value::<types::AlertOwner>(owner.clone())
                    .ok()
                    .and_then(|owner| owner.connection);
                cx.update(|cx| cx.set_global(AlertOwner(owner)));
            }
            core.events.update(cx, |_, cx| cx.emit(event));
        }
    })
    .detach();
    Ok(())
}

/// The answer to a host request, given on the UI thread.
fn answer_host(ask: HostAsk, cx: &mut App) {
    let reply = match ask.request {
        // With a local stand-in for Lichess (the test switches), the page a sign-in opens is
        // written to the profile for the test to follow, instead of the person's browser.
        HostRequest::OpenExternal(url)
            if cx
                .global::<AppCore>()
                .test
                .as_ref()
                .is_some_and(|test| test.lichess_origin.is_some()) =>
        {
            let line = format!("{url}\n");
            let written = crate::profile::data_dir().and_then(|dir| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dir.join("opened-urls.txt"))
                    .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()))
            });
            match written {
                Ok(()) => HostReply::done(),
                Err(cause) => HostReply::Error(cause.to_string()),
            }
        }
        HostRequest::OpenExternal(url) if kchess_host::opens_in_browser(&url) => {
            cx.open_url(&url);
            HostReply::done()
        }
        HostRequest::OpenExternal(_) => HostReply::Error(kchess_host::NOT_A_WEB_PAGE.into()),
        HostRequest::Focus => {
            cx.activate(true);
            HostReply::done()
        }
        HostRequest::OnBattery => HostReply::Value(Value::Bool(crate::power::on_battery())),
        request => HostReply::Error(format!("Unknown host capability {}", request.kind())),
    };
    let delivery = cx.global::<AppCore>().bridge.reply(ask.id, reply);
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
    use super::{Answer, Reads};

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
