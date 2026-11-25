//! Strong Lichess games through the core's methods, against a local copy of Lichess's archive of
//! rated games: `list.txt`, `sha256sums.txt` and tiny monthly `.pgn.zst` files, of which only the
//! games of two players rated at least the floor, slower than bullet and without a bot, are kept.

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

const ARCHIVE: &str = "/database/standard/";

/// One game of a month: its players' ratings, time control and a bot's title.
fn game(month: &str, n: usize, elos: (u32, u32), control: &str, bot: bool) -> String {
    let title = if bot { "[BlackTitle \"BOT\"]\n" } else { "" };
    format!(
        "[Event \"Rated game\"]\n[Site \"https://lichess.org/{month}{n}\"]\n\
         [White \"Ana{n}\"]\n[Black \"Ben{n}\"]\n[Result \"1-0\"]\n[Date \"{}.01\"]\n\
         [WhiteElo \"{}\"]\n[BlackElo \"{}\"]\n{title}[TimeControl \"{control}\"]\n\n\
         1. e4 {{ [%clk 0:03:00] }} 1... e5 {{ [%clk 0:03:00] }} 2. Nf3 1-0\n\n",
        month.replace('-', "."),
        elos.0,
        elos.1,
    )
}

/// A month's file, packed as the archive packs it: of its six games, two are strong at 2400
/// (a blitz game and a correspondence game), and two more at 2000.
fn month_file(month: &str) -> Vec<u8> {
    let pgn = [
        game(month, 1, (2450, 2410), "180+2", false),
        game(month, 2, (2600, 2550), "60+0", false),
        game(month, 3, (2100, 2050), "600+0", false),
        game(month, 4, (2500, 2900), "300+0", true),
        game(month, 5, (2400, 2400), "-", false),
        game(month, 6, (2399, 2700), "900+10", false),
    ]
    .concat();
    ruzstd::encoding::compress_to_vec(pgn.as_bytes(), ruzstd::encoding::CompressionLevel::Fastest)
}

fn file_name(month: &str) -> String {
    format!("lichess_db_standard_rated_{month}.pgn.zst")
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
            list.push(format!("https://database.lichess.org/standard/{name}"));
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

    /// Streams `month`'s body through the sender this answers, never ending it.
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
            if request.method == "GET"
                && let Some(stream) = archive.streams.lock().unwrap().remove(name)
            {
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

fn month_requests(fixture: &Fixture, month: &str) -> usize {
    fixture
        .to(&format!("{ARCHIVE}{}", file_name(month)))
        .iter()
        .filter(|request| request.method == "GET")
        .count()
}

async fn install(core: &Core, request: Value) -> kchess_contracts::error::Result<Value> {
    core.call("gameDbLichessInstall", vec![request]).await
}

#[tokio::test]
async fn a_month_keeps_only_its_strong_games_and_the_next_call_brings_another_month() {
    let profile = tempfile::tempdir().expect("profile");
    let archive = Archive::default();
    archive.publish(&["2026-09", "2026-10"], None);
    let fixture = archive.serve().await;
    let (core, host) = open(profile.path(), &fixture.base);
    let installed = install(&core, json!({ "minRating": 2400 }))
        .await
        .expect("installs");
    // The newest month first: its blitz and correspondence games of two 2400s.
    assert_eq!(installed["added"], 2);
    let database = &installed["database"];
    assert_eq!(database["id"], "lichess-2400");
    assert_eq!(database["name"], "Lichess 2400+");
    assert_eq!(database["source"], "lichess");
    assert_eq!(database["lichessRating"], 2400);
    assert!(database.get("attribution").is_none(), "{database}");
    assert_eq!(
        ["2026-09", "2026-10"].map(|m| month_requests(&fixture, m)),
        [0, 1]
    );
    let progress = host.named("gamedb:progress");
    assert!(
        progress
            .iter()
            .any(|p| p["phase"] == "downloading" && p["message"] == "2026-10"),
        "{progress:?}"
    );
    assert_eq!(progress.last().expect("reported")["phase"], "done");
    assert!(leftovers(profile.path()).is_empty());

    // The same month again is refused; the month it lacks comes in.
    let again = install(&core, json!({ "month": "2026-10", "minRating": 2400 }))
        .await
        .expect_err("already there");
    assert_eq!(again.message, "Lichess 2400+ already has 2026-10.");
    let older = install(&core, json!({ "minRating": 2400 }))
        .await
        .expect("brings the month it lacks");
    assert_eq!(older["added"], 2);
    assert_eq!(older["database"]["games"], 4);
    let full = install(&core, json!({ "minRating": 2400 }))
        .await
        .expect_err("nothing left");
    assert_eq!(full.message, "Lichess 2400+ has every month.");

    // Another floor is a database of its own.
    let lower = install(&core, json!({ "month": "2026-09", "minRating": 2000 }))
        .await
        .expect("installs");
    assert_eq!(lower["added"], 4);
    assert_eq!(lower["database"]["id"], "lichess-2000");

    let months = core
        .call("gameDbLichessMonths", vec![])
        .await
        .expect("lists");
    assert_eq!(months["months"], json!(["2026-10", "2026-09"]));
    assert_eq!(
        months["held"],
        json!([
            { "minRating": 2000, "months": ["2026-09"] },
            { "minRating": 2400, "months": ["2026-10", "2026-09"] },
        ])
    );

    // Its games are searchable as any database's, the players' ratings kept.
    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");
    let page = core
        .call(
            "gameDbGames",
            vec![json!({ "database": "lichess-2400", "offset": 0, "limit": 20 })],
        )
        .await
        .expect("searches");
    assert_eq!(page["total"], 4);
    core.close().await;
}

#[tokio::test]
async fn a_month_says_its_size_and_a_month_lichess_lacks_is_refused() {
    let profile = tempfile::tempdir().expect("profile");
    let archive = Archive::default();
    archive.publish(&["2026-10"], None);
    let fixture = archive.serve().await;
    let (core, _) = open(profile.path(), &fixture.base);
    let size = core
        .call("gameDbLichessSize", vec![json!("2026-10")])
        .await
        .expect("sized");
    assert_eq!(size, json!(month_file("2026-10").len()));
    let missing = core
        .call("gameDbLichessSize", vec![json!("2026-11")])
        .await
        .expect_err("no such month");
    assert_eq!(missing.message, "Lichess has no games for 2026-11.");
    let invalid = core
        .call("gameDbLichessSize", vec![json!("2026-13")])
        .await
        .expect_err("not a month");
    assert_eq!(invalid.message, "Invalid month.");
    let floor = install(&core, json!({ "minRating": 2450 }))
        .await
        .expect_err("not a floor");
    assert_eq!(floor.message, "Invalid rating.");
    let unlisted = install(&core, json!({ "month": "2026-11", "minRating": 2400 }))
        .await
        .expect_err("not listed");
    assert_eq!(unlisted.message, "Lichess has no games for 2026-11 yet.");
    assert_eq!(month_requests(&fixture, "2026-10"), 0);
    core.close().await;
}

#[tokio::test]
async fn a_month_that_fails_its_checksum_adds_nothing() {
    let profile = tempfile::tempdir().expect("profile");
    let archive = Archive::default();
    archive.publish(&["2026-09"], None);
    let fixture = archive.serve().await;
    let (core, host) = open(profile.path(), &fixture.base);
    install(&core, json!({ "minRating": 2400 }))
        .await
        .expect("installs");
    archive.publish(&["2026-09", "2026-10"], Some("2026-10"));
    let failed = install(&core, json!({ "minRating": 2400 }))
        .await
        .expect_err("the digest does not match");
    assert_eq!(
        failed.message,
        "The Lichess games of 2026-10 did not arrive intact. Try again."
    );
    let last: Value = host.named("gamedb:progress").pop().expect("reported");
    assert_eq!(last["phase"], "failed");
    assert_eq!(last["message"], failed.message);
    let status = core.call("gameDbStatus", vec![]).await.expect("status");
    assert_eq!(status["databases"][0]["games"], 2);
    assert!(leftovers(profile.path()).is_empty());
    // A first month that fails leaves no database at all.
    archive.publish(&["2026-10"], Some("2026-10"));
    install(&core, json!({ "minRating": 2000 }))
        .await
        .expect_err("the digest does not match");
    let status = core.call("gameDbStatus", vec![]).await.expect("status");
    assert_eq!(status["databases"].as_array().map(Vec::len), Some(1));
    assert!(leftovers(profile.path()).is_empty());
    core.close().await;
}

#[tokio::test]
async fn a_cancelled_download_leaves_no_database() {
    let profile = tempfile::tempdir().expect("profile");
    let archive = Archive::default();
    archive.publish(&["2026-10"], None);
    let _body = archive.stream("2026-10");
    let fixture = archive.serve().await;
    let (core, host) = open(profile.path(), &fixture.base);
    let installing = {
        let core = Arc::clone(&core);
        tokio::spawn(async move { install(&core, json!({ "minRating": 2400 })).await })
    };
    let started = async {
        while !host
            .named("gamedb:progress")
            .iter()
            .any(|p| p["phase"] == "downloading" && p["message"] == "2026-10")
        {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    };
    tokio::time::timeout(fixtures::REQUEST_WAIT, started)
        .await
        .expect("the month starts to arrive");
    core.call("gameDbCancel", vec![]).await.expect("cancels");
    let answer = tokio::time::timeout(fixtures::REQUEST_WAIT, installing)
        .await
        .expect("the install ends")
        .expect("the install task");
    assert_eq!(
        answer.expect_err("cancelled").message,
        "Download cancelled."
    );
    let last: Value = host.named("gamedb:progress").pop().expect("reported");
    assert_eq!(last["phase"], "cancelled");
    let status = core.call("gameDbStatus", vec![]).await.expect("status");
    assert_eq!(status["databases"], json!([]));
    assert!(leftovers(profile.path()).is_empty());
    core.close().await;
}
