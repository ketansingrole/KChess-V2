//! A profile the Electron app last used, opened by a core with no Electron to hand its
//! `localStorage` over: the first library and view preference reads take it over themselves, once.
//! The fixture is what Electron 45 wrote (`kchess-electron-legacy/tests/fixtures`).
// Tests set process environment (`std::env::set_var`), which is unsafe in edition 2024.
#![allow(unsafe_code)]

use std::path::Path;
use std::sync::Arc;

use kchess_core::{Config, Core, Host, Level};
use serde_json::{Value, json};

struct Quiet;
impl Host for Quiet {
    fn log(&self, _: Level, _: &str, _: &str) {}
    fn emit(&self, _: &str, _: Value) {}
}

fn electron_profile() -> tempfile::TempDir {
    // SAFETY: set once before any core opens; every test here sets the same value.
    unsafe { std::env::set_var("KCHESS_STORE_DEBUG", "1") };
    let profile = tempfile::tempdir().unwrap();
    let target = profile.path().join("Local Storage").join("leveldb");
    std::fs::create_dir_all(&target).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../kchess-electron-legacy/tests/fixtures/local-storage");
    for entry in std::fs::read_dir(fixture).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
    }
    profile
}

#[tokio::test]
async fn the_first_reads_take_over_what_electron_left() {
    let profile = electron_profile();
    let core = Core::open(Config::new(profile.path().to_path_buf()), Arc::new(Quiet)).unwrap();

    let library = core.call("library", vec![]).await.unwrap();
    assert_eq!(library["imported"], json!(true));
    let studies = library["studies"].as_array().unwrap();
    assert_eq!(studies.len(), 1);
    assert_eq!(studies[0]["name"], "Ünïcode ♞ study");
    assert_eq!(studies[0]["chapters"][0]["pgn"], "1. e4 e5 *");

    let preferences = core.call("viewPreferences", vec![]).await.unwrap();
    assert_eq!(preferences["kchess:studies-sort"], json!("updated"));
    assert_eq!(preferences["kchess:sidebar-open"], json!(false));
    assert_eq!(
        preferences["kchess:puzzle-angle"],
        json!({ "theme": "fork" })
    );
    assert!(
        preferences.get("kchess:studies:v1").is_none(),
        "documents are not preferences"
    );

    // Taken over once: a later change here is not undone by the leftovers.
    core.call(
        "setViewPreference",
        vec![json!("kchess:studies-sort"), json!("name")],
    )
    .await
    .unwrap();
    let preferences = core.call("viewPreferences", vec![]).await.unwrap();
    assert_eq!(preferences["kchess:studies-sort"], json!("name"));
    core.close().await;
}

#[tokio::test]
async fn an_unreadable_local_storage_is_tried_again_later() {
    let profile = electron_profile();
    let log = profile.path().join("Local Storage/leveldb/000003.log");
    let good = std::fs::read(&log).unwrap();
    // A MANIFEST naming a missing table: LevelDB refuses to open the copy.
    let manifest = profile.path().join("Local Storage/leveldb/MANIFEST-000001");
    let kept = std::fs::read(&manifest).unwrap();
    std::fs::write(&manifest, b"not a manifest").unwrap();
    let core = Core::open(Config::new(profile.path().to_path_buf()), Arc::new(Quiet)).unwrap();
    let library = core.call("library", vec![]).await.unwrap();
    assert_eq!(
        library["imported"],
        json!(false),
        "nothing is marked as taken over"
    );
    assert!(library["studies"].as_array().unwrap().is_empty());

    std::fs::write(&manifest, kept).unwrap();
    std::fs::write(&log, good).unwrap();
    let library = core.call("library", vec![]).await.unwrap();
    assert_eq!(library["imported"], json!(true));
    assert_eq!(library["studies"].as_array().unwrap().len(), 1);
    core.close().await;
}
