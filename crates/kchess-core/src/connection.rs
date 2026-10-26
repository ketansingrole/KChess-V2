//! The clients of one core (`docs/shared-core.md`). An app that embeds the core is connection
//! `APP`; the shared core service adds one connection per socket. A call runs with its
//! connection in `CURRENT`, so the services that hold one caller's activity (analysis, watching,
//! voice) find whose it is without threading an id through every call.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// The connection of an app that embeds the core, and of calls made outside any connection.
pub const APP: u64 = 0;

/// Something a client can do for the core when it asks (`host:request`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    /// Open a web page in the browser (`openExternal`).
    Browser,
    /// Bring its window to the front (`focus`).
    Focus,
    /// Open logins sealed by an earlier Electron release (`secrets.*`, `safeStorage`).
    LegacySecrets,
}

/// Who a client is, as it says when it connects.
#[derive(Clone, Debug)]
pub struct ClientInfo {
    /// `electron`, `gpui`, `cli`, `mcp`...
    pub kind: String,
    /// The KChess release it was built from.
    pub release: String,
    pub capabilities: Vec<Capability>,
}

impl ClientInfo {
    /// An app that embeds the core: it can do everything a window can.
    pub fn app() -> ClientInfo {
        ClientInfo {
            kind: "app".into(),
            release: String::new(),
            capabilities: vec![
                Capability::Browser,
                Capability::Focus,
                Capability::LegacySecrets,
            ],
        }
    }
}

struct Client {
    info: ClientInfo,
    /// When it last reported its window focused, in the order of `clock`; 0 never.
    focused: u64,
    /// When it connected, in the order of `clock`.
    connected: u64,
}

/// The connected clients, and which one alerts the person.
pub struct Connections {
    clients: Mutex<BTreeMap<u64, Client>>,
    next: AtomicU64,
    /// Orders connections and focus reports.
    clock: AtomicU64,
}

impl Default for Connections {
    fn default() -> Connections {
        Connections::new()
    }
}

impl Connections {
    /// The embedding app's connection (`APP`) is there from the start.
    pub fn new() -> Connections {
        let connections = Connections {
            clients: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(APP + 1),
            clock: AtomicU64::new(1),
        };
        connections.insert(APP, ClientInfo::app());
        connections
    }

    fn insert(&self, id: u64, info: ClientInfo) {
        let connected = self.clock.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut clients) = self.clients.lock() {
            clients.insert(
                id,
                Client {
                    info,
                    focused: 0,
                    connected,
                },
            );
        }
    }

    /// A new client; its connection id.
    pub fn connect(&self, info: ClientInfo) -> u64 {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        self.insert(id, info);
        id
    }

    /// Forgets a client. `APP` stays for the embedding app's lifetime.
    pub fn disconnect(&self, id: u64) {
        if id == APP {
            return;
        }
        if let Ok(mut clients) = self.clients.lock() {
            clients.remove(&id);
        }
    }

    /// The service has its own clients: the embedding app's connection goes.
    pub fn without_app(&self) {
        if let Ok(mut clients) = self.clients.lock() {
            clients.remove(&APP);
        }
    }

    /// A client reports its window gaining (or losing) focus.
    pub fn focus(&self, id: u64, focused: bool) {
        let at = self.clock.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut clients) = self.clients.lock()
            && let Some(client) = clients.get_mut(&id)
        {
            client.focused = if focused { at } else { 0 };
        }
    }

    /// The connected ids, oldest first.
    pub fn ids(&self) -> Vec<u64> {
        self.clients
            .lock()
            .map(|clients| clients.keys().copied().collect())
            .unwrap_or_default()
    }

    pub fn is_connected(&self, id: u64) -> bool {
        self.clients
            .lock()
            .is_ok_and(|clients| clients.contains_key(&id))
    }

    /// What `id` said when it connected.
    pub fn info(&self, id: u64) -> Option<ClientInfo> {
        self.clients
            .lock()
            .ok()
            .and_then(|clients| clients.get(&id).map(|client| client.info.clone()))
    }

    /// The client that alerts the person: the window focused last, else the window that
    /// connected last. Clients without windows (a CLI, an MCP server) never alert.
    pub fn alert_owner(&self) -> Option<u64> {
        let clients = self.clients.lock().ok()?;
        clients
            .iter()
            .filter(|(_, client)| client.info.capabilities.contains(&Capability::Focus))
            .max_by_key(|(_, client)| (client.focused, client.connected))
            .map(|(id, _)| *id)
    }

    /// Where a host request for `capability` goes: the client that made the call when it can do
    /// it, else the alert owner when it can, else any client that can.
    pub fn route(&self, capability: Capability, caller: Option<u64>) -> Option<u64> {
        let can = |id: u64| {
            self.info(id)
                .is_some_and(|info| info.capabilities.contains(&capability))
        };
        if let Some(caller) = caller.filter(|id| can(*id)) {
            return Some(caller);
        }
        if let Some(owner) = self.alert_owner().filter(|id| can(*id)) {
            return Some(owner);
        }
        self.ids().into_iter().find(|id| can(*id))
    }
}

tokio::task_local! {
    static CURRENT: u64;
}

/// Runs `work` as a call of `connection`.
pub async fn scope<F: Future>(connection: u64, work: F) -> F::Output {
    CURRENT.scope(connection, work).await
}

/// The connection of the call running on this task, when there is one.
pub fn current() -> Option<u64> {
    CURRENT.try_with(|id| *id).ok()
}

/// The connection of the call running on this task, or `APP`.
pub fn current_or_app() -> u64 {
    current().unwrap_or(APP)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(kind: &str) -> ClientInfo {
        ClientInfo {
            kind: kind.into(),
            release: "2026.10.12".into(),
            capabilities: vec![Capability::Browser, Capability::Focus],
        }
    }

    #[test]
    fn the_app_is_connected_until_the_service_takes_over() {
        let connections = Connections::new();
        assert_eq!(connections.ids(), vec![APP]);
        connections.disconnect(APP);
        assert!(connections.is_connected(APP));
        connections.without_app();
        assert!(connections.ids().is_empty());
    }

    #[test]
    fn the_window_focused_last_alerts_and_tools_never_do() {
        let connections = Connections::new();
        connections.without_app();
        let electron = connections.connect(window("electron"));
        let gpui = connections.connect(window("gpui"));
        let cli = connections.connect(ClientInfo {
            kind: "cli".into(),
            release: String::new(),
            capabilities: Vec::new(),
        });
        // Nobody focused yet: the window that connected last.
        assert_eq!(connections.alert_owner(), Some(gpui));
        connections.focus(electron, true);
        assert_eq!(connections.alert_owner(), Some(electron));
        connections.focus(gpui, true);
        assert_eq!(connections.alert_owner(), Some(gpui));
        connections.disconnect(gpui);
        assert_eq!(connections.alert_owner(), Some(electron));
        assert_ne!(connections.alert_owner(), Some(cli));
    }

    #[test]
    fn host_requests_go_to_a_client_that_can_answer() {
        let connections = Connections::new();
        connections.without_app();
        let gpui = connections.connect(window("gpui"));
        let cli = connections.connect(ClientInfo {
            kind: "cli".into(),
            release: String::new(),
            capabilities: Vec::new(),
        });
        // The CLI asked, but it has no browser: the window opens it.
        assert_eq!(
            connections.route(Capability::Browser, Some(cli)),
            Some(gpui)
        );
        assert_eq!(
            connections.route(Capability::Browser, Some(gpui)),
            Some(gpui)
        );
        // Nobody can open Electron's sealed logins.
        assert_eq!(
            connections.route(Capability::LegacySecrets, Some(gpui)),
            None
        );
        let electron = connections.connect(ClientInfo {
            capabilities: vec![Capability::Focus, Capability::LegacySecrets],
            ..window("electron")
        });
        assert_eq!(
            connections.route(Capability::LegacySecrets, Some(gpui)),
            Some(electron)
        );
    }

    #[tokio::test]
    async fn a_call_knows_its_connection() {
        assert_eq!(current(), None);
        assert_eq!(scope(7, async { current() }).await, Some(7));
        assert_eq!(current_or_app(), APP);
    }
}
