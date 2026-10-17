//! Panics reach the host's log. A panic is caught at the N-API boundary (`catch_unwind`) or by
//! tokio when a task panics, so the process survives; without this hook its message would go
//! only to stderr, which a packaged app does not keep.

use std::panic::PanicHookInfo;
use std::sync::{Arc, Once, RwLock, Weak};

use crate::host::{Host, Level};

/// The host of the core opened last; weak, so a closed core's host is not kept alive.
static HOST: RwLock<Option<Weak<dyn Host>>> = RwLock::new(None);
static INSTALL: Once = Once::new();

/// Send panics to `host`'s log from now on, keeping the previous hook (stderr) as well.
pub(crate) fn report_to(host: &Arc<dyn Host>) {
    if let Ok(mut current) = HOST.write() {
        *current = Some(Arc::downgrade(host));
    }
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            report(info);
            previous(info);
        }));
    });
}

fn report(info: &PanicHookInfo<'_>) {
    // `try_read`: a panic while the host is being replaced must not deadlock the hook.
    let Some(host) = HOST
        .try_read()
        .ok()
        .and_then(|current| current.as_ref().and_then(Weak::upgrade))
    else {
        return;
    };
    host.log(Level::Error, "panic", &describe(info));
}

/// `thread 'name' panicked at file:line: message`, as Rust's default hook words it.
fn describe(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("Box<dyn Any>");
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("<unnamed>");
    match info.location() {
        Some(at) => format!(
            "thread '{name}' panicked at {}:{}: {message}",
            at.file(),
            at.line()
        ),
        None => format!("thread '{name}' panicked: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Lines(Mutex<Vec<String>>);
    impl Host for Lines {
        fn log(&self, level: Level, scope: &str, message: &str) {
            self.0
                .lock()
                .unwrap()
                .push(format!("{} [{scope}] {message}", level.name()));
        }
        fn emit(&self, _event: &str, _payload: Value) {}
    }

    #[test]
    fn logs_a_caught_panic_to_the_host() {
        let lines = Arc::new(Lines::default());
        let host: Arc<dyn Host> = lines.clone();
        report_to(&host);
        let caught = std::thread::Builder::new()
            .name("panicking".into())
            .spawn(|| panic!("boom {}", 7))
            .unwrap()
            .join();
        assert!(caught.is_err());
        let logged = lines.0.lock().unwrap().clone();
        assert!(
            logged.iter().any(|line| line
                .starts_with("error [panic] thread 'panicking' panicked at ")
                && line.contains("panics.rs:")
                && line.ends_with(": boom 7")),
            "{logged:?}"
        );
        // A dropped host is not called (nor kept alive).
        drop(host);
        drop(lines);
        let _ = std::thread::spawn(|| panic!("after")).join();
    }
}
