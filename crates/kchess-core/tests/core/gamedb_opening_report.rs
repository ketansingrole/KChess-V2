//! A position's opening report through the core: how its games ended in every database (Your
//! games aside), its main lines, the most played move after move, and its strongest games.

use kchess_core::{Config, Core};
use serde_json::{Value, json};

use crate::support;

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

fn game(white: &str, black: &str, elo: u32, result: &str, moves: &str) -> String {
    format!(
        "[White \"{white}\"]\n[Black \"{black}\"]\n[WhiteElo \"{elo}\"]\n[BlackElo \"{elo}\"]\n\
         [Result \"{result}\"]\n\n{moves} {result}\n\n"
    )
}

async fn import(core: &Core, files: &std::path::Path, name: &str, pgn: &str) {
    let path = files.join(format!("{name}.pgn"));
    std::fs::write(&path, pgn).expect("writes");
    core.call(
        "gameDbImport",
        vec![json!(path.to_string_lossy()), Value::Null, json!(name)],
    )
    .await
    .expect("imports");
}

fn line(report: &Value, index: usize) -> (String, Value) {
    let line = &report["lines"][index];
    let sans: Vec<&str> = line["moves"]
        .as_array()
        .expect("moves")
        .iter()
        .map(|m| m["san"].as_str().expect("SAN"))
        .collect();
    (
        sans.join(" "),
        json!([line["white"], line["draws"], line["black"]]),
    )
}

#[tokio::test]
async fn a_position_reads_as_its_main_lines_and_strongest_games() {
    let (profile, files) = (
        tempfile::tempdir().expect("profile"),
        tempfile::tempdir().expect("files"),
    );
    let core =
        Core::open(Config::new(profile.path().to_path_buf()), support::host()).expect("opens");
    core.call("resumeOnline", vec![])
        .await
        .expect("no game is live");
    let empty = core
        .call("gameDbOpeningReport", vec![json!(START)])
        .await
        .expect("reports");
    assert_eq!(
        empty,
        json!({ "white": 0, "draws": 0, "black": 0, "lines": [], "best": [] })
    );
    import(
        &core,
        files.path(),
        "Club",
        &[
            game("A", "B", 2000, "1-0", "1. e4 e5 2. Nf3 Nc6 3. Bb5 a6"),
            game("C", "D", 2100, "1/2-1/2", "1. e4 e5 2. Nf3 Nc6 3. Bc4"),
            game("E", "F", 2200, "0-1", "1. e4 c5 2. Nf3"),
            game("G", "H", 2300, "1-0", "1. d4 d5"),
        ]
        .concat(),
    )
    .await;
    // Another database's games count too, and its strongest game leads.
    import(
        &core,
        files.path(),
        "Masters",
        &game(
            "Magnus",
            "Hikaru",
            2850,
            "1-0",
            "1. e4 e5 2. Nf3 Nc6 3. Bb5 Nf6",
        ),
    )
    .await;

    let report = core
        .call("gameDbOpeningReport", vec![json!(START)])
        .await
        .expect("reports");
    assert_eq!(
        (&report["white"], &report["draws"], &report["black"]),
        (&json!(3), &json!(1), &json!(1))
    );
    // 1. e4: its most played move after move while two games went on; then 1. d4 d5.
    assert_eq!(
        line(&report, 0),
        ("e4 e5 Nf3 Nc6 Bb5".into(), json!([2, 1, 1]))
    );
    assert_eq!(line(&report, 1), ("d4".into(), json!([1, 0, 0])));
    assert_eq!(report["lines"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        report["lines"][0]["moves"][0],
        json!({ "san": "e4", "uci": "e2e4", "games": 4 })
    );
    let best: Vec<(&str, &str)> = report["best"]
        .as_array()
        .expect("best")
        .iter()
        .map(|found| {
            (
                found["game"]["white"].as_str().expect("White"),
                found["game"]["black"].as_str().expect("Black"),
            )
        })
        .collect();
    // Each move's best rated, and the main lines' moves': all five, the strongest first.
    assert_eq!(
        best,
        [
            ("Magnus", "Hikaru"),
            ("G", "H"),
            ("E", "F"),
            ("C", "D"),
            ("A", "B")
        ]
    );

    // From a later position, through a transposition.
    let after = "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3";
    let report = core
        .call("gameDbOpeningReport", vec![json!(after)])
        .await
        .expect("reports");
    assert_eq!(line(&report, 0), ("Bb5".into(), json!([2, 0, 0])));
    assert_eq!(line(&report, 1), ("Bc4".into(), json!([0, 1, 0])));
}
