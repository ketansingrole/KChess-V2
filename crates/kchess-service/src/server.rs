//! The shared core service (`docs/shared-core.md`): opens the profile's core, writes
//! `core.token` and `core.json`, and serves every client over the local socket until the last
//! one has been gone for the idle period.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kchess_core::connection::ClientInfo;
use kchess_core::{Config, Core, Host, Level};
use serde_json::Value;
use tokio::sync::{Notify, mpsc};
use tokio::task::AbortHandle;

use crate::discovery::{self, CoreInfo};
use crate::protocol::{self, ClientSummary, Frame, RefuseReason, read_frame, write_frame};
use crate::transport::{self, Listener, Stream};

/// How long a client has to say `hello`.
const HELLO_DEADLINE: Duration = Duration::from_secs(5);
/// Frames waiting for a client that reads slowly.
const QUEUE: usize = 1024;
/// Events a slow client may miss: the next one carries the whole state.
const COALESCED: [&str; 3] = ["engine:analysis", "watch:frame", "puzzledb:progress"];
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

    /// Queues an event for one client. A full queue drops a coalesced event; for any other event
    /// the client is cut off, and reconnects to read the state afresh.
    fn send(&self, connection: u64, sender: &mpsc::Sender<Frame>, event: &str, payload: Value) {
        let frame = Frame::Event {
            name: event.to_string(),
            payload,
        };
        if let Err(mpsc::error::TrySendError::Full(_)) = sender.try_send(frame)
            && !COALESCED.contains(&event)
        {
            self.log(
                Level::Warn,
                "service",
                &format!("Client {connection} fell behind on {event}; disconnecting it."),
            );
            self.cut_off(connection);
        }
    }
}

impl Host for ServiceHost {
    fn log(&self, level: Level, scope: &str, message: &str) {
        if let Ok(mut log) = self.log.lock()
            && let Some(file) = log.as_mut()
        {
            let at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_millis());
            let _ = writeln!(file, "{at} {} [{scope}] {message}", level.name());
        }
    }

    fn emit(&self, event: &str, payload: Value) {
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
            self.send(connection, &sender, event, payload.clone());
        }
    }

    fn emit_to(&self, connection: u64, event: &str, payload: Value) {
        let sender = self
            .clients
            .lock()
            .ok()
            .and_then(|clients| clients.get(&connection).map(|(tx, _)| tx.clone()));
        if let Some(sender) = sender {
            self.send(connection, &sender, event, payload);
        }
    }
}

/// What every connection shares.
struct State {
    core: Arc<Core>,
    host: Arc<ServiceHost>,
    token: String,
    release: String,
    protocol: String,
    /// Connected clients.
    active: AtomicUsize,
    /// A client connected or left: the idle countdown starts or stops.
    changed: Notify,
    shutdown: Notify,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
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

    let token = discovery::new_token().map_err(|error| format!("No token: {error}"))?;
    discovery::write_private(&profile.join(discovery::TOKEN_FILE), token.as_bytes())
        .map_err(|error| format!("The token could not be written: {error}"))?;
    let socket = discovery::socket_path(&profile)
        .map_err(|error| format!("The socket folder is unavailable: {error}"))?;
    let listener =
        Listener::bind(&socket).map_err(|error| format!("The socket could not open: {error}"))?;
    let info = CoreInfo {
        pid: std::process::id(),
        socket: socket.to_string_lossy().to_string(),
        release: protocol::release(),
        protocol: protocol::protocol(),
        started_at: now_ms(),
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
        changed: Notify::new(),
        shutdown: Notify::new(),
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
                    if state.active.load(Ordering::SeqCst) == 0 {
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

/// One client, from `hello` to its last frame.
async fn handle(state: Arc<State>, stream: Stream, same_user: bool) {
    let (mut reader, mut writer) = tokio::io::split(stream);
    let hello = match tokio::time::timeout(HELLO_DEADLINE, read_frame(&mut reader)).await {
        Ok(Ok(Some(hello))) => hello,
        _ => return,
    };
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
        let _ = write_frame(
            &mut writer,
            &Frame::Refuse {
                reason: RefuseReason::Version,
                message: format!(
                    "KChess {} runs this profile's core; this app is {release}.",
                    state.release
                ),
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

    let connection = state.core.connect(ClientInfo {
        kind,
        release,
        capabilities: capabilities.into_iter().map(Into::into).collect(),
    });
    let (sender, mut queue) = mpsc::channel::<Frame>(QUEUE);
    let cut = Arc::new(Notify::new());
    state.host.add(connection, sender.clone(), Arc::clone(&cut));
    state.active.fetch_add(1, Ordering::SeqCst);
    state.changed.notify_one();
    let writing = tokio::spawn(async move {
        while let Some(frame) = queue.recv().await {
            if write_frame(&mut writer, &frame).await.is_err() {
                break;
            }
        }
    });
    let _ = sender
        .send(Frame::Welcome {
            connection,
            release: state.release.clone(),
            protocol: state.protocol.clone(),
        })
        .await;

    let calls: Arc<Mutex<HashMap<u64, AbortHandle>>> = Arc::default();
    loop {
        let frame = tokio::select! {
            read = read_frame(&mut reader) => match read {
                Ok(Some(frame)) => frame,
                _ => break,
            },
            () = cut.notified() => break,
        };
        match frame {
            Frame::Call { id, method, args } => {
                let (core, sender, running) =
                    (Arc::clone(&state.core), sender.clone(), Arc::clone(&calls));
                let task = tokio::spawn(async move {
                    let answer = core.call_as(connection, &method, args).await;
                    if let Ok(mut running) = running.lock() {
                        running.remove(&id);
                    }
                    let frame = match answer {
                        Ok(value) => Frame::Result {
                            id,
                            value: Some(value),
                            error: None,
                        },
                        Err(error) => Frame::Result {
                            id,
                            value: None,
                            error: Some(error.message),
                        },
                    };
                    let _ = sender.send(frame).await;
                });
                if let Ok(mut running) = calls.lock() {
                    running.insert(id, task.abort_handle());
                }
            }
            Frame::Cancel { id } => {
                let task = calls
                    .lock()
                    .ok()
                    .and_then(|mut running| running.remove(&id));
                if let Some(task) = task {
                    task.abort();
                    let _ = sender
                        .send(Frame::Result {
                            id,
                            value: None,
                            error: Some("The call was cancelled.".into()),
                        })
                        .await;
                }
            }
            Frame::Focus { focused } => state.core.focus(connection, focused),
            _ => {}
        }
    }

    if let Ok(mut running) = calls.lock() {
        for (_, task) in running.drain() {
            task.abort();
        }
    }
    state.host.remove(connection);
    drop(sender);
    let _ = writing.await;
    state.core.disconnect(connection);
    state.active.fetch_sub(1, Ordering::SeqCst);
    state.changed.notify_one();
}
