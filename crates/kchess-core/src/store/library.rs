//! The local library: studies, played games, mistake drills,
//! unfinished sessions, joined tournaments and repertoire notes, each a bounded, versioned
//! document in `kchess.db`. The decoders and semantic validation are `kchess_domain`'s.
//!
//! The codec, document primitives, studies and the legacy import live in the sibling
//! `json_codec`, `documents`, `studies` and `library_import` modules; this module keeps the
//! `call` router, the helpers the other storage modules share, the games/mistakes/tournaments
//! writers, and re-exports so every `crate::store::library::X` path keeps working.

use getrandom::fill;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

use super::StoreContext;
use crate::error::{CoreError, Result};
use crate::host::{Host, Level};
use kchess_domain::js;
use kchess_domain::library::{MAX_ARCHIVED_GAMES, MAX_MISTAKES};
use kchess_domain::misc::MAX_DOCUMENT;

pub(crate) const STUDY_LIBRARY_MAX_TEXT: usize = 1_800_000;

pub(crate) const STUDIES: &str = "studies";
pub(crate) const GAMES: &str = "games";
pub(crate) const MISTAKES: &str = "mistakes";
pub(crate) const JOINED: &str = "tournaments:joined";
pub(crate) const MISSES: &str = "repertoire:misses";
pub(crate) const IMPORTED: &str = "library:imported";

pub(crate) const SESSION_KINDS: [&str; 6] = [
    "analysis",
    "local",
    "computer",
    "archive:computer",
    "archive:board",
    "archive:clock",
];

/// `kchess:<legacy key>` → library target, in the order the import takes them.
pub(crate) const LEGACY_TARGETS: [(&str, &str); 11] = [
    ("kchess:studies:v1", STUDIES),
    ("kchess:game-history:v1", GAMES),
    ("kchess:mistakes:v1", MISTAKES),
    ("kchess:analysis:v1", "session:analysis"),
    ("kchess:local:v1", "session:local"),
    ("kchess:computer:v1", "session:computer"),
    (
        "kchess:history-session:computer",
        "session:archive:computer",
    ),
    ("kchess:history-session:board", "session:archive:board"),
    ("kchess:history-session:clock", "session:archive:clock"),
    ("kchess:tournaments-joined", JOINED),
    ("kchess:repertoire-misses", MISSES),
];

const GAMES_FULL: &str =
    "Game history is full. Export saved games before making room for new games.";

/// This module's storage methods; None when the method is not one of them.
pub fn call(ctx: &StoreContext, method: &str, args: &[Value]) -> Option<Result<Value>> {
    let db = ctx.db;
    let host = ctx.host;
    let result = match method {
        "store.library.library" => now_arg(args, 0).and_then(|now| library(host, db, now)),
        "store.library.readSession" => {
            str_arg(args, 0, "kind").and_then(|kind| read_session(host, db, kind))
        }
        "store.library.importLibrary" => import_library(host, db, args.first()),
        "store.library.studyCommand" => now_arg(args, 1).and_then(|now| {
            let mut command = args.first().cloned().unwrap_or(Value::Null);
            if let Some(based_on) = args.get(3).and_then(Value::as_str) {
                let name = check_based_on(db, &command, based_on)?;
                // A content save changes the chapter's moves only, never the study's name.
                if let (Some(name), Some(fields)) = (name, command.as_object_mut())
                    && fields.get("op").and_then(Value::as_str) == Some("save")
                {
                    fields.insert("name".into(), Value::String(name));
                }
            }
            study_command(db, &command, now, args.get(2) == Some(&Value::Bool(true)))
        }),
        "store.library.saveArchivedGame" => {
            write_game(db, args.first().unwrap_or(&Value::Null)).map(|()| Value::Null)
        }
        "store.library.removeArchivedGame" => str_arg(args, 0, "id").and_then(|id| {
            db.execute("DELETE FROM archived_games WHERE id = ?1", params![id])
                .map_err(db_err)
                .map(|_| Value::Null)
        }),
        "store.library.addMistakes" => str_arg(args, 0, "reviewKey").and_then(|key| {
            let color = opt_str(args, 1);
            now_arg(args, 2).and_then(|now| add_mistakes(host, db, key, color, now))
        }),
        "store.library.answerMistake" => str_arg(args, 0, "id").and_then(|id| {
            let solved = truthy(args.get(1));
            now_arg(args, 2).and_then(|now| answer_mistake(db, id, solved, now))
        }),
        "store.library.mistakeGames" => mistake_games(host, db),
        "store.library.saveSession" => str_arg(args, 0, "kind").and_then(|kind| {
            let value = args.get(1).cloned().unwrap_or(Value::Null);
            let encoded = misc("encodeSession", vec![json!(kind), value])?;
            write_document(
                db,
                &format!("session:{kind}"),
                &encoded,
                "This session is too large to save automatically. Export a PGN copy.",
            )
        }),
        "store.library.joinedTournaments" => {
            now_arg(args, 0).and_then(|now| joined_tournaments(host, db, now))
        }
        "store.library.rememberTournament" => {
            let entry = args.first().cloned().unwrap_or(Value::Null);
            now_arg(args, 1).and_then(|now| remember_tournament(host, db, &entry, now))
        }
        "store.library.forgetTournament" => str_arg(args, 0, "system").and_then(|system| {
            str_arg(args, 1, "id")
                .and_then(|id| forget_tournament(host, db, system, id, opt_str(args, 2)))
        }),
        "store.library.forgetTournamentsOf" => forget_tournaments_of(host, db, args.first()),
        "store.library.recordRepertoireMiss" => str_arg(args, 0, "key").and_then(|key| {
            str_arg(args, 1, "fen").and_then(|fen| record_repertoire_miss(host, db, key, fen))
        }),
        "store.library.clearRepertoireMisses" => {
            str_arg(args, 0, "key").and_then(|key| clear_repertoire_misses(host, db, key))
        }
        _ => return None,
    };
    Some(result)
}

/* ── Shared helpers (also used by the other storage modules) ── */

/// `Date.now()`, as whole milliseconds.
pub fn now_ms() -> Result<f64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| CoreError::new(e.to_string()))?;
    Ok(elapsed.as_millis() as f64)
}

/// An optional `now` argument: absent or null means `Date.now()`.
pub fn now_arg(args: &[Value], i: usize) -> Result<f64> {
    match opt_number(args, i) {
        Some(now) => Ok(now),
        None => now_ms(),
    }
}

/// A rusqlite error as the core's message.
pub fn db_err(error: rusqlite::Error) -> CoreError {
    CoreError::new(error.to_string())
}

/// A JSON number: whole numbers without a fraction, as `JSON.stringify` writes them.
pub fn num(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        json!(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

/// `crypto.randomUUID()`: a version 4 UUID.
pub fn random_uuid() -> Result<String> {
    let mut bytes = [0u8; 16];
    fill(&mut bytes).map_err(|e| CoreError::new(e.to_string()))?;
    Ok(uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .hyphenated()
        .to_string())
}

/// `BEGIN IMMEDIATE` … `COMMIT`, rolled back on any error (`db.exec` in the TypeScript).
pub fn transaction<T>(db: &Connection, work: impl FnOnce() -> Result<T>) -> Result<T> {
    db.execute_batch("BEGIN IMMEDIATE").map_err(db_err)?;
    match work().and_then(|value| db.execute_batch("COMMIT").map_err(db_err).map(|()| value)) {
        Ok(value) => Ok(value),
        // The original error is the one the caller needs; a failed rollback is appended to it.
        Err(error) => Err(match db.execute_batch("ROLLBACK") {
            Ok(()) => error,
            Err(rollback) => error.with_rollback_failure(rollback),
        }),
    }
}

/// An optional numeric argument: absent or null is None.
pub fn opt_number(args: &[Value], i: usize) -> Option<f64> {
    args.get(i).and_then(Value::as_f64)
}

/// An optional string argument: absent or null is None.
pub fn opt_str(args: &[Value], i: usize) -> Option<&str> {
    args.get(i).and_then(Value::as_str)
}

/// A required string argument.
pub fn str_arg<'a>(args: &'a [Value], i: usize, name: &str) -> Result<&'a str> {
    opt_str(args, i).ok_or_else(|| CoreError::new(format!("{name} must be a string")))
}

/// `Boolean(v)` for an optional JSON value; absent is false.
pub fn truthy(value: Option<&Value>) -> bool {
    !js::falsy(value)
}

/// `array.slice(-n)`: the last `n` items.
pub fn keep_last<T>(mut items: Vec<T>, n: usize) -> Vec<T> {
    if items.len() > n {
        items.drain(..items.len() - n);
    }
    items
}

/// The strings of a JSON array; anything else is empty.
pub fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// A JSON object, or an error naming what was expected.
pub fn object<'a>(value: &'a Value, name: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| CoreError::new(format!("{name} must be an object")))
}

/// A `kchess_domain` document decoder or encoder by its TypeScript name. Values are JSON, so
/// the lossless sidecar the TypeScript wrappers send is empty.
pub fn misc(method: &str, args: Vec<Value>) -> Result<Value> {
    let mut full = vec![json!([])];
    full.extend(args);
    match kchess_domain::misc::call(method, &full) {
        Some(result) => result.map_err(CoreError::new),
        None => Err(CoreError::new(format!("Unknown core method {method}."))),
    }
}

/* ── Split modules: the codec, documents, studies and the legacy import live beside this
module now; their bodies moved unchanged. The re-exports keep every
`crate::store::library::X` path working. ── */

pub use super::documents::read_document;
pub(crate) use super::documents::write_document;
pub use super::json_codec::{clean_name, js_json, js_slice, utf16_len};
pub use super::library_import::library;
pub(crate) use super::library_import::{
    import_library, read_joined, read_misses, read_mistakes, read_session, until_after,
};
pub(crate) use super::studies::{check_based_on, id_of, spread, study_command};

/* ── Played games: one row each, so a move rewrites only its own game ── */

/// The size the archive had as one document, so its limit stays what it was.
fn archive_overhead() -> i64 {
    utf16_len(&js_json(&json!({ "version": 1, "games": [] }))) as i64
}

/// Add or replace a game; it becomes the most recent.
pub(crate) fn write_game(db: &Connection, game: &Value) -> Result<()> {
    let id = game.get("id").and_then(Value::as_str).unwrap_or_default();
    let body = js_json(game);
    let (count, bytes): (i64, i64) = db
        .query_row(
            "SELECT COUNT(*) AS count, COALESCE(SUM(length(body)), 0) AS bytes
             FROM archived_games WHERE id != ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(db_err)?;
    // Never silently discard older games to make room for a new one.
    let size = archive_overhead() + bytes + count + utf16_len(&body) as i64;
    if count + 1 > MAX_ARCHIVED_GAMES as i64 || size > MAX_DOCUMENT as i64 {
        return Err(CoreError::new(GAMES_FULL));
    }
    db.execute(
        "INSERT INTO archived_games (id, body, seq)
         VALUES (?1, ?2, (SELECT COALESCE(MAX(seq), 0) + 1 FROM archived_games))
         ON CONFLICT(id) DO UPDATE SET body = excluded.body, seq = excluded.seq",
        params![id, body],
    )
    .map_err(db_err)?;
    Ok(())
}

/* ── Mistake drills ── */

fn write_mistakes(db: &Connection, items: Vec<Value>) -> Result<()> {
    write_document(
        db,
        MISTAKES,
        &json!({ "version": 1, "items": items }),
        "Too many mistakes are saved to keep more.",
    )
    .map(|_| ())
}

/// `addMistakes(reviewKey, color, now)`: drills from a completed review's mistakes.
fn add_mistakes(
    host: &dyn Host,
    db: &Connection,
    review_key: &str,
    color: Option<&str>,
    now: f64,
) -> Result<Value> {
    let mut items = read_mistakes(db)?;
    let review = match super::reviews::read_review(host, db, review_key)? {
        Some(review) if truthy(review.get("complete")) => review,
        _ => return Ok(json!({ "added": 0, "items": items })),
    };
    let fen = review
        .get("fen")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let moves = string_list(review.get("moves"));
    let positions = kchess_domain::replay::replay_positions(&fen, &moves);
    let analysis = super::reviews::analyse_stored(&review)?;
    let source = review
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let stored_key = review
        .get("key")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let evals = review.get("evals").and_then(Value::as_array);
    let mut added = 0;
    let judged = analysis
        .get("moves")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (index, mv) in judged.iter().enumerate() {
        let Some(judgment) = mv
            .get("judgment")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if color.is_some_and(|wanted| mv.get("color").and_then(Value::as_str) != Some(wanted)) {
            continue;
        }
        let evaluation = evals
            .and_then(|list| list.get(index))
            .filter(|e| e.is_object());
        let best = evaluation
            .and_then(|e| e.get("best"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        let position = positions.get(index).map(|p| p.fen.clone());
        let (Some(best), Some(evaluation), Some(position)) = (best, evaluation, position) else {
            continue;
        };
        let depth = match evaluation.get("depth") {
            None | Some(Value::Null) => 0.0,
            other => js::to_number(other),
        };
        if moves.get(index).map(String::as_str) == Some(best) || (source == "local" && depth < 10.0)
        {
            continue;
        }
        let pv = evaluation.get("pv").and_then(Value::as_array);
        let mut solution: Vec<String> = match pv {
            Some(pv) if pv.first().and_then(Value::as_str) == Some(best) => {
                string_list(Some(&Value::Array(pv.iter().take(7).cloned().collect())))
            }
            _ => vec![best.to_string()],
        };
        let playable = kchess_domain::replay::replay_positions(&position, &solution).len();
        solution.truncate(playable.saturating_sub(1));
        if solution.len().is_multiple_of(2) {
            solution.pop();
        }
        if solution.is_empty() {
            continue;
        }
        let id = format!("{stored_key}:{index}");
        if items.iter().any(|item| id_of(item) == Some(id.as_str())) {
            continue;
        }
        items.push(json!({
            "id": id,
            "fen": position,
            "solution": solution,
            "judgment": judgment,
            "dueAt": num(now),
            "streak": 0,
            "attempts": 0,
        }));
        added += 1;
    }
    let items = keep_last(items, MAX_MISTAKES);
    if added > 0 {
        write_mistakes(db, items.clone())?;
    }
    Ok(json!({ "added": added, "items": items }))
}

/// The review key of a line: that of its legal moves from its normalized start, as a review of
/// it is stored. None when no move replays.
fn line_review_key(fen: &str, moves: &[String]) -> Option<String> {
    let positions = kchess_domain::replay::replay_positions(fen, moves);
    let start = positions.first()?;
    if positions.len() < 2 {
        return None;
    }
    let args = json!([start.fen, moves[..positions.len() - 1]]).to_string();
    let key: Value =
        serde_json::from_str(&kchess_domain::api::call("reviewKey", &args).ok()?).ok()?;
    key.as_str().map(str::to_string)
}

/// `mistakeGames()`: each review with positions in practice, most recently played first, with
/// its game when that is known: a synced Lichess game through its review link, or a game played
/// here whose line it reviews (your side there is the one named "You").
fn mistake_games(host: &dyn Host, db: &Connection) -> Result<Value> {
    let mut keys: Vec<String> = Vec::new();
    for item in read_mistakes(db)? {
        let Some((key, _)) = id_of(&item).and_then(|id| id.rsplit_once(':')) else {
            continue;
        };
        if !keys.iter().any(|known| known == key) {
            keys.push(key.to_string());
        }
    }
    if keys.is_empty() {
        return Ok(json!([]));
    }
    let mut played = Map::new();
    for game in super::library_import::read_games(host, db)? {
        let fen = game
            .pointer("/setup/fen")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some(key) = line_review_key(fen, &string_list(game.get("moves"))) {
            played.entry(key).or_insert(game);
        }
    }
    let mut lichess = db
        .prepare(
            "SELECT g.id, g.account, g.color, g.opponent, g.createdAt
             FROM game_reviews gr JOIN games g ON g.id = gr.gameId
             WHERE gr.reviewKey = ?1 ORDER BY g.createdAt DESC LIMIT 1",
        )
        .map_err(db_err)?;
    let mut games = Vec::new();
    for key in keys {
        let Some(review) = super::reviews::read_review(host, db, &key)? else {
            continue;
        };
        let mut entry = Map::new();
        entry.insert("key".into(), json!(key));
        entry.insert(
            "fen".into(),
            review.get("fen").cloned().unwrap_or(json!("")),
        );
        entry.insert("moves".into(), json!(string_list(review.get("moves"))));
        let mut at = js::to_number(review.get("updatedAt"));
        let synced = lichess
            .query_row(params![key], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .optional()
            .map_err(db_err)?;
        if let Some((id, account, color, opponent, created)) = synced {
            let (white, black) = if color == "white" {
                (&account, &opponent)
            } else {
                (&opponent, &account)
            };
            entry.insert("white".into(), json!(white));
            entry.insert("black".into(), json!(black));
            entry.insert("player".into(), json!(color));
            entry.insert("gameId".into(), json!(id));
            entry.insert("account".into(), json!(account));
            #[allow(clippy::cast_precision_loss)]
            {
                at = created as f64;
            }
        } else if let Some(game) = played.get(&key) {
            let white = game
                .get("white")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let black = game
                .get("black")
                .and_then(Value::as_str)
                .unwrap_or_default();
            entry.insert("white".into(), json!(white));
            entry.insert("black".into(), json!(black));
            if (white == "You") != (black == "You") {
                entry.insert(
                    "player".into(),
                    json!(if white == "You" { "white" } else { "black" }),
                );
            }
            at = js::to_number(game.get("startedAt"));
        }
        entry.insert("playedAt".into(), num(at));
        games.push((at, Value::Object(entry)));
    }
    games.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(Value::Array(
        games.into_iter().map(|(_, game)| game).collect(),
    ))
}

/// `answerMistake(id, solved, now)`: schedule a drill again.
fn answer_mistake(db: &Connection, id: &str, solved: bool, now: f64) -> Result<Value> {
    let mut items = read_mistakes(db)?;
    let Some(index) = items.iter().position(|entry| id_of(entry) == Some(id)) else {
        return Ok(Value::Array(items));
    };
    let item = object(&items[index], "mistake")?.clone();
    let attempts = item
        .get("attempts")
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN)
        + 1.0;
    let streak = if solved {
        item.get("streak")
            .and_then(Value::as_f64)
            .unwrap_or(f64::NAN)
            + 1.0
    } else {
        0.0
    };
    let wait = if solved {
        2f64.powf(streak - 1.0).min(30.0) * 86_400_000.0
    } else {
        10.0 * 60_000.0
    };
    items[index] = Value::Object(spread(
        &item,
        vec![
            ("attempts", num(attempts)),
            ("streak", num(streak)),
            ("dueAt", num(now + wait)),
        ],
    ));
    write_mistakes(db, items.clone())?;
    Ok(Value::Array(items))
}

/* ── Tournaments joined from KChess ── */

fn write_joined(db: &Connection, entries: Vec<Value>) -> Result<()> {
    write_document(
        db,
        JOINED,
        &Value::Array(entries),
        "Too many joined tournaments.",
    )
    .map(|_| ())
}

fn joined_tournaments(host: &dyn Host, db: &Connection, now: f64) -> Result<Value> {
    Ok(Value::Array(
        read_joined(host, db)?
            .into_iter()
            .filter(|entry| until_after(entry, now))
            .collect(),
    ))
}

/// A membership's account, compared without case.
fn joined_account(entry: &Value) -> String {
    entry
        .get("account")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase()
}

/// Memberships belong to an account: the same event joined by another account is another entry.
fn remember_tournament(host: &dyn Host, db: &Connection, entry: &Value, now: f64) -> Result<Value> {
    let account = joined_account(entry);
    let mut entries: Vec<Value> = read_joined(host, db)?
        .into_iter()
        .filter(|e| {
            until_after(e, now)
                && !(e.get("system") == entry.get("system")
                    && e.get("id") == entry.get("id")
                    && joined_account(e) == account)
        })
        .collect();
    entries.push(entry.clone());
    write_joined(db, entries)?;
    Ok(Value::Null)
}

/// Forgets one account's membership, or every account's when `account` is `None` (the event ended).
fn forget_tournament(
    host: &dyn Host,
    db: &Connection,
    system: &str,
    id: &str,
    account: Option<&str>,
) -> Result<Value> {
    let account = account.map(str::to_lowercase);
    let kept: Vec<Value> = read_joined(host, db)?
        .into_iter()
        .filter(|e| {
            !(e.get("system").and_then(Value::as_str) == Some(system)
                && e.get("id").and_then(Value::as_str) == Some(id)
                && account
                    .as_ref()
                    .is_none_or(|name| joined_account(e) == *name))
        })
        .collect();
    write_joined(db, kept)?;
    Ok(Value::Null)
}

/// Signed-out accounts must not reopen event streams for their tournaments.
fn forget_tournaments_of(
    host: &dyn Host,
    db: &Connection,
    accounts: Option<&Value>,
) -> Result<Value> {
    let names: Vec<String> = string_list(accounts)
        .iter()
        .map(|name| name.to_lowercase())
        .collect();
    let entries = read_joined(host, db)?;
    let total = entries.len();
    let kept: Vec<Value> = entries
        .into_iter()
        .filter(|e| {
            let account = e
                .get("account")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_lowercase();
            !names.contains(&account)
        })
        .collect();
    if kept.len() != total {
        let removed = total - kept.len();
        write_joined(db, kept)?;
        host.log(
            Level::Debug,
            "library",
            &format!("Forgot joined tournaments of signed-out accounts: count={removed}"),
        );
    }
    Ok(Value::Null)
}

/* ── Repertoire training ── */

fn record_repertoire_miss(host: &dyn Host, db: &Connection, key: &str, fen: &str) -> Result<Value> {
    let mut misses = object(&read_misses(host, db)?, "repertoire misses")?.clone();
    let table = misses
        .entry(key.to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| CoreError::new("repertoire table must be an object"))?;
    let count = table.get(fen).and_then(Value::as_f64).unwrap_or(0.0) + 1.0;
    table.insert(fen.to_string(), num(count));
    let misses = Value::Object(misses);
    write_document(db, MISSES, &misses, "Too many repertoire notes are saved.")?;
    Ok(misses)
}

fn clear_repertoire_misses(host: &dyn Host, db: &Connection, key: &str) -> Result<Value> {
    let mut misses = object(&read_misses(host, db)?, "repertoire misses")?.clone();
    misses.shift_remove(key);
    let misses = Value::Object(misses);
    write_document(db, MISSES, &misses, "Too many repertoire notes are saved.")?;
    Ok(misses)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::documents::{DAMAGED_STUDIES, read_document_text};
    use crate::store::library_import::read_studies;

    #[test]
    fn failed_rollback_is_reported_with_the_original_error() {
        // The work ends the transaction itself, so the transaction's own ROLLBACK fails. That
        // failure used to go to a `tracing` macro with no subscriber and was lost.
        let db = Connection::open_in_memory().expect("open");
        let result: Result<()> = transaction(&db, || {
            db.execute_batch("ROLLBACK").map_err(db_err)?;
            Err(CoreError::new("work failed"))
        });
        let error = result.expect_err("work failed");
        assert!(
            error.message.starts_with("work failed; rollback failed: "),
            "{}",
            error.message
        );
        assert!(!error.aborted);
    }

    #[test]
    fn stored_text_matches_javascript_stringify() {
        let value = json!({ "b": 1.0, "a": [0.5, null, true, "é\n"], "n": -2 });
        assert_eq!(
            js_json(&value),
            r#"{"b":1,"a":[0.5,null,true,"é\n"],"n":-2}"#
        );
        assert_eq!(num(3.0), json!(3));
        assert_eq!(num(0.25), json!(0.25));
    }

    #[test]
    fn lengths_and_slices_count_utf16_units() {
        assert_eq!(utf16_len("a😀"), 3);
        // The emoji takes two units, so a cut at three keeps only the letters before it.
        assert_eq!(js_slice("ab😀cd", 3), "ab");
        assert_eq!(js_slice("ab😀cd", 4), "ab😀");
        assert_eq!(clean_name("  Name  "), "Name");
        assert_eq!(clean_name(&"x".repeat(200)).len(), 120);
    }

    #[test]
    fn uuids_are_version_four() {
        let id = random_uuid().expect("random bytes");
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(id, random_uuid().expect("random bytes"));
    }

    #[test]
    fn keeps_the_newest_items() {
        assert_eq!(keep_last(vec![1, 2, 3], 2), vec![2, 3]);
        assert_eq!(keep_last(vec![1], 5), vec![1]);
    }

    /// A host that records nothing: the store's logs are not under test here.
    struct Quiet;

    impl Host for Quiet {
        fn log(&self, _: Level, _: &str, _: &str) {}
        fn emit(&self, _: &str, _: Value) {}
    }

    /// An arena that has ended is dropped when the next one is remembered, and
    /// forgetting an account drops its tournaments whatever the case of its name.
    #[test]
    fn ended_tournaments_and_those_of_signed_out_accounts_are_forgotten() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        crate::store::migrations::migrate(&db).unwrap();
        let config = crate::host::Config::new(std::path::PathBuf::new());
        let cache = crate::store::StoreCache::default();
        let ctx = crate::store::StoreContext {
            db: &db,
            host: &Quiet,
            config: &config,
            cache: &cache,
        };
        let now = 1_700_000_000_000.0;
        let entry = |system: &str, id: &str, account: &str, name: &str, until: f64| json!({ "system": system, "id": id, "account": account, "name": name, "until": until });
        let remember = |item: Value| {
            crate::store::call(
                &ctx,
                "store.library.rememberTournament",
                &[item, json!(now)],
            )
            .unwrap()
        };
        remember(entry("arena", "ended", "Alice", "Old", now - 1.0));
        remember(entry("arena", "live", "Alice", "Live", now + 60_000.0));
        remember(entry("swiss", "other", "Bob", "Swiss", now + 60_000.0));
        let ids = |value: Value| -> Vec<String> {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["id"].as_str().unwrap().to_string())
                .collect()
        };
        let joined = |ctx: &crate::store::StoreContext| {
            crate::store::call(ctx, "store.library.joinedTournaments", &[json!(now)]).unwrap()
        };
        assert_eq!(ids(joined(&ctx)), ["live", "other"]);
        crate::store::call(
            &ctx,
            "store.library.forgetTournamentsOf",
            &[json!(["alice"])],
        )
        .unwrap();
        assert_eq!(ids(joined(&ctx)), ["other"]);
    }

    // Viewing an arena as Bob (not in it) forgot Alice's membership, and Bob joining the same
    // event replaced hers: memberships were keyed without the account.
    #[test]
    fn tournament_memberships_belong_to_their_account() {
        let db = Connection::open_in_memory().expect("database");
        super::super::migrations::migrate(&db).expect("migrate");
        let config = crate::host::Config::new(std::path::PathBuf::new());
        let cache = crate::store::StoreCache::default();
        let ctx = crate::store::StoreContext {
            db: &db,
            host: &Quiet,
            config: &config,
            cache: &cache,
        };
        let now = 1_700_000_000_000.0;
        let call = |method: &str, args: &[Value]| crate::store::call(&ctx, method, args).unwrap();
        let entry = |account: &str| json!({ "system": "arena", "id": "live", "account": account, "name": "Live", "until": now + 60_000.0 });
        let accounts = || -> Vec<String> {
            call("store.library.joinedTournaments", &[json!(now)])
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["account"].as_str().unwrap().to_string())
                .collect()
        };
        call(
            "store.library.rememberTournament",
            &[entry("Alice"), json!(now)],
        );
        call(
            "store.library.rememberTournament",
            &[entry("Bob"), json!(now)],
        );
        call(
            "store.library.rememberTournament",
            &[entry("bob"), json!(now)],
        );
        assert_eq!(accounts(), ["Alice", "bob"]);
        call(
            "store.library.forgetTournament",
            &[json!("arena"), json!("live"), json!("BOB")],
        );
        assert_eq!(accounts(), ["Alice"]);
        call(
            "store.library.forgetTournament",
            &[json!("arena"), json!("live"), json!("Carol")],
        );
        assert_eq!(accounts(), ["Alice"]);
        call(
            "store.library.rememberTournament",
            &[entry("Bob"), json!(now)],
        );
        // The event ending forgets every account's place.
        call(
            "store.library.forgetTournament",
            &[json!("arena"), json!("live")],
        );
        assert!(accounts().is_empty());
    }

    fn study_db() -> Connection {
        let db = Connection::open_in_memory().expect("database");
        super::super::migrations::migrate(&db).expect("migrate");
        db
    }

    #[test]
    fn damaged_study_documents_are_preserved_and_cannot_be_overwritten() {
        for body in [
            "{broken",
            r#"{"version":2,"items":"nope"}"#,
            r#"{"version":99,"items":[]}"#,
            r#"{"version":2,"items":[{"id":"bad","name":"Bad","pgn":"1. e5 *","updatedAt":1}]}"#,
        ] {
            let db = study_db();
            db.execute("INSERT INTO documents VALUES ('studies', ?1, 1)", [body])
                .expect("seed");
            assert!(read_studies(&db).is_err());
            let session = misc("encodeSession", vec![json!("analysis"), json!({"pgn":"1. d4 *","path":"","orientation":"white","study":"","chapter":""})]).expect("session");
            write_document(&db, "session:analysis", &session, "").expect("save session");
            let snapshot = library(&Quiet, &db, 1.0).expect("other documents remain readable");
            assert_eq!(snapshot["studyError"], json!(DAMAGED_STUDIES));
            assert_eq!(snapshot["sessions"]["analysis"]["pgn"], json!("1. d4 *"));
            assert!(
                study_command(
                    &db,
                    &json!({ "op":"save", "name":"New", "pgn":"1. e4 *" }),
                    1.0,
                    true
                )
                .is_err()
            );
            assert_eq!(
                read_document_text(&db, STUDIES).expect("read").as_deref(),
                Some(body)
            );
            let count: i64 = db
                .query_row("SELECT COUNT(*) FROM studies", [], |row| row.get(0))
                .expect("count");
            assert_eq!(count, 0);
        }
    }

    #[test]
    fn study_migration_preserves_chapters_and_rolls_back_failed_inserts() {
        let db = study_db();
        let body = js_json(
            &json!({"version":2,"items":[{"id":"s","name":"Study","pgn":"1. e4 *","updatedAt":1,
            "chapters":[{"id":"c","name":"Chapter","pgn":"1. e4 {Keep} e5 (1... c5 $1) *"}]}]}),
        );
        db.execute("INSERT INTO documents VALUES ('studies', ?1, 1)", [&body])
            .expect("seed");
        db.execute_batch("CREATE TRIGGER refuse_study BEFORE INSERT ON studies BEGIN SELECT RAISE(ABORT, 'disk failure'); END;").expect("trigger");
        assert!(read_studies(&db).is_err());
        assert_eq!(read_document_text(&db, STUDIES).expect("read"), Some(body));
        db.execute_batch("DROP TRIGGER refuse_study").expect("drop");
        let studies = read_studies(&db).expect("migrate");
        assert!(
            studies[0]["chapters"][0]["pgn"]
                .as_str()
                .expect("pgn")
                .contains("c5 $1")
        );
        assert_eq!(read_studies(&db).expect("repeat"), studies);
    }

    /// Two windows edit one study (`docs/shared-core.md`): the second save, based on the chapter
    /// as it was before the first, is refused instead of overwriting it. Saved in the same
    /// millisecond or after a rename, the comparison still holds: it is of the chapter's moves.
    #[test]
    fn a_save_based_on_an_older_chapter_is_refused() {
        let db = study_db();
        let save = |pgn: &str| json!({"op":"save", "id":"a", "name":"A", "pgn":pgn});
        study_command(&db, &save("1. e4 *"), 1.0, true).expect("create");
        // A study removed since the chapter was read is not created again under its old id.
        let removed = check_based_on(
            &db,
            &json!({"op":"save", "id":"removed", "name":"N"}),
            "1. e4 *",
        )
        .expect_err("removed");
        assert_eq!(removed.message, super::super::studies::STALE_STUDY);
        check_based_on(&db, &save("1. d4 *"), "1. e4 *").expect("based on the current chapter");
        study_command(&db, &save("1. d4 *"), 1.0, true).expect("first window saves");
        let stale = check_based_on(&db, &save("1. e4 e5 *"), "1. e4 *").expect_err("second window");
        assert_eq!(stale.message, super::super::studies::STALE_STUDY);
        check_based_on(&db, &save("1. c4 *"), "1. d4 *").expect("after reloading");
        study_command(
            &db,
            &json!({"op":"rename", "id":"a", "name":"B"}),
            2.0,
            true,
        )
        .expect("rename");
        check_based_on(&db, &save("1. c4 *"), "1. d4 *").expect("a rename changes no moves");
    }

    /// A content save from one window, based on the chapter it shows, after another window
    /// renamed the study or removed it: the rename stays, and the removed study is not recreated
    /// with a new chapter id that the open board can never save to.
    #[test]
    fn a_based_on_chapter_save_keeps_another_windows_rename_and_removal() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        crate::store::migrations::migrate(&db).unwrap();
        let config = crate::host::Config::new(std::path::PathBuf::new());
        let cache = crate::store::StoreCache::default();
        let ctx = crate::store::StoreContext {
            db: &db,
            host: &Quiet,
            config: &config,
            cache: &cache,
        };
        let call = |args: &[Value]| crate::store::call(&ctx, "store.library.studyCommand", args);
        let created = call(&[
            json!({"op":"save", "name":"Original", "pgn":"1. e4 *"}),
            json!(1.0),
            json!(true),
        ])
        .unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        let chapter = created["studies"][0]["chapters"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        call(&[
            json!({"op":"rename", "id":id, "name":"Remote rename"}),
            json!(2.0),
            json!(true),
        ])
        .unwrap();
        let save = |pgn: &str, based_on: &str| {
            call(&[
                json!({"op":"save", "name":"Original", "pgn":pgn, "id":id, "chapterId":chapter}),
                json!(3.0),
                json!(true),
                json!(based_on),
            ])
        };
        save("1. e4 e5 *", "1. e4 *").expect("autosave");
        let studies = read_studies(&db).unwrap();
        assert_eq!(studies[0]["name"], "Remote rename");
        assert_eq!(studies[0]["chapters"][0]["pgn"], "1. e4 e5 *");
        call(&[json!({"op":"remove", "id":id}), json!(4.0), json!(true)]).unwrap();
        let removed = save("1. e4 e5 2. Nf3 *", "1. e4 e5 *").expect_err("removed");
        assert_eq!(removed.message, super::super::studies::STALE_STUDY);
        assert!(read_studies(&db).unwrap().is_empty());
    }

    #[test]
    fn incremental_study_edits_write_and_return_only_the_changed_document() {
        let db = study_db();
        for id in ["a", "b", "c"] {
            study_command(
                &db,
                &json!({"op":"save", "id":id, "name":id, "pgn":"1. e4 *"}),
                1.0,
                true,
            )
            .expect("save");
        }
        db.execute_batch("CREATE TRIGGER unchanged_body BEFORE UPDATE OF body ON studies WHEN OLD.id != 'a' BEGIN SELECT RAISE(ABORT, 'unrelated study rewritten'); END;").expect("trigger");
        let result = study_command(
            &db,
            &json!({"op":"save", "id":"a", "name":"A", "pgn":"1. d4 {Notes} d5 (1... Nf6 $1) *"}),
            2.0,
            true,
        )
        .expect("edit");
        assert_eq!(result["order"], json!(["a", "c", "b"]));
        assert_eq!(result["studies"].as_array().expect("studies").len(), 1);
        assert_eq!(result["studies"][0]["id"], json!("a"));
        assert_eq!(read_studies(&db).expect("snapshot").len(), 3);
        let removed =
            study_command(&db, &json!({"op":"remove", "id":"a"}), 3.0, true).expect("remove");
        assert_eq!(removed["order"], json!(["c", "b"]));
        assert_eq!(removed["studies"], json!([]));
        assert_eq!(removed["removed"]["id"], json!("a"));
        let restored = study_command(
            &db,
            &json!({"op":"restore", "study":removed["removed"]}),
            4.0,
            true,
        )
        .expect("restore");
        assert_eq!(restored["order"], json!(["a", "c", "b"]));
    }

    #[test]
    fn failed_study_updates_keep_the_previous_body_and_order() {
        let db = study_db();
        study_command(
            &db,
            &json!({"op":"save","id":"a","name":"A","pgn":"1. e4 *"}),
            1.0,
            true,
        )
        .expect("save");
        let before = read_studies(&db).expect("snapshot");
        db.execute_batch("CREATE TRIGGER refuse_study BEFORE UPDATE ON studies BEGIN SELECT RAISE(ABORT, 'disk failure'); END;").expect("trigger");
        assert!(
            study_command(
                &db,
                &json!({"op":"rename","id":"a","name":"New"}),
                2.0,
                true
            )
            .is_err()
        );
        assert_eq!(read_studies(&db).expect("snapshot"), before);
    }

    #[test]
    fn spreads_keep_key_order_and_add_new_keys_last() {
        let base = json!({ "id": "s", "name": "old", "pgn": "p" });
        let merged = spread(
            base.as_object().expect("object"),
            vec![
                ("pgn", json!("q")),
                ("updatedAt", json!(1)),
                ("name", json!("new")),
            ],
        );
        assert_eq!(
            js_json(&Value::Object(merged)),
            r#"{"id":"s","name":"new","pgn":"q","updatedAt":1}"#
        );
    }
}
