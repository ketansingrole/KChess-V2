//! The offline tablebases download from a local stand-in for Lichess's mirror: each file kept only
//! when it matches its checksum, a download goes on from the files it has and stops when
//! cancelled, and the core answers a tablebase lookup from the tables on disk without asking
//! Lichess.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use kchess_contracts::types::{TablebaseProgress, TablebaseStatus};
use kchess_core::tablebase::{FILES, TableFile, Tablebases, parse_files};
use kchess_core::usage::Unmetered;
use kchess_core::{Config, Core, TestSwitches};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::fixtures::{self, Fixture, Reply};
use crate::support;

const KQVK_WIN: &str = "7k/8/6K1/8/8/8/8/1Q6 w - - 0 1";

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tablebase/fixtures")
}

/// The four fixture tables, as the list names them.
fn listed() -> Vec<TableFile> {
    parse_files(FILES)
        .unwrap()
        .into_iter()
        .filter(|file| file.name.starts_with("KQvK.") || file.name.starts_with("KRvK."))
        .collect()
}

/// The mirror's stand-in: each table's bytes by name, served under its folder; `gate` holds back
/// `KRvK.rtbz` until it opens.
#[derive(Clone)]
struct Mirror {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    gate: watch::Receiver<bool>,
}

impl Mirror {
    fn new(gate: watch::Receiver<bool>) -> Mirror {
        let files = listed()
            .into_iter()
            .map(|file| {
                let bytes = std::fs::read(fixtures_dir().join(&file.name)).unwrap();
                (file.name, bytes)
            })
            .collect();
        Mirror {
            files: Arc::new(Mutex::new(files)),
            gate,
        }
    }

    fn damage(&self, name: &str, damaged: bool) {
        let mut bytes = std::fs::read(fixtures_dir().join(name)).unwrap();
        if damaged {
            bytes[0] ^= 0xff;
        }
        self.files.lock().unwrap().insert(name.into(), bytes);
    }

    async fn serve(&self) -> Fixture {
        let mirror = self.clone();
        fixtures::serve(move |request| {
            let path = request.path();
            let name = path.rsplit('/').next().unwrap_or_default().to_string();
            let folder = if name.ends_with(".rtbz") {
                "dtz"
            } else {
                "wdl"
            };
            let expected = format!("/tablebase/tables/standard/3-4-5-{folder}/{name}");
            let bytes = mirror.files.lock().unwrap().get(&name).cloned();
            match bytes {
                Some(bytes) if path == expected => {
                    let reply = Reply::Bytes(200, Vec::new(), bytes);
                    if name == "KRvK.rtbz" {
                        Reply::Gated(mirror.gate.clone(), Box::new(reply))
                    } else {
                        reply
                    }
                }
                _ => Reply::Text(404, "text/plain", String::new()),
            }
        })
        .await
    }
}

fn tablebases(dir: &Path, base: &str) -> (Tablebases, Arc<support::Recording>) {
    let host = support::host();
    let tables = Tablebases::new(
        host.clone(),
        Arc::new(Unmetered),
        dir.to_path_buf(),
        format!("{base}/tablebase/tables/standard"),
        listed(),
    );
    (tables, host)
}

/// What the folder holds besides the tables.
fn leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !name.ends_with(".rtbw") && !name.ends_with(".rtbz"))
        .collect()
}

fn reports() -> (
    Arc<Mutex<Vec<TablebaseProgress>>>,
    impl Fn(TablebaseProgress) + Send + Sync + 'static,
) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    (seen, move |progress| sink.lock().unwrap().push(progress))
}

fn last_phase(seen: &Mutex<Vec<TablebaseProgress>>) -> Value {
    json!(seen.lock().unwrap().last().expect("reported").phase)
}

#[tokio::test]
async fn a_file_that_fails_its_checksum_is_not_kept_and_the_next_download_fetches_only_it() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("tablebases");
    let (open, gate) = watch::channel(true);
    let mirror = Mirror::new(gate);
    mirror.damage("KRvK.rtbz", true);
    let fixture = mirror.serve().await;
    let (tables, _host) = tablebases(&folder, &fixture.base);

    let (seen, report) = reports();
    let failed = tables
        .install(report)
        .await
        .expect_err("one file is damaged");
    assert_eq!(
        failed.message,
        "A tablebase file did not download correctly. Try again."
    );
    assert_eq!(last_phase(&seen), json!("failed"));
    let status = tables.status().await;
    assert_eq!((status.files, status.total_files), (3, 4));
    assert!(!status.installed && !status.busy);
    assert!(leftovers(&folder).is_empty(), "{:?}", leftovers(&folder));

    mirror.damage("KRvK.rtbz", false);
    let (seen, report) = reports();
    let status: TablebaseStatus = tables.install(report).await.expect("downloads");
    assert!(status.installed);
    assert_eq!(status.files, 4);
    let last = seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(json!(last.phase), json!("done"));
    assert_eq!(
        (last.received, last.total),
        (status.total_bytes, status.total_bytes)
    );
    // The files kept the first time were not fetched again.
    assert_eq!(
        fixture
            .to("/tablebase/tables/standard/3-4-5-wdl/KQvK.rtbw")
            .len(),
        1
    );
    assert_eq!(
        fixture
            .to("/tablebase/tables/standard/3-4-5-dtz/KRvK.rtbz")
            .len(),
        2
    );
    let found = tables.lookup(KQVK_WIN, 1).await.expect("answered here");
    assert_eq!(found.saved.category.as_deref(), Some("win"));
    drop(open);
}

#[tokio::test]
async fn a_cancelled_download_keeps_the_files_it_finished() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("tablebases");
    let (open, gate) = watch::channel(false);
    let fixture = Mirror::new(gate).serve().await;
    let (tables, _host) = tablebases(&folder, &fixture.base);

    let (seen, report) = reports();
    let running = tokio::spawn({
        let tables = tables.clone();
        async move { tables.install(report).await }
    });
    fixture
        .requested("/tablebase/tables/standard/3-4-5-dtz/KRvK.rtbz", 1)
        .await;
    assert!(tables.status().await.busy);
    let again = tables.install(|_| {}).await.expect_err("one at a time");
    assert_eq!(again.message, "The tablebases are already downloading.");
    let refused = tables.delete().await.expect_err("not while downloading");
    assert_eq!(refused.message, "The tablebases are still downloading.");

    tables.cancel();
    let status = running.await.unwrap().expect("a cancel is no failure");
    assert_eq!(last_phase(&seen), json!("cancelled"));
    assert!(!status.busy && !status.installed);
    assert!(status.files <= 3);
    assert!(leftovers(&folder).is_empty(), "{:?}", leftovers(&folder));
    drop(open);
}

#[tokio::test]
async fn the_core_answers_a_covered_tablebase_lookup_from_the_tables_on_disk() {
    let profile = tempfile::tempdir().unwrap();
    let folder = profile.path().join("tablebases");
    std::fs::create_dir_all(&folder).unwrap();
    for file in listed() {
        std::fs::copy(fixtures_dir().join(&file.name), folder.join(&file.name)).unwrap();
    }
    let lichess = fixtures::serve(|_| Reply::Text(404, "text/plain", String::new())).await;
    let host = support::host();
    let config = Config {
        test: Some(TestSwitches {
            lichess_origin: Some(lichess.base.clone()),
            ..TestSwitches::default()
        }),
        ..Config::new(profile.path().to_path_buf())
    };
    let core = Arc::new(Core::open(config, host.clone()).expect("opens"));

    let status = core.call("tablebaseStatus", vec![]).await.expect("status");
    assert_eq!(status["files"], 4);
    assert_eq!(status["totalFiles"], 290);
    assert_eq!(status["installed"], false);
    assert_eq!(status["pieces"], 5);

    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");
    let found = core
        .call("positionLookup", vec![json!("tablebase"), json!(KQVK_WIN)])
        .await
        .expect("looks up");
    assert_eq!(found["category"], "win");
    assert_eq!(found["moves"][0]["category"], "loss");
    assert!(
        lichess.requests().is_empty(),
        "asked Lichess: {:?}",
        lichess.requests()
    );

    let deleted = core.call("tablebaseDelete", vec![]).await.expect("deletes");
    assert_eq!(deleted["files"], 0);
    assert!(!folder.exists());
    core.close().await;
}
