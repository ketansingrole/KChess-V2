//! What the core needs from its host: somewhere to log, somewhere to send events, and its
//! configuration (`kchess_host::config`, which frontends write without depending on the core).

pub use kchess_contracts::CoreEvent;
pub use kchess_contracts::error::Level;
pub use kchess_host::config::{Config, TestSwitches, TestVoiceModel};

/// Host callbacks. They must not block: the core calls them from its worker threads.
pub trait Host: Send + Sync + 'static {
    /// One log line under a `[scope]`.
    fn log(&self, level: Level, scope: &str, message: &str);
    /// An event for the host to deliver.
    fn emit(&self, event: CoreEvent);
    /// An event for one client only (`crate::connection`): an analysis's updates, a watch's
    /// frames, a host request. A host with one client delivers it like any event.
    fn emit_to(&self, connection: u64, event: CoreEvent) {
        let _ = connection;
        self.emit(event);
    }
}
