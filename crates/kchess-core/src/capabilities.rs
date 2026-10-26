//! Host capabilities the core asks for while it runs: the OS secret store, opening the browser,
//! bringing the window forward and power state. The core emits `host:request`
//! `{ id, kind, payload }`; the host answers with `host.reply(id, { value } | { error })`.
//! Every request is bounded, so a host that never answers cannot hang the core.
//!
//! A host that uses the profile's own credentials (`Config::system_credentials`) is asked only to
//! open tokens an earlier release sealed; the core seals and opens the rest (`secrets.rs`).

use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

use crate::connection::{Capability, Connections};
use crate::error::{CoreError, Result};
use crate::host::Host;
use crate::secrets::{self, Credentials};

const DEADLINE: Duration = Duration::from_secs(30);

type Pending = Mutex<HashMap<u64, oneshot::Sender<std::result::Result<Value, String>>>>;

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

/// What a client must be able to do to answer a request of `kind`.
fn needed(kind: &str) -> Capability {
    match kind {
        "openExternal" => Capability::Browser,
        kind if kind.starts_with("secrets.") => Capability::LegacySecrets,
        // `focus`, `onBattery`: a window.
        _ => Capability::Focus,
    }
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
            pending: Mutex::new(HashMap::new()),
            credentials,
            connections: None,
        }
    }

    /// Requests go to the client that can answer them: the caller, else the window focused last.
    pub fn with_connections(mut self, connections: Arc<Connections>) -> Capabilities {
        self.connections = Some(connections);
        self
    }

    /// A log line through the host.
    pub fn log(&self, level: crate::host::Level, scope: &str, message: &str) {
        self.host.log(level, scope, message);
    }

    /// Whether tokens the host sealed should be resealed with the profile's credentials.
    pub fn reseals(&self) -> bool {
        self.credentials.is_some()
    }

    /// Ask the host; its error message becomes the error.
    pub async fn request(&self, kind: &str, payload: Value) -> Result<Value> {
        self.request_within(kind, payload, DEADLINE).await
    }

    /// `request` with its own bound, for host work that legitimately takes longer (Lichess).
    pub async fn request_within(
        &self,
        kind: &str,
        payload: Value,
        deadline: Duration,
    ) -> Result<Value> {
        let target = match &self.connections {
            Some(connections) => {
                match connections.route(needed(kind), crate::connection::current()) {
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
        self.pending
            .lock()
            .map_err(|_| CoreError::new("The host is unavailable."))?
            .insert(id, send);
        let request = json!({ "id": id, "kind": kind, "payload": payload });
        match target {
            Some(target) => self.host.emit_to(target, "host:request", request),
            None => self.host.emit("host:request", request),
        }
        let answer = tokio::time::timeout(deadline, receive).await;
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&id);
        }
        match answer {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(message))) => Err(CoreError::new(message)),
            Ok(Err(_)) => Err(CoreError::new("The host closed before answering.")),
            Err(_) => Err(CoreError::new(format!("The host did not answer {kind}."))),
        }
    }

    /// `host.reply(id, answer)`: `{ value }` or `{ error }`. Unknown ids are ignored.
    pub fn reply(&self, id: u64, answer: &Value) {
        let Some(send) = self.pending.lock().ok().and_then(|mut p| p.remove(&id)) else {
            return;
        };
        let result = match answer.get("error").and_then(Value::as_str) {
            Some(message) => Err(message.to_string()),
            None => Ok(answer.get("value").cloned().unwrap_or(Value::Null)),
        };
        let _ = send.send(result);
    }

    /// Fail every unanswered request (the core is closing).
    pub fn close(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.clear();
        }
    }

    /// `SecretStore.available()`, or whether the profile's credentials can be used.
    pub async fn secrets_available(&self) -> Result<bool> {
        if let Some(credentials) = &self.credentials {
            let credentials = Arc::clone(credentials);
            return blocking(move || Ok(credentials.available())).await;
        }
        Ok(self
            .request("secrets.available", Value::Null)
            .await?
            .as_bool()
            == Some(true))
    }

    /// Seal a token: with the profile's credentials, or `SecretStore.encrypt(plain)`.
    pub async fn encrypt(&self, plain: &str) -> Result<String> {
        if let Some(credentials) = &self.credentials {
            let (credentials, plain) = (Arc::clone(credentials), plain.to_string());
            return blocking(move || credentials.seal(&plain)).await;
        }
        text(self.request("secrets.encrypt", json!(plain)).await?)
    }

    /// Open a token. One the profile's credentials sealed opens here; any other was sealed by
    /// the host (`SecretStore.decrypt(base64)`), possibly in an earlier release.
    pub async fn decrypt(&self, ciphertext: &str) -> Result<String> {
        if secrets::is_sealed(ciphertext) {
            let Some(credentials) = &self.credentials else {
                return Err(CoreError::new(
                    "This account was saved by another KChess app. Reconnect your account.",
                ));
            };
            let (credentials, sealed) = (Arc::clone(credentials), ciphertext.to_string());
            return blocking(move || credentials.open(&sealed)).await;
        }
        text(self.request("secrets.decrypt", json!(ciphertext)).await?)
    }

    /// `CorePlatform.openExternal(url)`.
    pub async fn open_external(&self, url: &str) -> Result<()> {
        self.request("openExternal", json!(url)).await.map(|_| ())
    }

    /// `CorePlatform.focus?.()`.
    pub async fn focus(&self) -> Result<()> {
        self.request("focus", Value::Null).await.map(|_| ())
    }

    /// `CorePlatform.onBattery()`.
    pub async fn on_battery(&self) -> Result<bool> {
        Ok(self.request("onBattery", Value::Null).await?.as_bool() == Some(true))
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

    /// Answers every request on another task, as a JavaScript host answers through `host.reply`.
    struct Echo(Mutex<Option<Arc<Capabilities>>>);

    impl Host for Echo {
        fn log(&self, _: Level, _: &str, _: &str) {}
        fn emit(&self, event: &str, payload: Value) {
            assert_eq!(event, "host:request");
            let caps = self.0.lock().unwrap().clone().unwrap();
            tokio::spawn(async move {
                let id = payload["id"].as_u64().unwrap();
                let answer = match payload["kind"].as_str().unwrap() {
                    "secrets.decrypt" => {
                        json!({ "value": format!("plain:{}", payload["payload"].as_str().unwrap()) })
                    }
                    "onBattery" => json!({ "value": true }),
                    _ => json!({ "error": "Not supported." }),
                };
                caps.reply(id, &answer);
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
        assert!(caps.pending.lock().unwrap().is_empty());
    }
}
