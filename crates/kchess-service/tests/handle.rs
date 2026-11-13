//! A frontend's `CoreHandle` on the shared core, against a stand-in core on a real local socket:
//! what it sends the core, what it hands on, and how it reconnects when the core stops.

#![cfg(unix)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kchess_contracts::types::AlertOwner;
use kchess_contracts::{CoreEvent, Level};
use kchess_core::Config;
use kchess_service::protocol::{self, Frame, read_frame, write_frame};
use kchess_service::{
    Capability, Client, ClientEvent, CoreHandle, Hello, Log, Mode, Options, discovery,
};
use serde_json::json;
use tokio::net::{UnixListener, UnixStream};

const WAIT: Duration = Duration::from_secs(10);

fn window() -> Hello {
    Hello {
        kind: "gpui".into(),
        capabilities: vec![Capability::Browser, Capability::Focus],
    }
}

/// Writes the profile's `core.json` and token for a stand-in core listening on `fake.sock`.
fn advertise(profile: &Path) {
    let info = discovery::CoreInfo {
        pid: std::process::id(),
        socket: profile.join("fake.sock").to_string_lossy().into(),
        release: protocol::release(),
        protocol: protocol::protocol(),
        started_at: 0,
    };
    discovery::write_private(
        &profile.join(discovery::INFO_FILE),
        &serde_json::to_vec(&info).expect("info"),
    )
    .expect("core.json");
    discovery::write_private(&profile.join(discovery::TOKEN_FILE), b"test-token").expect("token");
}

/// A stand-in core for `profile`, advertised in its `core.json`.
fn fake_core(profile: &Path) -> UnixListener {
    let listener = UnixListener::bind(profile.join("fake.sock")).expect("bind");
    advertise(profile);
    listener
}

/// Accepts the next client and lets it in as `connection`.
async fn welcome(listener: &UnixListener, connection: u64) -> UnixStream {
    let (mut stream, _) = tokio::time::timeout(WAIT, listener.accept())
        .await
        .expect("a client in time")
        .expect("accept");
    let hello = read_frame(&mut stream)
        .await
        .expect("hello")
        .expect("a frame");
    assert!(matches!(hello, Frame::Hello { .. }), "{hello:?}");
    let welcome = Frame::Welcome {
        connection,
        release: protocol::release(),
        protocol: protocol::protocol(),
    };
    write_frame(&mut stream, &welcome).await.expect("welcome");
    stream
}

/// The next frame the client sends.
async fn next(stream: &mut UnixStream) -> Frame {
    tokio::time::timeout(WAIT, read_frame(stream))
        .await
        .expect("a frame in time")
        .expect("read")
        .expect("a frame")
}

async fn received(events: &mut tokio::sync::mpsc::UnboundedReceiver<ClientEvent>) -> ClientEvent {
    tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("an event in time")
        .expect("the events go on")
}

fn options(profile: &Path, reconnect: bool, log: Log) -> Options {
    Options {
        config: Config::new(profile.to_path_buf()),
        hello: window(),
        // Never started: the stand-in core is found first, and a restart fails to start this.
        mode: Mode::Service {
            binary: profile.join("no-such-service"),
        },
        reconnect,
        log,
        answer: None,
    }
}

fn quiet() -> Log {
    Arc::new(|_, _, _| {})
}

/// The handle names its connection, tells the core about focus and suspension, hands on the
/// core's events, and without `reconnect` ends with `Closed` when the core stops.
#[tokio::test]
async fn a_shared_core_handle_talks_to_its_core() {
    let profile = tempfile::tempdir().expect("profile");
    let listener = fake_core(profile.path());
    let (opened, mut stream) = tokio::join!(
        CoreHandle::open(options(profile.path(), false, quiet())),
        welcome(&listener, 7)
    );
    let (core, mut events) = opened.expect("opened");
    assert_eq!(core.connection(), 7);

    core.focus(true);
    assert_eq!(next(&mut stream).await, Frame::Focus { focused: true });
    core.suspend();
    assert_eq!(next(&mut stream).await, Frame::Suspend);

    let owner = CoreEvent::AlertsOwner(AlertOwner {
        connection: Some(7),
    });
    let event = Frame::Event {
        name: "alerts:owner".into(),
        payload: json!({ "connection": 7 }),
    };
    write_frame(&mut stream, &event).await.expect("event");
    let got = received(&mut events).await;
    assert_eq!(got.core(), Some(&owner));
    assert_eq!(got, ClientEvent::Event(owner));

    drop(stream);
    let closed = received(&mut events).await;
    assert_eq!(closed, ClientEvent::Closed);
    assert_eq!(closed.core(), None);
}

/// With `reconnect`, a core that stops is started again with a wait that doubles after each
/// failed start, and the new connection is told the window's focus.
#[tokio::test]
async fn a_stopped_core_is_reconnected_with_growing_waits() {
    let profile = tempfile::tempdir().expect("profile");
    let runtime = tempfile::tempdir().expect("runtime");
    kchess_host::env::set("KCHESS_RUNTIME_DIR", runtime.path());
    let listener = fake_core(profile.path());
    let (failed, mut failures) = tokio::sync::mpsc::unbounded_channel();
    let log: Log = Arc::new(move |level, _, message| {
        if level == Level::Warn && message.starts_with("The core could not restart") {
            let _ = failed.send(tokio::time::Instant::now());
        }
    });
    let (opened, mut stream) = tokio::join!(
        CoreHandle::open(options(profile.path(), true, log)),
        welcome(&listener, 1)
    );
    let (core, mut events) = opened.expect("opened");
    core.focus(true);
    assert_eq!(next(&mut stream).await, Frame::Focus { focused: true });

    // The core stops and its record goes: every start fails until it is back.
    std::fs::remove_file(profile.path().join(discovery::INFO_FILE)).expect("removed");
    drop(stream);
    let mut at = Vec::new();
    while at.len() < 4 {
        let failure = tokio::time::timeout(WAIT, failures.recv()).await;
        at.push(failure.expect("a start in time").expect("logged"));
    }
    let gaps: Vec<Duration> = at.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert!(
        gaps[1] >= Duration::from_millis(400) && gaps[2] >= Duration::from_millis(800),
        "{gaps:?}"
    );

    advertise(profile.path());
    let mut again = welcome(&listener, 2).await;
    assert_eq!(next(&mut again).await, Frame::Focus { focused: true });
    assert_eq!(received(&mut events).await, ClientEvent::Reconnected);
    assert_eq!(core.connection(), 2);
}

/// A call answered by the core is finished: the client never asks the core to cancel it.
#[tokio::test]
async fn an_answered_call_is_not_cancelled() {
    let profile = tempfile::tempdir().expect("profile");
    let listener = fake_core(profile.path());
    let hello = window();
    let (connected, mut stream) = tokio::join!(
        Client::connect(profile.path(), &hello),
        welcome(&listener, 3)
    );
    let (client, _events) = connected.expect("connected");
    let calling = tokio::spawn(client.call("library", vec![]));
    let Frame::Call { id, .. } = next(&mut stream).await else {
        panic!("a call");
    };
    let answer = Frame::Result {
        id,
        value: Some(json!(1)),
        error: None,
        kind: None,
    };
    write_frame(&mut stream, &answer).await.expect("answer");
    assert_eq!(calling.await.expect("joined"), Ok(json!(1)));
    let _second = tokio::spawn(client.call("settings", vec![]));
    let after = next(&mut stream).await;
    assert!(
        matches!(&after, Frame::Call { method, .. } if method == "settings"),
        "{after:?}"
    );
}
