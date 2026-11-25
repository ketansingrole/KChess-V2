//! The engine games download through the core's methods, against a local stand-in for TCEC's
//! GitHub releases: the latest release's answer, with the zip's digest, and a tiny compact zip
//! holding every game, the standard competitions' games and Chess960's.

use std::io::Write as _;
use std::path::Path;
use std::sync::{Arc, Mutex};

use kchess_core::{Config, Core, TestSwitches};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::fixtures::{self, Fixture, Reply};
use crate::support;

const ASSET: &str = "TCEC-everything-compact.zip";

fn game(round: usize, white: &str, black: &str) -> String {
    format!(
        "[Event \"TCEC Season 29\"]\n[Date \"2026.0{round}.01\"]\n[Round \"{round}\"]\n\
         [White \"{white}\"]\n[Black \"{black}\"]\n[Result \"1/2-1/2\"]\n\n\
         1. e4 {{ book }} e5 2. Nf3 Nc6 1/2-1/2\n\n"
    )
}

/// The compact zip of a release whose competitions played `standard` games: every game in one
/// file, then the competitions' and Chess960's files, as TCEC packs them.
fn compact_zip(standard: usize) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let games: Vec<String> = (1..=standard)
        .map(|round| game(round, "Stockfish", "Leela"))
        .collect();
    let frc = game(9, "Fischer", "Random");
    let everything = format!("{}{frc}", games.concat());
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, text) in [
        ("compact/everything/TCEC-everything.pgn", everything),
        (
            "compact/everything/TCEC-everything-compet-traditional.pgn",
            games.concat(),
        ),
        ("compact/everything/TCEC-everything-compet-frc.pgn", frc),
    ] {
        writer.start_file(name, options).expect("starts a file");
        writer.write_all(text.as_bytes()).expect("writes it");
    }
    writer.finish().expect("finishes").into_inner()
}

/// The release the stand-in serves: its tag, zip, and the digest it gives (the zip's, unless a
/// test says otherwise).
#[derive(Clone)]
struct Published {
    tag: String,
    zip: Vec<u8>,
    digest: Option<String>,
}

#[derive(Clone, Default)]
struct Releases {
    latest: Arc<Mutex<Option<Published>>>,
}

impl Releases {
    fn publish(&self, tag: &str, standard: usize) {
        let zip = compact_zip(standard);
        let digest = format!("sha256:{}", kchess_delta::hex(&Sha256::digest(&zip)));
        *self.latest.lock().unwrap() = Some(Published {
            tag: tag.into(),
            zip,
            digest: Some(digest),
        });
    }

    fn digest(&self, digest: Option<String>) {
        if let Some(latest) = self.latest.lock().unwrap().as_mut() {
            latest.digest = digest;
        }
    }

    async fn serve(&self) -> Fixture {
        let releases = self.clone();
        fixtures::serve(move |request| {
            let latest = releases.latest.lock().unwrap().clone();
            let Some(Published { tag, zip, digest }) = latest else {
                return Reply::Text(404, "text/plain", String::new());
            };
            if request.path() == "/tcec/latest" {
                let answer = json!({
                    "tag_name": tag,
                    "assets": [
                        { "name": "MD5SUM", "size": 10, "digest": null },
                        { "name": ASSET, "size": zip.len(), "digest": digest },
                    ],
                });
                return Reply::Json(200, answer.to_string());
            }
            if request.path() == format!("/tcec/download/{tag}/{ASSET}") {
                return Reply::Bytes(200, vec![("Content-Type", "application/zip".into())], zip);
            }
            Reply::Text(404, "text/plain", String::new())
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

fn zip_requests(fixture: &Fixture, tag: &str) -> usize {
    fixture.to(&format!("/tcec/download/{tag}/{ASSET}")).len()
}

#[tokio::test]
async fn the_engine_games_are_the_standard_competitions_and_a_new_release_replaces_them() {
    let profile = tempfile::tempdir().expect("profile");
    let releases = Releases::default();
    releases.publish("S29-final", 2);
    let fixture = releases.serve().await;
    let (core, host) = open(profile.path(), &fixture.base);
    let installed = core
        .call("gameDbTcecInstall", vec![])
        .await
        .expect("installs");
    // The competitions' file alone: neither every game again nor Chess960's.
    assert_eq!(installed["added"], 2);
    let database = &installed["database"];
    assert_eq!(database["id"], "tcec");
    assert_eq!(database["name"], "TCEC engine games");
    assert_eq!(database["source"], "tcec");
    assert_eq!(database["games"], 2);
    assert_eq!(
        database["attribution"],
        "TCEC games, tcec-chess.com, CC BY-SA 3.0"
    );
    let last: Value = host.named("gamedb:progress").pop().expect("reported");
    assert_eq!(last["phase"], "done");
    assert!(leftovers(profile.path()).is_empty());

    // The same release again: nothing downloaded.
    let again = core
        .call("gameDbTcecInstall", vec![])
        .await
        .expect("up to date");
    assert_eq!(again["added"], 0);
    assert_eq!(again["database"]["games"], 2);
    assert_eq!(zip_requests(&fixture, "S29-final"), 1);

    // A new release holds every game so far: it replaces the old ones.
    releases.publish("S30-interim-1", 3);
    let newer = core
        .call("gameDbTcecInstall", vec![])
        .await
        .expect("updates");
    assert_eq!(newer["added"], 3);
    assert_eq!(newer["database"]["games"], 3);
    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");
    let page = core
        .call(
            "gameDbGames",
            vec![json!({ "database": "tcec", "player": "stockfish", "offset": 0, "limit": 20 })],
        )
        .await
        .expect("searches");
    assert_eq!(page["total"], 3);
    core.close().await;
}

#[tokio::test]
async fn a_zip_that_fails_its_checksum_or_has_none_adds_nothing() {
    let profile = tempfile::tempdir().expect("profile");
    let releases = Releases::default();
    releases.publish("S29-final", 2);
    releases.digest(Some(format!("sha256:{}", "0".repeat(64))));
    let fixture = releases.serve().await;
    let (core, host) = open(profile.path(), &fixture.base);
    let failed = core
        .call("gameDbTcecInstall", vec![])
        .await
        .expect_err("the digest does not match");
    assert_eq!(
        failed.message,
        "The engine games did not arrive intact. Try again."
    );
    let last: Value = host.named("gamedb:progress").pop().expect("reported");
    assert_eq!(last["phase"], "failed");
    releases.digest(None);
    let unready = core
        .call("gameDbTcecInstall", vec![])
        .await
        .expect_err("no digest to check");
    assert!(unready.message.contains("not ready"), "{}", unready.message);
    assert_eq!(zip_requests(&fixture, "S29-final"), 1);
    let status = core.call("gameDbStatus", vec![]).await.expect("status");
    assert_eq!(status["databases"], json!([]));
    assert!(leftovers(profile.path()).is_empty());
    core.close().await;
}
