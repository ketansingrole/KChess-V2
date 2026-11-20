//! What the crate's tests share: a real core, embedded, over a fresh profile.

use std::sync::Arc;

use gpui_kit::{App, AppContext as _};
use kchess_client::{CoreHandle, Hello, Mode, Options};

use crate::core::{AppCore, CoreBridge, CoreEvents};

/// The profile a test's core keeps, removed when the test ends.
pub struct TestCore {
    _profile: tempfile::TempDir,
}

/// A core of the test's own, embedded over a fresh profile, as the app holds it
/// (`cx.global::<AppCore>()`).
pub fn start_core(cx: &mut App) -> TestCore {
    start_core_with_lichess(cx, None)
}

/// `start_core`, with Lichess (its API and its puzzle database) at `origin`: a local stand-in.
pub fn start_core_with_lichess(cx: &mut App, origin: Option<String>) -> TestCore {
    open(cx, origin, false)
}

/// `start_core_with_lichess`, whose events reach the app's views as in the app (`CoreEvents`),
/// such as the core's engine-help lock once its startup check ends. They arrive from the core's
/// threads, so only a test that waits in real time (`allow_parking`) takes them.
pub fn start_core_delivering(cx: &mut App, origin: Option<String>) -> TestCore {
    open(cx, origin, true)
}

fn open(cx: &mut App, origin: Option<String>, deliver: bool) -> TestCore {
    let profile = tempfile::tempdir().expect("a profile");
    let runtime = kchess_client::core_runtime().expect("the core's runtime");
    let mut config = kchess_core::Config::new(profile.path().to_path_buf());
    config.test = origin.map(|origin| kchess_core::TestSwitches {
        lichess_origin: Some(origin),
        ..Default::default()
    });
    let options = Options {
        config,
        hello: Hello {
            kind: "test".into(),
            capabilities: Vec::new(),
        },
        mode: Mode::Embedded,
        embed: kchess_service::embed(),
        reconnect: false,
        log: Arc::new(|_, _, _| {}),
        answer: None,
    };
    let (handle, mut events) = runtime
        .block_on(CoreHandle::open(options))
        .expect("the core");
    let core = AppCore {
        bridge: Arc::new(CoreBridge::new(runtime, handle)),
        events: cx.new(|_| CoreEvents),
    };
    cx.set_global(core.clone());
    if !deliver {
        return TestCore { _profile: profile };
    }
    // The test core names no host capability, so it asks the host nothing to answer.
    cx.spawn(async move |cx| {
        while let Some(event) = events.recv().await {
            core.events.update(cx, |_, cx| cx.emit(event));
        }
    })
    .detach();
    TestCore { _profile: profile }
}

/// The process's log, started the first time in a folder of the tests' own: what has been
/// written to it so far. The log starts once a process, so a test counts the lines of its own
/// area that came after it began.
pub fn logged() -> String {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().expect("a temporary folder"));
    crate::log::init(dir.path());
    crate::log::read().join("")
}

/// How many lines of the log are `area`'s (`[area] `).
pub fn logged_in(area: &str) -> usize {
    let marker = format!("[{area}] ");
    logged()
        .lines()
        .filter(|line| line.contains(&marker))
        .count()
}
