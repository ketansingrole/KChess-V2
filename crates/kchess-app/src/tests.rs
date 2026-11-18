//! What the crate's tests share: a real core, embedded, over a fresh profile.

use std::sync::Arc;

use gpui_kit::{App, AppContext as _};
use kchess_service::{CoreHandle, Hello, Mode, Options};

use crate::core::{AppCore, CoreBridge, CoreEvents};

/// The profile a test's core keeps, removed when the test ends.
pub struct TestCore {
    _profile: tempfile::TempDir,
}

/// A core of the test's own, embedded over a fresh profile, as the app holds it
/// (`cx.global::<AppCore>()`).
pub fn start_core(cx: &mut App) -> TestCore {
    let profile = tempfile::tempdir().expect("a profile");
    let runtime = kchess_service::core_runtime().expect("the core's runtime");
    let options = Options {
        config: kchess_core::Config::new(profile.path().to_path_buf()),
        hello: Hello {
            kind: "test".into(),
            capabilities: Vec::new(),
        },
        mode: Mode::Embedded,
        reconnect: false,
        log: Arc::new(|_, _, _| {}),
        answer: None,
    };
    let (handle, _events) = runtime
        .block_on(CoreHandle::open(options))
        .expect("the core");
    let core = AppCore {
        bridge: Arc::new(CoreBridge::new(runtime, handle)),
        events: cx.new(|_| CoreEvents),
    };
    cx.set_global(core);
    TestCore { _profile: profile }
}
