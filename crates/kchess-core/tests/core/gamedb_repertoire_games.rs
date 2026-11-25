//! A repertoire checked against your own games through the core: each of Your games where you
//! had the repertoire's side, followed through its positions until a move of yours it does not
//! play or a reply of your opponent's it has no answer to, the most played first.

use kchess_core::{Config, Core};
use serde_json::{Value, json};

use crate::support;

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const E4: &str = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
const E4_E5: &str = "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2";
const E4_C5: &str = "rnbqkbnr/pp1ppppp/8/2p5/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2";

/// White's 1. e4: after 1...e5 or 1...c5, 2. Nf3.
fn repertoire() -> Value {
    json!([
        { "fen": START, "moves": ["e2e4"] },
        { "fen": E4, "moves": ["e7e5", "c7c5"] },
        { "fen": E4_E5, "moves": ["g1f3"] },
        { "fen": E4_C5, "moves": ["g1f3"] },
    ])
}

/// A synced game of `white` against `black`, played on `date`.
fn game(id: &str, white: &str, black: &str, date: &str, moves: &str, result: &str) -> Value {
    let pgn = format!(
        "[White \"{white}\"]\n[Black \"{black}\"]\n[Date \"{date}\"]\n[Result \"{result}\"]\n\n\
         {moves} {result}\n"
    );
    support::store::library_game(id, &[("pgn", json!(pgn))])
}

#[tokio::test]
async fn your_games_show_where_they_left_the_repertoire() {
    use kchess_core::store::games;
    let profile = tempfile::tempdir().expect("profile");
    let core =
        Core::open(Config::new(profile.path().to_path_buf()), support::host()).expect("opens");
    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");
    // No games yet: nothing to check.
    let none = core
        .call("gameDbRepertoireGames", vec![repertoire(), json!("white")])
        .await
        .expect("checks");
    assert_eq!(none, json!({ "games": 0, "reached": 0, "gaps": [] }));

    core.call("addAccount", vec![json!("Alice")])
        .await
        .expect("adds the account");
    let synced = json!([
        // In the repertoire to its end, three times; the name in any case.
        game(
            "g1",
            "Alice",
            "Bob",
            "2024.03.01",
            "1. e4 e5 2. Nf3 Nc6",
            "1-0"
        ),
        game(
            "g2",
            "alice",
            "Bob",
            "2024.03.02",
            "1. e4 e5 2. Nf3 d6",
            "1-0"
        ),
        game(
            "g3",
            "Alice",
            "Cy",
            "2024.03.03",
            "1. e4 e5 2. Nf3 Nf6",
            "1/2-1/2"
        ),
        // 2. Nc3 where the repertoire plays 2. Nf3: lost once, drawn once.
        game("g4", "Alice", "Bob", "2024.01.05", "1. e4 c5 2. Nc3", "0-1"),
        game(
            "g5",
            "Alice",
            "Cy",
            "2024.01.06",
            "1. e4 c5 2. Nc3",
            "1/2-1/2"
        ),
        // 1...e6, which the repertoire has no answer to.
        game("g6", "Alice", "Bob", "2024.01.01", "1. e4 e6 2. d4", "1-0"),
        // 1. d4, not the repertoire's first move.
        game("g7", "Alice", "Bob", "2024.02.01", "1. d4 d5", "1-0"),
        // As Black: not a game of the repertoire's side.
        game("g8", "Bob", "Alice", "2024.04.01", "1. d4 d5", "0-1"),
    ]);
    core.database_for_tests()
        .write_async(move |c| {
            games::save_games(
                c.host,
                c.db,
                "Alice",
                support::store::list(&synced),
                Some(1),
            )
        })
        .await
        .expect("synced");
    core.own_games_settled_for_tests().await;

    let checked = core
        .call("gameDbRepertoireGames", vec![repertoire(), json!("white")])
        .await
        .expect("checks");
    assert_eq!(
        (&checked["games"], &checked["reached"]),
        (&json!(7), &json!(7))
    );
    // The most played first; then the latest.
    assert_eq!(
        checked["gaps"],
        json!([
            { "step": 3, "uci": "b1c3", "san": "Nc3", "yours": true, "games": 2, "won": 0,
              "drawn": 1, "lost": 1, "last": "2024.01.06" },
            { "step": 0, "uci": "d2d4", "san": "d4", "yours": true, "games": 1, "won": 1,
              "drawn": 0, "lost": 0, "last": "2024.02.01" },
            { "step": 1, "uci": "e7e6", "san": "e6", "yours": false, "games": 1, "won": 1,
              "drawn": 0, "lost": 0, "last": "2024.01.01" },
        ])
    );
    // As Black, the one game of Black's goes in at its first move: 1. d4 is not 1. e4.
    let black = core
        .call("gameDbRepertoireGames", vec![repertoire(), json!("black")])
        .await
        .expect("checks");
    assert_eq!(black["games"], 1);
    assert_eq!(black["gaps"][0]["san"], "d4");
    assert_eq!(black["gaps"][0]["yours"], false);

    let refused = core
        .call("gameDbRepertoireGames", vec![repertoire(), json!("red")])
        .await
        .expect_err("a side is white or black");
    assert!(refused.message.contains("white, black"), "{refused:?}");
    core.close().await;
}
