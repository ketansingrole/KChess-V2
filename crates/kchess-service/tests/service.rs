//! The shared core over a real socket (`docs/shared-core.md`): two clients on one profile,
//! the checks at `hello`, the version check and the idle stop. Starting the service binary sets
//! the runtime folder in the environment, so it runs in a binary of its own (`service_start.rs`).

use std::path::Path;
use std::time::Duration;

use kchess_contracts::CoreEvent;
use kchess_core::Config;
use kchess_service::protocol::{self, Frame, RefuseReason, read_frame, write_frame};
use kchess_service::server::{ServeOptions, serve};
use kchess_service::{Capability, Client, ClientEvent, ConnectError, Hello, discovery, transport};
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
/// The idle period of the tests that wait for the service to stop. It also bounds how long the
/// first client may take to connect, since it runs from the service's start: 300 ms was too
/// short for a loaded runner, whose client then found no service.
const IDLE: Duration = Duration::from_secs(2);

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
                Some(ClientEvent::Event(got)) if got.name() == name => return got.into_payload(),
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
    let (second, mut second_events) = connect(profile.path(), "second").await;
    assert_ne!(gpui.connection(), second.connection());
    assert_eq!(second.release(), protocol::release());

    let saved = gpui
        .call(
            "studyCommand",
            vec![json!({ "op": "save", "name": "Shared", "pgn": "1. e4 *" })],
        )
        .await
        .expect("saved");
    let id = saved["id"].as_str().expect("an id").to_string();
    let change = event(&mut second_events, "profile:changed").await;
    assert_eq!(change["areas"], json!(["studies"]));
    assert_eq!(change["origin"], json!(gpui.connection()));

    let library = second.call("library", vec![]).await.expect("library");
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
    let stale = second
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
    assert!(stale.message.contains("another KChess window"), "{stale:?}");

    drop((gpui, second));
    service.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_calls_the_core_and_answers_host_requests() {
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path(), Duration::from_secs(30));
    let (gpui, _events) = connect(profile.path(), "gpui").await;

    // Every call is answered in time: a service that stopped answering fails here at once
    // rather than leaving the test waiting.
    let answered = |call| async move {
        tokio::time::timeout(WAIT, call)
            .await
            .expect("the call is answered")
    };
    answered(gpui.call("library", vec![]))
        .await
        .expect("a CoreApi method");
    answered(gpui.call("host.reply", vec![json!(0), json!({ "value": null })]))
        .await
        .expect("host replies still arrive");

    drop(gpui);
    service.abort();
}

/// A `hello` is read up to 64 KiB before its token is checked; a larger one is dropped unread.
#[tokio::test(flavor = "multi_thread")]
async fn hello_is_read_up_to_its_limit() {
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path(), Duration::from_secs(30));
    let (_gpui, _events) = connect(profile.path(), "gpui").await;
    let near = "w".repeat(60_000);
    let (near, _) = Client::connect(profile.path(), &window(&near))
        .await
        .expect("welcomed");
    drop(near);
    let over = "w".repeat(70_000);
    assert!(
        Client::connect(profile.path(), &window(&over))
            .await
            .is_err()
    );
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
    let token = discovery::read_token(profile.path()).expect("token");
    // Until the core has let go of the window that left, it refuses with `sole: false` and
    // stays open; once it has, the refusal is the only client's and the core closes.
    tokio::time::timeout(WAIT, async {
        while !matches!(
            raw_hello(profile.path(), token.clone(), "another-release".into()).await,
            Frame::Refuse { sole: true, .. }
        ) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the core lets go of the window that left");
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
    // The idle period runs from the start, before anyone connected: long enough that a loaded
    // runner's first connection lands inside it.
    let service = spawn_service(profile.path(), IDLE);
    let (gpui, mut events) = connect(profile.path(), "gpui").await;

    let second = serve(ServeOptions {
        config: Config::new(profile.path().to_path_buf()),
        idle: Duration::from_secs(1),
    })
    .await
    .expect_err("the profile is taken");
    assert!(second.contains("already running"), "{second}");

    // Connected, the service waits past its idle period.
    tokio::time::sleep(IDLE * 2).await;
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

/// A host request goes to the window that can answer it (the caller here), not to every window.
#[tokio::test(flavor = "multi_thread")]
async fn a_host_request_reaches_only_the_window_that_answers_it() {
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path(), Duration::from_secs(30));
    let (gpui, mut gpui_events) = connect(profile.path(), "gpui").await;
    let (_second, mut second_events) = connect(profile.path(), "second").await;
    // Signing in opens the browser through the window that asked.
    let signing_in = tokio::spawn(async move { gpui.call("connectLichess", vec![]).await });
    // The core also asks a window whether it runs on battery; that one may go anywhere.
    let request = loop {
        let request = event(&mut gpui_events, "host:request").await;
        if request["kind"] == "openExternal" {
            break request;
        }
    };
    assert!(
        request["payload"]
            .as_str()
            .is_some_and(|url| url.contains("/oauth"))
    );
    let elsewhere = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            match second_events.recv().await {
                Some(ClientEvent::Event(CoreEvent::HostRequest(payload)))
                    if payload["kind"] == "openExternal" =>
                {
                    return;
                }
                Some(_) => {}
                None => std::future::pending::<()>().await,
            }
        }
    })
    .await;
    assert!(elsewhere.is_err(), "the other window is not asked");
    signing_in.abort();
    service.abort();
}

/// 2026-11-11: no client sent `cancel`, so a call whose caller gave up (a page that moved on, a
/// command stopped) kept running in the shared core until it ended. Dropping the call cancels
/// it there: the sign-in it started stops listening for the browser's return.
#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_call_stops_its_work_in_the_core() {
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path(), Duration::from_secs(30));
    let (gpui, mut events) = connect(profile.path(), "gpui").await;
    // The client stays connected: only the call goes.
    let signing_in = tokio::spawn(gpui.call("connectLichess", vec![]));
    let request = loop {
        let request = event(&mut events, "host:request").await;
        if request["kind"] == "openExternal" {
            break request;
        }
    };
    let page = request["payload"].as_str().expect("the sign-in page");
    let port: u16 = page
        .split("redirect_uri=http%3A%2F%2F127.0.0.1%3A")
        .nth(1)
        .and_then(|rest| rest.split('%').next())
        .and_then(|port| port.parse().ok())
        .unwrap_or_else(|| panic!("a loopback redirect in {page}"));
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok(),
        "the sign-in listens for the browser"
    );
    signing_in.abort();
    let stopped = tokio::time::timeout(WAIT, async {
        while tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        stopped.is_ok(),
        "the sign-in still listens after its call was dropped"
    );
    gpui.call("library", vec![])
        .await
        .expect("the client is still connected");
    service.abort();
}

/// A client may cancel a call it made: the call stops and is answered as cancelled.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_call_is_answered_as_cancelled() {
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path(), Duration::from_secs(30));
    let (_gpui, _events) = connect(profile.path(), "gpui").await;
    let info = discovery::read_info(profile.path()).expect("core.json");
    let stream = transport::connect(Path::new(&info.socket))
        .await
        .expect("socket");
    let (mut reader, mut writer) = tokio::io::split(stream);
    let hello = Frame::Hello {
        token: discovery::read_token(profile.path()).expect("token"),
        kind: "raw".into(),
        release: protocol::release(),
        protocol: protocol::protocol(),
        capabilities: vec![Capability::Browser],
    };
    write_frame(&mut writer, &hello).await.expect("hello");
    let next = async |reader: &mut tokio::io::ReadHalf<transport::Stream>| {
        tokio::time::timeout(WAIT, read_frame(reader))
            .await
            .expect("a frame in time")
            .expect("read")
            .expect("a frame")
    };
    assert!(matches!(next(&mut reader).await, Frame::Welcome { .. }));
    // Signing in waits for the browser this client never opens.
    let call = Frame::Call {
        id: 1,
        method: "connectLichess".into(),
        args: Vec::new(),
    };
    write_frame(&mut writer, &call).await.expect("call");
    loop {
        if let Frame::Event { name, payload } = next(&mut reader).await
            && name == "host:request"
            && payload["kind"] == "openExternal"
        {
            break;
        }
    }
    write_frame(&mut writer, &Frame::Cancel { id: 1 })
        .await
        .expect("cancel");
    let answer = loop {
        if let Frame::Result { id: 1, error, .. } = next(&mut reader).await {
            break error;
        }
    };
    assert_eq!(answer.as_deref(), Some("The call was cancelled."));
    service.abort();
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
    let (second, mut events) = connect(profile.path(), "second").await;
    let first = tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("an event")
        .expect("open");
    assert_eq!(
        first,
        ClientEvent::Event(CoreEvent::AlertsOwner(
            json!({ "connection": gpui.connection() })
        ))
    );
    drop((gpui, second));
    service.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_window_that_stops_reading_is_cut_off_and_cleaned_up() {
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path(), IDLE);
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

    // Every focus change moves the alerts and queues an event for every window, the stalled one
    // included. GPUI's window reads its own: a call's answer comes after the events queued before
    // it, so answering one every 512 changes keeps it well inside its queue. Without that it was
    // the one cut off on a loaded Windows runner, behind on a 40 000-event burst ("The KChess core
    // stopped" at the next call), while the stalled window's backlog still grows past the queue.
    for round in 0..40_000 {
        gpui.focus(round % 2 == 0);
        if round % 512 == 511 {
            gpui.call("library", vec![]).await.expect("kept up");
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

#[cfg(unix)]
#[tokio::test]
async fn a_refusal_reaches_the_client_with_its_reason() {
    let profile = tempfile::tempdir().expect("profile");
    let _core = fake_core(profile.path(), |mut stream| async move {
        read_frame(&mut stream).await.expect("hello");
        let refuse = Frame::Refuse {
            reason: RefuseReason::Version,
            message: "Another KChess release is open.".into(),
            sole: false,
            clients: vec![protocol::ClientSummary {
                kind: "second".into(),
                release: "2026.1.1".into(),
            }],
        };
        write_frame(&mut stream, &refuse).await.expect("refuse");
    })
    .await;
    let refused = Client::connect(profile.path(), &window("gpui"))
        .await
        .err()
        .expect("refused");
    assert_eq!(refused.to_string(), "Another KChess release is open.");
    let ConnectError::Refused {
        reason,
        sole,
        clients,
        ..
    } = refused
    else {
        panic!("refused: {refused:?}");
    };
    assert_eq!(reason, RefuseReason::Version);
    assert!(!sole);
    assert_eq!(clients[0].kind, "second");
}

#[tokio::test]
async fn connect_errors_read_as_their_message() {
    let profile = tempfile::tempdir().expect("profile");
    let missing = Client::connect(profile.path(), &window("gpui"))
        .await
        .err()
        .expect("no core");
    assert_eq!(missing, ConnectError::NotRunning);
    assert_eq!(missing.to_string(), "No KChess core runs for this profile.");
    assert_eq!(
        ConnectError::Failed("Broken.".into()).to_string(),
        "Broken."
    );
}

/// `kchess-service version` prints what a client of this build says in `hello`.
#[test]
fn the_binary_prints_its_release_and_protocol() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_kchess-service"))
        .arg("version")
        .output()
        .expect("ran");
    assert!(output.status.success());
    let printed: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(
        printed,
        json!({ "release": protocol::release(), "protocol": protocol::protocol() })
    );
    let usage = std::process::Command::new(env!("CARGO_BIN_EXE_kchess-service"))
        .output()
        .expect("ran");
    assert_eq!(usage.status.code(), Some(2));
}

/// A window that suspends stops only its own work, and no call closes the shared core: neither
/// lifecycle step is a core method.
#[tokio::test(flavor = "multi_thread")]
async fn a_window_cannot_close_the_shared_core() {
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path(), Duration::from_secs(30));
    let (gpui, _events) = connect(profile.path(), "gpui").await;
    gpui.suspend();
    for name in ["suspend", "close"] {
        let refused = gpui.call(name, vec![]).await.expect_err("not a method");
        assert!(refused.message.contains("Unknown core method"), "{refused}");
    }
    gpui.call("library", vec![]).await.expect("still served");
    assert!(!service.is_finished());
    service.abort();
}

/// Once a core decides to close for another release, nobody else is let in.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_closing_core_lets_nobody_in() {
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path(), Duration::from_secs(30));
    let (gpui, _events) = connect(profile.path(), "gpui").await;
    let socket =
        std::path::PathBuf::from(discovery::read_info(profile.path()).expect("info").socket);
    drop(gpui);
    let token = discovery::read_token(profile.path()).expect("token");
    let info = discovery::read_info(profile.path()).expect("core.json");
    let say_hello = |stream: transport::Stream, protocol: String| {
        let token = token.clone();
        async move {
            let (mut reader, mut writer) = tokio::io::split(stream);
            let hello = Frame::Hello {
                token,
                kind: "test".into(),
                release: "1999.1.0".into(),
                protocol,
                capabilities: Vec::new(),
            };
            write_frame(&mut writer, &hello).await.expect("hello");
            read_frame(&mut reader).await.ok().flatten()
        }
    };
    // Both connect before the refusal, so the second is answered after the core decided. Until
    // the core has let go of the window that left, the refusal is not the only client's
    // (`sole: false`) and changes nothing: try again.
    let second = tokio::time::timeout(WAIT, async {
        loop {
            let first = transport::connect(Path::new(&info.socket))
                .await
                .expect("socket");
            let second = transport::connect(Path::new(&info.socket))
                .await
                .expect("socket");
            match say_hello(first, "another-release".into()).await {
                Some(Frame::Refuse { sole: true, .. }) => return second,
                Some(Frame::Refuse { sole: false, .. }) => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                answer => panic!("not refused for its release: {answer:?}"),
            }
        }
    })
    .await
    .expect("the core lets go of the window that left");
    let late = say_hello(second, protocol::protocol()).await;
    assert!(
        !matches!(late, Some(Frame::Welcome { .. })),
        "welcomed into a closing core: {late:?}"
    );
    tokio::time::timeout(WAIT, service)
        .await
        .expect("the service closes")
        .expect("joined")
        .expect("closed cleanly");
    assert!(!socket.exists(), "the socket goes with it");
}
