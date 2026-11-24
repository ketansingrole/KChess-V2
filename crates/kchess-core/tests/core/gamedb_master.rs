//! The master games download through the core's methods, against a local copy of Lichess's
//! broadcast archive: `list.txt`, `sha256sums.txt` and tiny monthly `.pgn.zst` files.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kchess_core::{Config, Core, TestSwitches};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::sync::mpsc;

use crate::fixtures::{self, Fixture, Reply};
use crate::support;

const ARCHIVE: &str = "/database/broadcast/";

/// Plies of `long_line`: past the opening tree (40), so its games reach the position index.
const LONG_PLIES: usize = 44;

/// Knights out and back, `LONG_PLIES` plies: the start's pawns and pieces throughout.
fn long_line() -> String {
    (0..LONG_PLIES / 2)
        .map(|n| {
            let (white, black) = if n % 2 == 0 {
                ("Nf3", "Nf6")
            } else {
                ("Ng1", "Ng8")
            };
            format!("{}. {white} {black}", n + 1)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A month's file: two games, the first one long, packed as the archive packs them.
fn month_file(month: &str) -> Vec<u8> {
    let date = month.replace('-', ".");
    let long = long_line();
    let pgn = format!(
        "[Event \"Open {month}\"]\n[Date \"{date}.01\"]\n[White \"Ana {month}\"]\n\
         [Black \"Ben\"]\n[Result \"1-0\"]\n[WhiteElo \"2700\"]\n[BlackElo \"2650\"]\n\n\
         {long} 1-0\n\n\
         [Event \"Open {month}\"]\n[Date \"{date}.02\"]\n[White \"Ben\"]\n\
         [Black \"Ana {month}\"]\n[Result \"1/2-1/2\"]\n\n1. d4 d5 1/2-1/2\n"
    );
    ruzstd::encoding::compress_to_vec(pgn.as_bytes(), ruzstd::encoding::CompressionLevel::Fastest)
}

fn file_name(month: &str) -> String {
    format!("lichess_db_broadcast_{month}.pgn.zst")
}

/// The archive the fixture serves: its files by name, and months whose body a test streams.
#[derive(Clone, Default)]
struct Archive {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    streams: Arc<Mutex<HashMap<String, mpsc::UnboundedReceiver<String>>>>,
}

impl Archive {
    /// Publishes `months` (newest first in `list.txt`, as Lichess lists them), each with its
    /// digest, except `wrong`, whose digest is not its file's.
    fn publish(&self, months: &[&str], wrong: Option<&str>) {
        let mut files = self.files.lock().unwrap();
        let mut list = Vec::new();
        let mut sums = Vec::new();
        for month in months {
            let name = file_name(month);
            let body = month_file(month);
            let mut digest = kchess_delta::hex(&Sha256::digest(&body));
            if wrong == Some(*month) {
                digest = "0".repeat(64);
            }
            // Lichess lists its own links: only the names are taken from them.
            list.push(format!("https://database.lichess.org/broadcast/{name}"));
            sums.push(format!("{digest}  {name}"));
            files.insert(name, body);
        }
        list.reverse();
        files.insert("list.txt".into(), list.join("\n").into_bytes());
        files.insert(
            "sha256sums.txt".into(),
            format!("{}\n", sums.join("\n")).into_bytes(),
        );
    }

    /// Lists `month` and streams its body through the sender this answers, never ending it.
    fn stream(&self, month: &str) -> mpsc::UnboundedSender<String> {
        let (send, receive) = mpsc::unbounded_channel();
        self.streams
            .lock()
            .unwrap()
            .insert(file_name(month), receive);
        send
    }

    async fn serve(&self) -> Fixture {
        let archive = self.clone();
        fixtures::serve(move |request| {
            let Some(name) = request.path().strip_prefix(ARCHIVE) else {
                return Reply::Text(404, "text/plain", String::new());
            };
            if let Some(stream) = archive.streams.lock().unwrap().remove(name) {
                return Reply::StreamText(200, "application/octet-stream", stream);
            }
            match archive.files.lock().unwrap().get(name) {
                Some(body) => Reply::Bytes(
                    200,
                    vec![("Content-Type", "application/octet-stream".into())],
                    body.clone(),
                ),
                None => Reply::Text(404, "text/plain", String::new()),
            }
        })
        .await
    }
}

fn open(profile: &Path, base: &str) -> (Arc<Core>, Arc<support::Recording>) {
    let host = support::host();
    let config = Config {
        test: Some(TestSwitches {
            lichess_origin: Some(base.to_string()),
            ..TestSwitches::default()
        }),
        ..Config::new(profile.to_path_buf())
    };
    (
        Arc::new(Core::open(config, host.clone()).expect("opens")),
        host,
    )
}

/// What the databases folder holds besides the databases themselves.
fn leftovers(profile: &Path) -> Vec<String> {
    std::fs::read_dir(profile.join("databases"))
        .map(|entries| {
            entries
                .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
                .filter(|name| {
                    !name.ends_with(".kdb") && !name.ends_with("-wal") && !name.ends_with("-shm")
                })
                .collect()
        })
        .unwrap_or_default()
}

fn phases(host: &support::Recording) -> Vec<(String, Option<String>)> {
    host.named("gamedb:progress")
        .iter()
        .map(|p| {
            (
                p["phase"].as_str().expect("a phase").to_string(),
                p["message"].as_str().map(str::to_string),
            )
        })
        .collect()
}

/// The games the master database's position index lists past the tree for the start's
/// structure, where only the long games stay.
fn deep_games(profile: &Path) -> usize {
    use shakmaty::variant::{Variant, VariantPosition};
    let db = rusqlite::Connection::open_with_flags(
        profile.join("databases/master.kdb"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("opens the database");
    let start = VariantPosition::new(Variant::Chess);
    kchess_core::gamedb::deep::games(&db, kchess_domain::gamedb::structure_key(&start))
        .expect("reads the index")
        .len()
}

async fn master_games(core: &Core) -> u64 {
    let status = core.call("gameDbStatus", vec![]).await.expect("status");
    status["databases"]
        .as_array()
        .expect("databases")
        .iter()
        .find(|database| database["id"] == "master")
        .map_or(0, |database| database["games"].as_u64().expect("a count"))
}

fn month_requests(fixture: &Fixture, month: &str) -> usize {
    fixture.to(&format!("{ARCHIVE}{}", file_name(month))).len()
}

/// Waits until some of `month` is in its download file, reported as downloading.
async fn downloading(profile: &Path, host: &support::Recording, month: &str) {
    let partial = profile.join(format!("databases/master-{month}.pgn.zst.partial"));
    let arrived = async {
        while std::fs::metadata(&partial).map_or(0, |m| m.len()) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    };
    tokio::time::timeout(fixtures::REQUEST_WAIT, arrived)
        .await
        .expect("the month starts to arrive");
    assert!(
        host.named("gamedb:progress")
            .iter()
            .any(|p| p["phase"] == "downloading" && p["message"] == month),
        "{:?}",
        phases(host)
    );
}

#[tokio::test]
async fn the_master_games_install_by_month_and_a_later_call_brings_only_new_months() {
    let profile = tempfile::tempdir().expect("profile");
    let archive = Archive::default();
    archive.publish(&["2026-08", "2026-09"], None);
    let fixture = archive.serve().await;
    let (core, host) = open(profile.path(), &fixture.base);
    let installed = core
        .call("gameDbMasterInstall", vec![])
        .await
        .expect("installs");
    assert_eq!(installed["added"], 4);
    let database = &installed["database"];
    assert_eq!(database["id"], "master");
    assert_eq!(database["name"], "Master games");
    assert_eq!(database["source"], "master");
    assert_eq!(database["games"], 4);
    assert_eq!(
        database["attribution"],
        "Lichess broadcast games, database.lichess.org, CC BY-SA 4.0"
    );
    let reported = phases(&host);
    // Each month arrives, then is read; the run ends once, as done.
    let downloads: Vec<Option<String>> = reported
        .iter()
        .filter(|(phase, _)| phase == "downloading")
        .map(|(_, month)| month.clone())
        .collect();
    assert!(downloads.contains(&Some("2026-08".into())), "{reported:?}");
    assert!(downloads.contains(&Some("2026-09".into())), "{reported:?}");
    assert!(reported.iter().any(|(phase, _)| phase == "indexing"));
    assert_eq!(
        reported.last().map(|(phase, _)| phase.as_str()),
        Some("done")
    );
    assert_eq!(
        reported.iter().filter(|(phase, _)| phase == "done").count(),
        1
    );
    assert!(leftovers(profile.path()).is_empty());

    // A month later: only the new month is downloaded.
    archive.publish(&["2026-08", "2026-09", "2026-10"], None);
    let topped = core
        .call("gameDbMasterInstall", vec![])
        .await
        .expect("tops up");
    assert_eq!(topped["added"], 2);
    assert_eq!(topped["database"]["games"], 6);
    assert_eq!(
        ["2026-08", "2026-09", "2026-10"].map(|m| month_requests(&fixture, m)),
        [1, 1, 1]
    );
    // The added month reaches the position index past the tree, as an import's games do: each
    // month's long game is there.
    assert_eq!(deep_games(profile.path()), 3);
    // Nothing new: nothing downloaded, the database as it was.
    let again = core
        .call("gameDbMasterInstall", vec![json!(2)])
        .await
        .expect("nothing to bring");
    assert_eq!(again["added"], 0);
    assert_eq!(again["database"]["games"], 6);
    assert_eq!(month_requests(&fixture, "2026-10"), 1);
    // The games are searchable as any database's.
    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");
    let page = core
        .call(
            "gameDbGames",
            vec![
                json!({ "database": "master", "player": "ana 2026-10", "offset": 0, "limit": 20 }),
            ],
        )
        .await
        .expect("searches");
    assert_eq!(page["total"], 2);
    core.close().await;
}

#[tokio::test]
async fn keeping_to_the_newest_months_brings_only_those() {
    let profile = tempfile::tempdir().expect("profile");
    let archive = Archive::default();
    archive.publish(&["2026-08", "2026-09", "2026-10"], None);
    let fixture = archive.serve().await;
    let (core, _) = open(profile.path(), &fixture.base);
    let refused = core
        .call("gameDbMasterInstall", vec![json!(0)])
        .await
        .expect_err("at least a month");
    assert_eq!(refused.message, "Invalid number of months.");
    let installed = core
        .call("gameDbMasterInstall", vec![json!(1)])
        .await
        .expect("installs");
    assert_eq!(installed["added"], 2);
    assert_eq!(
        ["2026-08", "2026-09", "2026-10"].map(|m| month_requests(&fixture, m)),
        [0, 0, 1]
    );
    core.close().await;
}

#[tokio::test]
async fn a_month_that_fails_its_checksum_is_removed_and_the_months_in_are_kept() {
    let profile = tempfile::tempdir().expect("profile");
    let archive = Archive::default();
    archive.publish(&["2026-08", "2026-09"], None);
    let fixture = archive.serve().await;
    let (core, host) = open(profile.path(), &fixture.base);
    core.call("gameDbMasterInstall", vec![])
        .await
        .expect("installs");
    archive.publish(&["2026-08", "2026-09", "2026-10"], Some("2026-10"));
    let failed = core
        .call("gameDbMasterInstall", vec![])
        .await
        .expect_err("the digest does not match");
    assert_eq!(
        failed.message,
        "The master games of 2026-10 did not arrive intact. Try again."
    );
    assert_eq!(month_requests(&fixture, "2026-10"), 1);
    assert!(leftovers(profile.path()).is_empty());
    assert_eq!(master_games(&core).await, 4);
    let last: Value = host.named("gamedb:progress").pop().expect("reported");
    assert_eq!(last["phase"], "failed");
    assert_eq!(last["message"], failed.message);
    // Fixed at the source, the month comes in.
    archive.publish(&["2026-08", "2026-09", "2026-10"], None);
    let topped = core
        .call("gameDbMasterInstall", vec![])
        .await
        .expect("tops up");
    assert_eq!(topped["added"], 2);
    core.close().await;
}

#[tokio::test]
async fn a_bad_first_month_leaves_no_database() {
    let profile = tempfile::tempdir().expect("profile");
    let archive = Archive::default();
    archive.publish(&["2026-08"], Some("2026-08"));
    let fixture = archive.serve().await;
    let (core, _) = open(profile.path(), &fixture.base);
    core.call("gameDbMasterInstall", vec![])
        .await
        .expect_err("the digest does not match");
    let status = core.call("gameDbStatus", vec![]).await.expect("status");
    assert_eq!(status["databases"], json!([]));
    assert!(leftovers(profile.path()).is_empty());
    core.close().await;
}

#[tokio::test]
async fn a_cancelled_download_removes_its_month_and_keeps_the_months_in() {
    let profile = tempfile::tempdir().expect("profile");
    let archive = Archive::default();
    archive.publish(&["2026-08", "2026-09"], None);
    let body = archive.stream("2026-09");
    let fixture = archive.serve().await;
    let (core, host) = open(profile.path(), &fixture.base);
    let install = {
        let core = Arc::clone(&core);
        tokio::spawn(async move { core.call("gameDbMasterInstall", vec![]).await })
    };
    body.send("the first bytes of the month".into())
        .expect("the fixture streams");
    downloading(profile.path(), &host, "2026-09").await;
    assert_eq!(
        leftovers(profile.path()),
        ["master-2026-09.pgn.zst.partial"]
    );
    let status = core.call("gameDbStatus", vec![]).await.expect("status");
    assert_eq!(status["busy"], true);
    core.call("gameDbCancel", vec![]).await.expect("cancels");
    let answer = tokio::time::timeout(fixtures::REQUEST_WAIT, install)
        .await
        .expect("the install ends")
        .expect("the install task");
    assert!(answer.is_err(), "{answer:?}");
    let last: Value = host.named("gamedb:progress").pop().expect("reported");
    assert_eq!(last["phase"], "cancelled");
    assert!(leftovers(profile.path()).is_empty());
    assert_eq!(master_games(&core).await, 2);
    core.close().await;
}

#[tokio::test]
async fn closing_the_core_stops_a_download() {
    let profile = tempfile::tempdir().expect("profile");
    let archive = Archive::default();
    archive.publish(&["2026-08"], None);
    let body = archive.stream("2026-08");
    let fixture = archive.serve().await;
    let (core, host) = open(profile.path(), &fixture.base);
    let install = {
        let core = Arc::clone(&core);
        tokio::spawn(async move { core.call("gameDbMasterInstall", vec![]).await })
    };
    body.send("the first bytes".into())
        .expect("the fixture streams");
    downloading(profile.path(), &host, "2026-08").await;
    tokio::time::timeout(fixtures::REQUEST_WAIT, core.close())
        .await
        .expect("closing returns while the download runs");
    let answer = install.await.expect("the install task");
    assert!(answer.is_err(), "{answer:?}");
    assert!(leftovers(profile.path()).is_empty());
    assert!(!profile.path().join("databases/master.kdb").exists());
}
