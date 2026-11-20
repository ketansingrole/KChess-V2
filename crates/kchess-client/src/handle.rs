//! The profile's core as a frontend holds it (`docs/shared-core.md`): the shared core service,
//! started when none runs, or a core embedded in the frontend's process. Either way a frontend
//! calls it by `CoreApi` method or typed call, hears its events on one channel, and answers its
//! host requests in one place: as events, or through the `answer` it opened the core with.
//!
//! An embedded core is the host's to make (`Options::embed`, `kchess_service::embed`): the handle
//! knows it only as an `EmbeddedCore`, so the crates a frontend is built from never build the core.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use kchess_contracts::calls::CoreCall;
use kchess_contracts::{CoreError, CoreEvent, HostReply, HostRequest, Level};
use kchess_host::config::Config;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::client::{Client, ClientEvent, ConnectError, Hello, Start, connect_or_start};

const STOPPED: &str = "The KChess core stopped.";

/// The connection an embedded core gives its one frontend (`kchess_core::connection::APP`):
/// the `origin` of that frontend's writes.
pub const EMBEDDED_CONNECTION: u64 = 0;

/// A future an embedded core answers with.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// A core in this process, as the handle uses it.
pub trait EmbeddedCore: Send + Sync + 'static {
    /// A `CoreApi` method by name.
    fn call(
        self: Arc<Self>,
        method: String,
        args: Vec<Value>,
    ) -> BoxFuture<Result<Value, CoreError>>;
    /// The answer to the host request `id`.
    fn host_reply(&self, id: u64, reply: HostReply) -> Result<(), CoreError>;
    /// Whether the frontend's window is focused.
    fn focus(&self, focused: bool);
    /// The frontend has no window open: what it started stops.
    fn suspend(&self);
    /// Cancels the core's work and releases the profile.
    fn close(self: Arc<Self>) -> BoxFuture<()>;
}

/// What an embedded core opens with.
pub struct Embedding {
    pub config: Config,
    pub hello: Hello,
    /// Where its events go.
    pub events: mpsc::UnboundedSender<ClientEvent>,
    /// Where it logs.
    pub log: Log,
}

/// Opens a core in this process, or says why it could not: the host's (`kchess_service::embed`).
pub type Embed =
    Arc<dyn Fn(Embedding) -> Result<Arc<dyn EmbeddedCore>, String> + Send + Sync + 'static>;
/// The first wait before starting a stopped core again, doubled up to `RESTART_LIMIT`.
const RESTART_FIRST: Duration = Duration::from_millis(200);
const RESTART_LIMIT: Duration = Duration::from_secs(5);

/// The stack of every thread the core runs on: what macOS and Linux give a main thread. Windows
/// gives its main thread 1 MB, and a sign-in overflowed a smaller stack in a debug build; in the
/// shared service an overflow aborts the core for every window and command at once.
pub const CORE_STACK: usize = 8 * 1024 * 1024;

/// The runtime a frontend (or the service) runs its core on.
pub fn core_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("kchess-core")
        .thread_stack_size(CORE_STACK)
        .build()
}

/// Where a frontend writes a log line.
pub type Log = Arc<dyn Fn(Level, &str, &str) + Send + Sync>;
/// A frontend's answer to a host request, given where it arrives.
pub type Answer = Arc<dyn Fn(HostRequest) -> HostReply + Send + Sync>;

/// Where the core runs.
pub enum Mode {
    /// In this process, for this frontend alone.
    Embedded,
    /// The profile's shared core, started from `binary` when none runs.
    Service { binary: PathBuf },
}

/// How a frontend opens its core.
pub struct Options {
    pub config: Config,
    pub hello: Hello,
    pub mode: Mode,
    /// How to open the core in this process, when `mode` says it runs there.
    pub embed: Embed,
    /// When the shared core stops, start it again and carry on with a new connection, reported
    /// as `ClientEvent::Reconnected`; otherwise `ClientEvent::Closed` ends the events.
    pub reconnect: bool,
    /// The embedded core's log lines, and the handle's own.
    pub log: Log,
    /// Answers host requests as they arrive, for a frontend without a thread of its own to answer
    /// them on; without it they arrive as `CoreEvent::HostRequest` events.
    pub answer: Option<Answer>,
}

#[derive(Clone)]
enum Backend {
    Embedded(Arc<dyn EmbeddedCore>),
    /// The connection, replaced when the core restarts; `None` once closed.
    Service(Arc<Mutex<Option<Client>>>),
}

/// What the event pump keeps of the handle, without keeping it open.
enum WeakBackend {
    Embedded(Weak<dyn EmbeddedCore>),
    Service(Weak<Mutex<Option<Client>>>),
}

/// A frontend's core. Cloning shares it.
#[derive(Clone)]
pub struct CoreHandle {
    backend: Backend,
    /// The window's focus as last reported (0 unknown, 1 blurred, 2 focused), sent again on each
    /// new connection to the shared core.
    focused: Arc<AtomicU8>,
}

impl CoreHandle {
    /// Opens the profile's core (`options.config.data_dir`) and starts delivering its events.
    /// Must run in a Tokio runtime (`core_runtime`), where the core and its event pump then run.
    pub async fn open(
        options: Options,
    ) -> Result<(CoreHandle, mpsc::UnboundedReceiver<ClientEvent>), ConnectError> {
        let Options {
            config,
            hello,
            mode,
            embed,
            reconnect,
            log,
            answer,
        } = options;
        let (events, received) = mpsc::unbounded_channel();
        let focused: Arc<AtomicU8> = Arc::default();
        match mode {
            Mode::Embedded => {
                let (sender, incoming) = mpsc::unbounded_channel();
                let core = embed(Embedding {
                    config,
                    hello,
                    events: sender,
                    log,
                })
                .map_err(ConnectError::Failed)?;
                let weak = WeakBackend::Embedded(Arc::downgrade(&core));
                tokio::spawn(pump(weak, incoming, events, answer));
                let handle = CoreHandle {
                    backend: Backend::Embedded(core),
                    focused,
                };
                Ok((handle, received))
            }
            Mode::Service { binary } => {
                let profile = config.data_dir.clone();
                let start = Start { binary, config };
                let (client, incoming) = connect_or_start(&profile, &hello, &start).await?;
                let slot = Arc::new(Mutex::new(Some(client)));
                let restart = reconnect.then(|| Restart {
                    profile,
                    hello,
                    start,
                    log,
                    focused: Arc::clone(&focused),
                });
                let weak = WeakBackend::Service(Arc::downgrade(&slot));
                tokio::spawn(service_pump(weak, incoming, events, answer, restart));
                let handle = CoreHandle {
                    backend: Backend::Service(slot),
                    focused,
                };
                Ok((handle, received))
            }
        }
    }

    fn client(&self) -> Option<Client> {
        match &self.backend {
            Backend::Embedded(_) => None,
            Backend::Service(slot) => lock(slot).clone(),
        }
    }

    /// This frontend's connection on the core: the `origin` of its own writes in
    /// `profile:changed`.
    pub fn connection(&self) -> u64 {
        self.client()
            .map_or(EMBEDDED_CONNECTION, |client| client.connection())
    }

    /// A `CoreApi` method by name.
    pub fn call(
        &self,
        method: &str,
        args: Vec<Value>,
    ) -> impl Future<Output = Result<Value, CoreError>> + Send + 'static + use<> {
        let method = method.to_string();
        let (core, client) = match &self.backend {
            Backend::Embedded(core) => (Some(Arc::clone(core)), None),
            Backend::Service(slot) => (None, lock(slot).clone()),
        };
        async move {
            match (core, client) {
                (Some(core), _) => core.call(method, args).await,
                (None, Some(client)) => client.call(&method, args).await,
                (None, None) => Err(CoreError::new(STOPPED)),
            }
        }
    }

    /// A typed core call (`kchess_contracts::calls`): its method with its arguments, and its
    /// answer read in its shape.
    pub fn request<C: CoreCall>(
        &self,
        call: C,
    ) -> impl Future<Output = Result<C::Answer, CoreError>> + Send + 'static + use<C> {
        let answer = call.args().map(|args| self.call(C::METHOD, args));
        async move { kchess_contracts::calls::decode::<C>(answer?.await?) }
    }

    pub fn reply(
        &self,
        id: u64,
        reply: HostReply,
    ) -> impl Future<Output = Result<(), CoreError>> + Send + 'static + use<> {
        let backend = self.backend.clone();
        async move { reply_to(&backend, id, reply).await }
    }

    /// Tells the core whether this frontend's window is focused: the window focused last raises
    /// system notifications.
    pub fn focus(&self, focused: bool) {
        self.focused
            .store(if focused { 2 } else { 1 }, Ordering::SeqCst);
        match &self.backend {
            Backend::Embedded(core) => core.focus(focused),
            Backend::Service(_) => {
                if let Some(client) = self.client() {
                    client.focus(focused);
                }
            }
        }
    }

    /// This frontend has no window open: what it started (an analysis, a watch, voice input)
    /// stops, and every other client's work carries on.
    pub fn suspend(&self) {
        match &self.backend {
            Backend::Embedded(core) => core.suspend(),
            Backend::Service(_) => {
                if let Some(client) = self.client() {
                    client.suspend();
                }
            }
        }
    }

    /// This frontend is done with its core: an embedded core cancels its work and releases the
    /// profile; the shared core forgets this connection, and closes on its own once no client
    /// has used it for a while. Later calls fail.
    pub async fn close(&self) {
        match &self.backend {
            Backend::Embedded(core) => Arc::clone(core).close().await,
            Backend::Service(slot) => drop(lock(slot).take()),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

async fn reply_to(backend: &Backend, id: u64, reply: HostReply) -> Result<(), CoreError> {
    match backend {
        Backend::Embedded(core) => core.host_reply(id, reply),
        Backend::Service(slot) => {
            let client = lock(slot).clone().ok_or_else(|| CoreError::new(STOPPED))?;
            let reply = serde_json::to_value(reply)
                .map_err(|cause| CoreError::wrap("host reply", cause))?;
            client
                .call(kchess_contracts::host::REPLY_METHOD, vec![json!(id), reply])
                .await
                .map(|_| ())
        }
    }
}

impl WeakBackend {
    fn upgrade(&self) -> Option<Backend> {
        match self {
            WeakBackend::Embedded(core) => core.upgrade().map(Backend::Embedded),
            WeakBackend::Service(slot) => slot.upgrade().map(Backend::Service),
        }
    }
}

/// Hands `event` on, or answers it here when it is a host request and `answer` is given.
async fn deliver(
    backend: &WeakBackend,
    event: ClientEvent,
    events: &mpsc::UnboundedSender<ClientEvent>,
    answer: Option<&Answer>,
) {
    if let (Some(answer), ClientEvent::Event(CoreEvent::HostRequest(ask))) = (answer, &event) {
        let reply = answer(ask.request.clone());
        if let Some(backend) = backend.upgrade() {
            // The request has a deadline: a reply that does not arrive fails it there.
            let _ = reply_to(&backend, ask.id, reply).await;
        }
        return;
    }
    let _ = events.send(event);
}

/// An embedded core's events, until it is gone.
async fn pump(
    backend: WeakBackend,
    mut incoming: mpsc::UnboundedReceiver<ClientEvent>,
    events: mpsc::UnboundedSender<ClientEvent>,
    answer: Option<Answer>,
) {
    while let Some(event) = incoming.recv().await {
        deliver(&backend, event, &events, answer.as_ref()).await;
    }
}

struct Restart {
    profile: PathBuf,
    hello: Hello,
    start: Start,
    log: Log,
    focused: Arc<AtomicU8>,
}

impl Restart {
    /// Connects again, retrying with a growing wait until it does or the handle is gone.
    async fn reconnect(
        &self,
        slot: &Weak<Mutex<Option<Client>>>,
    ) -> Option<mpsc::UnboundedReceiver<ClientEvent>> {
        (self.log)(Level::Warn, "core", "The core stopped; reconnecting.");
        let mut delay = RESTART_FIRST;
        loop {
            match connect_or_start(&self.profile, &self.hello, &self.start).await {
                Ok((client, incoming)) => {
                    match self.focused.load(Ordering::SeqCst) {
                        0 => {}
                        known => client.focus(known == 2),
                    }
                    let slot = slot.upgrade()?;
                    let mut current = lock(&slot);
                    // Closed meanwhile: nothing to carry on with.
                    current.as_ref()?;
                    *current = Some(client);
                    return Some(incoming);
                }
                Err(error) => {
                    (self.log)(
                        Level::Warn,
                        "core",
                        &format!("The core could not restart: {error}"),
                    );
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(RESTART_LIMIT);
                }
            }
            slot.upgrade()?;
        }
    }
}

/// The shared core's events; on `Closed`, reconnects when `restart` says how.
async fn service_pump(
    backend: WeakBackend,
    mut incoming: mpsc::UnboundedReceiver<ClientEvent>,
    events: mpsc::UnboundedSender<ClientEvent>,
    answer: Option<Answer>,
    restart: Option<Restart>,
) {
    loop {
        while let Some(event) = incoming.recv().await {
            if event == ClientEvent::Closed {
                break;
            }
            deliver(&backend, event, &events, answer.as_ref()).await;
        }
        let slot = match &backend {
            WeakBackend::Service(slot) => slot,
            WeakBackend::Embedded(_) => return,
        };
        let closed = slot.upgrade().is_none_or(|slot| lock(&slot).is_none());
        let Some(restart) = restart.as_ref().filter(|_| !closed) else {
            let _ = events.send(ClientEvent::Closed);
            return;
        };
        match restart.reconnect(slot).await {
            Some(again) => {
                incoming = again;
                let _ = events.send(ClientEvent::Reconnected);
            }
            None => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-11: the app built its own runtime on the default 2 MiB worker stack, so its
    /// embedded core ran on less than the command line's and the service's. Every frontend's core
    /// runs on `core_runtime`, whose threads hold a deep call.
    #[test]
    fn the_core_runtime_holds_a_deep_call() {
        #[inline(never)]
        fn deep(depth: usize) -> u8 {
            let frame = std::hint::black_box([depth as u8; 64 * 1024]);
            if depth == 0 {
                frame[0]
            } else {
                deep(depth - 1).wrapping_add(frame[frame.len() - 1])
            }
        }
        let runtime = core_runtime().expect("runtime");
        // Past the default 2 MiB worker stack (at least 3 MiB of frames), within the core's.
        let answer = runtime.block_on(async { tokio::spawn(async { deep(48) }).await });
        assert!(answer.is_ok());
    }
}
