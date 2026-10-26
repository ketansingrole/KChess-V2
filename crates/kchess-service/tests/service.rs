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

    // A save based on the study before another window changed it is refused.
    let updated_at = library["studies"]
        .as_array()
        .expect("studies")
        .iter()
        .find(|study| study["id"] == json!(id))
        .and_then(|study| study["updatedAt"].as_f64())
        .expect("updatedAt");
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
                json!(false),
                json!(updated_at),
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

/// Points the runtime folder at a test folder. `set_var` is unsafe in edition 2024 because other
/// threads may read the environment; only this test reads `KCHESS_RUNTIME_DIR`, and it sets it
/// before starting anything.
#[allow(unsafe_code)]
fn use_runtime_dir(dir: &Path) {
    // SAFETY: no other thread of this test reads the environment while it is set.
    unsafe { std::env::set_var("KCHESS_RUNTIME_DIR", dir) };
}
