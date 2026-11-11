//! The bytes of the puzzle, library, view-preference and system methods' answers, as a frontend
//! reads them through the core (`support::wire`). What the core stamps itself (times, sizes on
//! disk, new ids) is replaced by a fixed value of the same JSON kind before comparing.

use std::collections::HashMap;
use std::sync::Arc;

use kchess_core::puzzles::queries::DbPuzzle;
use kchess_core::{Config, Core};
use serde_json::{Value, json};

use crate::fixtures::{Reply, serve};
use crate::signed_in;
use crate::support::wire::assert_wire;

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// Keys whose numbers the core stamps (a clock, a size on disk).
const STAMPED: &[&str] = &[
    "at",
    "playedAt",
    "updatedAt",
    "exportedAt",
    "importedAt",
    "dueAt",
    "since",
    "dbBytes",
    "bytes",
    "due",
];

/// `answer` with what the core stamped replaced: numbers under `STAMPED` keys by a zero of the
/// same kind, generated ids (UUIDs) by `id-N` in order of appearance, and ISO times by `time`.
fn steady(answer: Value, ids: &mut HashMap<String, String>) -> Value {
    match answer {
        Value::Object(map) => {
            Value::Object(
                map.into_iter()
                    .map(|(key, value)| {
                        let value =
                            match value {
                                Value::Number(n) if STAMPED.contains(&key.as_str()) => {
                                    if n.is_f64() { json!(0.0) } else { json!(0) }
                                }
                                Value::String(_) if key == "time" => json!("time"),
                                value => steady(value, ids),
                            };
                        (key, value)
                    })
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(|v| steady(v, ids)).collect()),
        Value::String(text) if is_uuid(&text) => {
            let next = format!("id-{}", ids.len() + 1);
            Value::String(ids.entry(text).or_insert(next).clone())
        }
        other => other,
    }
}

fn is_uuid(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    parts.iter().map(|part| part.len()).eq([8, 4, 4, 4, 12])
        && parts
            .iter()
            .all(|part| part.bytes().all(|b| b.is_ascii_hexdigit()))
}

struct Recorder<'a> {
    core: &'a Core,
    ids: HashMap<String, String>,
}

impl Recorder<'_> {
    /// `method(args)` answered, and its answer's bytes as recorded under `name`.
    async fn wire(&mut self, name: &str, method: &str, args: Vec<Value>) -> Value {
        let answer = self
            .core
            .call(method, args)
            .await
            .unwrap_or_else(|error| panic!("{name}: {}", error.message));
        let answer = steady(answer, &mut self.ids);
        assert_wire(name, &answer);
        answer
    }
}

fn puzzle_rows() -> Vec<DbPuzzle> {
    vec![
        DbPuzzle {
            id: "aaaaa".into(),
            fen: START.into(),
            moves: "e2e4 e7e5 g1f3".into(),
            rating: 1500.0,
            plays: Some(10.0),
            themes: "fork short".into(),
            openings: String::new(),
            game_id: "abcdefgh".into(),
        },
        DbPuzzle {
            id: "bbbbb".into(),
            fen: START.into(),
            moves: "d2d4 d7d5 c2c4".into(),
            rating: 1800.0,
            plays: None,
            themes: "mateIn2".into(),
            openings: String::new(),
            game_id: String::new(),
        },
    ]
}

fn attempt(retry: bool, wrong_moves: bool) -> Value {
    let mut attempt = json!({
        "puzzle": {
            "id": "aaaaa", "fen": START, "lastMove": "e2e4", "solution": ["e7e5", "g1f3"],
            "rating": 1500, "themes": ["fork"], "plays": 10, "gameId": "abcdefgh",
        },
        "mode": "offline", "retry": retry, "win": !wrong_moves, "revealed": false,
        "hinted": false, "found": 1, "wrong": u8::from(wrong_moves), "total": 1,
        "day": "2026-11-10",
    });
    if wrong_moves {
        attempt["wrongMoves"] = json!([{ "index": 0, "uci": "d7d5" }]);
    }
    attempt
}

#[tokio::test]
async fn local_answers_keep_their_bytes() {
    let dir = tempfile::tempdir().expect("profile");
    kchess_core::puzzles::write_database(dir.path(), &puzzle_rows()).expect("puzzles");
    let host = crate::support::host();
    let core = Core::open(Config::new(dir.path().to_path_buf()), host).expect("opens");
    let mut r = Recorder {
        core: &core,
        ids: HashMap::new(),
    };

    // System.
    r.wire("usage.empty", "usage", vec![]).await;
    r.wire("resetUsage", "resetUsage", vec![]).await;

    // View preferences.
    r.wire("viewPreferences.empty", "viewPreferences", vec![])
        .await;
    r.wire(
        "setViewPreference",
        "setViewPreference",
        vec![
            json!("kchess:tab"),
            json!({ "open": true, "n": 2, "x": 1.5 }),
        ],
    )
    .await;
    r.wire(
        "setViewPreference.text",
        "setViewPreference",
        vec![json!("kchess:sort"), json!("name")],
    )
    .await;
    r.wire("viewPreferences.full", "viewPreferences", vec![])
        .await;
    r.wire(
        "setViewPreference.forget",
        "setViewPreference",
        vec![json!("kchess:tab"), Value::Null],
    )
    .await;
    r.wire("viewPreferences.after", "viewPreferences", vec![])
        .await;

    // The voice log.
    let full = json!({
        "source": "computer", "heard": "knight f three", "confidence": 0.75,
        "words": [{ "word": "knight", "conf": 1 }, { "word": "f", "conf": 0.5 }],
        "outcome": "pending", "parsed": "Nf3", "expected": "Nf3", "fen": START,
    });
    let first = r
        .wire("saveVoiceAttempt", "saveVoiceAttempt", vec![full])
        .await;
    let short = json!({
        "source": "puzzles", "heard": "e four", "confidence": 1, "words": [],
        "outcome": "invalid", "retryOf": first,
    });
    r.wire("saveVoiceAttempt.short", "saveVoiceAttempt", vec![short])
        .await;
    r.wire(
        "updateVoiceAttempt",
        "updateVoiceAttempt",
        vec![
            first.clone(),
            json!({ "outcome": "confirmed", "expected": "Nc3" }),
        ],
    )
    .await;
    r.wire("voiceHistory", "voiceHistory", vec![json!(10)])
        .await;
    let document = r
        .core
        .call("voiceHistoryDocument", vec![])
        .await
        .expect("document");
    let parsed: Value = serde_json::from_str(document.as_str().expect("text")).expect("JSON");
    assert_wire("voiceHistoryDocument", &steady(parsed, &mut HashMap::new()));
    r.wire("clearVoiceHistory", "clearVoiceHistory", vec![])
        .await;
    r.wire("voiceHistory.empty", "voiceHistory", vec![json!(10)])
        .await;

    // Runs.
    let run = json!({
        "kind": "storm", "variant": "3min", "score": 12,
        "detail": { "combo": 4, "accuracy": 0.5, "mode": "fast", "timed": true },
    });
    r.wire("saveRun", "saveRun", vec![run]).await;
    let lower = json!({ "kind": "storm", "variant": "3min", "score": 2.5, "detail": {} });
    r.wire("saveRun.lower", "saveRun", vec![lower]).await;
    r.wire("runSummary", "runSummary", vec![json!("storm")])
        .await;
    r.wire("runSummary.empty", "runSummary", vec![json!("rush")])
        .await;
    r.wire("clearRuns.kind", "clearRuns", vec![json!("storm")])
        .await;
    r.wire("clearRuns", "clearRuns", vec![]).await;

    // The puzzle database.
    r.wire("puzzleDbStatus", "puzzleDbStatus", vec![]).await;
    r.wire(
        "localPuzzles.theme",
        "localPuzzles",
        vec![json!({ "theme": "fork", "count": 5 })],
    )
    .await;
    r.wire(
        "localPuzzles.rating",
        "localPuzzles",
        vec![json!({ "minRating": 1700, "maxRating": 2000, "count": 5 })],
    )
    .await;
    // The ladder draws at random: its puzzles are compared in the order of their ids.
    let mut ladder = core
        .call(
            "localLadder",
            vec![json!({ "from": 1000, "to": 2000, "count": 2 })],
        )
        .await
        .expect("ladder");
    if let Value::Array(items) = &mut ladder {
        items.sort_by_key(|item| item["id"].as_str().unwrap_or_default().to_string());
    }
    assert_wire("localLadder", &ladder);
    r.wire(
        "recordPuzzleAttempt",
        "recordPuzzleAttempt",
        vec![attempt(false, true)],
    )
    .await;
    r.wire(
        "recordPuzzleAttempt.retry",
        "recordPuzzleAttempt",
        vec![attempt(true, false)],
    )
    .await;
    r.wire(
        "puzzleProgress",
        "puzzleProgress",
        vec![json!({ "today": "2026-11-10", "limit": 5 })],
    )
    .await;
    r.wire("puzzleDbCancel", "puzzleDbCancel", vec![]).await;
    r.wire("puzzleDbDelete", "puzzleDbDelete", vec![]).await;
    r.wire("puzzleDbStatus.deleted", "puzzleDbStatus", vec![])
        .await;

    core.close().await;
}

/// A complete review of a game whose sixth move (Black's) blunders mate in one.
fn store_review(core: &Core) -> String {
    const BLUNDER: [&str; 7] = ["e2e4", "e7e5", "d1h5", "b8c6", "f1c4", "g8f6", "h5f7"];
    let key = kchess_domain::records::review::review_key(START, &BLUNDER, None);
    let mut evals: Vec<Value> = (0..5)
        .map(|_| json!({ "cp": 30, "depth": 20, "best": "a2a3" }))
        .collect();
    evals.push(json!({ "cp": 30, "depth": 20, "best": "g7g6", "pv": ["g7g6", "h5f3", "g8f6"] }));
    evals.push(json!({ "mate": 1, "depth": 20, "best": "h5f7", "pv": ["h5f7"] }));
    evals.push(json!({ "mate": 0, "depth": 20 }));
    let review = json!({
        "key": key, "fen": START, "moves": BLUNDER, "source": "local",
        "complete": true, "depth": 20, "updatedAt": 7, "evals": evals,
    });
    core.database_for_tests()
        .write(|ctx| kchess_core::store::reviews::write_review(ctx.host, ctx.db, &review))
        .expect("review");
    key
}

#[tokio::test]
async fn library_answers_keep_their_bytes() {
    let dir = tempfile::tempdir().expect("profile");
    let host = crate::support::host();
    let core = Core::open(Config::new(dir.path().to_path_buf()), host).expect("opens");
    let mut r = Recorder {
        core: &core,
        ids: HashMap::new(),
    };
    r.wire("library.empty", "library", vec![]).await;

    // Studies, one command of each kind.
    let saved = r
        .wire(
            "studyCommand.saveChapters",
            "studyCommand",
            vec![json!({
                "op": "saveChapters", "name": "Openings",
                "chapters": [
                    { "name": "King pawn", "pgn": "1. e4 e5 *" },
                    { "name": "Queen pawn", "pgn": "1. d4 d5 *" },
                ],
            })],
        )
        .await;
    let study = core.call("library", vec![]).await.expect("library")["studies"][0].clone();
    let id = study["id"].clone();
    let chapter = study["chapters"][0]["id"].clone();
    let second = study["chapters"][1]["id"].clone();
    assert_eq!(saved["id"], json!("id-1"));
    r.wire(
        "studyCommand.save",
        "studyCommand",
        vec![json!({ "op": "save", "name": "Sicilian", "pgn": "1. e4 c5 *" })],
    )
    .await;
    r.wire(
        "studyCommand.save-chapter",
        "studyCommand",
        vec![
            json!({ "op": "save", "name": "Openings", "pgn": "1. e4 e5 2. Nf3 *",
                    "id": id, "chapterId": chapter }),
            json!(true),
            json!("1. e4 e5 *"),
        ],
    )
    .await;
    r.wire(
        "studyCommand.addChapter",
        "studyCommand",
        vec![json!({ "op": "addChapter", "id": id, "name": "Third" })],
    )
    .await;
    r.wire(
        "studyCommand.renameChapter",
        "studyCommand",
        vec![
            json!({ "op": "renameChapter", "id": id, "chapterId": chapter, "name": "e4" }),
            json!(true),
        ],
    )
    .await;
    r.wire(
        "studyCommand.duplicateChapter",
        "studyCommand",
        vec![json!({ "op": "duplicateChapter", "id": id, "chapterId": chapter })],
    )
    .await;
    r.wire(
        "studyCommand.moveChapter",
        "studyCommand",
        vec![json!({ "op": "moveChapter", "id": id, "chapterId": second, "index": 0 })],
    )
    .await;
    let removed = r
        .wire(
            "studyCommand.removeChapter",
            "studyCommand",
            vec![json!({ "op": "removeChapter", "id": id, "chapterId": second })],
        )
        .await;
    assert!(removed.get("id").is_some());
    r.wire(
        "studyCommand.restoreChapter",
        "studyCommand",
        vec![json!({
            "op": "restoreChapter", "id": id, "index": 1,
            "chapter": { "id": second, "name": "Queen pawn", "pgn": "1. d4 d5 *" },
        })],
    )
    .await;
    r.wire(
        "studyCommand.markCloud",
        "studyCommand",
        vec![json!({
            "op": "markCloud", "id": id, "account": "Alice", "remoteId": "abcdEFGH",
            "baseline": "1. e4 *",
        })],
    )
    .await;
    r.wire(
        "studyCommand.offline",
        "studyCommand",
        vec![json!({
            "op": "offline", "account": "Alice", "remote": { "id": "abcdEFGH", "name": "Cloud" },
            "chapters": [{ "name": "One", "pgn": "1. c4 *" }],
        })],
    )
    .await;
    r.wire(
        "studyCommand.rename",
        "studyCommand",
        vec![json!({ "op": "rename", "id": id, "name": "Renamed" })],
    )
    .await;
    r.wire(
        "studyCommand.duplicate",
        "studyCommand",
        vec![json!({ "op": "duplicate", "id": id })],
    )
    .await;
    let gone = core
        .call("studyCommand", vec![json!({ "op": "remove", "id": id })])
        .await
        .expect("removed");
    assert_wire("studyCommand.remove", &steady(gone.clone(), &mut r.ids));
    r.wire(
        "studyCommand.restore",
        "studyCommand",
        vec![json!({ "op": "restore", "study": gone["removed"] })],
    )
    .await;
    r.wire(
        "studyCommand.incremental",
        "studyCommand",
        vec![
            json!({ "op": "rename", "id": "missing", "name": "x" }),
            json!(true),
        ],
    )
    .await;

    // Played games and sessions.
    let game = json!({
        "id": "game-1", "source": "computer", "startedAt": 5, "updatedAt": 6,
        "white": "You", "black": "Stockfish", "result": "*", "reason": "Unfinished",
        "timeControl": "-", "finished": false,
        "setup": { "variant": "standard", "fen": START }, "moves": ["e2e4", "e7e5"],
    });
    r.wire("saveArchivedGame", "saveArchivedGame", vec![game])
        .await;
    r.wire(
        "saveSession",
        "saveSession",
        vec![
            json!("analysis"),
            json!({ "pgn": "1. e4 e5 *", "path": "e2e4" }),
        ],
    )
    .await;
    r.wire(
        "saveSession.archive",
        "saveSession",
        vec![
            json!("archive:board"),
            json!({ "id": "game-1", "startedAt": 5 }),
        ],
    )
    .await;
    r.wire("session", "session", vec![json!("analysis")]).await;
    r.wire("session.none", "session", vec![json!("local")])
        .await;
    r.wire("joinedTournaments", "joinedTournaments", vec![])
        .await;

    // Mistake drills.
    let key = store_review(&core);
    r.wire(
        "addMistakes.white",
        "addMistakes",
        vec![json!(key), json!("white")],
    )
    .await;
    let added = r.wire("addMistakes", "addMistakes", vec![json!(key)]).await;
    let drill = added["items"][0]["id"].clone();
    r.wire("mistakes", "mistakes", vec![]).await;
    r.wire(
        "answerMistake",
        "answerMistake",
        vec![drill.clone(), json!(true)],
    )
    .await;
    r.wire(
        "answerMistake.missed",
        "answerMistake",
        vec![drill, json!(false)],
    )
    .await;
    r.wire(
        "answerMistake.unknown",
        "answerMistake",
        vec![json!("nothing"), json!(true)],
    )
    .await;
    r.wire("mistakeGames", "mistakeGames", vec![]).await;

    // Repertoire notes.
    let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
    r.wire(
        "recordRepertoireMiss",
        "recordRepertoireMiss",
        vec![json!("study:white"), json!(fen)],
    )
    .await;
    r.wire(
        "recordRepertoireMiss.again",
        "recordRepertoireMiss",
        vec![json!("study:white"), json!(fen)],
    )
    .await;
    r.wire(
        "clearRepertoireMisses",
        "clearRepertoireMisses",
        vec![json!("study:white")],
    )
    .await;
    r.wire(
        "restoreRepertoireMisses",
        "restoreRepertoireMisses",
        vec![json!("study:white"), json!({ fen: 2 })],
    )
    .await;
    r.wire(
        "reviewRepertoireLine",
        "reviewRepertoireLine",
        vec![json!("study:white"), json!("e2e4 e7e5"), json!(true)],
    )
    .await;
    r.wire(
        "reviewRepertoireLine.wrong",
        "reviewRepertoireLine",
        vec![json!("study:white"), json!("d2d4"), json!(false)],
    )
    .await;
    r.wire("library.full", "library", vec![]).await;
    core.close().await;
}

fn raw_puzzle(id: &str) -> Value {
    json!({
        "game": { "id": "gameid12", "pgn": "e4 e5 Nf3" },
        "puzzle": {
            "id": id, "rating": 1612, "plays": 345, "initialPly": 2,
            "solution": ["b8c6", "f1b5"], "themes": ["opening", "short"],
        },
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lichess_puzzle_answers_keep_their_bytes() {
    let fixture = serve(|seen| match seen.path() {
        "/api/stream/event" => Reply::Hang,
        "/api/account/playing" => Reply::Json(200, json!({ "nowPlaying": [] }).to_string()),
        "/api/puzzle/daily" => Reply::Json(200, raw_puzzle("daily").to_string()),
        "/api/puzzle/abcde" => Reply::Json(200, raw_puzzle("abcde").to_string()),
        "/api/puzzle/next" => Reply::Json(200, raw_puzzle("anon1").to_string()),
        "/api/puzzle/batch/fork" if seen.method == "GET" => Reply::Json(
            200,
            json!({
                "puzzles": [raw_puzzle("batch")],
                "glicko": { "rating": 1500.5, "deviation": 60, "provisional": false },
            })
            .to_string(),
        ),
        "/api/puzzle/batch/mix" => Reply::Json(
            200,
            json!({ "puzzles": [], "glicko": { "rating": 1500, "deviation": 60 } }).to_string(),
        ),
        "/api/puzzle/batch/fork" => Reply::Json(
            200,
            json!({ "rounds": [{ "id": "batch", "ratingDiff": 12 }] }).to_string(),
        ),
        "/api/puzzle/batch/short" => Reply::Json(
            200,
            json!({ "rounds": [{ "id": "batch", "ratingDiff": null }] }).to_string(),
        ),
        "/api/puzzle/dashboard/30" => Reply::Json(
            200,
            json!({ "days": 30, "global": { "nb": 3, "firstWins": 2, "performance": 1600 },
                    "themes": {} })
            .to_string(),
        ),
        "/api/puzzle/activity" => Reply::Json(
            200,
            [
                json!({ "date": 1_700_000_000_000_i64, "win": true,
                        "puzzle": { "id": "act01", "fen": START, "lastMove": "e2e4",
                                    "solution": ["e7e5"], "rating": 1400, "themes": ["fork"],
                                    "plays": 3 } }),
                json!({ "date": 1_700_000_000_001_i64, "win": false,
                        "puzzle": { "id": "act02", "fen": START, "lastMove": "d2d4",
                                    "solution": ["d7d5"], "rating": 1450.5,
                                    "themes": [] } }),
            ]
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
        ),
        "/api/storm/dashboard/Alice" => Reply::Json(
            200,
            json!({ "high": { "allTime": 30, "day": 10 }, "days": [] }).to_string(),
        ),
        _ => Reply::Json(200, "{}".into()),
    })
    .await;
    let profile = signed_in::open(&fixture.base).await;
    let core = Arc::clone(&profile.core);
    let mut r = Recorder {
        core: &core,
        ids: HashMap::new(),
    };
    r.wire("puzzleDaily", "puzzleDaily", vec![]).await;
    r.wire("puzzleById", "puzzleById", vec![json!("abcde")])
        .await;
    r.wire(
        "puzzleNext.anonymous",
        "puzzleNext",
        vec![json!({ "account": "", "angle": "mix", "difficulty": "normal" })],
    )
    .await;
    r.wire(
        "puzzleNext",
        "puzzleNext",
        vec![
            json!({ "account": "Alice", "angle": "fork", "difficulty": "harder",
                     "color": "white" }),
        ],
    )
    .await;
    r.wire(
        "puzzleNext.none-left",
        "puzzleNext",
        vec![json!({ "account": "Alice", "angle": "mix", "difficulty": "normal" })],
    )
    .await;
    r.wire(
        "puzzleNext.reconnect",
        "puzzleNext",
        vec![json!({ "account": "Bob", "angle": "mix", "difficulty": "normal" })],
    )
    .await;
    r.wire(
        "puzzleSolve",
        "puzzleSolve",
        vec![
            json!({ "account": "Alice", "angle": "fork", "id": "batch", "win": true,
                     "rated": true }),
        ],
    )
    .await;
    r.wire(
        "puzzleSolve.null",
        "puzzleSolve",
        vec![
            json!({ "account": "Alice", "angle": "short", "id": "batch", "win": true,
                     "rated": true }),
        ],
    )
    .await;
    r.wire(
        "puzzleSolve.unrated",
        "puzzleSolve",
        vec![
            json!({ "account": "Alice", "angle": "fork", "id": "batch", "win": false,
                     "rated": false }),
        ],
    )
    .await;
    r.wire(
        "puzzleDashboard",
        "puzzleDashboard",
        vec![json!("Alice"), json!(30)],
    )
    .await;
    r.wire(
        "puzzleActivity",
        "puzzleActivity",
        vec![json!("Alice"), json!(10)],
    )
    .await;
    r.wire(
        "stormDashboard",
        "stormDashboard",
        vec![json!("Alice"), json!(30)],
    )
    .await;
    // What those downloads counted, per account and kind.
    r.wire("usage", "usage", vec![]).await;
}
