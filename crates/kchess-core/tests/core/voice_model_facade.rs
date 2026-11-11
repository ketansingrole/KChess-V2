//! The core's speech model methods (`voiceModelStatus`, `voiceModelInstall`): a client asks the
//! core whether the model is installed and has the core download it, hearing its progress as
//! `voicemodel:progress`. The model comes from a local zip through the test switches.

use kchess_contracts::CoreEvent;
use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};

use kchess_core::{Config, Core, Host, Level, TestSwitches, TestVoiceModel};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const NAME: &str = "test-model";
const MODEL_FILE: &[u8] = b"test model";
const CONF: &[u8] = b"--sample-frequency=16000";

/// Keeps every event the core sends.
#[derive(Default)]
struct Events(Mutex<Vec<(String, Value)>>);

impl Events {
    fn of(&self, name: &str) -> Vec<Value> {
        self.0
            .lock()
            .expect("events")
            .iter()
            .filter(|(event, _)| event == name)
            .map(|(_, payload)| payload.clone())
            .collect()
    }
}

impl Host for Events {
    fn log(&self, _level: Level, _scope: &str, _message: &str) {}

    fn emit(&self, event: CoreEvent) {
        let (event, payload) = (event.name(), event.into_payload());
        self.0
            .lock()
            .expect("events")
            .push((event.to_string(), payload));
    }
}

fn model_zip() -> Vec<u8> {
    let options = zip::write::SimpleFileOptions::default();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_directory(format!("{NAME}/"), options)
        .expect("directory");
    for (name, data) in [
        ("test-model/am/final.mdl", MODEL_FILE),
        ("test-model/conf/model.conf", CONF),
    ] {
        writer.start_file(name, options).expect("start");
        writer.write_all(data).expect("write");
    }
    writer.finish().expect("finish").into_inner()
}

/// A profile at `dir` whose model archive is `archive`, with the digest `sha256`.
fn with_model(dir: &std::path::Path, archive: &std::path::Path, sha256: &str) -> Config {
    Config {
        test: Some(TestSwitches {
            voice_model: Some(TestVoiceModel {
                archive: archive.to_path_buf(),
                name: NAME.into(),
                sha256: sha256.into(),
            }),
            ..TestSwitches::default()
        }),
        ..Config::new(dir.to_path_buf())
    }
}

#[tokio::test]
async fn the_core_downloads_the_model_and_reports_its_progress() {
    let fixtures = tempfile::tempdir().expect("fixtures");
    let zip = model_zip();
    let archive = fixtures.path().join("model.zip");
    std::fs::write(&archive, &zip).expect("archive");
    let digest: String = Sha256::digest(&zip)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let profile = tempfile::tempdir().expect("profile");
    let events = Arc::new(Events::default());
    let core = Core::open(
        with_model(profile.path(), &archive, &digest),
        events.clone(),
    )
    .expect("core");

    let before = core.call("voiceModelStatus", vec![]).await.expect("status");
    assert_eq!(
        before,
        json!({ "installed": false, "bytes": 0, "busy": false })
    );

    let installed = core
        .call("voiceModelInstall", vec![])
        .await
        .expect("install");
    assert_eq!(installed, Value::Null);
    let phases: Vec<Value> = events
        .of("voicemodel:progress")
        .iter()
        .map(|progress| progress["phase"].clone())
        .collect();
    assert!(phases.contains(&json!("downloading")), "{phases:?}");
    assert_eq!(phases.last(), Some(&json!("preparing")), "{phases:?}");

    let after = core.call("voiceModelStatus", vec![]).await.expect("status");
    let bytes = (MODEL_FILE.len() + CONF.len()) as u64;
    assert_eq!(
        after,
        json!({ "installed": true, "bytes": bytes, "busy": false })
    );
    // The recognizer reads the model the cache published.
    assert!(
        kchess_core::voice::model_dir(profile.path())
            .join("am/final.mdl")
            .is_file()
    );

    // A model that cannot be had is the install's error, and nothing is published.
    let other = tempfile::tempdir().expect("profile");
    let missing = fixtures.path().join("missing.zip");
    let core_elsewhere = Core::open(
        with_model(other.path(), &missing, &digest),
        Arc::new(Events::default()),
    )
    .expect("core");
    let failed = core_elsewhere
        .call("voiceModelInstall", vec![])
        .await
        .expect_err("no archive");
    assert!(
        failed.message.contains("Voice model is unavailable"),
        "{}",
        failed.message
    );
    assert_eq!(
        core_elsewhere
            .call("voiceModelStatus", vec![])
            .await
            .expect("status")["installed"],
        json!(false)
    );
    core.close().await;
    core_elsewhere.close().await;
}
