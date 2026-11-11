//! Host capabilities the core asks for while it runs: the OS secret store, opening the browser,
//! bringing the window forward and power state. The core emits `host:request` (a `HostAsk`); the
//! host answers with `host.reply(id, reply)` (a `HostReply`, `Core::host_reply`). Every request
//! is bounded, so a host that never answers cannot hang the core.
//!
//! With the profile's own credentials (`Config::system_credentials`) the core seals and opens every
//! token itself (`secrets.rs`); without them (tests, `--no-credentials`) the host does.

use kchess_contracts::{CoreEvent, HostAsk, HostReply, HostRequest};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

use crate::connection::Connections;
use crate::host::Host;
use crate::secrets::{self, Credentials};
use crate::tasks::lock;
use kchess_contracts::error::{CoreError, Result};

const DEADLINE: Duration = Duration::from_secs(30);
const HOST_CLOSED: &str = "The host closed before answering.";

/// An unanswered request: the connection it was routed to (`None` when every client heard it),
/// and where its answer goes.
type Waiting = (
    Option<u64>,
    oneshot::Sender<std::result::Result<Value, String>>,
);

type Pending = Mutex<Requests>;

#[derive(Default)]
struct Requests {
    waiting: HashMap<u64, Waiting>,
    /// Set by `close`, under the same lock, so no request is taken after it: a host request
    /// started by work the closing core had spawned (a battery refresh first polled after close
    /// on a loaded runner) would otherwise wait out `DEADLINE` for an answer no host will send.
    closed: bool,
}

pub struct Capabilities {
    host: Arc<dyn Host>,
    next: AtomicU64,
    pending: Pending,
    /// The profile's credentials, when the host uses them instead of sealing tokens itself.
    credentials: Option<Arc<Credentials>>,
    /// The core's clients: a request goes to one that can answer it (`Connections::route`).
    /// Without them (tests), requests go to the host as any event.
    connections: Option<Arc<Connections>>,
}

impl Capabilities {
    pub fn new(host: Arc<dyn Host>) -> Capabilities {
        Capabilities::with_credentials(host, None)
    }

    pub fn with_credentials(
        host: Arc<dyn Host>,
        credentials: Option<Arc<Credentials>>,
    ) -> Capabilities {
        Capabilities {
            host,
            next: AtomicU64::new(1),
            pending: Mutex::new(Requests::default()),
            credentials,
            connections: None,
        }
    }

    /// Requests go to the client that can answer them: the caller, else the window focused last.
    pub fn with_connections(mut self, connections: Arc<Connections>) -> Capabilities {
        self.connections = Some(connections);
        self
    }

    /// Ask the host; its error message becomes the error.
    pub async fn request(&self, request: HostRequest) -> Result<Value> {
        self.request_within(request, DEADLINE).await
    }

    /// `request` with its own bound, for host work that legitimately takes longer (Lichess).
    pub async fn request_within(&self, request: HostRequest, deadline: Duration) -> Result<Value> {
        let kind = request.kind();
        let target = match &self.connections {
            Some(connections) => {
                match connections.route(request.needs(), crate::connection::current()) {
                    Some(target) => Some(target),
                    None => {
                        return Err(CoreError::new(format!(
                            "No KChess window can answer {kind} now."
                        )));
                    }
                }
            }
            None => None,
        };
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (send, receive) = oneshot::channel();
        {
            let mut pending = lock(&self.pending);
            if pending.closed {
                return Err(CoreError::new(HOST_CLOSED));
            }
            pending.waiting.insert(id, (target, send));
        }
        // Forgotten however the wait ends: a caller that was cancelled while its window had
        // gone would otherwise leave the entry until the core closed.
        let _forget = crate::tasks::on_drop(|| {
            lock(&self.pending).waiting.remove(&id);
        });
        let event = CoreEvent::HostRequest(HostAsk { id, request }.to_value());
        match target {
            Some(target) => self.host.emit_to(target, event),
            None => self.host.emit(event),
        }
        let answer = tokio::time::timeout(deadline, receive).await;
        match answer {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(message))) => Err(CoreError::new(message)),
            Ok(Err(_)) => Err(CoreError::new(HOST_CLOSED)),
            Err(_) => Err(CoreError::new(format!("The host did not answer {kind}."))),
        }
    }

    /// `host.reply(id, answer)`. Unknown ids are ignored, and so is an answer from any client
    /// but the one the request was sent to: another client of the shared core could otherwise
    /// answer a window's `secrets.decrypt` with a token of its own.
    pub fn reply(&self, id: u64, answer: HostReply) {
        let send = {
            let mut pending = lock(&self.pending);
            let replier = crate::connection::current();
            match pending.waiting.get(&id) {
                // In-process callers run outside any connection's scope; every socket client's
                // call is scoped (`Core::call_as`).
                Some((Some(target), _)) if replier.is_some_and(|replier| replier != *target) => {
                    None
                }
                Some(_) => pending.waiting.remove(&id).map(|(_, send)| send),
                None => None,
            }
        };
        let Some(send) = send else {
            return;
        };
        let _ = send.send(answer.into_result());
    }

    /// Fail every unanswered request, and every later one (the core is closing).
    pub fn close(&self) {
        let mut pending = lock(&self.pending);
        pending.closed = true;
        pending.waiting.clear();
    }

    /// Whether tokens can be sealed: by the profile's credentials, or else by the host.
    pub async fn secrets_available(&self) -> Result<bool> {
        if let Some(credentials) = &self.credentials {
            let credentials = Arc::clone(credentials);
            return blocking(move || Ok(credentials.available())).await;
        }
        Ok(self.request(HostRequest::SecretsAvailable).await?.as_bool() == Some(true))
    }

    /// Seal a token: with the profile's credentials, or else by the host.
    pub async fn encrypt(&self, plain: &str) -> Result<String> {
        if let Some(credentials) = &self.credentials {
            let (credentials, plain) = (Arc::clone(credentials), plain.to_string());
            return blocking(move || credentials.seal(&plain)).await;
        }
        text(
            self.request(HostRequest::SecretsEncrypt(plain.to_string()))
                .await?,
        )
    }

    /// Open a token: with the profile's credentials, or else by the host.
    pub async fn decrypt(&self, ciphertext: &str) -> Result<String> {
        let Some(credentials) = &self.credentials else {
            if secrets::is_sealed(ciphertext) {
                return Err(CoreError::new(
                    "This account was saved by another KChess app. Reconnect your account.",
                ));
            }
            return text(
                self.request(HostRequest::SecretsDecrypt(ciphertext.to_string()))
                    .await?,
            );
        };
        if !secrets::is_sealed(ciphertext) {
            return Err(CoreError::new(
                "This account was saved by an older KChess. Reconnect your account.",
            ));
        }
        let (credentials, sealed) = (Arc::clone(credentials), ciphertext.to_string());
        blocking(move || credentials.open(&sealed)).await
    }

    /// Opens `url` in the browser of the client that can.
    pub async fn open_external(&self, url: &str) -> Result<()> {
        self.request(HostRequest::OpenExternal(url.to_string()))
            .await
            .map(|_| ())
    }

    pub async fn focus(&self) -> Result<()> {
        self.request(HostRequest::Focus).await.map(|_| ())
    }

    pub async fn on_battery(&self) -> Result<bool> {
        Ok(self.request(HostRequest::OnBattery).await?.as_bool() == Some(true))
    }
}

/// OS credential stores block (and may prompt): keep them off the core's worker threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| CoreError::new("The credential store is unavailable."))?
}

fn text(value: Value) -> Result<String> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| CoreError::new("The host answered with something other than text."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::Level;
    use crate::test_host::TestHost;
    use kchess_contracts::CoreEvent;
    use serde_json::json;

    /// Answers every request on another task, as a service client answers through `host.reply`.
    struct Echo(Mutex<Option<Arc<Capabilities>>>);

    impl Host for Echo {
        fn log(&self, _: Level, _: &str, _: &str) {}
        fn emit(&self, event: CoreEvent) {
            let (event, payload) = (event.name(), event.into_payload());
            assert_eq!(event, "host:request");
            let caps = self.0.lock().unwrap().clone().unwrap();
            let ask: HostAsk = serde_json::from_value(payload).unwrap();
            tokio::spawn(async move {
                let answer = match ask.request {
                    HostRequest::SecretsDecrypt(sealed) => {
                        HostReply::Value(json!(format!("plain:{sealed}")))
                    }
                    HostRequest::OnBattery => HostReply::Value(json!(true)),
                    _ => HostReply::Error("Not supported.".into()),
                };
                caps.reply(ask.id, answer);
            });
        }
    }

    #[tokio::test]
    async fn answers_come_back_to_the_request_that_asked() {
        let echo = Arc::new(Echo(Mutex::new(None)));
        let caps = Arc::new(Capabilities::new(echo.clone()));
        *echo.0.lock().unwrap() = Some(Arc::clone(&caps));
        assert_eq!(caps.decrypt("abc").await.unwrap(), "plain:abc");
        assert!(caps.on_battery().await.unwrap());
        assert_eq!(caps.focus().await.unwrap_err().message, "Not supported.");
        assert!(caps.pending.lock().unwrap().waiting.is_empty());
    }

    /// A credential store in memory.
    struct Keys(Mutex<Option<String>>);

    impl crate::secrets::KeyStore for Keys {
        fn available(&self) -> bool {
            true
        }
        fn read(&self) -> Result<Option<String>> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn write(&self, key: &str) -> Result<()> {
            *self.0.lock().unwrap() = Some(key.to_string());
            Ok(())
        }
    }

    /// 2026-11-07: with the Electron takeover gone, a login the Electron app sealed
    /// (`safeStorage`'s `v10`) is not sent to a host that cannot open it: it asks to reconnect,
    /// while the profile's own logins still open.
    #[tokio::test]
    async fn a_login_the_profile_did_not_seal_asks_to_reconnect() {
        let echo = Arc::new(Echo(Mutex::new(None)));
        let credentials = Arc::new(Credentials::new(Box::new(Keys(Mutex::new(None)))));
        let caps = Arc::new(Capabilities::with_credentials(
            echo.clone(),
            Some(credentials),
        ));
        *echo.0.lock().unwrap() = Some(Arc::clone(&caps));
        let electron = "djEwuQ7Zl0Jx";
        assert_eq!(
            caps.decrypt(electron).await.unwrap_err().message,
            "This account was saved by an older KChess. Reconnect your account."
        );
        let sealed = caps.encrypt("lip_token").await.unwrap();
        assert_eq!(caps.decrypt(&sealed).await.unwrap(), "lip_token");
        assert!(caps.pending.lock().unwrap().waiting.is_empty());
    }

    /// A request routed to one client is answered by that client alone.
    #[tokio::test]
    async fn only_the_asked_client_answers() {
        let (send, receive) = oneshot::channel();
        let echo = Arc::new(Echo(Mutex::new(None)));
        let caps = Capabilities::new(echo);
        caps.pending
            .lock()
            .unwrap()
            .waiting
            .insert(7, (Some(2), send));
        crate::connection::scope(3, async {
            caps.reply(7, HostReply::Value(json!("forged")));
        })
        .await;
        assert!(
            caps.pending.lock().unwrap().waiting.contains_key(&7),
            "another client's answer"
        );
        crate::connection::scope(2, async {
            caps.reply(7, HostReply::Value(json!("real")));
        })
        .await;
        assert_eq!(receive.await.unwrap(), Ok(json!("real")));
    }

    /// A request made once the capabilities have closed fails at once, unheard: it is no longer
    /// left waiting `DEADLINE` for an answer that cannot come.
    #[tokio::test]
    async fn a_request_after_close_fails_at_once() {
        let host = TestHost::new();
        let caps = Capabilities::new(host.clone());
        caps.close();
        let answer = tokio::time::timeout(Duration::from_secs(1), caps.on_battery())
            .await
            .expect("answered at once");
        assert_eq!(
            answer.unwrap_err().message,
            "The host closed before answering."
        );
        assert!(host.events().is_empty(), "the host was asked");
        assert!(caps.pending.lock().unwrap().waiting.is_empty());
    }

    /// 2026-11-10, CI on Windows: a core's battery refresh, spawned by its first call, ran only
    /// after the core had closed its capabilities. Its request then waited 30 s for an answer, so
    /// 14 of 20 cores opened and closed in a row were still held, read as 972 B kept per cycle
    /// (`tests/memory.rs`, `cores_opened_and_closed`). The refresh now ends with the core.
    #[tokio::test]
    async fn a_battery_refresh_that_starts_after_close_ends_at_once() {
        let host: Arc<dyn Host> = TestHost::new();
        let caps = Arc::new(Capabilities::new(Arc::clone(&host)));
        let battery = crate::engine::bridge::Battery::new(host, Arc::clone(&caps));
        let alive = || {
            tokio::runtime::Handle::current()
                .metrics()
                .num_alive_tasks()
        };
        let before = alive();
        // One thread: the refresh is not polled before the capabilities close.
        battery.refresh();
        assert_eq!(alive(), before + 1);
        caps.close();
        battery.close();
        tokio::time::timeout(Duration::from_secs(1), async {
            while alive() > before {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the refresh ended with the core");
    }
}
