//! Editing a game database through the core: games added from the board, one replaced, some
//! deleted by id or by search, copied into another database and saved as a PGN file. Whatever
//! is removed, the opening tree and the position search answer as a fresh import of the games
//! left would.

use kchess_core::{Config, Core};
use serde_json::{Value, json};

use crate::support;

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const AFTER_E4: &str = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";

fn game(white: &str, black: &str, result: &str, event: &str, moves: &str) -> String {
    format!(
        "[Event \"{event}\"]\n[White \"{white}\"]\n[Black \"{black}\"]\n[Result \"{result}\"]\n\
         [WhiteElo \"2400\"]\n[BlackElo \"2300\"]\n[Date \"2024.05.01\"]\n\n{moves} {result}\n\n"
    )
}

fn four_games() -> String {
    [
        game("Ana", "Ben", "1-0", "Open", "1. e4 e5 2. Nf3 Nc6"),
        game("Carl", "Ana", "0-1", "Open", "1. e4 c5 2. Nf3 d6"),
        game("Ben", "Dora", "1/2-1/2", "Club", "1. d4 d5 2. c4"),
        game("Dora", "Carl", "1-0", "Club", "1. e4 e5 2. Bc4"),
    ]
    .concat()
}

async fn open() -> (tempfile::TempDir, tempfile::TempDir, Core) {
    let (profile, files) = (
        tempfile::tempdir().expect("profile"),
        tempfile::tempdir().expect("files"),
    );
    let core =
        Core::open(Config::new(profile.path().to_path_buf()), support::host()).expect("opens");
    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");
    (profile, files, core)
}

/// Imports `pgn` as a new database named `name`; answers its id.
async fn import(core: &Core, files: &std::path::Path, name: &str, pgn: &str) -> String {
    let path = files.join(format!("{name}.pgn"));
    std::fs::write(&path, pgn).expect("writes");
    let imported = core
        .call(
            "gameDbImport",
            vec![json!(path.to_string_lossy()), Value::Null, json!(name)],
        )
        .await
        .expect("imports");
    imported["database"]["id"].as_str().expect("an id").into()
}

async fn search(core: &Core, query: Value) -> Value {
    core.call("gameDbGames", vec![query])
        .await
        .expect("searches")
}

/// The names of the games a search finds, White first, in id order.
async fn names(core: &Core, id: &str) -> Vec<String> {
    let page = search(
        core,
        json!({ "database": id, "sort": "oldest", "offset": 0, "limit": 50 }),
    )
    .await;
    page["games"]
        .as_array()
        .expect("games")
        .iter()
        .map(|game| {
            format!(
                "{}-{}",
                game["white"].as_str().unwrap_or(""),
                game["black"].as_str().unwrap_or("")
            )
        })
        .collect()
}

/// A position's moves and games, without the ids that differ between two databases.
async fn tree(core: &Core, id: &str, fen: &str) -> Value {
    let mut lookup = core
        .call(
            "positionLookup",
            vec![json!("database"), json!(fen), json!({ "database": id })],
        )
        .await
        .expect("looks up");
    strip(&mut lookup);
    json!({ "total": lookup["total"], "moves": lookup["moves"], "games": lookup["games"] })
}

fn strip(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("id");
            map.remove("database");
            map.values_mut().for_each(strip);
        }
        Value::Array(items) => items.iter_mut().for_each(strip),
        _ => {}
    }
}

#[tokio::test]
async fn a_game_is_added_replaced_and_deleted() {
    let (_profile, files, core) = open().await;
    let id = import(&core, files.path(), "Club", &four_games()).await;
    let added = core
        .call(
            "gameDbAddGames",
            vec![
                json!(game("Eve", "Ana", "1-0", "Board", "1. c4 e5")),
                json!(id),
            ],
        )
        .await
        .expect("adds");
    assert_eq!(added["database"]["games"], 5);
    assert_eq!(added["game"], 5, "the new game's id");
    let pgn = core
        .call("gameDbGame", vec![json!(id), json!(5)])
        .await
        .expect("reads it");
    assert!(pgn.as_str().expect("text").contains("1. c4 e5"), "{pgn}");
    assert_eq!(tree(&core, &id, START).await["total"], 5);
    // Replaced: the edited game takes a new id, and the old one is gone.
    let replaced = core
        .call(
            "gameDbReplaceGame",
            vec![
                json!(id),
                json!(5),
                json!(game("Eve", "Ana", "0-1", "Board", "1. c4 c5")),
            ],
        )
        .await
        .expect("replaces");
    assert_eq!(replaced["database"]["games"], 5);
    let new = replaced["game"].as_u64().expect("its id");
    assert_ne!(new, 5);
    core.call("gameDbGame", vec![json!(id), json!(5)])
        .await
        .expect_err("the old game is gone");
    let after_c4 = "rnbqkbnr/pppppppp/8/8/2P5/8/PP1PPPPP/RNBQKBNR b KQkq - 0 1";
    let c4 = tree(&core, &id, after_c4).await;
    assert_eq!(c4["moves"].as_array().map(Vec::len), Some(1), "{c4}");
    assert_eq!(c4["moves"][0]["san"], "c5", "{c4}");
    let status = core.call("gameDbStatus", vec![]).await.expect("status");
    assert_eq!(status["databases"][0]["games"], 5);
    // Your games is the core's own.
    let board = game("A", "B", "1-0", "E", "1. e4");
    let refused = core
        .call("gameDbAddGames", vec![json!(board), json!("own")])
        .await
        .expect_err("Your games");
    assert!(refused.message.starts_with("Your games"), "{refused:?}");
    // Or into a new database, named.
    let saved = core
        .call(
            "gameDbAddGames",
            vec![json!(board), Value::Null, json!("From the board")],
        )
        .await
        .expect("makes a database");
    assert_eq!(saved["game"], 1);
    assert_eq!(saved["database"]["name"], "From the board");
    assert_eq!(saved["database"]["games"], 1);
    let refused = core
        .call("gameDbAddGames", vec![json!(board)])
        .await
        .expect_err("neither");
    assert_eq!(refused.message, "Choose a database, or name a new one.");
    core.close().await;
}

#[tokio::test]
async fn deleting_games_leaves_the_tree_a_fresh_import_would_make() {
    let (_profile, files, core) = open().await;
    let id = import(&core, files.path(), "Club", &four_games()).await;
    // A position search, kept by the core, sees the games deleted at once.
    let at_e4 = json!({ "database": id, "position": AFTER_E4, "offset": 0, "limit": 20 });
    assert_eq!(search(&core, at_e4.clone()).await["total"], 3);
    let removed = core
        .call("gameDbDeleteGames", vec![json!(id), json!([1, 99])])
        .await
        .expect("deletes");
    assert_eq!(removed, 1, "a game the database never held is not counted");
    assert_eq!(
        names(&core, &id).await,
        ["Carl-Ana", "Ben-Dora", "Dora-Carl"]
    );
    assert_eq!(search(&core, at_e4).await["total"], 2);
    // By search: every game Carl played, whatever page it is on.
    let removed = core
        .call(
            "gameDbDeleteGames",
            vec![
                json!(id),
                Value::Null,
                json!({ "database": id, "player": "carl", "offset": 0, "limit": 1 }),
            ],
        )
        .await
        .expect("deletes");
    assert_eq!(removed, 2);
    assert_eq!(names(&core, &id).await, ["Ben-Dora"]);
    let status = core.call("gameDbStatus", vec![]).await.expect("status");
    assert_eq!(status["databases"][0]["games"], 1, "{status}");
    // The names only those games had go too: no search finds Carl or the Open.
    let carl = search(
        &core,
        json!({ "database": id, "player": "carl", "offset": 0, "limit": 20 }),
    )
    .await;
    assert_eq!(carl["total"], 0);
    let open = search(
        &core,
        json!({ "database": id, "event": "open", "offset": 0, "limit": 20 }),
    )
    .await;
    assert_eq!(open["total"], 0);
    // Every position answers as a database of the game left would.
    let fresh = import(
        &core,
        files.path(),
        "Fresh",
        &game("Ben", "Dora", "1/2-1/2", "Club", "1. d4 d5 2. c4"),
    )
    .await;
    let after_d4 = "rnbqkbnr/pppppppp/8/8/3P4/8/PPP1PPPP/RNBQKBNR b KQkq - 0 1";
    for fen in [START, AFTER_E4, after_d4] {
        assert_eq!(
            tree(&core, &id, fen).await,
            tree(&core, &fresh, fen).await,
            "{fen}"
        );
    }
    let refused = core
        .call("gameDbDeleteGames", vec![json!(id)])
        .await
        .expect_err("neither games nor a search");
    assert_eq!(refused.message, "Choose the games, or a search.");
    let refused = core
        .call(
            "gameDbDeleteGames",
            vec![
                json!(id),
                Value::Null,
                json!({ "database": fresh, "offset": 0, "limit": 20 }),
            ],
        )
        .await
        .expect_err("another database's search");
    assert_eq!(refused.message, "That search is of another database.");
    core.close().await;
}

#[tokio::test]
async fn games_are_copied_and_saved_as_a_file() {
    let (profile, files, core) = open().await;
    let id = import(&core, files.path(), "Club", &four_games()).await;
    let ana = json!({ "database": id, "player": "ana", "offset": 0, "limit": 20 });
    // Into a new database, named.
    let copied = core
        .call(
            "gameDbCopyGames",
            vec![
                json!(id),
                Value::Null,
                ana.clone(),
                Value::Null,
                json!("Ana's games"),
            ],
        )
        .await
        .expect("copies");
    assert_eq!(copied["added"], 2);
    assert_eq!(copied["database"]["name"], "Ana's games");
    let anas = copied["database"]["id"]
        .as_str()
        .expect("an id")
        .to_string();
    assert_eq!(names(&core, &anas).await, ["Ana-Ben", "Carl-Ana"]);
    // Into that one again, by id: the games join those it has.
    let copied = core
        .call(
            "gameDbCopyGames",
            vec![json!(id), json!([3]), Value::Null, json!(anas)],
        )
        .await
        .expect("copies");
    assert_eq!(copied["database"]["games"], 3);
    let pgn = core
        .call("gameDbGame", vec![json!(anas), json!(1)])
        .await
        .expect("reads a copy");
    assert!(
        pgn.as_str().expect("text").contains("[WhiteElo \"2400\"]"),
        "{pgn}"
    );
    let refused = core
        .call(
            "gameDbCopyGames",
            vec![json!(id), json!([1]), Value::Null, json!(id)],
        )
        .await
        .expect_err("into itself");
    assert_eq!(
        refused.message,
        "Choose another database to copy them into."
    );
    // No copy is left behind in the databases' folder.
    let left: Vec<_> = std::fs::read_dir(profile.path().join("databases"))
        .expect("the folder")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.'))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    // As a PGN file, every game a search finds; read back, it is those games.
    let path = files.path().join("ana.pgn");
    let saved = core
        .call(
            "gameDbExport",
            vec![json!(id), json!(path.to_string_lossy()), Value::Null, ana],
        )
        .await
        .expect("exports");
    assert_eq!(saved, 2);
    let back = import(
        &core,
        files.path(),
        "Back",
        &std::fs::read_to_string(&path).expect("the file"),
    )
    .await;
    assert_eq!(names(&core, &back).await, ["Ana-Ben", "Carl-Ana"]);
    // Every game a search by position finds, whatever its page: the three that opened 1. e4,
    // read from the position index rather than the games' columns.
    let path = files.path().join("e4.pgn");
    let e4 = json!({ "database": id, "position": AFTER_E4, "offset": 0, "limit": 1 });
    let saved = core
        .call(
            "gameDbExport",
            vec![json!(id), json!(path.to_string_lossy()), Value::Null, e4],
        )
        .await
        .expect("exports");
    assert_eq!(saved, 3);
    let back = import(
        &core,
        files.path(),
        "E4",
        &std::fs::read_to_string(&path).expect("the file"),
    )
    .await;
    assert_eq!(
        names(&core, &back).await,
        ["Ana-Ben", "Carl-Ana", "Dora-Carl"]
    );
    let refused = core
        .call(
            "gameDbExport",
            vec![
                json!(id),
                json!(files.path().join("gone/ana.pgn").to_string_lossy()),
                json!([1]),
            ],
        )
        .await
        .expect_err("into a folder that is not there");
    assert!(
        refused
            .message
            .starts_with("Could not save the games there"),
        "{}",
        refused.message
    );
    let refused = core
        .call(
            "gameDbCopyGames",
            vec![json!("gone"), json!([1]), Value::Null, Value::Null],
        )
        .await
        .expect_err("from a database that is not there");
    assert_eq!(refused.message, "That database is not on this computer.");
    let inside = profile.path().join("ana.pgn");
    let refused = core
        .call(
            "gameDbExport",
            vec![json!(id), json!(inside.to_string_lossy()), json!([1])],
        )
        .await
        .expect_err("inside the profile");
    assert_eq!(refused.message, "Save them outside KChess's own folder.");
    let refused = core
        .call(
            "gameDbExport",
            vec![json!(id), json!("ana.pgn"), json!([1])],
        )
        .await
        .expect_err("a relative path");
    assert!(!refused.message.is_empty());
    core.close().await;
}
