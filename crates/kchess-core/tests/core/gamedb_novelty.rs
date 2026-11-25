//! A line's novelty through the core: the first move no game of the game databases played,
//! known as far as any database knows it (transpositions too), and never the line's own game.

use kchess_core::{Config, Core};
use serde_json::{Value, json};

use crate::support;

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

fn game(white: &str, black: &str, moves: &str) -> String {
    format!("[White \"{white}\"]\n[Black \"{black}\"]\n[Result \"1-0\"]\n\n{moves} 1-0\n\n")
}

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

async fn novelty(core: &Core, moves: &str, rest: Vec<Value>) -> Value {
    let moves: Vec<&str> = moves.split(' ').collect();
    let mut args = vec![json!(START), json!(moves)];
    args.extend(rest);
    core.call("gameDbNovelty", args).await.expect("finds it")
}

#[tokio::test]
async fn a_line_leaves_the_databases_at_its_first_unplayed_move() {
    let (profile, files) = (
        tempfile::tempdir().expect("profile"),
        tempfile::tempdir().expect("files"),
    );
    let core =
        Core::open(Config::new(profile.path().to_path_buf()), support::host()).expect("opens");
    let ruy = "e2e4 e7e5 g1f3 b8c6 f1b5";
    let refused = core
        .call(
            "gameDbNovelty",
            vec![json!(START), json!(ruy.split(' ').collect::<Vec<_>>())],
        )
        .await
        .expect_err("help is withheld until no game is live");
    assert_eq!(refused.kind, Some(kchess_contracts::ErrorKind::Withheld));
    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");
    // No database: nothing is new.
    assert_eq!(
        novelty(&core, ruy, vec![]).await,
        json!({ "games": 0, "played": [] })
    );
    let club = import(
        &core,
        files.path(),
        "Club",
        &[
            game("Ana", "Ben", "1. e4 e5 2. Nf3 Nc6 3. Bb5 a6"),
            game("Carl", "Dora", "1. e4 e5 2. Nf3 Nc6 3. Bc4 Bc5"),
            game("Eve", "Finn", "1. e4 e5 2. Nf3 Nf6"),
        ]
        .concat(),
    )
    .await;

    // 3...Nf6 was never played: one game reached the Ruy Lopez, and played 3...a6.
    let found = novelty(&core, "e2e4 e7e5 g1f3 b8c6 f1b5 g8f6", vec![]).await;
    assert_eq!(
        found,
        json!({ "ply": 5, "games": 1, "played": [{ "san": "a6", "uci": "a7a6", "games": 1 }] })
    );
    // 3. d4 neither: two games went on, with 3. Bb5 and 3. Bc4.
    let found = novelty(&core, "e2e4 e7e5 g1f3 b8c6 d2d4", vec![]).await;
    assert_eq!(found["ply"], 4);
    assert_eq!(found["games"], 2);
    let sans: Vec<&str> = found["played"]
        .as_array()
        .expect("moves")
        .iter()
        .filter_map(|m| m["san"].as_str())
        .collect();
    assert_eq!(sans.len(), 2);
    assert!(sans.contains(&"Bb5") && sans.contains(&"Bc4"), "{sans:?}");

    // Another database reached the same position by other moves and played 3. d4 there: the
    // line is known in full.
    import(
        &core,
        files.path(),
        "Other",
        &game("Gus", "Hal", "1. Nf3 Nc6 2. e4 e5 3. d4"),
    )
    .await;
    let found = novelty(&core, "e2e4 e7e5 g1f3 b8c6 d2d4", vec![]).await;
    assert_eq!(found, json!({ "games": 0, "played": [] }));

    // The line's own game does not count against it: Ana against Ben's 3...a6 is new but for
    // that game itself, by its players or as the game it was opened from.
    let own = "e2e4 e7e5 g1f3 b8c6 f1b5 a7a6";
    assert_eq!(novelty(&core, own, vec![]).await["ply"], Value::Null);
    let by_players = novelty(&core, own, vec![json!("Ana"), json!("Ben")]).await;
    // The other database's game went on there too, by transposition.
    let expected = json!({
        "ply": 4, "games": 2, "played": [
            { "san": "Bc4", "uci": "f1c4", "games": 1 },
            { "san": "d4", "uci": "d2d4", "games": 1 },
        ]
    });
    assert_eq!(by_players, expected);
    let page = core
        .call(
            "gameDbGames",
            vec![json!({ "database": club, "player": "ana", "offset": 0, "limit": 5 })],
        )
        .await
        .expect("searches");
    let id = page["games"][0]["id"].clone();
    let opened = novelty(
        &core,
        own,
        vec![Value::Null, Value::Null, json!(club), id.clone()],
    )
    .await;
    assert_eq!(opened, expected);
    // Changed on the board after 3. Bb5, it is still its own game up to there.
    let changed = novelty(
        &core,
        "e2e4 e7e5 g1f3 b8c6 f1b5 g8f6",
        vec![Value::Null, Value::Null, json!(club), id],
    )
    .await;
    assert_eq!(changed["ply"], 4, "{changed}");
    core.close().await;
}
