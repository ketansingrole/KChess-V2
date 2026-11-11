//! A connection to the shared core (`docs/shared-core.md`), under a frontend's `CoreHandle`. It finds
//! the profile's core through `core.json`, proves itself with `core.token`, and otherwise starts
//! the core from this release's copy of the service binary (`connect_or_start`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kchess_contracts::CoreEvent;
use kchess_contracts::{CoreError, ErrorKind};
use kchess_core::Config;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::discovery;
use crate::protocol::{
    self, Capability, ClientSummary, Frame, RefuseReason, read_frame, write_frame,
};
use crate::transport;

/// How long a started core has to accept its first client: a bound, not a wait, since a client
/// connects the moment the core serves. Starting copies the core into this release's folder and
/// opens the profile; on a loaded Windows PC, whose virus scanner reads the new executable first,
/// that took over 5 s, and the app said it could not open the profile while the core came up.
const START_DEADLINE: Duration = Duration::from_secs(30);
/// How long a core has to answer `hello`.
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(5);
const STOPPED: &str = "The KChess core stopped.";
const RETRY: Duration = Duration::from_millis(50);

/// Who this client is, for `hello`.
#[derive(Clone, Debug)]
pub struct Hello {
    /// `gpui`, `cli`, `mcp`...
    pub kind: String,
    pub capabilities: Vec<Capability>,
}

/// What reaches a client without being asked.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientEvent {
    Event(CoreEvent),
    /// The connection closed: the core stopped, or cut this client off.
    Closed,
    /// A frontend that reconnects after `Closed` reached a core again: what it had started on
    /// the old connection ended with it. The handle reports this once it has reconnected.
    Reconnected,
}

impl ClientEvent {
    /// The core event this is, if it is one.
    pub fn core(&self) -> Option<&CoreEvent> {
        match self {
            ClientEvent::Event(event) => Some(event),
            _ => None,
        }
    }
}

/// Why a client could not connect.
#[derive(Clone, Debug, PartialEq)]
pub enum ConnectError {
    /// No core runs for the profile.
    NotRunning,
    /// The core did not let this client in.
    Refused {
        reason: RefuseReason,
        message: String,
        /// No other client used that core; it is closing.
        sole: bool,
        clients: Vec<ClientSummary>,
    },
    Failed(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::NotRunning => write!(f, "No KChess core runs for this profile."),
            ConnectError::Refused { message, .. } => write!(f, "{message}"),
            ConnectError::Failed(message) => write!(f, "{message}"),
        }
    }
}

/// Calls waiting for their answers; `None` once the connection closed, so a later call fails at
/// once instead of waiting for a reader that has gone.
type Pending = Arc<Mutex<Option<HashMap<u64, oneshot::Sender<Result<Value, CoreError>>>>>>;

/// The connection closed: no call is admitted, and every waiting one fails.
fn close(pending: &Pending) {
    let waiting = pending.lock().ok().and_then(|mut pending| pending.take());
    for (_, waiting) in waiting.into_iter().flatten() {
        let _ = waiting.send(Err(STOPPED.into()));
    }
}

/// A connection to the profile's core. Cloning shares it.
#[derive(Clone)]
pub struct Client {
    sender: mpsc::Sender<Frame>,
    pending: Pending,
    next: Arc<AtomicU64>,
    connection: u64,
    release: String,
}

impl Client {
    /// Connects to the core running for `profile`. A core that does not answer `hello` in time
    /// fails the connection.
    pub async fn connect(
        profile: &Path,
        hello: &Hello,
    ) -> Result<(Client, mpsc::UnboundedReceiver<ClientEvent>), ConnectError> {
        tokio::time::timeout(HANDSHAKE_DEADLINE, Self::handshake(profile, hello))
            .await
            .unwrap_or_else(|_| {
                Err(ConnectError::Failed(
                    "The KChess core did not answer.".into(),
                ))
            })
    }

    async fn handshake(
        profile: &Path,
        hello: &Hello,
    ) -> Result<(Client, mpsc::UnboundedReceiver<ClientEvent>), ConnectError> {
        let profile = discovery::canonical(profile).map_err(|error| {
            ConnectError::Failed(format!("The profile is unavailable: {error}"))
        })?;
        let Some(info) = discovery::read_info(&profile) else {
            return Err(ConnectError::NotRunning);
        };
        let token = discovery::read_token(&profile).map_err(|_| ConnectError::NotRunning)?;
        let stream = transport::connect(Path::new(&info.socket))
            .await
            .map_err(|_| ConnectError::NotRunning)?;
        let (mut reader, mut writer) = tokio::io::split(stream);
        write_frame(
            &mut writer,
            &Frame::Hello {
                token,
                kind: hello.kind.clone(),
                release: protocol::release(),
                protocol: protocol::protocol(),
                capabilities: hello.capabilities.clone(),
            },
        )
        .await
        .map_err(|_| ConnectError::NotRunning)?;
        let (connection, release) = match read_frame(&mut reader).await {
            Ok(Some(Frame::Welcome {
                connection,
                release,
                ..
            })) => (connection, release),
            Ok(Some(Frame::Refuse {
                reason,
                message,
                sole,
                clients,
            })) => {
                return Err(ConnectError::Refused {
                    reason,
                    message,
                    sole,
                    clients,
                });
            }
            _ => return Err(ConnectError::NotRunning),
        };

        let pending: Pending = Arc::new(Mutex::new(Some(HashMap::new())));
        let (sender, mut queue) = mpsc::channel::<Frame>(1024);
        // Dropped when the writer ends, which ends the reader too: the connection closes only
        // once both halves are gone.
        let (writer_done, mut written) = oneshot::channel::<()>();
        {
            let pending = Arc::clone(&pending);
            tokio::spawn(async move {
                let _writer_done = writer_done;
                while let Some(frame) = queue.recv().await {
                    if write_frame(&mut writer, &frame).await.is_err() {
                        // Nothing more reaches the core: waiting calls would never be answered.
                        close(&pending);
                        return;
                    }
                }
                // Every handle was dropped: the core sees this client leave. A Unix socket
                // sees the shutdown at once; a Windows named pipe has no half-close and closes
                // only when the reader lets go too.
                let _ = tokio::io::AsyncWriteExt::shutdown(&mut writer).await;
            });
        }
        let (events, received) = mpsc::unbounded_channel();
        {
            let pending = Arc::clone(&pending);
            tokio::spawn(async move {
                loop {
                    let frame = tokio::select! {
                        read = read_frame(&mut reader) => match read {
                            Ok(Some(frame)) => frame,
                            _ => break,
                        },
                        _ = &mut written => break,
                    };
                    match frame {
                        Frame::Result {
                            id,
                            value,
                            error,
                            kind,
                        } => {
                            let waiting = pending
                                .lock()
                                .ok()
                                .and_then(|mut p| p.as_mut().and_then(|p| p.remove(&id)));
                            if let Some(waiting) = waiting {
                                let _ = waiting.send(match error {
                                    Some(error) => {
                                        let mut error = CoreError::new(error);
                                        error.kind = kind.as_deref().and_then(ErrorKind::parse);
                                        Err(error)
                                    }
                                    None => Ok(value.unwrap_or(Value::Null)),
                                });
                            }
                        }
                        // The handshake matched the protocol, so every name is known.
                        Frame::Event { name, payload } => {
                            if let Some(event) = CoreEvent::from_wire(&name, payload) {
                                let _ = events.send(ClientEvent::Event(event));
                            }
                        }
                        _ => {}
                    }
                }
                close(&pending);
                let _ = events.send(ClientEvent::Closed);
            });
        }
        Ok((
            Client {
                sender,
                pending,
                next: Arc::new(AtomicU64::new(1)),
                connection,
                release,
            },
            received,
        ))
    }

    pub fn connection(&self) -> u64 {
        self.connection
    }

    pub fn release(&self) -> &str {
        &self.release
    }

    /// Runs a core method; the future resolves on any executor. Dropped before its answer, it
    /// cancels the call: the core stops the work, not only the reply (`cancel`).
    pub fn call(
        &self,
        method: &str,
        args: Vec<Value>,
    ) -> impl std::future::Future<Output = Result<Value, CoreError>> + Send + 'static + use<> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (answer, answered) = oneshot::channel();
        // Admitted under the lock that closing drains: a call racing the close fails too.
        let admitted = self
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.as_mut().map(|p| p.insert(id, answer)))
            .is_some();
        let (sender, pending) = (self.sender.clone(), Arc::clone(&self.pending));
        let frame = Frame::Call {
            id,
            method: method.to_string(),
            args,
        };
        async move {
            if !admitted {
                return Err(STOPPED.into());
            }
            let mut call = Unanswered {
                id,
                sender,
                pending,
                sent: false,
            };
            if call.sender.send(frame).await.is_err() {
                return Err(STOPPED.into());
            }
            call.sent = true;
            answered.await.unwrap_or_else(|_| Err(STOPPED.into()))
        }
    }

    pub fn focus(&self, focused: bool) {
        let _ = self.sender.try_send(Frame::Focus { focused });
    }

    /// Tells the core this client has no window open: what it started stops.
    pub fn suspend(&self) {
        let _ = self.sender.try_send(Frame::Suspend);
    }
}

/// A call on its way. Dropped before its answer arrived (which takes it out of `pending`), it
/// forgets the call and, once sent, tells the core to cancel it.
struct Unanswered {
    id: u64,
    sender: mpsc::Sender<Frame>,
    pending: Pending,
    sent: bool,
}

impl Drop for Unanswered {
    fn drop(&mut self) {
        let waiting = self
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.as_mut().and_then(|p| p.remove(&self.id)));
        // A full queue means the core is not reading; the connection then ends, and the call
        // with it.
        if self.sent && waiting.is_some() {
            let _ = self.sender.try_send(Frame::Cancel { id: self.id });
        }
    }
}

/// How to start the core when none runs.
#[derive(Clone, Debug)]
pub struct Start {
    /// The service binary shipped with this app.
    pub binary: PathBuf,
    /// The core's configuration; the profile is the one connected to.
    pub config: Config,
}

/// Connects to the profile's core, starting it when none runs (`docs/shared-core.md`, "Start"):
/// a running core is reused; else this release's copy is started; else the bundled binary is
/// copied to the runtime folder first. A core of another release used by no other client closes
/// and is replaced; one that others use is refused, and the error names them.
pub async fn connect_or_start(
    profile: &Path,
    hello: &Hello,
    start: &Start,
) -> Result<(Client, mpsc::UnboundedReceiver<ClientEvent>), ConnectError> {
    connect_or_start_within(profile, hello, start, START_DEADLINE).await
}

/// `connect_or_start`, giving a core it starts (or a closing one it waits for) `wait` to be ready.
pub async fn connect_or_start_within(
    profile: &Path,
    hello: &Hello,
    start: &Start,
    wait: Duration,
) -> Result<(Client, mpsc::UnboundedReceiver<ClientEvent>), ConnectError> {
    match Client::connect(profile, hello).await {
        Err(ConnectError::NotRunning) => {}
        Err(ConnectError::Refused {
            reason: RefuseReason::Version,
            sole: true,
            ..
        }) => wait_until_gone(profile, wait).await,
        other => return other,
    }
    launch(profile, start)?;
    let deadline = tokio::time::Instant::now() + wait;
    let not_started = || ConnectError::Failed("The KChess core did not start.".into());
    loop {
        // The start deadline bounds each attempt too, not only the waits between them.
        match tokio::time::timeout_at(deadline, Client::connect(profile, hello)).await {
            Err(_) => return Err(not_started()),
            Ok(Err(ConnectError::NotRunning)) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(RETRY).await;
            }
            Ok(Err(ConnectError::NotRunning)) => return Err(not_started()),
            Ok(other) => return other,
        }
    }
}

/// Waits for a closing core to release the profile.
async fn wait_until_gone(profile: &Path, wait: Duration) {
    let deadline = tokio::time::Instant::now() + wait;
    while discovery::read_info(profile).is_some() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(RETRY).await;
    }
}

/// The program to start: this release's runtime copy, except in the App Sandbox. There a copy the
/// app wrote is quarantined and refused, and the App Store quits KChess before it replaces the
/// bundle, so the service runs from the bundle itself.
fn service_binary(bundled: &Path, sandboxed: bool) -> PathBuf {
    if sandboxed {
        return bundled.to_path_buf();
    }
    crate::runtime::install(bundled).unwrap_or_else(|_| bundled.to_path_buf())
}

/// Starts the service for `profile` from this release's runtime copy, detached from this app:
/// closing the window that started it does not stop it. Two clients starting at once are
/// settled by the profile lock: the second service exits.
fn launch(profile: &Path, start: &Start) -> Result<(), ConnectError> {
    let binary = service_binary(&start.binary, kchess_core::app_store());
    let config = serde_json::to_string(&start.config)
        .map_err(|error| ConnectError::Failed(error.to_string()))?;
    let mut command = std::process::Command::new(binary);
    command
        .arg("serve")
        .arg("--profile")
        .arg(profile)
        .arg("--config")
        .arg(config)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    kchess_core::env::forward(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let mut child = command.spawn().map_err(|error| {
        ConnectError::Failed(format!("The KChess core could not start: {error}"))
    })?;
    // Reaped when it ends, so a core that stops while this app runs leaves no zombie behind.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sandbox_starts_the_service_from_the_bundle() {
        // A copy the sandboxed app wrote was quarantined: "Operation not permitted" on start.
        let bundled = Path::new("/Applications/KChess.app/Contents/MacOS/kchess-service");
        assert_eq!(service_binary(bundled, true), bundled);
    }
}
