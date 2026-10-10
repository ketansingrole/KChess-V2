//! Lichess analysis as a review (`reviewFromLichess` in `core/src/services/lichess.ts`). Ports
//! `core/tests/unit/lichess-review.test.ts`.

use kchess_core::lichess::reviews::review_from_lichess;
use kchess_domain::review::analyse_review;
use serde_json::{Value, json};

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const MOVES: [&str; 7] = ["e2e4", "e7e5", "d1h5", "b8c6", "f1c4", "g8f6", "h5f7"];

fn player(name: &str, accuracy: f64) -> Value {
    json!({
        "user": { "id": name, "name": name },
        "rating": 1500,
        "analysis": {
            "inaccuracy": 0,
            "mistake": 0,
            "blunder": if accuracy < 50.0 { 1 } else { 0 },
            "acpl": 10,
            "accuracy": accuracy
        }
    })
}

/// 1. e4 e5 2. Qh5 Nc6 3. Bc4 Nf6?? 4. Qxf7#, as Lichess exports it with its analysis.
fn game() -> Value {
    json!({
        "id": "abcdefgh",
        "rated": true,
        "variant": "standard",
        "speed": "blitz",
        "perf": "blitz",
        "createdAt": 0,
        "lastMoveAt": 0,
        "status": "mate",
        "players": { "white": player("alice", 95.0), "black": player("bob", 40.0) },
        "moves": "e4 e5 Qh5 Nc6 Bc4 Nf6 Qxf7#",
        "analysis": [
            { "eval": 30 },
            { "eval": 40 },
            { "eval": 20 },
            { "eval": 30 },
            { "eval": 50 },
            {
                "mate": 1,
                "best": "g7g6",
                "variation": "g6 Qf3 Nf6",
                "judgment": { "name": "Blunder", "comment": "Checkmate is now unavoidable. g6 was best." }
            }
        ]
    })
}

#[test]
fn maps_scores_the_better_move_and_the_labels_onto_the_positions() {
    let review = review_from_lichess(&game()).expect("analysed game");
    let key = kchess_domain::records::call("reviewKey", &[json!(START), json!(MOVES)])
        .unwrap()
        .unwrap();
    assert_eq!(review["key"], key);
    assert_eq!(review["moves"], json!(MOVES));
    assert_eq!(review["source"], "lichess");
    assert_eq!(review["complete"], true);
    assert_eq!(review["gameId"], "abcdefgh");
    let evals = review["evals"].as_array().unwrap();
    // Lichess counts the start as +0.15; entry i scores the position after move i+1.
    assert_eq!(evals[0], json!({ "cp": 15 }));
    assert_eq!(evals[1], json!({ "cp": 30 }));
    assert_eq!(evals[6], json!({ "mate": 1 }));
    // The better move belongs to the position before the blunder.
    assert_eq!(
        evals[5],
        json!({ "cp": 50, "best": "g7g6", "pv": ["g7g6", "h5f3", "g8f6"] })
    );
    // The mating move has no entry: the position speaks for itself.
    assert_eq!(evals[7], Value::Null);
    assert_eq!(
        review["judgments"],
        json!([null, null, null, null, null, "blunder", null])
    );
    assert_eq!(review["accuracy"], json!({ "white": 95.0, "black": 40.0 }));
}

#[test]
fn keeps_lichess_labels_and_accuracy_when_summarized() {
    let review = review_from_lichess(&game()).expect("analysed game");
    let result = analyse_review(&review).expect("summary");
    assert_eq!(result["black"]["accuracy"], 40.0);
    assert_eq!(result["black"]["blunder"], 1);
    assert_eq!(result["white"]["accuracy"], 95.0);
    assert_eq!(result["white"]["blunder"], 0);
    assert_eq!(
        result["moves"].as_array().unwrap().last().unwrap()["chances"],
        1.0
    );
}

#[test]
fn skips_games_lichess_has_not_analysed_and_variants() {
    let mut unanalysed = game();
    unanalysed.as_object_mut().unwrap().remove("analysis");
    assert!(review_from_lichess(&unanalysed).is_none());
    let mut variant = game();
    variant["variant"] = json!("chess960");
    assert!(review_from_lichess(&variant).is_none());
}
