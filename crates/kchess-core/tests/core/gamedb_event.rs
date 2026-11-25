//! An event's table from a game database through the core: the edition of the chosen game's event
//! name, its standings by points and Sonneborn–Berger, each player's results by round and their
//! performance; withheld during a live game.

use kchess_core::{Config, Core};
use serde_json::{Value, json};

use crate::support;

fn game(
    event: Option<&str>,
    white: &str,
    black: &str,
    date: &str,
    round: &str,
    result: &str,
) -> String {
    let elo = |name: &str| match name {
        "A" => 2500,
        "B" => 2400,
        "C" => 2300,
        "D" => 2200,
        _ => 2000,
    };
    let event = event
        .map(|event| format!("[Event \"{event}\"]\n[Site \"Hastings ENG\"]\n"))
        .unwrap_or_default();
    format!(
        "{event}[White \"{white}\"]\n[Black \"{black}\"]\n[WhiteElo \"{}\"]\n[BlackElo \"{}\"]\n\
         [Date \"{date}\"]\n[Round \"{round}\"]\n[Result \"{result}\"]\n\n1. e4 e5 {result}\n\n",
        elo(white),
        elo(black)
    )
}

/// The id of the game between `white` and `black` on `date`.
fn id_of(page: &Value, white: &str, black: &str, date: &str) -> u64 {
    page["games"]
        .as_array()
        .expect("games")
        .iter()
        .find(|row| row["white"] == white && row["black"] == black && row["date"] == date)
        .and_then(|row| row["id"].as_u64())
        .expect("the game")
}

#[tokio::test]
async fn an_event_s_edition_reads_as_a_table() {
    let (profile, files) = (
        tempfile::tempdir().expect("profile"),
        tempfile::tempdir().expect("files"),
    );
    let core =
        Core::open(Config::new(profile.path().to_path_buf()), support::host()).expect("opens");
    let hastings = Some("Hastings");
    let pgn = [
        // The 2023/24 edition, across the new year, its last round still being played.
        game(hastings, "A", "B", "2023.12.28", "1", "1-0"),
        game(hastings, "C", "D", "2023.12.28", "1", "1/2-1/2"),
        game(hastings, "A", "C", "2023.12.29", "2", "1/2-1/2"),
        game(hastings, "B", "D", "2023.12.29", "2", "1-0"),
        game(hastings, "D", "A", "2024.01.03", "3", "0-1"),
        game(hastings, "B", "C", "2024.01.03", "3", "*"),
        // The next edition, under the same name.
        game(hastings, "E", "F", "2024.12.29", "1", "1-0"),
        game(None, "A", "F", "2024.02.01", "?", "1-0"),
    ]
    .concat();
    let path = files.path().join("hastings.pgn");
    std::fs::write(&path, pgn).expect("writes");
    let imported = core
        .call(
            "gameDbImport",
            vec![
                json!(path.to_string_lossy()),
                Value::Null,
                json!("Hastings"),
            ],
        )
        .await
        .expect("imports");
    let id = imported["database"]["id"]
        .as_str()
        .expect("an id")
        .to_string();
    let refused = core
        .call("gameDbEvent", vec![json!(id), json!(1)])
        .await
        .expect_err("withheld until no game is live");
    assert_eq!(refused.kind, Some(kchess_contracts::ErrorKind::Withheld));
    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");

    let page = core
        .call(
            "gameDbGames",
            vec![json!({ "database": id, "sort": "oldest", "offset": 0, "limit": 50 })],
        )
        .await
        .expect("searches");
    let chosen = id_of(&page, "B", "D", "2023.12.29");

    let table = core
        .call("gameDbEvent", vec![json!(id), json!(chosen)])
        .await
        .expect("tables");
    assert_eq!(table["name"], "Hastings");
    assert_eq!(table["site"], "Hastings ENG");
    assert_eq!(
        (&table["firstDate"], &table["lastDate"]),
        (&json!("2023.12.28"), &json!("2024.01.03"))
    );
    assert_eq!(table["games"], 6);
    let players = table["players"].as_array().expect("players");
    let standings: Vec<(&str, u64, u64)> = players
        .iter()
        .map(|player| {
            (
                player["name"].as_str().expect("a name"),
                player["halfPoints"].as_u64().expect("points"),
                player["tiebreakQuarters"].as_u64().expect("a tiebreak"),
            )
        })
        .collect();
    // B and C have a point each; C drew A and D, B beat only D, so C is ahead.
    assert_eq!(
        standings,
        [("A", 5, 8), ("C", 2, 6), ("B", 2, 2), ("D", 1, 2)]
    );
    let first = &players[0];
    assert_eq!(
        (&first["wins"], &first["draws"], &first["losses"]),
        (&json!(2), &json!(1), &json!(0))
    );
    assert_eq!(first["rating"], 2500);
    // 2½ of 3 (83%) against an average of 2300.
    assert_eq!(first["performance"], 2573);
    assert_eq!(
        first["results"],
        json!([
            { "game": id_of(&page, "A", "B", "2023.12.28"), "opponent": 2, "white": true, "halfPoints": 2, "round": "1" },
            { "game": id_of(&page, "A", "C", "2023.12.29"), "opponent": 1, "white": true, "halfPoints": 1, "round": "2" },
            { "game": id_of(&page, "D", "A", "2024.01.03"), "opponent": 3, "white": false, "halfPoints": 2, "round": "3" },
        ])
    );
    // The unfinished game is in the count but in no one's results.
    assert_eq!(players[2]["results"].as_array().expect("results").len(), 2);

    let next = core
        .call(
            "gameDbEvent",
            vec![json!(id), json!(id_of(&page, "E", "F", "2024.12.29"))],
        )
        .await
        .expect("tables");
    let names: Vec<&str> = next["players"]
        .as_array()
        .expect("players")
        .iter()
        .map(|player| player["name"].as_str().expect("a name"))
        .collect();
    assert_eq!(names, ["E", "F"]);

    let unnamed = core
        .call(
            "gameDbEvent",
            vec![json!(id), json!(id_of(&page, "A", "F", "2024.02.01"))],
        )
        .await
        .expect_err("no event");
    assert_eq!(unnamed.message, "That game names no event.");
    let missing = core
        .call("gameDbEvent", vec![json!(id), json!(9_999)])
        .await
        .expect_err("no such game");
    assert_eq!(missing.message, "That game is not in this database.");
}
