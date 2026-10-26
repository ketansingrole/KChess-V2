//! The shared core over a real socket (`docs/shared-core.md`): two clients on one profile,
//! the checks at `hello`, the version check, the idle stop, and starting the service binary.

use std::path::Path;
use std::time::Duration;

use kchess_core::Config;
use kchess_service::protocol::{self, Frame, RefuseReason, read_frame, write_frame};
use kchess_service::server::{ServeOptions, serve};
use kchess_service::{
    Capability, Client, ClientEvent, ConnectError, Hello, ServiceConfig, Start, connect_or_start,
    discovery, transport,
};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

const WAIT: Duration = Duration::from_secs(10);

fn window(kind: &str) -> Hello {
    Hello {
        kind: kind.into(),
        capabilities: vec![Capability::Browser, Capability::Focus],
    }
}

/// Serves `profile` in this process; resolves when the service stops.
fn spawn_service(profile: &Path, idle: Duration) -> tokio::task::JoinHandle<Result<(), String>> {
    let options = ServeOptions {
        config: Config::new(profile.to_path_buf()),
        idle,
    };
    tokio::spawn(serve(options))
}

async fn connect(profile: &Path, kind: &str) -> (Client, UnboundedReceiver<ClientEvent>) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        match Client::connect(profile, &window(kind)).await {
            Ok(connected) => return connected,
            Err(ConnectError::NotRunning) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(error) => panic!("could not connect: {error}"),
        }
    }
}

/// The next event named `name`, skipping others.
async fn event(events: &mut UnboundedReceiver<ClientEvent>, name: &str) -> Value {
    tokio::time::timeout(WAIT, async {
        loop {
            match events.recv().await {
                Some(ClientEvent::Event { name: got, payload }) if got == name => return payload,
                Some(_) => {}
                None => panic!("the events ended before {name}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no {name} event"))
}

/// Says `hello` by hand, to test what a well-behaved client never sends.
async fn raw_hello(profile: &Path, token: String, protocol: String) -> Frame {
    let info = discovery::read_info(profile).expect("core.json");
    let stream = transport::connect(Path::new(&info.socket))
        .await
        .expect("socket");
    let (mut reader, mut writer) = tokio::io::split(stream);
    write_frame(
        &mut writer,
        &Frame::Hello {
            token,
            kind: "test".into(),
            release: "1999.1.0".into(),
            protocol,
            capabilities: Vec::new(),
        },
    )
    .await
    .expect("hello");
    read_frame(&mut reader)
        .await
        .expect("answer")
        .expect("a frame")
}

#[tokio::test(flavor = "multi_thread")]
async fn two_windows_share_one_core_and_see_each_others_writes() {
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path(), Duration::from_secs(30));
    let (gpui, _gpui_events) = connect(profile.path(), "gpui").await;
    let (electron, mut electron_events) = connect(profile.path(), "electron").await;
    assert_ne!(gpui.connection(), electron.connection());
    assert_eq!(electron.release(), protocol::release());

    let saved = gpui
        .call(
            "studyCommand",
            vec![json!({ "op": "save", "name": "Shared", "pgn": "1. e4 *" })],
        )
        .await
        .expect("saved");
    let id = saved["id"].as_str().expect("an id").to_string();
    let change = event(&mut electron_events, "profile:changed").await;
    assert_eq!(change["areas"], json!(["studies"]));
    assert_eq!(change["origin"], json!(gpui.connection()));

    let library = electron.call("library", vec![]).await.expect("library");
    let names: Vec<&str> = library["studies"]
        .as_array()
        .expect("studies")
        .iter()
        .filter_map(|study| study["name"].as_str())
        .collect();
    assert!(
        names.contains(&"Shared"),
        "the other window reads it: {names:?}"
    );

    // A save based on the chapter before another window changed it is refused.
    let read = library["studies"]
        .as_array()
        .expect("studies")
        .iter()
        .find(|study| study["id"] == json!(id))
        .and_then(|study| study["chapters"][0]["pgn"].as_str())
        .expect("the chapter's moves")
        .to_string();
    gpui.call(
        "studyCommand",
        vec![json!({ "op": "save", "id": id, "name": "Shared", "pgn": "1. d4 *" })],
    )
    .await
    .expect("the first window saves again");
    let stale = electron
        .call(
            "studyCommand",
            vec![
                json!({ "op": "save", "id": id, "name": "Shared", "pgn": "1. c4 *" }),
                json!(true),
                json!(read),
            ],
        )
        .await
        .expect_err("based on an older study");
    assert!(stale.contains("another KChess window"), "{stale}");

    drop((gpui, electron));
    service.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn hello_is_refused_without_the_token_or_with_another_release() {
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path(), Duration::from_secs(30));
    let (_gpui, _events) = connect(profile.path(), "gpui").await;

    let wrong = raw_hello(profile.path(), "0".repeat(64), protocol::protocol()).await;
    assert!(matches!(
        wrong,
        Frame::Refuse {
            reason: RefuseReason::Unauthorized,
            ..
        }
    ));

    let token = discovery::read_token(profile.path()).expect("token");
    let other = raw_hello(profile.path(), token, "another-release".into()).await;
    let Frame::Refuse {
        reason,
        sole,
        clients,
        ..
    } = other
    else {
        panic!("refused: {other:?}");
    };
    assert_eq!(reason, RefuseReason::Version);
    assert!(!sole, "the GPUI window still uses this core");
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].kind, "gpui");
    assert_eq!(clients[0].release, protocol::release());
    service.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_core_nobody_uses_closes_for_another_release() {
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path(), Duration::from_secs(30));
    let (gpui, _events) = connect(profile.path(), "gpui").await;
    drop(gpui);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let token = discovery::read_token(profile.path()).expect("token");
    let refused = raw_hello(profile.path(), token, "another-release".into()).await;
    assert!(matches!(refused, Frame::Refuse { sole: true, .. }));
    tokio::time::timeout(WAIT, service)
        .await
        .expect("the service closes")
        .expect("joined")
        .expect("closed cleanly");
    assert!(
        discovery::read_info(profile.path()).is_none(),
        "core.json goes with it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn one_core_per_profile_and_it_stops_when_idle() {
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path(), Duration::from_millis(300));
    let (gpui, mut events) = connect(profile.path(), "gpui").await;

    let second = serve(ServeOptions {
        config: Config::new(profile.path().to_path_buf()),
        idle: Duration::from_secs(1),
    })
    .await
    .expect_err("the profile is taken");
    assert!(second.contains("already running"), "{second}");

    // Connected, the service waits past its idle period.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(!service.is_finished());
    drop(gpui);
    tokio::time::timeout(WAIT, service)
        .await
        .expect("idle stop")
        .expect("joined")
        .expect("closed cleanly");
    // A dropped client's events end with Closed once the core is gone.
    let closed = tokio::time::timeout(WAIT, async {
        loop {
            match events.recv().await {
                Some(ClientEvent::Closed) | None => return,
                Some(_) => {}
            }
        }
    })
    .await;
    assert!(closed.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_app_starts_the_service_and_the_second_reuses_it() {
    let profile = tempfile::tempdir().expect("profile");
    let runtime = tempfile::tempdir().expect("runtime");
    use_runtime_dir(runtime.path());
    let start = Start {
        binary: env!("CARGO_BIN_EXE_kchess-service").into(),
        config: ServiceConfig::default(),
    };
    let (gpui, _gpui_events) = connect_or_start(profile.path(), &window("gpui"), &start)
        .await
        .expect("started");
    let info = discovery::read_info(profile.path()).expect("core.json");
    assert_ne!(info.pid, std::process::id(), "a process of its own");
    let copied = runtime.path().join("core").join(protocol::release()).join(
        Path::new(env!("CARGO_BIN_EXE_kchess-service"))
            .file_name()
            .expect("name"),
    );
    assert!(copied.exists(), "it runs from this release's copy");

    let (electron, _electron_events) =
        connect_or_start(profile.path(), &window("electron"), &start)
            .await
            .expect("reused");
    assert_eq!(
        discovery::read_info(profile.path()).expect("core.json").pid,
        info.pid
    );
    assert_ne!(gpui.connection(), electron.connection());
    electron.call("library", vec![]).await.expect("works");

    // Both gone: the service stops after its idle period (10 s by default).
    drop((gpui, electron));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while discovery::read_info(profile.path()).is_some() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the service stops when idle"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_window_that_joins_learns_which_window_alerts() {
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path(), Duration::from_secs(30));
    let (gpui, _gpui_events) = connect(profile.path(), "gpui").await;
    gpui.focus(true);
    // The focus frame is read before the call that follows it.
    gpui.call("library", vec![]).await.expect("library");

    // The owner does not change when the second window joins; it still learns it, first.
    let (electron, mut events) = connect(profile.path(), "electron").await;
    let first = tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("an event")
        .expect("open");
    assert_eq!(
        first,
        ClientEvent::Event {
            name: "alerts:owner".into(),
            payload: json!({ "connection": gpui.connection() }),
        }
    );
    drop((gpui, electron));
    service.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_window_that_stops_reading_is_cut_off_and_cleaned_up() {
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path(), Duration::from_millis(300));
    let (gpui, _gpui_events) = connect(profile.path(), "gpui").await;

    // A window that says hello and then never reads; it joined last, so it alerts unless the
    // GPUI window is focused.
    let info = discovery::read_info(profile.path()).expect("core.json");
    let stream = transport::connect(Path::new(&info.socket))
        .await
        .expect("socket");
    let (_reader, mut writer) = tokio::io::split(stream);
    write_frame(
        &mut writer,
        &Frame::Hello {
            token: discovery::read_token(profile.path()).expect("token"),
            kind: "stalled".into(),
            release: protocol::release(),
            protocol: protocol::protocol(),
            capabilities: vec![Capability::Focus],
        },
    )
    .await
    .expect("hello");
    gpui.call("library", vec![]).await.expect("joined");

    // Every focus change moves the alerts and queues an event for the stalled window.
    for round in 0..40_000 {
        gpui.focus(round % 2 == 0);
        if round % 128 == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    gpui.call("library", vec![]).await.expect("still served");
    drop(gpui);
    // The stalled window keeps its socket open, yet the service is idle and stops.
    tokio::time::timeout(WAIT, service)
        .await
        .expect("the stalled window was disconnected")
        .expect("joined")
        .expect("closed cleanly");
    drop(writer);
}

/// A stand-in core for `profile` that runs `peer` on its first connection.
#[cfg(unix)]
async fn fake_core<F: Future<Output = ()> + Send + 'static>(
    profile: &Path,
    peer: impl FnOnce(tokio::net::UnixStream) -> F + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    let socket = profile.join("fake.sock");
    let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
    let info = discovery::CoreInfo {
        pid: std::process::id(),
        socket: socket.to_string_lossy().into(),
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
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        peer(stream).await;
    })
}

#[cfg(unix)]
#[tokio::test]
async fn a_call_after_the_core_closed_fails_at_once() {
    let profile = tempfile::tempdir().expect("profile");
    let core = fake_core(profile.path(), |mut stream| async move {
        read_frame(&mut stream).await.expect("hello");
        let welcome = Frame::Welcome {
            connection: 1,
            release: protocol::release(),
            protocol: protocol::protocol(),
        };
        write_frame(&mut stream, &welcome).await.expect("welcome");
    })
    .await;
    let (client, mut events) = Client::connect(profile.path(), &window("gpui"))
        .await
        .expect("connected");
    core.await.expect("the core closed");
    assert_eq!(events.recv().await, Some(ClientEvent::Closed));
    let answer = tokio::time::timeout(WAIT, client.call("library", vec![]))
        .await
        .expect("answered");
    assert_eq!(answer, Err("The KChess core stopped.".into()));
}

#[cfg(unix)]
#[tokio::test]
async fn a_core_that_never_answers_hello_fails_the_connection() {
    let profile = tempfile::tempdir().expect("profile");
    let _core = fake_core(profile.path(), |mut stream| async move {
        read_frame(&mut stream).await.expect("hello");
        std::future::pending::<()>().await;
    })
    .await;
    let connected = Client::connect(profile.path(), &window("gpui")).await;
    assert!(
        matches!(connected, Err(ConnectError::Failed(ref message)) if message.contains("did not answer")),
        "{:?}",
        connected.err()
    );
}

/// Points the runtime folder at a test folder. `set_var` is unsafe in edition 2024 because other
/// threads may read the environment; only this test reads `KCHESS_RUNTIME_DIR`, and it sets it
/// before starting anything.
#[allow(unsafe_code)]
fn use_runtime_dir(dir: &Path) {
    // SAFETY: no other thread of this test reads the environment while it is set.
    unsafe { std::env::set_var("KCHESS_RUNTIME_DIR", dir) };
}
