//! A client of the shared core (`docs/shared-core.md`): the GPUI app, tools and tests. It finds
//! the profile's core through `core.json`, proves itself with `core.token`, and otherwise starts
//! the core from this release's copy of the service binary (`connect_or_start`).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kchess_core::Config;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::discovery;
use crate::protocol::{
    self, Capability, ClientSummary, Frame, RefuseReason, read_frame, write_frame,
};
use crate::transport;

/// How long a started core has to accept its first client.
const START_DEADLINE: Duration = Duration::from_secs(5);
/// How long a core has to answer `hello`.
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(5);
const STOPPED: &str = "The KChess core stopped.";
const RETRY: Duration = Duration::from_millis(50);

/// Who this client is, for `hello`.
#[derive(Clone, Debug)]
pub struct Hello {
    /// `electron`, `gpui`, `cli`, `mcp`...
    pub kind: String,
    pub capabilities: Vec<Capability>,
}

/// What reaches a client without being asked.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientEvent {
    /// A core event (`CoreEvents` names, and `host:request`).
    Event { name: String, payload: Value },
    /// The connection closed: the core stopped, or cut this client off.
    Closed,
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
type Pending = Arc<Mutex<Option<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>>;

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
        {
            let pending = Arc::clone(&pending);
            tokio::spawn(async move {
                while let Some(frame) = queue.recv().await {
                    if write_frame(&mut writer, &frame).await.is_err() {
                        // Nothing more reaches the core: waiting calls would never be answered.
                        close(&pending);
                        return;
                    }
                }
                // Every handle was dropped: the core sees this client leave.
                let _ = tokio::io::AsyncWriteExt::shutdown(&mut writer).await;
            });
        }
        let (events, received) = mpsc::unbounded_channel();
        {
            let pending = Arc::clone(&pending);
            tokio::spawn(async move {
                while let Ok(Some(frame)) = read_frame(&mut reader).await {
                    match frame {
                        Frame::Result { id, value, error } => {
                            let waiting = pending
                                .lock()
                                .ok()
                                .and_then(|mut p| p.as_mut().and_then(|p| p.remove(&id)));
                            if let Some(waiting) = waiting {
                                let _ = waiting.send(match error {
                                    Some(error) => Err(error),
                                    None => Ok(value.unwrap_or(Value::Null)),
                                });
                            }
                        }
                        Frame::Event { name, payload } => {
                            let _ = events.send(ClientEvent::Event { name, payload });
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

    /// This client's connection id on the core.
    pub fn connection(&self) -> u64 {
        self.connection
    }

    /// The release of the core it is connected to.
    pub fn release(&self) -> &str {
        &self.release
    }

    /// Runs a core method; the future resolves on any executor.
    pub fn call(
        &self,
        method: &str,
        args: Vec<Value>,
    ) -> impl std::future::Future<Output = Result<Value, String>> + Send + 'static + use<> {
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
            if sender.send(frame).await.is_err() {
                if let Ok(mut pending) = pending.lock()
                    && let Some(pending) = pending.as_mut()
                {
                    pending.remove(&id);
                }
                return Err(STOPPED.into());
            }
            answered.await.unwrap_or_else(|_| Err(STOPPED.into()))
        }
    }

    /// Tells the core this client's window gained or lost focus.
    pub fn focus(&self, focused: bool) {
        let _ = self.sender.try_send(Frame::Focus { focused });
    }
}

/// The core's configuration, as the starting client passes it to the service.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServiceConfig {
    pub legacy_database_path: Option<PathBuf>,
    pub bundled_engine_path: Option<PathBuf>,
    pub runtime_path: Option<PathBuf>,
    #[serde(default)]
    pub runtime_env: BTreeMap<String, String>,
    pub managed_engine_dir: Option<PathBuf>,
    pub voice_helper_path: Option<PathBuf>,
    #[serde(default)]
    pub system_credentials: bool,
}

impl ServiceConfig {
    /// The core's `Config` for `profile`.
    pub fn into_config(self, profile: PathBuf) -> Config {
        Config {
            legacy_database_path: self.legacy_database_path,
            bundled_engine_path: self.bundled_engine_path,
            runtime_path: self.runtime_path,
            runtime_env: self.runtime_env,
            managed_engine_dir: self.managed_engine_dir,
            voice_helper_path: self.voice_helper_path,
            system_credentials: self.system_credentials,
            ..Config::new(profile)
        }
    }
}

/// How to start the core when none runs.
#[derive(Clone, Debug)]
pub struct Start {
    /// The service binary shipped with this app.
    pub binary: PathBuf,
    pub config: ServiceConfig,
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
    match Client::connect(profile, hello).await {
        Err(ConnectError::NotRunning) => {}
        Err(ConnectError::Refused {
            reason: RefuseReason::Version,
            sole: true,
            ..
        }) => wait_until_gone(profile).await,
        other => return other,
    }
    launch(profile, start)?;
    let deadline = tokio::time::Instant::now() + START_DEADLINE;
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
async fn wait_until_gone(profile: &Path) {
    let deadline = tokio::time::Instant::now() + START_DEADLINE;
    while discovery::read_info(profile).is_some() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(RETRY).await;
    }
}

/// Starts the service for `profile` from this release's runtime copy, detached from this app:
/// closing the window that started it does not stop it. Two clients starting at once are
/// settled by the profile lock: the second service exits.
fn launch(profile: &Path, start: &Start) -> Result<(), ConnectError> {
    let binary = crate::runtime::install(&start.binary).unwrap_or_else(|_| start.binary.clone());
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
