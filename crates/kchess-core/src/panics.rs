//! Panics reach the host's log. A panic is caught around each core call (`tasks::caught`) or by
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
    host.log(Level::Error, "panic", &kchess_host::panic_message(info));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_host::TestHost;

    #[test]
    fn logs_a_caught_panic_to_the_host() {
        let lines = TestHost::new();
        let host: Arc<dyn Host> = lines.clone();
        report_to(&host);
        let caught = std::thread::Builder::new()
            .name("panicking".into())
            .spawn(|| panic!("boom {}", 7))
            .unwrap()
            .join();
        assert!(caught.is_err());
        let logged = lines.logs();
        assert!(
            logged.iter().any(|line| line
                .starts_with("error [panic] thread 'panicking' panicked at ")
                && line.contains("panics.rs:")
                && line.ends_with(": boom 7")),
            "{logged:?}"
        );
        // The hook keeps no host alive, and a panic after the host went is still caught.
        let weak = Arc::downgrade(&lines);
        drop(host);
        drop(lines);
        assert!(weak.upgrade().is_none(), "the hook keeps the host alive");
        assert!(std::thread::spawn(|| panic!("after")).join().is_err());
    }
}
