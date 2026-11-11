//! The shared core service (`docs/shared-core.md`): opens the profile's core, writes
//! `core.token` and `core.json`, and serves every client over the local socket until the last
//! one has been gone for the idle period.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kchess_contracts::CoreEvent;
use kchess_core::connection::ClientInfo;
use kchess_core::{Config, Core, Host, Level};
use tokio::sync::{Notify, Semaphore, mpsc};
use tokio::task::JoinHandle;

use crate::discovery::{self, CoreInfo};
use crate::protocol::{
    self, ClientSummary, Frame, MAX_HELLO, RefuseReason, read_frame, read_frame_within, write_frame,
};
use crate::transport::{self, Listener, Stream};

/// How long a client has to say `hello`.
const HELLO_DEADLINE: Duration = Duration::from_secs(5);
/// Connections that have not yet sent their `hello`; one past this is closed at once.
const HANDSHAKES: usize = 32;
/// Frames waiting for a client that reads slowly.
const QUEUE: usize = 1024;
/// How long a leaving client's queued frames may take to flush.
const FLUSH_DEADLINE: Duration = Duration::from_secs(2);
/// The service log is started afresh beyond this size.
const LOG_LIMIT: u64 = 5 * 1024 * 1024;

/// How the service runs.
pub struct ServeOptions {
    /// The core's configuration; `data_dir` is the profile.
    pub config: Config,
    /// How long the service waits after its last client leaves, or for its first client.
    pub idle: Duration,
}

/// A client's queue of frames, and what cuts it off.
type Outlet = (mpsc::Sender<Frame>, Arc<Notify>);

/// The core's host in the service: events become frames for the connections they are for.
struct ServiceHost {
    /// Each client's queue, and what cuts it off.
    clients: Mutex<HashMap<u64, Outlet>>,
    log: Mutex<Option<std::fs::File>>,
}

impl ServiceHost {
    fn new(profile: &std::path::Path) -> ServiceHost {
        let path = profile.join("logs").join("service.log");
        let log = std::fs::create_dir_all(profile.join("logs"))
            .ok()
            .and_then(|()| {
                let fresh = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > LOG_LIMIT);
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(!fresh)
                    .write(true)
                    .truncate(fresh)
                    .open(&path)
                    .ok()
            });
        ServiceHost {
            clients: Mutex::new(HashMap::new()),
            log: Mutex::new(log),
        }
    }

    fn add(&self, connection: u64, sender: mpsc::Sender<Frame>, cut: Arc<Notify>) {
        if let Ok(mut clients) = self.clients.lock() {
            clients.insert(connection, (sender, cut));
        }
    }

    fn remove(&self, connection: u64) {
        if let Ok(mut clients) = self.clients.lock() {
            clients.remove(&connection);
        }
    }

    /// Stops sending to a client and closes its connection.
    fn cut_off(&self, connection: u64) {
        let removed = self
            .clients
            .lock()
            .ok()
            .and_then(|mut clients| clients.remove(&connection));
        if let Some((_, cut)) = removed {
            cut.notify_one();
        }
    }

    /// Queues an event for one client. A full queue drops a progress event
    /// (`CoreEvent::coalesced`); for any other event, a terminal update included, the client is
    /// cut off, and reconnects to read the state afresh.
    fn send(&self, connection: u64, sender: &mpsc::Sender<Frame>, event: CoreEvent) {
        let droppable = event.coalesced();
        let name = event.name();
        if let Err(mpsc::error::TrySendError::Full(_)) = sender.try_send(frame(event))
            && !droppable
        {
            self.log(
                Level::Warn,
                "service",
                &format!("Client {connection} fell behind on {name}; disconnecting it."),
            );
            self.cut_off(connection);
        }
    }
}

fn frame(event: CoreEvent) -> Frame {
    Frame::Event {
        name: event.name().to_string(),
        payload: event.into_payload(),
    }
}

impl Host for ServiceHost {
    fn log(&self, level: Level, scope: &str, message: &str) {
        if let Ok(mut log) = self.log.lock()
            && let Some(file) = log.as_mut()
        {
            let at = kchess_core::time::now_ms();
            let message = kchess_core::redact::redact(message);
            // One write per line: `writeln!` on an unbuffered file writes each piece separately.
            let line = format!("{at} {} [{scope}] {message}\n", level.name());
            let _ = file.write_all(line.as_bytes());
        }
    }

    fn emit(&self, event: CoreEvent) {
        let clients: Vec<(u64, mpsc::Sender<Frame>)> = self
            .clients
            .lock()
            .map(|clients| {
                clients
                    .iter()
                    .map(|(id, (tx, _))| (*id, tx.clone()))
                    .collect()
            })
            .unwrap_or_default();
        for (connection, sender) in clients {
            self.send(connection, &sender, event.clone());
        }
    }

    fn emit_to(&self, connection: u64, event: CoreEvent) {
        let sender = self
            .clients
            .lock()
            .ok()
            .and_then(|clients| clients.get(&connection).map(|(tx, _)| tx.clone()));
        if let Some(sender) = sender {
            self.send(connection, &sender, event);
        }
    }
}

struct State {
    core: Arc<Core>,
    host: Arc<ServiceHost>,
    token: String,
    release: String,
    protocol: String,
    active: AtomicUsize,
    /// Room for connections still in their handshake (`HANDSHAKES`).
    handshakes: Semaphore,
    /// A client connected or left: the idle countdown starts or stops.
    changed: Notify,
    shutdown: Notify,
    /// Set, under its lock, when a sole client of another release closes this core: admission
    /// takes the same lock, so no client is welcomed into a core that is closing.
    closing: Mutex<bool>,
}

/// Opens the profile's core and serves it until it has been idle for `options.idle`. Fails when
/// another core holds the profile (`A KChess core is already running for this profile.`).
pub async fn serve(options: ServeOptions) -> Result<(), String> {
    let profile: PathBuf = discovery::canonical(&options.config.data_dir)
        .map_err(|error| format!("The profile is unavailable: {error}"))?;
    let host = Arc::new(ServiceHost::new(&profile));
    let mut config = options.config;
    config.data_dir = profile.clone();
    let core = Core::open(config, Arc::clone(&host) as Arc<dyn Host>).map_err(|e| e.message)?;
    core.serve_connections();
    let core = Arc::new(core);

    // The profile's lock is held (`Core::open`), so any discovery files are a dead core's: gone
    // before binding, a client never sends a token to whatever holds the socket's name instead.
    let _ = std::fs::remove_file(profile.join(discovery::INFO_FILE));
    let _ = std::fs::remove_file(profile.join(discovery::TOKEN_FILE));
    let socket = discovery::socket_path(&profile)
        .map_err(|error| format!("The socket folder is unavailable: {error}"))?;
    let listener =
        Listener::bind(&socket).map_err(|error| format!("The socket could not open: {error}"))?;
    let token = discovery::new_token().map_err(|error| format!("No token: {error}"))?;
    discovery::write_private(&profile.join(discovery::TOKEN_FILE), token.as_bytes())
        .map_err(|error| format!("The token could not be written: {error}"))?;
    let info = CoreInfo {
        pid: std::process::id(),
        socket: socket.to_string_lossy().to_string(),
        release: protocol::release(),
        protocol: protocol::protocol(),
        started_at: u64::try_from(kchess_core::time::now_ms()).unwrap_or(0),
    };
    let text = serde_json::to_vec_pretty(&info).map_err(|error| error.to_string())?;
    discovery::write_private(&profile.join(discovery::INFO_FILE), &text)
        .map_err(|error| format!("core.json could not be written: {error}"))?;
    host.log(
        Level::Info,
        "service",
        &format!(
            "Serving {} ({} {})",
            profile.display(),
            info.release,
            info.protocol
        ),
    );

    let state = Arc::new(State {
        core: Arc::clone(&core),
        host: Arc::clone(&host),
        token,
        release: info.release.clone(),
        protocol: info.protocol.clone(),
        active: AtomicUsize::new(0),
        handshakes: Semaphore::new(HANDSHAKES),
        changed: Notify::new(),
        shutdown: Notify::new(),
        closing: Mutex::new(false),
    });
    tokio::spawn(idle_watch(Arc::clone(&state), options.idle));

    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, same_user)) => {
                    let state = Arc::clone(&state);
                    tokio::spawn(async move { handle(state, stream, same_user).await });
                }
                Err(error) => host.log(Level::Warn, "service", &format!("Accept failed: {error}")),
            },
            () = state.shutdown.notified() => break,
            () = terminated() => break,
        }
    }

    host.log(Level::Info, "service", "Closing.");
    core.close().await;
    transport::remove(&socket);
    if discovery::read_info(&profile).is_some_and(|written| written.pid == std::process::id()) {
        let _ = std::fs::remove_file(profile.join(discovery::INFO_FILE));
        let _ = std::fs::remove_file(profile.join(discovery::TOKEN_FILE));
    }
    Ok(())
}

/// Resolves when the system asks the service to stop.
async fn terminated() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Stops the service once no client has been connected for `idle`.
async fn idle_watch(state: Arc<State>, idle: Duration) {
    loop {
        if state.active.load(Ordering::SeqCst) == 0 {
            tokio::select! {
                () = tokio::time::sleep(idle) => {
                    // Decided with admission held off, as a version handoff is: a client
                    // welcomed at the deadline keeps the core open.
                    let mut closing =
                        state.closing.lock().unwrap_or_else(|poison| poison.into_inner());
                    if state.active.load(Ordering::SeqCst) == 0 {
                        *closing = true;
                        drop(closing);
                        state.shutdown.notify_one();
                        return;
                    }
                }
                () = state.changed.notified() => {}
            }
        } else {
            state.changed.notified().await;
        }
    }
}

/// Why a client of another protocol is refused: its release, or, when both sides are the same
/// release, a different build of it (a development build after the contracts changed).
fn version_refusal(core: &str, client: &str) -> String {
    if core == client {
        format!(
            "Another build of KChess {core} runs this profile's core. Quit it to open this app."
        )
    } else {
        format!("KChess {core} runs this profile's core; this app is {client}.")
    }
}

/// One client, from `hello` to its last frame.
async fn handle(state: Arc<State>, stream: Stream, same_user: bool) {
    let Ok(handshake) = state.handshakes.try_acquire() else {
        return;
    };
    let (mut reader, mut writer) = tokio::io::split(stream);
    let Ok(Ok(Some(hello))) =
        tokio::time::timeout(HELLO_DEADLINE, read_frame_within(&mut reader, MAX_HELLO)).await
    else {
        return;
    };
    drop(handshake);
    let Frame::Hello {
        token,
        kind,
        release,
        protocol,
        capabilities,
    } = hello
    else {
        return;
    };
    if !same_user || !discovery::same_token(&token, &state.token) {
        let _ = write_frame(
            &mut writer,
            &Frame::Refuse {
                reason: RefuseReason::Unauthorized,
                message: "This client may not use the KChess core.".into(),
                sole: false,
                clients: Vec::new(),
            },
        )
        .await;
        return;
    }
    if protocol != state.protocol {
        // Decided with admission held off: a client welcomed meanwhile keeps the core open.
        let (clients, sole) = {
            let mut closing = state
                .closing
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let clients: Vec<ClientSummary> = state
                .core
                .connections()
                .ids()
                .into_iter()
                .filter_map(|id| state.core.connections().info(id))
                .map(|info| ClientSummary {
                    kind: info.kind,
                    release: info.release,
                })
                .collect();
            let sole = clients.is_empty();
            *closing |= sole;
            (clients, sole)
        };
        let _ = write_frame(
            &mut writer,
            &Frame::Refuse {
                reason: RefuseReason::Version,
                message: version_refusal(&state.release, &release),
                sole,
                clients,
            },
        )
        .await;
        if sole {
            // Nobody else uses this core: it closes so the client can start its own release's.
            state.shutdown.notify_one();
        }
        return;
    }

    let connection = {
        let closing = state
            .closing
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if *closing {
            // The core closes for another release; this client starts or finds the next one.
            return;
        }
        let connection = state.core.connect(ClientInfo {
            kind,
            release,
            capabilities,
        });
        // Counted before the lock is let go, so an idle check that follows sees this client.
        state.active.fetch_add(1, Ordering::SeqCst);
        connection
    };
    let (sender, mut queue) = mpsc::channel::<Frame>(QUEUE);
    let cut = Arc::new(Notify::new());
    // `welcome` comes first, then which window alerts now, then every event: a client that joins
    // while another window keeps focus learns the owner although it does not change.
    let _ = sender.try_send(Frame::Welcome {
        connection,
        release: state.release.clone(),
        protocol: state.protocol.clone(),
    });
    state.core.with_alert_owner(|owner| {
        let _ = sender.try_send(frame(owner));
        state.host.add(connection, sender.clone(), Arc::clone(&cut));
    });
    state.changed.notify_one();
    let mut writing = tokio::spawn(async move {
        while let Some(frame) = queue.recv().await {
            if write_frame(&mut writer, &frame).await.is_err() {
                break;
            }
        }
    });

    let calls: Arc<Mutex<HashMap<u64, JoinHandle<()>>>> = Arc::default();
    // Why the loop ended: a cut-off client's queue is not flushed, and a writer that failed is done.
    let (mut behind, mut written) = (false, false);
    loop {
        let frame = tokio::select! {
            read = read_frame(&mut reader) => match read {
                Ok(Some(frame)) => frame,
                _ => break,
            },
            () = cut.notified() => {
                behind = true;
                break;
            }
            _ = &mut writing => {
                written = true;
                break;
            }
        };
        match frame {
            Frame::Call { id, method, args } => {
                let (core, sender, running) =
                    (Arc::clone(&state.core), sender.clone(), Arc::clone(&calls));
                // Held until the call is registered: another worker may finish a quick call
                // before `spawn` returns, and its `remove` must find the entry, or the entry
                // (and the finished task it keeps allocated) would stay until the window closes.
                let Ok(mut registered) = calls.lock() else {
                    break;
                };
                let task = tokio::spawn(async move {
                    let answer = core.call_as(connection, &method, args).await;
                    let still_ours = running
                        .lock()
                        .is_ok_and(|mut running| running.remove(&id).is_some());
                    if !still_ours {
                        return;
                    }
                    let frame = match answer {
                        Ok(value) => Frame::Result {
                            id,
                            value: Some(value),
                            error: None,
                            kind: None,
                        },
                        Err(error) => Frame::Result {
                            id,
                            value: None,
                            kind: error.kind.map(|kind| kind.as_str().to_string()),
                            error: Some(error.message),
                        },
                    };
                    let _ = sender.send(frame).await;
                });
                // A reused id replaces nothing still running: the earlier call is stopped.
                if let Some(earlier) = registered.insert(id, task) {
                    earlier.abort();
                }
                drop(registered);
            }
            Frame::Cancel { id } => {
                let task = calls
                    .lock()
                    .ok()
                    .and_then(|mut running| running.remove(&id));
                if let Some(task) = task {
                    task.abort();
                    let cancelled = sender.try_send(Frame::Result {
                        id,
                        value: None,
                        error: Some("The call was cancelled.".into()),
                        kind: None,
                    });
                    if cancelled.is_err() {
                        behind = true;
                        break;
                    }
                }
            }
            Frame::Focus { focused } => state.core.focus(connection, focused),
            // A client without a window stops only its own work, not the other clients'.
            Frame::Suspend => state.core.suspend_connection(connection),
            _ => {}
        }
    }

    stop_calls(&calls).await;
    state.host.remove(connection);
    drop(sender);
    // A client that stopped reading can hold a write open forever: what it started still ends.
    if !written
        && (behind
            || tokio::time::timeout(FLUSH_DEADLINE, &mut writing)
                .await
                .is_err())
    {
        writing.abort();
    }
    state.core.disconnect(connection);
    state.active.fetch_sub(1, Ordering::SeqCst);
    state.changed.notify_one();
}

/// Stops a connection's calls and waits until none is running. An abort lands at a call's next
/// await: one being run on another worker right now could otherwise start this connection's
/// analysis (or a watch) after `disconnect` forgot it, and nothing would ever stop that engine.
/// Each ends at its next await, so the wait is short.
async fn stop_calls(calls: &Mutex<HashMap<u64, JoinHandle<()>>>) {
    let stopping: Vec<JoinHandle<()>> = calls
        .lock()
        .map(|mut running| running.drain().map(|(_, task)| task).collect())
        .unwrap_or_default();
    for task in &stopping {
        task.abort();
    }
    for task in stopping {
        let _ = task.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Two releases are named; one release with two protocols is said to be two builds of it.
    #[test]
    fn a_version_refusal_names_what_differs() {
        assert_eq!(
            version_refusal("2026.11.5", "2026.11.4"),
            "KChess 2026.11.5 runs this profile's core; this app is 2026.11.4."
        );
        assert_eq!(
            version_refusal("2026.11.5", "2026.11.5"),
            "Another build of KChess 2026.11.5 runs this profile's core. Quit it to open this app."
        );
    }

    /// A log past its limit starts afresh; a smaller one is added to.
    #[test]
    fn the_service_log_starts_afresh_past_its_limit() {
        let profile = tempfile::tempdir().expect("profile");
        let path = profile.path().join("logs").join("service.log");
        std::fs::create_dir_all(path.parent().expect("logs")).expect("logs");
        let kept = 2 * 1024 * 1024;
        std::fs::write(&path, vec![b'.'; kept]).expect("log");
        ServiceHost::new(profile.path()).log(Level::Info, "test", "kept");
        let length = std::fs::metadata(&path).expect("log").len();
        assert!(length > kept as u64, "{length}");

        std::fs::write(&path, vec![b'.'; LOG_LIMIT as usize + 1]).expect("log");
        ServiceHost::new(profile.path()).log(Level::Info, "test", "afresh");
        let text = std::fs::read_to_string(&path).expect("log");
        assert!(
            text.len() < 1024 && text.contains("afresh"),
            "{}",
            text.len()
        );
    }

    /// A client that falls behind misses progress, but is cut off rather than miss anything else.
    #[test]
    fn a_full_queue_drops_only_progress() {
        let host = ServiceHost {
            clients: Mutex::new(HashMap::new()),
            log: Mutex::new(None),
        };
        let connected = |host: &ServiceHost| host.clients.lock().expect("clients").contains_key(&1);
        let (sender, _queue) = mpsc::channel(1);
        host.add(1, sender.clone(), Arc::new(Notify::new()));
        host.send(1, &sender, CoreEvent::WatchFrame(json!({})));
        host.send(1, &sender, CoreEvent::WatchFrame(json!({})));
        assert!(connected(&host), "a progress frame is dropped");
        host.send(1, &sender, CoreEvent::Analysis(json!({ "done": true })));
        assert!(!connected(&host), "a terminal update is never dropped");
    }

    /// A call in the middle of a step when its connection goes finishes that step before the
    /// connection's state is forgotten, so it cannot recreate it (an engine nothing would stop).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_connection_waits_for_the_call_it_stops() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let (started, stepped) = (Arc::new(Notify::new()), Arc::new(AtomicBool::new(false)));
        let calls: Mutex<HashMap<u64, JoinHandle<()>>> = Mutex::default();
        let task = tokio::spawn({
            let (started, stepped) = (Arc::clone(&started), Arc::clone(&stepped));
            async move {
                started.notify_one();
                // A step without an await, as `startAnalysis` opening its engine is.
                std::thread::sleep(std::time::Duration::from_millis(200));
                stepped.store(true, Ordering::SeqCst);
                std::future::pending::<()>().await;
            }
        });
        calls.lock().expect("calls").insert(1, task);
        started.notified().await;
        stop_calls(&calls).await;
        assert!(stepped.load(Ordering::SeqCst), "the step ended first");
        assert!(calls.lock().expect("calls").is_empty());
    }
}
