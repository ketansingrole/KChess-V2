//! UCI engine processes (`core/src/services/uci.ts`). A controller owns the process, its pipes,
//! every deadline, termination and the option cache; consumers own positions and the meaning of
//! `info` lines. Cancellation flows through `CancellationToken`s. Dropping a controller
//! terminates its process; a dropped future never leaves a search running unobserved.
//!
//! Deviations from the TypeScript, kept deliberately:
//! - Spawn failures are returned from `UciController::start` instead of rejecting `ready()`.
//! - Engine registries (maintenance, `closeEngines`) live in an `EngineHub`, not a module
//!   singleton, so independent cores and parallel tests do not share one another's engines.
//! - `search` and `command` take `Option<Duration>` where TypeScript used `0` for "no deadline".
//! - Performance timings (`timed`) are not mirrored; the Rust core has no performance module yet.

use std::collections::HashMap;
use std::future::Future;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use futures_util::future::join_all;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{Instant, sleep_until};
use tokio_util::sync::CancellationToken;

use crate::error::{CoreError, Result};
use crate::host::{Host, Level};

/// The constructor's default deadline for the handshake and `isready`.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(10);
/// How long a search may take to acknowledge `stop` before the engine is failed.
const STOP_DEADLINE: Duration = Duration::from_secs(2);
/// After a termination request, the process is killed if it has not exited by then.
const TERMINATION_DEADLINE: Duration = Duration::from_secs(1);
/// How long maintenance and shutdown wait for engines to exit.
pub const EXIT_DEADLINE: Duration = Duration::from_secs(4);
/// Bytes of stderr kept for failure diagnostics.
const STDERR_LIMIT: usize = 2048;

type Matcher = Arc<dyn Fn(&str) -> bool + Send + Sync>;
type OnLine = Box<dyn FnMut(&str) + Send>;
type LineSink = Arc<Mutex<OnLine>>;

/// `SearchCancelled`: the search was stopped on purpose. Crosses the bridge as `AbortError`.
pub fn search_cancelled() -> CoreError {
    CoreError::aborted("Engine search cancelled.")
}

/// `AbortSignal.throwIfAborted()` without a reason: the default `AbortError` message.
pub(crate) fn signal_aborted() -> CoreError {
    CoreError::aborted("This operation was aborted.")
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Resolves when `cancel` fires; never resolves without a token.
pub(crate) async fn cancelled_or_pending(cancel: Option<&CancellationToken>) {
    match cancel {
        Some(token) => token.cancelled().await,
        None => std::future::pending().await,
    }
}

fn stopped_unexpectedly() -> CoreError {
    CoreError::new("Stockfish stopped unexpectedly. Choose another engine or retry.")
}

/// Owns the engines this core started, so maintenance and shutdown can reach them.
pub struct EngineHub {
    host: Arc<dyn Host>,
    exit_deadline: Duration,
    maintenance: AtomicBool,
    controllers: Mutex<Vec<Weak<Inner>>>,
}

/// Clears the maintenance flag on every exit path.
struct MaintenanceFlag<'a>(&'a AtomicBool);

impl Drop for MaintenanceFlag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl EngineHub {
    pub fn new(host: Arc<dyn Host>) -> EngineHub {
        EngineHub {
            host,
            exit_deadline: EXIT_DEADLINE,
            maintenance: AtomicBool::new(false),
            controllers: Mutex::new(Vec::new()),
        }
    }

    /// Replaces the 4 s exit deadline; tests use it to reach the retained-executable path quickly.
    pub fn with_exit_deadline(mut self, deadline: Duration) -> EngineHub {
        self.exit_deadline = deadline;
        self
    }

    /// `assertEngineAvailable`: refuses new engines while an executable is being replaced.
    pub fn assert_available(&self) -> Result<()> {
        if self.maintenance.load(Ordering::SeqCst) {
            return Err(CoreError::new("Stockfish is being updated. Retry shortly."));
        }
        Ok(())
    }

    /// `withEngineMaintenance`: stops every engine, waits for all to exit within the deadline,
    /// then runs `action`. The flag clears on every exit path.
    pub async fn with_maintenance<T, Fut>(
        &self,
        stop_owners: impl FnOnce(),
        action: impl FnOnce() -> Fut,
    ) -> Result<T>
    where
        Fut: Future<Output = Result<T>>,
    {
        self.assert_available()?;
        self.maintenance.store(true, Ordering::SeqCst);
        let _flag = MaintenanceFlag(&self.maintenance);
        stop_owners();
        self.close_all("Stockfish did not exit. The previous engine was retained.")
            .await?;
        action().await
    }

    /// `closeEngines`: terminates every engine and waits for their exit.
    pub async fn close_engines(&self) -> Result<()> {
        self.close_all("Stockfish did not exit during shutdown.")
            .await
    }

    async fn close_all(&self, message: &str) -> Result<()> {
        let exits = join_all(self.live().into_iter().map(|engine| async move {
            engine.close();
            engine.closed().await;
        }));
        tokio::time::timeout(self.exit_deadline, exits)
            .await
            .map(|_| ())
            .map_err(|_| CoreError::new(message))
    }

    fn register(&self, engine: &Arc<Inner>) {
        let mut list = lock(&self.controllers);
        list.retain(|weak| weak.upgrade().is_some_and(|live| !live.exited()));
        list.push(Arc::downgrade(engine));
    }

    fn live(&self) -> Vec<Arc<Inner>> {
        let mut list = lock(&self.controllers);
        list.retain(|weak| weak.upgrade().is_some_and(|live| !live.exited()));
        list.iter().filter_map(Weak::upgrade).collect()
    }
}

/// A running UCI process. Dropping it terminates the process.
pub struct UciController {
    inner: Arc<Inner>,
}

impl UciController {
    /// Spawns `command` with piped stdio and starts the `uci`/`isready` handshake in the
    /// background; `ready` reports its outcome. Must be called inside a Tokio runtime.
    pub fn start(
        hub: &EngineHub,
        mut command: Command,
        deadline: Duration,
    ) -> Result<UciController> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|error| CoreError::new(error.to_string()))?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(CoreError::new("Engine pipes are unavailable."));
        };
        let (writes, write_queue) = mpsc::unbounded_channel();
        let (kill, kill_requests) = mpsc::unbounded_channel();
        let (exit_tx, _) = watch::channel(false);
        let (ready_tx, _) = watch::channel(None);
        let inner = Arc::new(Inner {
            host: Arc::clone(&hub.host),
            deadline,
            state: Mutex::new(State::default()),
            options: Mutex::new(HashMap::new()),
            stderr: Mutex::new(String::new()),
            next_id: AtomicU64::new(1),
            writes,
            kill,
            exit_tx,
            ready_tx,
        });
        hub.register(&inner);
        tokio::spawn(reap(Arc::clone(&inner), child, kill_requests));
        tokio::spawn(write_commands(Arc::clone(&inner), stdin, write_queue));
        tokio::spawn(read_lines(Arc::clone(&inner), stdout));
        tokio::spawn(read_stderr(Arc::clone(&inner), stderr));
        tokio::spawn(initialize(Arc::clone(&inner)));
        Ok(UciController { inner })
    }

    /// Resolves once `uci` and `isready` have been answered, or fails with the startup error.
    pub async fn ready(&self) -> Result<()> {
        self.inner.ready().await
    }

    /// The error that failed this engine, if any.
    pub fn failed(&self) -> Option<CoreError> {
        self.inner.failure()
    }

    /// Queues one command line; it is written in order and never contains a line break.
    pub fn write(&self, command: &str) -> Result<()> {
        self.inner.write(command)
    }

    pub async fn sync(&self) -> Result<String> {
        self.inner.sync().await
    }

    /// Sends `command` and waits for the first line matching `matches`. `on_line` sees every
    /// line while the wait lasts. A `timeout` that elapses fails the whole engine.
    pub async fn command(
        &self,
        command: &str,
        matches: impl Fn(&str) -> bool + Send + Sync + 'static,
        timeout: Option<Duration>,
        on_line: Option<OnLine>,
    ) -> Result<String> {
        self.inner.command(command, matches, timeout, on_line).await
    }

    /// Runs one search. Cancelling `cancel` sends `stop`; a search that does not acknowledge it
    /// within 2 s fails the engine. A cancelled search reports `search_cancelled()`.
    pub async fn search(
        &self,
        command: &str,
        on_line: impl FnMut(&str) + Send + 'static,
        timeout: Option<Duration>,
        cancel: Option<&CancellationToken>,
    ) -> Result<String> {
        self.inner.search(command, on_line, timeout, cancel).await
    }

    /// Asks the active search (if any) to stop, as cancelling its token does.
    pub fn stop(&self) {
        self.inner.stop();
    }

    /// Fails the engine with `search_cancelled()`, terminates the process and forgets its options.
    pub fn close(&self) {
        self.inner.close();
    }

    /// Resolves when the process has exited.
    pub async fn closed(&self) {
        self.inner.closed().await;
    }

    /// Writes only options whose value changed since the last confirmed sync, then `isready`
    /// once when anything changed. Order follows `options`.
    pub async fn ensure_options(&self, options: &[(&str, &str)]) -> Result<()> {
        self.inner.ensure_options(options).await
    }

    /// Forgets which options this process has confirmed; the next `ensure_options` resends them.
    pub fn clear_engine_options(&self) {
        self.inner.clear_engine_options();
    }
}

impl Drop for UciController {
    fn drop(&mut self) {
        self.inner.close();
    }
}

#[derive(Default)]
struct State {
    failure: Option<CoreError>,
    listeners: Vec<Waiter>,
    searching: bool,
    stop: Option<CancellationToken>,
}

struct Waiter {
    id: u64,
    matches: Matcher,
    on_line: Option<LineSink>,
    reply: Option<oneshot::Sender<Result<String>>>,
}

/// Shared by the controller handle and its background tasks. Tasks hold `Arc<Inner>`; the
/// public handle does not, so dropping the handle closes the engine.
struct Inner {
    host: Arc<dyn Host>,
    deadline: Duration,
    state: Mutex<State>,
    options: Mutex<HashMap<String, String>>,
    stderr: Mutex<String>,
    next_id: AtomicU64,
    writes: mpsc::UnboundedSender<String>,
    kill: mpsc::UnboundedSender<()>,
    exit_tx: watch::Sender<bool>,
    ready_tx: watch::Sender<Option<Result<()>>>,
}

/// Removes a command's waiter when its future completes or is dropped.
struct ListenerGuard<'a> {
    inner: &'a Inner,
    id: u64,
}

impl Drop for ListenerGuard<'_> {
    fn drop(&mut self) {
        lock(&self.inner.state)
            .listeners
            .retain(|waiter| waiter.id != self.id);
    }
}

/// Clears the searching flag and the stop token on every exit path.
struct SearchGuard<'a>(&'a Inner);

impl Drop for SearchGuard<'_> {
    fn drop(&mut self) {
        let mut state = lock(&self.0.state);
        state.searching = false;
        state.stop = None;
    }
}

impl Inner {
    fn log(&self, level: Level, message: String) {
        self.host.log(level, "uci", &message);
    }

    fn failure(&self) -> Option<CoreError> {
        lock(&self.state).failure.clone()
    }

    fn exited(&self) -> bool {
        *self.exit_tx.borrow()
    }

    async fn closed(&self) {
        let mut rx = self.exit_tx.subscribe();
        let _ = rx.wait_for(|done| *done).await;
    }

    async fn ready(&self) -> Result<()> {
        let mut rx = self.ready_tx.subscribe();
        let _ = rx.wait_for(Option::is_some).await;
        let value: Option<Result<()>> = rx.borrow().clone();
        value.unwrap_or_else(|| Err(stopped_unexpectedly()))
    }

    fn write(&self, command: &str) -> Result<()> {
        if let Some(error) = self.failure() {
            return Err(error);
        }
        if command.contains(['\r', '\n']) {
            return Err(CoreError::new("Invalid engine command."));
        }
        self.writes
            .send(format!("{command}\n"))
            .map_err(|_| self.failure().unwrap_or_else(stopped_unexpectedly))
    }

    async fn sync(&self) -> Result<String> {
        self.command(
            "isready",
            |line| line == "readyok",
            Some(self.deadline),
            None,
        )
        .await
    }

    async fn command(
        &self,
        command: &str,
        matches: impl Fn(&str) -> bool + Send + Sync + 'static,
        timeout: Option<Duration>,
        on_line: Option<OnLine>,
    ) -> Result<String> {
        let limit = timeout.filter(|duration| !duration.is_zero());
        let (reply, mut waiting) = oneshot::channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut state = lock(&self.state);
            if let Some(error) = &state.failure {
                return Err(error.clone());
            }
            state.listeners.push(Waiter {
                id,
                matches: Arc::new(matches),
                on_line: on_line.map(|callback| Arc::new(Mutex::new(callback))),
                reply: Some(reply),
            });
        }
        let _listener = ListenerGuard { inner: self, id };
        if let Err(error) = self.write(command) {
            self.log(
                Level::Warn,
                format!(
                    "Engine command write failed: {} {}",
                    uci_command_name(command),
                    error_summary(&error)
                ),
            );
            self.fail(error.clone());
            return Err(error);
        }
        let received = match limit {
            None => (&mut waiting).await,
            Some(limit) => match tokio::time::timeout(limit, &mut waiting).await {
                Ok(received) => received,
                Err(_elapsed) => {
                    self.log(
                        Level::Warn,
                        format!(
                            "Engine command timed out: {} timeoutMs={}",
                            uci_command_name(command),
                            limit.as_millis()
                        ),
                    );
                    self.fail(CoreError::new(
                        "Stockfish timed out. Retry or choose another engine.",
                    ));
                    (&mut waiting).await
                }
            },
        };
        match received {
            Ok(result) => result,
            Err(_) => Err(self.failure().unwrap_or_else(stopped_unexpectedly)),
        }
    }

    async fn search(
        &self,
        command: &str,
        on_line: impl FnMut(&str) + Send + 'static,
        timeout: Option<Duration>,
        cancel: Option<&CancellationToken>,
    ) -> Result<String> {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(signal_aborted());
        }
        let stop = CancellationToken::new();
        {
            let mut state = lock(&self.state);
            if state.searching {
                return Err(CoreError::new("An engine search is already active."));
            }
            state.searching = true;
            state.stop = Some(stop.clone());
        }
        let _searching = SearchGuard(self);
        let search = self.command(
            command,
            |line| line.starts_with("bestmove ") || line.starts_with("Nodes searched"),
            timeout,
            Some(Box::new(on_line) as OnLine),
        );
        tokio::pin!(search);
        let mut cancelled = false;
        let mut stop_at: Option<Instant> = None;
        let result = loop {
            tokio::select! {
                result = &mut search => break result,
                _ = stop.cancelled(), if !cancelled => {
                    cancelled = true;
                    if let Err(cause) = self.write("stop") {
                        self.log(Level::Debug, format!("Engine stop failed: {}", error_summary(&cause)));
                    }
                    stop_at = Some(Instant::now() + STOP_DEADLINE);
                }
                _ = cancelled_or_pending(cancel), if cancel.is_some_and(|token| !token.is_cancelled()) => {
                    stop.cancel();
                }
                _ = sleep_until(stop_at.unwrap_or_else(Instant::now)), if stop_at.is_some() => {
                    stop_at = None;
                    self.fail(CoreError::new("Stockfish did not stop. Retry the search."));
                }
            }
        };
        let line = result?;
        if cancelled {
            return Err(search_cancelled());
        }
        Ok(line)
    }

    fn stop(&self) {
        let token = lock(&self.state).stop.clone();
        if let Some(token) = token {
            token.cancel();
        }
    }

    fn close(&self) {
        self.fail(search_cancelled());
        self.clear_engine_options();
    }

    fn clear_engine_options(&self) {
        lock(&self.options).clear();
    }

    async fn ensure_options(&self, options: &[(&str, &str)]) -> Result<()> {
        let pending: Vec<(&str, &str)> = {
            let known = lock(&self.options);
            options
                .iter()
                .filter(|(name, value)| known.get(*name).map(String::as_str) != Some(*value))
                .copied()
                .collect()
        };
        if pending.is_empty() {
            return Ok(());
        }
        for (name, value) in &pending {
            self.write(&format!("setoption name {name} value {value}"))?;
        }
        if let Err(cause) = self.sync().await {
            // A failed engine is recreated; its replacement must configure from scratch.
            self.clear_engine_options();
            return Err(cause);
        }
        let mut known = lock(&self.options);
        for (name, value) in pending {
            known.insert(name.to_string(), value.to_string());
        }
        Ok(())
    }

    /// Records a failure once: waiters get the error and the process is terminated (the reaper
    /// escalates to a kill after `TERMINATION_DEADLINE`).
    fn fail(&self, error: CoreError) {
        let waiters = {
            let mut state = lock(&self.state);
            if state.failure.is_some() {
                drop(state);
                self.log(
                    Level::Debug,
                    format!("Engine failure already reported: {}", error_summary(&error)),
                );
                return;
            }
            state.failure = Some(error.clone());
            std::mem::take(&mut state.listeners)
        };
        if is_expected_cancellation(&error) {
            self.log(
                Level::Debug,
                format!("Engine stopped: {}", error_summary(&error)),
            );
        } else {
            self.log(
                Level::Warn,
                format!("Engine failed: {}", error_summary(&error)),
            );
        }
        for mut waiter in waiters {
            if let Some(reply) = waiter.reply.take() {
                let _ = reply.send(Err(error.clone()));
            }
        }
        // The reaper is already gone when the process exited on its own; nothing to kill then.
        let _ = self.kill.send(());
    }

    /// Hands one stdout line to every listener: `on_line` first, then the match, as TypeScript
    /// does. Callbacks run without the state lock, so they may call back into the controller.
    fn dispatch(&self, line: &str) {
        let listeners: Vec<(u64, Matcher, Option<LineSink>)> = lock(&self.state)
            .listeners
            .iter()
            .map(|waiter| {
                (
                    waiter.id,
                    Arc::clone(&waiter.matches),
                    waiter.on_line.clone(),
                )
            })
            .collect();
        if listeners.is_empty() {
            return;
        }
        let mut matched = Vec::new();
        for (id, matches, sink) in &listeners {
            if let Some(sink) = sink {
                let mut callback = lock(sink);
                let callback: &mut OnLine = &mut callback;
                callback(line);
            }
            if matches(line) {
                matched.push(*id);
            }
        }
        if matched.is_empty() {
            return;
        }
        let mut state = lock(&self.state);
        for id in matched {
            // A listener that failed or was dropped meanwhile has already been answered.
            if let Some(at) = state.listeners.iter().position(|waiter| waiter.id == id) {
                let mut waiter = state.listeners.remove(at);
                if let Some(reply) = waiter.reply.take() {
                    let _ = reply.send(Ok(line.to_string()));
                }
            }
        }
    }
}

async fn initialize(inner: Arc<Inner>) {
    let result: Result<()> = async {
        inner
            .command("uci", |line| line == "uciok", Some(inner.deadline), None)
            .await?;
        inner.sync().await?;
        Ok(())
    }
    .await;
    if let Err(error) = &result {
        inner.log(
            Level::Debug,
            format!("Engine init deferred: {}", error_summary(error)),
        );
    }
    inner.ready_tx.send_replace(Some(result));
}

async fn write_commands(
    inner: Arc<Inner>,
    mut stdin: ChildStdin,
    mut queue: mpsc::UnboundedReceiver<String>,
) {
    let mut exited = inner.exit_tx.subscribe();
    // `borrow` before the loop: an exit that already happened is seen, not awaited.
    let done = *exited.borrow();
    if done {
        return;
    }
    loop {
        tokio::select! {
            line = queue.recv() => {
                let Some(line) = line else { break };
                if let Err(error) = stdin.write_all(line.as_bytes()).await {
                    inner.fail(CoreError::new(error.to_string()));
                    break;
                }
            }
            _ = exited.changed() => break,
        }
    }
}

async fn read_lines(inner: Arc<Inner>, stdout: ChildStdout) {
    let mut reader = BufReader::new(stdout);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer).await {
            Ok(0) => break,
            Ok(_) => {
                let raw = String::from_utf8_lossy(&buffer);
                let line = raw.trim();
                if !line.is_empty() {
                    inner.dispatch(line);
                }
            }
            Err(error) => {
                inner.log(Level::Warn, format!("Engine output read failed: {error}"));
                break;
            }
        }
    }
}

async fn read_stderr(inner: Arc<Inner>, mut stderr: ChildStderr) {
    let mut chunk = [0u8; 512];
    loop {
        match stderr.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => {
                let mut snippet = lock(&inner.stderr);
                if snippet.len() < STDERR_LIMIT {
                    let take = (STDERR_LIMIT - snippet.len()).min(read);
                    snippet.push_str(&String::from_utf8_lossy(&chunk[..take]));
                }
            }
            Err(error) => {
                inner.log(Level::Debug, format!("Engine stderr read failed: {error}"));
                break;
            }
        }
    }
}

/// Owns the child. Terminates it on request (SIGTERM, then SIGKILL after the deadline) and
/// records how it exited.
async fn reap(inner: Arc<Inner>, mut child: Child, mut kill: mpsc::UnboundedReceiver<()>) {
    let mut terminating = false;
    let mut kill_at: Option<Instant> = None;
    let status = loop {
        tokio::select! {
            status = child.wait() => break status,
            Some(()) = kill.recv(), if !terminating => {
                terminating = true;
                request_termination(&mut child);
                kill_at = Some(Instant::now() + TERMINATION_DEADLINE);
            }
            _ = sleep_until(kill_at.unwrap_or_else(Instant::now)), if kill_at.is_some() => {
                kill_at = None;
                let _ = child.start_kill();
            }
        }
    };
    inner.exit_tx.send_replace(true);
    let (code, signal) = match &status {
        Ok(status) => exit_fields(status),
        Err(_) => ("null".to_string(), "none".to_string()),
    };
    if inner.failure().is_some() {
        inner.log(
            Level::Debug,
            format!("Engine process exited: code={code} signal={signal}"),
        );
        return;
    }
    let stderr = lock(&inner.stderr).clone();
    let stderr = if stderr.is_empty() {
        "stderr=empty".to_string()
    } else {
        format!(
            "stderr={}",
            collapse_whitespace(&stderr)
                .chars()
                .take(200)
                .collect::<String>()
        )
    };
    inner.log(
        Level::Warn,
        format!("Engine process exited unexpectedly: code={code} signal={signal} {stderr}"),
    );
    inner.fail(stopped_unexpectedly());
}

#[cfg(unix)]
fn request_termination(child: &mut Child) {
    if let Some(pid) = child.id() {
        // The child has not been reaped (`wait` has not returned), so this pid still names it.
        // SAFETY: `kill` only sends a signal to a process id; no memory is shared.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
}

#[cfg(not(unix))]
fn request_termination(child: &mut Child) {
    let _ = child.start_kill();
}

fn exit_fields(status: &ExitStatus) -> (String, String) {
    let code = status
        .code()
        .map_or_else(|| "null".to_string(), |code| code.to_string());
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status
            .signal()
            .map_or_else(|| "none".to_string(), |signal| signal.to_string())
    };
    #[cfg(not(unix))]
    let signal = {
        let _ = status;
        "none".to_string()
    };
    (code, signal)
}

fn is_expected_cancellation(error: &CoreError) -> bool {
    if error.aborted {
        return true;
    }
    let message = error.message.to_lowercase();
    message.contains("superseded") || message.contains("cancelled")
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `truncateForLog`: one line, at most `max` characters.
pub(crate) fn truncate_for_log(text: &str, max: usize) -> String {
    let single = collapse_whitespace(text);
    let count = single.chars().count();
    if count <= max {
        return single;
    }
    let head: String = single.chars().take(max).collect();
    format!("{head}…({} more chars)", count - max)
}

/// `errorSummary` for a `CoreError`: the name and the message, bounded.
pub(crate) fn error_summary(error: &CoreError) -> String {
    let name = if error.aborted { "AbortError" } else { "Error" };
    truncate_for_log(&format!("{name}: {}", error.message), 300)
}

/// `uciCommandName`: the command word and the move count, never the position itself.
fn uci_command_name(command: &str) -> String {
    let head = command.split_whitespace().next().unwrap_or(command);
    let moves = if command.contains(" moves ") {
        let tail = command.split(" moves ").nth(1).unwrap_or("");
        let count = if tail.is_empty() {
            0
        } else {
            tail.split(' ').count()
        };
        format!(" moves({count})")
    } else {
        String::new()
    };
    truncate_for_log(&format!("{head}{moves}"), 40)
}
