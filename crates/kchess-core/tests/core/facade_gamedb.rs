//! The game database through the core's methods: checked arguments, an import, a search and a
//! game, and closing the core while an import runs.

use std::sync::Arc;
use std::time::Duration;

use kchess_core::{Config, Core};
use serde_json::{Value, json};

use crate::support;

const GAMES: &str = "[Event \"Open\"]\n[White \"Ana\"]\n[Black \"Ben\"]\n[Result \"1-0\"]\n\n\
    1. e4 e5 2. Nf3 1-0\n\n[Event \"Open\"]\n[White \"Ben\"]\n[Black \"Ana\"]\n[Result \"0-1\"]\n\n\
    1. d4 d5 0-1\n";

fn pgn_file(dir: &std::path::Path, text: &str) -> String {
    let path = dir.join("open.pgn");
    std::fs::write(&path, text).expect("writes");
    path.to_string_lossy().into_owned()
}

/// A `.zip` or a `.7z` goes through the same method: the core reads the PGN files in it.
#[tokio::test]
async fn an_archive_is_imported_through_the_core() {
    use std::io::Write as _;
    let (profile, files) = (
        tempfile::tempdir().expect("profile"),
        tempfile::tempdir().expect("files"),
    );
    let host = support::host();
    let core = Core::open(Config::new(profile.path().to_path_buf()), host.clone()).expect("opens");
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file("round1.pgn", zip::write::SimpleFileOptions::default())
        .expect("starts");
    writer.write_all(GAMES.as_bytes()).expect("writes");
    let path = files.path().join("twic1620g.zip");
    std::fs::write(&path, writer.finish().expect("finishes").into_inner()).expect("saves");
    let imported = core
        .call("gameDbImport", vec![json!(path.to_string_lossy())])
        .await
        .expect("imports");
    assert_eq!(imported["added"], 2);
    assert_eq!(imported["database"]["name"], "twic1620g");
    // And a `.7z`, as Lumbra's Gigabase and CCRL publish their games: the core's own fixture,
    // the unit tests' games forty times (`src/gamedb/fixtures/README.md`).
    let path = files.path().join("CCRL-4040.[120].pgn.7z");
    std::fs::write(&path, include_bytes!("../../src/gamedb/fixtures/many.7z")).expect("saves");
    let imported = core
        .call("gameDbImport", vec![json!(path.to_string_lossy())])
        .await
        .expect("imports");
    assert_eq!(imported["added"], 120);
    assert_eq!(imported["database"]["name"], "CCRL-4040.[120]");
    core.close().await;
}

#[tokio::test]
async fn a_file_is_imported_searched_and_read_through_the_core() {
    let (profile, files) = (
        tempfile::tempdir().expect("profile"),
        tempfile::tempdir().expect("files"),
    );
    let host = support::host();
    let core = Core::open(Config::new(profile.path().to_path_buf()), host.clone()).expect("opens");
    let refused = core
        .call("gameDbImport", vec![json!("relative/open.pgn")])
        .await
        .expect_err("a relative path is refused");
    assert_eq!(
        refused.message,
        "Choose a .pgn, .pgn.zst, .pgn.bz2, .zip or .7z file."
    );
    let imported = core
        .call(
            "gameDbImport",
            vec![
                json!(pgn_file(files.path(), GAMES)),
                Value::Null,
                json!(" Club open "),
            ],
        )
        .await
        .expect("imports");
    assert_eq!(imported["added"], 2);
    assert_eq!(imported["database"]["name"], "Club open");
    let id = imported["database"]["id"]
        .as_str()
        .expect("an id")
        .to_string();
    // Games are help a live game withholds: until the core knows no game is live, a search is
    // refused as an explorer lookup is.
    let withheld = core
        .call("gameDbGame", vec![json!(id), json!(1)])
        .await
        .expect_err("withheld until the game status is known");
    assert_eq!(withheld.kind, Some(kchess_contracts::ErrorKind::Withheld));
    let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    let in_database = json!({ "database": id, "player": "ana", "color": "black" });
    let lookup = core
        .call(
            "positionLookup",
            vec![json!("database"), json!(start), in_database.clone()],
        )
        .await
        .expect_err("the explorer is withheld too");
    assert_eq!(lookup.kind, Some(kchess_contracts::ErrorKind::Withheld));
    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");
    // The explorer's Database kind: Ana's games with Black, from this computer, never saved.
    let lookup = core
        .call(
            "positionLookup",
            vec![json!("database"), json!(start), in_database],
        )
        .await
        .expect("looks up");
    assert_eq!(lookup["kind"], "database");
    assert_eq!(lookup["cached"], false);
    assert_eq!(lookup["total"], 1);
    assert_eq!(lookup["moves"][0]["san"], "d4");
    assert_eq!(lookup["moves"][0]["black"], 1);
    assert_eq!(lookup["games"][0]["black"], "Ana");
    let refused = core
        .call("positionLookup", vec![json!("database"), json!(start)])
        .await
        .expect_err("no database named");
    assert_eq!(refused.message, "Choose a game database.");
    let page = core
        .call(
            "gameDbGames",
            vec![json!({ "database": id, "player": "ana", "side": "white", "offset": 0, "limit": 20 })],
        )
        .await
        .expect("searches");
    assert_eq!(page["total"], 1);
    let game = page["games"][0]["id"].clone();
    let pgn = core
        .call("gameDbGame", vec![json!(id), game])
        .await
        .expect("reads the game");
    assert!(
        pgn.as_str().expect("text").contains("1. e4 e5 2. Nf3 1-0"),
        "{pgn}"
    );
    let status = core.call("gameDbStatus", vec![]).await.expect("status");
    assert_eq!(status["databases"][0]["games"], 2);
    let changed: Vec<Value> = host.named("profile:changed");
    assert!(
        changed
            .iter()
            .any(|change| change.to_string().contains("gameDb")),
        "{changed:?}"
    );
    let deleted = core
        .call("gameDbDelete", vec![json!(id)])
        .await
        .expect("deletes");
    assert_eq!(deleted["databases"], json!([]));
    core.close().await;
}

#[tokio::test]
async fn closing_the_core_ends_an_import_and_leaves_no_database() {
    let (profile, files) = (
        tempfile::tempdir().expect("profile"),
        tempfile::tempdir().expect("files"),
    );
    let host = support::host();
    let core = Arc::new(
        Core::open(Config::new(profile.path().to_path_buf()), host.clone()).expect("opens"),
    );
    let path = pgn_file(files.path(), &GAMES.repeat(20_000));
    let import = {
        let core = Arc::clone(&core);
        tokio::spawn(async move { core.call("gameDbImport", vec![json!(path)]).await })
    };
    while host.named("gamedb:progress").is_empty() {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    tokio::time::timeout(Duration::from_secs(60), core.close())
        .await
        .expect("closing returns while the import runs");
    let answer = import.await.expect("the import task");
    assert!(answer.is_err(), "{answer:?}");
    let left: Vec<String> = std::fs::read_dir(profile.path().join("databases"))
        .map(|entries| {
            entries
                .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
}

/// The player's synced and played games become "Your games", which every list shows and the
/// explorer looks positions up in, and which is never deleted while it has games.
#[tokio::test]
async fn your_games_are_a_database_the_core_keeps() {
    use kchess_core::store::games;
    use support::store::{library_game, list};

    let profile = tempfile::tempdir().expect("profile");
    let host = support::host();
    let core = Core::open(Config::new(profile.path().to_path_buf()), host.clone()).expect("opens");
    // The first call starts the core's background work, "Your games" with it.
    let status = core.call("gameDbStatus", vec![]).await.expect("status");
    assert_eq!(status["databases"], json!([]));
    core.call("addAccount", vec![json!("Alice")])
        .await
        .expect("adds the account");
    let pgn = "[White \"Alice\"]\n[Black \"Bob\"]\n[Result \"1-0\"]\n\n1. e4 e5 2. Nf3 1-0\n";
    let synced = json!([
        library_game("g1", &[("pgn", json!(pgn))]),
        library_game("g2", &[("pgn", json!(pgn.replace("e4 e5", "d4 d5")))]),
    ]);
    core.database_for_tests()
        .write_async(move |c| games::save_games(c.host, c.db, "Alice", list(&synced), Some(1)))
        .await
        .expect("synced");
    let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    core.call(
        "saveArchivedGame",
        vec![json!({
            "id": "board1", "source": "board", "startedAt": 1, "updatedAt": 2, "white": "Ana",
            "black": "Ben", "result": "1/2-1/2", "reason": "agreement", "timeControl": "-",
            "finished": true, "setup": { "variant": "standard", "fen": start },
            "moves": ["e2e4", "c7c5"],
        })],
    )
    .await
    .expect("saves the played game");
    let report = core.own_games_settled_for_tests().await;
    assert_eq!(report.rebuilds, 1);
    let status = core.call("gameDbStatus", vec![]).await.expect("status");
    let own = &status["databases"][0];
    assert_eq!(
        (&own["id"], &own["name"], &own["source"], &own["games"]),
        (
            &json!("own"),
            &json!("Your games"),
            &json!("own"),
            &json!(3)
        )
    );
    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");
    let lookup = core
        .call(
            "positionLookup",
            vec![
                json!("database"),
                json!(start),
                json!({ "database": "own" }),
            ],
        )
        .await
        .expect("looks up");
    assert_eq!(lookup["total"], 3);
    let page = core
        .call(
            "gameDbGames",
            vec![json!({ "database": "own", "player": "alice", "offset": 0, "limit": 20 })],
        )
        .await
        .expect("searches");
    assert_eq!(page["total"], 2);
    let refused = core
        .call("gameDbDelete", vec![json!("own")])
        .await
        .expect_err("Your games stays");
    assert!(refused.message.starts_with("Your games"), "{refused:?}");
    let changed: Vec<Value> = host.named("profile:changed");
    assert!(
        changed
            .iter()
            .any(|change| change["areas"] == json!(["gameDb"])),
        "{changed:?}"
    );
    core.close().await;
}
