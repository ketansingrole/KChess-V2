//! A repertoire checked against the game databases through the core: each of its moves with the
//! games that played it and a verdict (played, rare, never played, or scoring well below the
//! main alternative), the most played others beside it, and Your games left out.

use kchess_core::{Config, Core};
use serde_json::{Value, json};

use crate::support;

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const AFTER_E4: &str = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";

/// `count` games of `moves` ending in `result`, rated 2400 and 2200.
fn games(count: usize, moves: &str, result: &str) -> String {
    let game = format!(
        "[White \"W\"]\n[Black \"B\"]\n[WhiteElo \"2400\"]\n[BlackElo \"2200\"]\n\
         [Result \"{result}\"]\n\n{moves} {result}\n\n"
    );
    game.repeat(count)
}

async fn check(core: &Core, steps: Value) -> Value {
    core.call("gameDbRepertoire", vec![steps])
        .await
        .expect("checks")
}

/// Each of a step's moves: its SAN, games and verdict (or null for the others).
fn moves(check: &Value) -> Vec<(String, u64, Value)> {
    check["moves"]
        .as_array()
        .expect("moves")
        .iter()
        .map(|m| {
            (
                m["san"].as_str().expect("SAN").to_string(),
                m["games"].as_u64().expect("games"),
                m.get("verdict").cloned().unwrap_or(Value::Null),
            )
        })
        .collect()
}

#[tokio::test]
async fn a_repertoire_s_moves_are_judged_by_the_databases_games() {
    let (profile, files) = (
        tempfile::tempdir().expect("profile"),
        tempfile::tempdir().expect("files"),
    );
    let core =
        Core::open(Config::new(profile.path().to_path_buf()), support::host()).expect("opens");
    let steps = json!([{ "fen": START, "moves": ["e2e4"] }]);
    let refused = core
        .call("gameDbRepertoire", vec![steps.clone()])
        .await
        .expect_err("help is withheld until no game is live");
    assert_eq!(refused.kind, Some(kchess_contracts::ErrorKind::Withheld));
    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");
    // No database: no games, and too few to judge.
    let none = check(&core, steps).await;
    assert_eq!(none[0]["games"], 0);
    assert_eq!(moves(&none[0]), [("e4".into(), 0, json!("played"))]);

    // 1. e4 scores 14 of 23 for White (61%), 1. d4 4½ of 12 (38%).
    let pgn = [
        games(11, "1. e4 e5", "1-0"),
        games(6, "1. e4 e5", "1/2-1/2"),
        games(5, "1. e4 e5", "0-1"),
        games(1, "1. e4 c5", "0-1"),
        games(3, "1. d4 d5", "1-0"),
        games(3, "1. d4 d5", "1/2-1/2"),
        games(6, "1. d4 d5", "0-1"),
    ]
    .concat();
    let path = files.path().join("club.pgn");
    std::fs::write(&path, pgn).expect("writes");
    core.call(
        "gameDbImport",
        vec![json!(path.to_string_lossy()), Value::Null, json!("Club")],
    )
    .await
    .expect("imports");

    let checks = check(
        &core,
        json!([
            { "fen": START, "moves": ["d2d4"] },
            { "fen": START, "moves": ["e2e4", "d2d4"] },
            { "fen": AFTER_E4, "moves": ["c7c5"] },
            { "fen": AFTER_E4, "moves": ["g8f6", "g8f6"] },
            { "fen": "8/8/8/8/8/8/8/8 w - - 0 1", "moves": ["e2e4"] },
        ]),
    )
    .await;
    let checks = checks.as_array().expect("one answer a step");
    assert_eq!(checks.len(), 5);
    assert_eq!(checks[0]["games"], 35);
    // d4 scores 38% where e4, the main alternative, scores 61%.
    assert_eq!(
        moves(&checks[0]),
        [
            ("d4".into(), 12, json!("weaker")),
            ("e4".into(), 23, Value::Null)
        ]
    );
    let d4 = &checks[0]["moves"][0];
    assert_eq!(
        (&d4["white"], &d4["draws"], &d4["black"], &d4["rating"]),
        (&json!(3), &json!(3), &json!(6), &json!(2300))
    );
    // Playing both, neither has another to be measured against.
    assert_eq!(
        moves(&checks[1]),
        [
            ("e4".into(), 23, json!("played")),
            ("d4".into(), 12, json!("played"))
        ]
    );
    // One game of 23 played the Sicilian: rare.
    assert_eq!(checks[2]["games"], 23);
    assert_eq!(
        moves(&checks[2]),
        [
            ("c5".into(), 1, json!("rare")),
            ("e5".into(), 22, Value::Null)
        ]
    );
    // No game played 1...Nf6; asked twice, it is judged once.
    assert_eq!(
        moves(&checks[3]),
        [
            ("Nf6".into(), 0, json!("unplayed")),
            ("e5".into(), 22, Value::Null),
            ("c5".into(), 1, Value::Null)
        ]
    );
    // A position that is not legal has no games.
    assert_eq!(checks[4]["games"], 0);
    assert_eq!(checks[4]["moves"], json!([]));

    let refused = core
        .call(
            "gameDbRepertoire",
            vec![json!([{ "fen": START, "moves": [] }])],
        )
        .await
        .expect_err("a step plays a move");
    assert_eq!(refused.message, "Invalid repertoire.");
}
