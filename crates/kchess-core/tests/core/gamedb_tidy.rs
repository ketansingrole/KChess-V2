//! Tidying a database through the core: its copies of a game removed, the first kept, and one
//! player's spellings merged into one name, which searches and the games' PGN then use.

use kchess_core::{Config, Core};
use serde_json::{Value, json};

use crate::support;

fn game(white: &str, black: &str, date: &str, moves: &str) -> String {
    format!(
        "[White \"{white}\"]\n[Black \"{black}\"]\n[Date \"{date}\"]\n[Result \"1-0\"]\n\n\
         {moves} 1-0\n\n"
    )
}

async fn players(core: &Core, database: &str, player: &str) -> Vec<(String, String)> {
    let query = json!({ "database": database, "player": player, "offset": 0, "limit": 50 });
    let page = core
        .call("gameDbGames", vec![query])
        .await
        .expect("searches");
    page["games"]
        .as_array()
        .expect("games")
        .iter()
        .map(|game| {
            let name = |side: &str| game[side].as_str().expect("a name").to_string();
            (name("white"), name("black"))
        })
        .collect()
}

#[tokio::test]
async fn a_database_drops_its_copies_and_merges_a_player_s_spellings() {
    let (profile, files) = (
        tempfile::tempdir().expect("profile"),
        tempfile::tempdir().expect("files"),
    );
    let core =
        Core::open(Config::new(profile.path().to_path_buf()), support::host()).expect("opens");
    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");
    let ruy = "1. e4 e5 2. Nf3 Nc6 3. Bb5 a6";
    let pgn = [
        game("Carlsen, Magnus", "Caruana, Fabiano", "2024.01.02", ruy),
        // The same game from another source, twice.
        game("Carlsen, Magnus", "Caruana, Fabiano", "2024.01.02", ruy),
        game("Carlsen, Magnus", "Caruana, Fabiano", "2024.01.02", ruy),
        // Not copies: another day, other moves, another opponent.
        game("Carlsen, Magnus", "Caruana, Fabiano", "2024.01.03", ruy),
        game(
            "Carlsen, Magnus",
            "Caruana, Fabiano",
            "2024.01.02",
            "1. d4 d5",
        ),
        game("Carlsen, Magnus", "Ding, Liren", "2024.01.02", ruy),
        // Other spellings, one game with a variation, which keeps its whole text.
        game(
            "Carlsen, M",
            "Ding, Liren",
            "2023.05.05",
            "1. c4 (1. d4 d5) e5",
        ),
        game("Magnus Carlsen", "Ding, Liren", "2023.05.06", "1. Nf3 d5"),
    ]
    .concat();
    let path = files.path().join("tidy.pgn");
    std::fs::write(&path, pgn).expect("writes");
    let imported = core
        .call(
            "gameDbImport",
            vec![json!(path.to_string_lossy()), Value::Null, json!("Tidy")],
        )
        .await
        .expect("imports");
    let id = imported["database"]["id"]
        .as_str()
        .expect("an id")
        .to_string();

    let duplicates = core
        .call("gameDbDuplicates", vec![json!(id)])
        .await
        .expect("counts");
    assert_eq!(duplicates, json!(2));
    let removed = core
        .call("gameDbRemoveDuplicates", vec![json!(id)])
        .await
        .expect("removes");
    assert_eq!(removed, json!(2));
    let duplicates = core
        .call("gameDbDuplicates", vec![json!(id)])
        .await
        .expect("counts again");
    assert_eq!(duplicates, json!(0));
    let status = core.call("gameDbStatus", vec![]).await.expect("status");
    assert_eq!(status["databases"][0]["games"], json!(6));
    // The tree counts the Ruy Lopez's games once each: three were left of five.
    let lookup = core
        .call(
            "gameDbNovelty",
            vec![
                json!("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"),
                json!(["e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "g8f6"]),
            ],
        )
        .await
        .expect("finds it");
    assert_eq!(lookup["games"], json!(3));

    let groups = core
        .call("gameDbNameGroups", vec![json!(id)])
        .await
        .expect("groups");
    assert_eq!(
        groups,
        json!([{ "names": [
            { "name": "Carlsen, Magnus", "games": 4 },
            { "name": "Carlsen, M", "games": 1 },
            { "name": "Magnus Carlsen", "games": 1 },
        ] }])
    );
    let refused = core
        .call(
            "gameDbMergeNames",
            vec![json!(id), json!("Carlsen, Mads"), json!(["Carlsen, M"])],
        )
        .await
        .expect_err("a name the database lacks");
    assert_eq!(refused.message, "That name is not in this database.");
    let changed = core
        .call(
            "gameDbMergeNames",
            vec![
                json!(id),
                json!("Carlsen, Magnus"),
                json!(["Carlsen, M", "Magnus Carlsen"]),
            ],
        )
        .await
        .expect("merges");
    assert_eq!(changed, json!(2));
    let found = players(&core, &id, "carlsen").await;
    assert_eq!(found.len(), 6);
    assert!(
        found.iter().all(|(white, _)| white == "Carlsen, Magnus"),
        "{found:?}"
    );
    assert!(players(&core, &id, "carlsen m").await.len() == 6);
    let groups = core
        .call("gameDbNameGroups", vec![json!(id)])
        .await
        .expect("groups");
    assert_eq!(groups, json!([]));
    // Every game opens with the merged name, the one kept as its text too.
    let page = core
        .call(
            "gameDbGames",
            vec![json!({ "database": id, "player": "ding", "offset": 0, "limit": 5 })],
        )
        .await
        .expect("searches");
    for game in page["games"].as_array().expect("games") {
        let pgn = core
            .call("gameDbGame", vec![json!(id), game["id"].clone()])
            .await
            .expect("opens");
        let pgn = pgn.as_str().expect("its PGN");
        assert!(pgn.contains("[White \"Carlsen, Magnus\"]"), "{pgn}");
    }
}
