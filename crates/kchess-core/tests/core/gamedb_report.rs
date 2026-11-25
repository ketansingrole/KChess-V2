//! A player's report from a game database through the core: their results and openings with
//! each colour, their rating by month and their best wins; withheld during a live game.

use kchess_core::{Config, Core};
use serde_json::{Value, json};

use crate::support;

#[allow(clippy::too_many_arguments)]
fn game(
    white: (&str, u32),
    black: (&str, u32),
    date: &str,
    opening: Option<&str>,
    result: &str,
    moves: &str,
) -> String {
    let opening = opening
        .map(|name| format!("[Opening \"{name}\"]\n"))
        .unwrap_or_default();
    format!(
        "[White \"{}\"]\n[Black \"{}\"]\n[WhiteElo \"{}\"]\n[BlackElo \"{}\"]\n[Date \"{date}\"]\n\
         {opening}[Result \"{result}\"]\n\n{moves} {result}\n\n",
        white.0, black.0, white.1, black.1
    )
}

#[tokio::test]
async fn a_player_s_games_read_as_a_report() {
    let (profile, files) = (
        tempfile::tempdir().expect("profile"),
        tempfile::tempdir().expect("files"),
    );
    let core =
        Core::open(Config::new(profile.path().to_path_buf()), support::host()).expect("opens");
    let me = "Carlsen, Magnus";
    let pgn = [
        game(
            (me, 2830),
            ("Ding, Liren", 2780),
            "2023.04.10",
            Some("Ruy Lopez: Berlin Defense"),
            "1-0",
            "1. e4 e5 2. Nf3 Nc6 3. Bb5 Nf6",
        ),
        game(
            (me, 2835),
            ("Caruana, Fabiano", 2800),
            "2023.04.20",
            Some("Ruy Lopez: Closed"),
            "1/2-1/2",
            "1. e4 e5 2. Nf3 Nc6 3. Bb5 a6",
        ),
        game(
            (me, 2840),
            ("Club, Player", 2100),
            "2023.06.01",
            None,
            "1-0",
            "1. d4 d5",
        ),
        game(
            ("Gukesh D", 2750),
            (me, 2830),
            "2024.01.21",
            Some("Sicilian Defense: Najdorf"),
            "1-0",
            "1. e4 c5",
        ),
        game(
            ("Nakamura, Hikaru", 2790),
            (me, 2825),
            "2024.02.03",
            None,
            "0-1",
            "1. c4 e5",
        ),
        game(
            ("Smith", 2000),
            ("Jones", 2000),
            "2024.02.03",
            None,
            "0-1",
            "1. c4 e5",
        ),
    ]
    .concat();
    let path = files.path().join("carlsen.pgn");
    std::fs::write(&path, pgn).expect("writes");
    let imported = core
        .call(
            "gameDbImport",
            vec![json!(path.to_string_lossy()), Value::Null, json!("Elite")],
        )
        .await
        .expect("imports");
    let id = imported["database"]["id"]
        .as_str()
        .expect("an id")
        .to_string();
    let refused = core
        .call("gameDbPlayer", vec![json!(id), json!(me)])
        .await
        .expect_err("withheld until no game is live");
    assert_eq!(refused.kind, Some(kchess_contracts::ErrorKind::Withheld));
    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");

    let report = core
        .call("gameDbPlayer", vec![json!(id), json!(me)])
        .await
        .expect("reports");
    let record = |games, wins, draws, losses| json!({ "games": games, "wins": wins, "draws": draws, "losses": losses });
    assert_eq!(report["name"], json!(me));
    assert_eq!(
        (&report["firstYear"], &report["lastYear"]),
        (&json!(2023), &json!(2024))
    );
    assert_eq!(report["white"]["record"], record(3, 2, 1, 0));
    assert_eq!(
        report["white"]["openings"],
        json!([
            { "name": "Ruy Lopez", "record": record(2, 1, 1, 0) },
            { "name": "1. d4 d5", "record": record(1, 1, 0, 0) },
        ])
    );
    assert_eq!(report["black"]["record"], record(2, 1, 0, 1));
    assert_eq!(
        report["black"]["openings"],
        json!([
            { "name": "1. c4 e5", "record": record(1, 1, 0, 0) },
            { "name": "Sicilian Defense", "record": record(1, 0, 0, 1) },
        ])
    );
    // April's last game's rating stands for April.
    assert_eq!(
        report["ratings"],
        json!([
            { "year": 2023, "month": 4, "rating": 2835 },
            { "year": 2023, "month": 6, "rating": 2840 },
            { "year": 2024, "month": 1, "rating": 2830 },
            { "year": 2024, "month": 2, "rating": 2825 },
        ])
    );
    let best: Vec<(&str, &str)> = report["best"]
        .as_array()
        .expect("best wins")
        .iter()
        .map(|game| {
            (
                game["white"].as_str().expect("White"),
                game["black"].as_str().expect("Black"),
            )
        })
        .collect();
    assert_eq!(
        best,
        [
            ("Nakamura, Hikaru", me),
            (me, "Ding, Liren"),
            (me, "Club, Player")
        ]
    );
    let missing = core
        .call("gameDbPlayer", vec![json!(id), json!("Carlsen, M")])
        .await
        .expect_err("no such player");
    assert_eq!(
        missing.message,
        "That player has no games in this database."
    );
}
