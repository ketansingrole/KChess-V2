//! Lichess analysis as a review.

use kchess_contracts::types::{Judgment, ReviewEval, ReviewSource};
use kchess_core::lichess::reviews::review_from_lichess;
use kchess_domain::review::analyse;
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
    let key = kchess_domain::records::review::review_key(START, &MOVES, None);
    assert_eq!(review.key, key);
    assert_eq!(review.moves, MOVES);
    assert_eq!(review.source, ReviewSource::Lichess);
    assert!(review.complete);
    assert_eq!(review.game_id.as_deref(), Some("abcdefgh"));
    let evals = &review.evals;
    let score = |cp: Option<f64>, mate: Option<f64>| {
        Some(ReviewEval {
            cp,
            mate,
            ..ReviewEval::default()
        })
    };
    // Lichess counts the start as +0.15; entry i scores the position after move i+1.
    assert_eq!(evals[0], score(Some(15.0), None));
    assert_eq!(evals[1], score(Some(30.0), None));
    assert_eq!(evals[6], score(None, Some(1.0)));
    // The better move belongs to the position before the blunder.
    assert_eq!(
        evals[5],
        Some(ReviewEval {
            cp: Some(50.0),
            best: Some("g7g6".into()),
            pv: Some(vec!["g7g6".into(), "h5f3".into(), "g8f6".into()]),
            ..ReviewEval::default()
        })
    );
    // The mating move has no entry: the position speaks for itself.
    assert_eq!(evals[7], None);
    let mut judgments = vec![None; 7];
    judgments[5] = Some(Judgment::Blunder);
    assert_eq!(review.judgments, Some(judgments));
    assert_eq!(
        serde_json::to_value(&review.accuracy).unwrap(),
        json!({ "white": 95.0, "black": 40.0 })
    );
}

#[test]
fn keeps_lichess_labels_and_accuracy_when_summarized() {
    let review = review_from_lichess(&game()).expect("analysed game");
    let result = serde_json::to_value(analyse(&review).expect("summary")).unwrap();
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
