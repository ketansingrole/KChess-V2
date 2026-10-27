//! Voice input belongs to one client at a time (`facade/voice.rs`): a client's disconnect or
//! suspension retires only its own recognizer session, never one another client started since.
//! A single-thread runtime makes the release task run only when the test yields, so the
//! interleavings below are deterministic. The helper is the scripted `kchess-fake-voice`.

use std::sync::Arc;

use kchess_core::connection::ClientInfo;
use kchess_core::{Config, Core, Host, Level};
use serde_json::{Value, json};

struct Quiet;

impl Host for Quiet {
    fn log(&self, _level: Level, _scope: &str, _message: &str) {}

    fn emit(&self, _event: &str, _payload: Value) {}
}

fn open(dir: &tempfile::TempDir) -> Core {
    std::fs::create_dir_all(kchess_core::voice::model_dir(dir.path())).expect("model folder");
    let mut config = Config::new(dir.path().into());
    config.voice_helper_path = Some(env!("CARGO_BIN_EXE_kchess-fake-voice").into());
    Core::open(config, Arc::new(Quiet)).expect("core")
}

/// Silent audio: the scripted helper answers a partial "e" while it is listening.
const SILENCE: &str = "AAA=";

// Audit 60a61e8 #3: A's disconnect cleanup ran after B's start and dropped B's helper, so B's
// next audio failed with "Voice input isn’t listening."
#[tokio::test(flavor = "current_thread")]
async fn a_disconnect_keeps_the_session_another_client_started_since() {
    let dir = tempfile::tempdir().expect("data folder");
    let core = open(&dir);
    let a = core.connect(ClientInfo::app());
    let b = core.connect(ClientInfo::app());
    core.call_as(a, "voiceStart", vec![json!(["e four"]), json!(16000)])
        .await
        .expect("A starts");
    core.disconnect(a);
    core.call_as(b, "voiceStart", vec![json!(["d four"]), json!(16000)])
        .await
        .expect("B starts");
    tokio::task::yield_now().await;
    let heard = core
        .call_as(b, "voiceAccept", vec![json!(SILENCE)])
        .await
        .expect("B is still listening");
    assert_eq!(heard["partial"], "e");
    core.call("close", vec![]).await.expect("close");
}

#[tokio::test(flavor = "current_thread")]
async fn a_suspension_keeps_the_session_another_client_started_since() {
    let dir = tempfile::tempdir().expect("data folder");
    let core = open(&dir);
    let a = core.connect(ClientInfo::app());
    let b = core.connect(ClientInfo::app());
    core.call_as(a, "voiceStart", vec![json!(["e four"]), json!(16000)])
        .await
        .expect("A starts");
    core.suspend_connection(a);
    core.call_as(b, "voiceStart", vec![json!(["d four"]), json!(16000)])
        .await
        .expect("B starts");
    tokio::task::yield_now().await;
    core.call_as(b, "voiceAccept", vec![json!(SILENCE)])
        .await
        .expect("B is still listening");
    core.call("close", vec![]).await.expect("close");
}

#[tokio::test(flavor = "current_thread")]
async fn a_disconnect_still_stops_its_own_session() {
    let dir = tempfile::tempdir().expect("data folder");
    let core = open(&dir);
    let a = core.connect(ClientInfo::app());
    let b = core.connect(ClientInfo::app());
    core.call_as(a, "voiceStart", vec![json!(["e four"]), json!(16000)])
        .await
        .expect("A starts");
    core.disconnect(a);
    tokio::task::yield_now().await;
    // Nobody has the microphone now: B's audio reaches a stopped recognizer.
    let refused = core
        .call_as(b, "voiceAccept", vec![json!(SILENCE)])
        .await
        .expect_err("A's session is stopped");
    assert_eq!(refused.message, "Voice input isn’t listening.");
    core.call("close", vec![]).await.expect("close");
}

#[tokio::test(flavor = "current_thread")]
async fn another_clients_audio_finish_and_stop_leave_the_owner_listening() {
    let dir = tempfile::tempdir().expect("data folder");
    let core = open(&dir);
    let a = core.connect(ClientInfo::app());
    let b = core.connect(ClientInfo::app());
    core.call_as(a, "voiceStart", vec![json!(["e four"]), json!(16000)])
        .await
        .expect("A starts");
    core.call_as(b, "voiceStart", vec![json!(["d four"]), json!(16000)])
        .await
        .expect("B takes the microphone");
    for (method, args) in [
        ("voiceAccept", vec![json!(SILENCE)]),
        ("voiceFinish", vec![]),
    ] {
        let refused = core
            .call_as(a, method, args)
            .await
            .expect_err("A no longer listens");
        assert_eq!(
            refused.message,
            "Voice input is in use in another KChess window."
        );
    }
    core.call_as(a, "voiceStop", vec![])
        .await
        .expect("a stale stop is answered");
    core.call_as(b, "voiceAccept", vec![json!(SILENCE)])
        .await
        .expect("B is still listening");
    core.call("close", vec![]).await.expect("close");
}
