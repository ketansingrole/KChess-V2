//! Tests only: a host that keeps every log line and event so a test can assert on them, or keeps
//! nothing (`TestHost::quiet`) where a test measures what the core itself holds on to. It answers
//! no host request; a test that needs answers replies through the core.

use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;

use crate::host::{CoreEvent, Host, Level};

pub struct TestHost {
    keep: bool,
    logs: Mutex<Vec<String>>,
    events: Mutex<Vec<(String, Value)>>,
}

/// A host that drops what it is sent, for calls that take `&dyn Host`.
pub static QUIET: TestHost = TestHost::keeping(false);

impl TestHost {
    const fn keeping(keep: bool) -> TestHost {
        TestHost {
            keep,
            logs: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        }
    }

    /// A host that keeps what it is sent.
    pub fn new() -> Arc<TestHost> {
        Arc::new(TestHost::default())
    }

    /// A host that drops what it is sent.
    pub fn quiet() -> Arc<TestHost> {
        Arc::new(TestHost::keeping(false))
    }

    /// The log lines so far, each `level [scope] message`.
    pub fn logs(&self) -> Vec<String> {
        lock(&self.logs).clone()
    }

    /// Whether any log line so far contains `needle`.
    pub fn logged(&self, needle: &str) -> bool {
        lock(&self.logs).iter().any(|line| line.contains(needle))
    }

    /// Every event so far, with its payload, in order.
    pub fn events(&self) -> Vec<(String, Value)> {
        lock(&self.events).clone()
    }

    /// The payloads of `event` so far, in order.
    pub fn named(&self, event: &str) -> Vec<Value> {
        lock(&self.events)
            .iter()
            .filter(|(name, _)| name == event)
            .map(|(_, payload)| payload.clone())
            .collect()
    }

    /// The `host:request` payloads of one capability kind so far.
    pub fn requests(&self, kind: &str) -> Vec<Value> {
        self.named("host:request")
            .into_iter()
            .filter(|payload| payload["kind"] == kind)
            .collect()
    }
}

impl Default for TestHost {
    fn default() -> TestHost {
        TestHost::keeping(true)
    }
}

impl Host for TestHost {
    fn log(&self, level: Level, scope: &str, message: &str) {
        if self.keep {
            lock(&self.logs).push(format!("{} [{scope}] {message}", level.name()));
        }
    }

    fn emit(&self, event: CoreEvent) {
        if self.keep {
            let name = event.name().to_string();
            lock(&self.events).push((name, event.into_payload()));
        }
    }
}

/// A test that panicked while holding a lock has already failed; the records stay readable.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
