//! The local library (`core/src/services/library.ts`): studies, played games, mistake drills,
//! unfinished sessions, joined tournaments and repertoire notes, each a bounded, versioned
//! document in `kchess.db`. The decoders and semantic validation are `kchess_domain`'s.
//!
//! This module also holds the helpers the other storage modules share: `JSON.stringify`
//! compatible text (stored documents must match the TypeScript bytes), `Date.now()`, UUIDs,
//! UTF-16 string lengths and slicing, and the `BEGIN IMMEDIATE` wrapper.

use getrandom::getrandom;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{CoreError, Result};
use kchess_domain::js;

pub const MAX_DOCUMENT: usize = 2_000_000;
const MAX_STUDIES: usize = 50;
const MAX_CHAPTERS: usize = 64;
const MAX_ARCHIVED_GAMES: i64 = 500;
const MAX_MISTAKES: usize = 500;
const STUDY_LIBRARY_MAX_TEXT: usize = 1_800_000;

const STUDIES: &str = "studies";
const GAMES: &str = "games";
const MISTAKES: &str = "mistakes";
const JOINED: &str = "tournaments:joined";
const MISSES: &str = "repertoire:misses";
const IMPORTED: &str = "library:imported";

const SESSION_KINDS: [&str; 6] = [
    "analysis",
    "local",
    "computer",
    "archive:computer",
    "archive:board",
    "archive:clock",
];

/// `kchess:<legacy key>` → library target, in the order the import takes them.
const LEGACY_TARGETS: [(&str, &str); 11] = [
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
pub fn call(db: &Connection, method: &str, args: &[Value]) -> Option<Result<Value>> {
    let result = match method {
        "store.library.library" => now_arg(args, 0).and_then(|now| library(db, now)),
        "store.library.readSession" => {
            str_arg(args, 0, "kind").and_then(|kind| read_session(db, kind))
        }
        "store.library.importLibrary" => import_library(db, args.first()),
        "store.library.studyCommand" => now_arg(args, 1)
            .and_then(|now| study_command(db, args.first().unwrap_or(&Value::Null), now)),
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
            now_arg(args, 2).and_then(|now| add_mistakes(db, key, color, now))
        }),
        "store.library.answerMistake" => str_arg(args, 0, "id").and_then(|id| {
            let solved = truthy(args.get(1));
            now_arg(args, 2).and_then(|now| answer_mistake(db, id, solved, now))
        }),
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
            now_arg(args, 0).and_then(|now| joined_tournaments(db, now))
        }
        "store.library.rememberTournament" => {
            let entry = args.first().cloned().unwrap_or(Value::Null);
            now_arg(args, 1).and_then(|now| remember_tournament(db, &entry, now))
        }
        "store.library.forgetTournament" => str_arg(args, 0, "system").and_then(|system| {
            str_arg(args, 1, "id").and_then(|id| forget_tournament(db, system, id))
        }),
        "store.library.forgetTournamentsOf" => forget_tournaments_of(db, args.first()),
        "store.library.recordRepertoireMiss" => str_arg(args, 0, "key").and_then(|key| {
            str_arg(args, 1, "fen").and_then(|fen| record_repertoire_miss(db, key, fen))
        }),
        "store.library.clearRepertoireMisses" => {
            str_arg(args, 0, "key").and_then(|key| clear_repertoire_misses(db, key))
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

/// `JSON.stringify(value)`: the same text for the values the core stores (keys in order, whole
/// numbers without `.0`, no spaces).
pub fn js_json(value: &Value) -> String {
    let mut out = String::new();
    write_js_json(value, &mut out);
    out
}

fn write_js_json(value: &Value, out: &mut String) {
    match value {
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_js_json(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (key, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                write_js_json(item, out);
            }
            out.push('}');
        }
        Value::Number(n) => match n.as_f64() {
            Some(x) if n.is_f64() && x.is_finite() && x.fract() == 0.0 && x.abs() < 9.0e15 => {
                out.push_str(&(x as i64).to_string());
            }
            _ => out.push_str(&n.to_string()),
        },
        other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
    }
}

/// `s.length`: UTF-16 code units.
pub fn utf16_len(s: &str) -> usize {
    js::utf16_len(s)
}

/// `s.slice(0, max)` in UTF-16 units. A cut inside a surrogate pair drops the whole character
/// (JavaScript would keep a lone surrogate, which a Rust string cannot hold).
pub fn js_slice(s: &str, max: usize) -> String {
    let mut units = 0;
    let mut end = 0;
    for (index, c) in s.char_indices() {
        let next = units + c.len_utf16();
        if next > max {
            break;
        }
        units = next;
        end = index + c.len_utf8();
    }
    s[..end].to_string()
}

/// `name.trim().slice(0, 120)`
pub fn clean_name(name: &str) -> String {
    js_slice(js::trim(name), 120)
}

/// `crypto.randomUUID()`: a version 4 UUID.
pub fn random_uuid() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom(&mut bytes).map_err(|e| CoreError::new(e.to_string()))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

/// `BEGIN IMMEDIATE` … `COMMIT`, rolled back on any error (`db.exec` in the TypeScript).
pub fn transaction<T>(db: &Connection, work: impl FnOnce() -> Result<T>) -> Result<T> {
    db.execute_batch("BEGIN IMMEDIATE").map_err(db_err)?;
    match work().and_then(|value| db.execute_batch("COMMIT").map_err(db_err).map(|()| value)) {
        Ok(value) => Ok(value),
        Err(error) => {
            // The original error is the one the caller needs; a failed rollback changes nothing.
            let _ = db.execute_batch("ROLLBACK");
            Err(error)
        }
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

/* ── Documents: one bounded JSON value per key ── */

fn read_document_text(db: &Connection, key: &str) -> Result<Option<String>> {
    db.query_row(
        "SELECT body FROM documents WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(db_err)
}

/// The parsed document; None when it is missing or not JSON (`readDocument` in the TypeScript).
pub fn read_document(db: &Connection, key: &str) -> Result<Option<Value>> {
    Ok(read_document_text(db, key)?.and_then(|body| serde_json::from_str(&body).ok()))
}

/// `writeDocument`: a document within its size limit, stored with its update time.
fn write_document(db: &Connection, key: &str, value: &Value, too_large: &str) -> Result<Value> {
    let body = js_json(value);
    if utf16_len(&body) > MAX_DOCUMENT {
        return Err(CoreError::new(too_large));
    }
    let now = now_ms()?;
    db.execute(
        "INSERT INTO documents (key, body, updatedAt) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET body = excluded.body, updatedAt = excluded.updatedAt",
        params![key, body, now as i64],
    )
    .map_err(db_err)?;
    Ok(Value::Null)
}

/// Whether the library already has this document (or, for games, any game).
fn stored(db: &Connection, key: &str) -> Result<bool> {
    if key == GAMES {
        let any = db
            .query_row("SELECT 1 FROM archived_games LIMIT 1", [], |_| Ok(()))
            .optional()
            .map_err(db_err)?;
        return Ok(any.is_some());
    }
    Ok(read_document(db, key)?.is_some())
}

fn parse_text(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok()
}

fn decode_studies_text(text: &str) -> Option<Vec<Value>> {
    parse_text(text).and_then(|raw| kchess_domain::library::decode_studies(&raw))
}

fn decode_mistakes_text(text: &str) -> Option<Vec<Value>> {
    parse_text(text).and_then(|raw| kchess_domain::library::decode_mistakes(&raw))
}

fn decode_archive_text(text: &str) -> Option<Vec<Value>> {
    parse_text(text).and_then(|raw| kchess_domain::library::decode_archive(&raw))
}

fn decode_archived_game_text(text: &str) -> Option<Value> {
    parse_text(text).and_then(|raw| kchess_domain::library::decode_archived_game(&raw))
}

fn read_studies(db: &Connection) -> Result<Vec<Value>> {
    Ok(read_document_text(db, STUDIES)?
        .and_then(|body| decode_studies_text(&body))
        .unwrap_or_default())
}

fn read_mistakes(db: &Connection) -> Result<Vec<Value>> {
    Ok(read_document_text(db, MISTAKES)?
        .and_then(|body| decode_mistakes_text(&body))
        .unwrap_or_default())
}

fn read_misses(db: &Connection) -> Result<Value> {
    let raw = read_document(db, MISSES)?.unwrap_or(Value::Null);
    misc("decodeRepertoireMisses", vec![raw])
}

fn read_joined(db: &Connection) -> Result<Vec<Value>> {
    let raw = read_document(db, JOINED)?.unwrap_or(Value::Null);
    match misc("decodeJoinedTournaments", vec![raw])? {
        Value::Array(entries) => Ok(entries),
        _ => Ok(Vec::new()),
    }
}

/// `readSession(kind)`: the decoded session, or null.
fn read_session(db: &Connection, kind: &str) -> Result<Value> {
    let raw = read_document(db, &format!("session:{kind}"))?.unwrap_or(Value::Null);
    misc("decodeStoredSession", vec![json!(kind), raw])
}

/// The played games, newest first; a game that no longer replays is skipped.
fn read_games(db: &Connection) -> Result<Vec<Value>> {
    let mut statement = db
        .prepare("SELECT body FROM archived_games ORDER BY seq DESC")
        .map_err(db_err)?;
    let bodies = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(db_err)?;
    let mut games = Vec::new();
    for body in bodies {
        if let Some(game) = decode_archived_game_text(&body.map_err(db_err)?) {
            games.push(game);
        }
    }
    Ok(games)
}

/// `library(now)`: the whole snapshot.
pub fn library(db: &Connection, now: f64) -> Result<Value> {
    let mut sessions = Map::new();
    for kind in SESSION_KINDS {
        let value = read_session(db, kind)?;
        if truthy(Some(&value)) {
            sessions.insert(kind.to_string(), value);
        }
    }
    let joined: Vec<Value> = read_joined(db)?
        .into_iter()
        .filter(|entry| until_after(entry, now))
        .collect();
    Ok(json!({
        "studies": read_studies(db)?,
        "games": read_games(db)?,
        "mistakes": read_mistakes(db)?,
        "sessions": Value::Object(sessions),
        "joinedTournaments": joined,
        "repertoireMisses": read_misses(db)?,
        "imported": read_document(db, IMPORTED)? == Some(Value::Bool(true)),
    }))
}

/// `entry.until > now`
fn until_after(entry: &Value, now: f64) -> bool {
    entry
        .get("until")
        .and_then(Value::as_f64)
        .is_some_and(|until| until > now)
}

/// `importLibrary(documents)`: take over documents an earlier release kept, once.
fn import_library(db: &Connection, documents: Option<&Value>) -> Result<Value> {
    if read_document(db, IMPORTED)? == Some(Value::Bool(true)) {
        return library(db, now_ms()?);
    }
    let documents = documents.and_then(Value::as_object);
    transaction(db, || {
        for (legacy, key) in LEGACY_TARGETS {
            let Some(text) = documents
                .and_then(|map| map.get(legacy))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if utf16_len(text) > MAX_DOCUMENT || stored(db, key)? {
                continue;
            }
            let Some(raw) = parse_text(text) else {
                continue;
            };
            let Some(value) = legacy_value(key, &raw, text)? else {
                continue;
            };
            if key == GAMES {
                // Oldest first, so the most recent game ends up most recent again.
                if let Value::Array(games) = &value {
                    for game in games.iter().rev() {
                        write_game(db, game)?;
                    }
                }
                continue;
            }
            write_document(db, key, &value, "A saved document is too large to import.")?;
        }
        write_document(db, IMPORTED, &Value::Bool(true), "")?;
        Ok(())
    })?;
    library(db, now_ms()?)
}

/// The value a legacy document becomes in the library, or None when it is invalid.
fn legacy_value(key: &str, raw: &Value, text: &str) -> Result<Option<Value>> {
    Ok(match key {
        STUDIES => decode_studies_text(text).map(|items| json!({ "version": 2, "items": items })),
        GAMES => decode_archive_text(text).map(Value::Array),
        MISTAKES => decode_mistakes_text(text).map(|items| json!({ "version": 1, "items": items })),
        JOINED => Some(misc("decodeJoinedTournaments", vec![raw.clone()])?),
        MISSES => Some(misc("decodeRepertoireMisses", vec![raw.clone()])?),
        _ => {
            let kind = key.strip_prefix("session:").unwrap_or_default();
            let session = misc("decodeStoredSession", vec![json!(kind), raw.clone()])?;
            if truthy(Some(&session)) {
                Some(misc("encodeSession", vec![json!(kind), session])?)
            } else {
                None
            }
        }
    })
}

/* ── Studies ── */

fn id_of(value: &Value) -> Option<&str> {
    value.get("id").and_then(Value::as_str)
}

/// `{ ...base, ...updates }`: keys keep their place; new keys follow in the order given.
fn spread(base: &Map<String, Value>, updates: Vec<(&str, Value)>) -> Map<String, Value> {
    let mut merged = base.clone();
    for (key, value) in updates {
        merged.insert(key.to_string(), value);
    }
    merged
}

/// The study list one command works on; `finish_studies` stores it.
struct Studies {
    items: Vec<Value>,
    now: f64,
}

impl Studies {
    fn find(&self, id: &str) -> Option<&Value> {
        self.items.iter().find(|item| id_of(item) == Some(id))
    }

    fn find_index(&self, id: &str) -> Option<usize> {
        self.items.iter().position(|item| id_of(item) == Some(id))
    }

    /// Put `candidate` first, replacing any study with its id; the list is unchanged on failure.
    fn commit(&mut self, candidate: Map<String, Value>) -> Result<String> {
        let id = candidate
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mut next = vec![Value::Object(candidate)];
        next.extend(
            self.items
                .iter()
                .filter(|item| id_of(item) != Some(id.as_str()))
                .cloned(),
        );
        if next.len() > MAX_STUDIES
            || utf16_len(&js_json(&Value::Array(next.clone()))) > STUDY_LIBRARY_MAX_TEXT
        {
            return Err(CoreError::new(
                "The study library is full. Export and remove a study before saving another.",
            ));
        }
        self.items = next;
        Ok(id)
    }

    /// `saveChapters(rawName, chapters, id?)`: a study from named PGN chapters.
    fn save_chapters(
        &mut self,
        raw_name: &str,
        chapters: &Value,
        id: Option<&str>,
    ) -> Result<String> {
        let name = clean_name(raw_name);
        let list = chapters.as_array().cloned().unwrap_or_default();
        let valid = !list.is_empty()
            && list.len() <= MAX_CHAPTERS
            && list.iter().all(|c| {
                c.get("pgn")
                    .and_then(Value::as_str)
                    .is_some_and(kchess_domain::tree::valid_pgn)
            });
        if name.is_empty() || !valid {
            return Err(CoreError::new(
                "Name the study and use up to 64 valid chapters.",
            ));
        }
        let old = id
            .filter(|id| !id.is_empty())
            .and_then(|id| self.find(id).cloned());
        let mut saved = Vec::with_capacity(list.len());
        for (index, chapter) in list.iter().enumerate() {
            let reused = old
                .as_ref()
                .and_then(|study| study.get("chapters"))
                .and_then(|chapters| chapters.get(index))
                .and_then(|c| c.get("id"))
                .filter(|v| !v.is_null())
                .cloned();
            let chapter_id = match reused {
                Some(value) => value,
                None => Value::String(random_uuid()?),
            };
            let chapter_name = chapter
                .get("name")
                .and_then(Value::as_str)
                .map(|n| js_slice(n, 120))
                .unwrap_or_default();
            let pgn = chapter.get("pgn").cloned().unwrap_or(Value::Null);
            saved.push(json!({ "id": chapter_id, "name": chapter_name, "pgn": pgn }));
        }
        let first_pgn = saved
            .first()
            .and_then(|c| c.get("pgn"))
            .cloned()
            .unwrap_or(Value::Null);
        let study_id = match id {
            Some(id) => id.to_string(),
            None => random_uuid()?,
        };
        let mut candidate = Map::new();
        candidate.insert("id".into(), json!(study_id));
        candidate.insert("name".into(), json!(name));
        candidate.insert("pgn".into(), first_pgn);
        candidate.insert("chapters".into(), Value::Array(saved));
        candidate.insert("updatedAt".into(), num(self.now));
        if let Some(cloud) = old
            .as_ref()
            .and_then(|s| s.get("cloud"))
            .filter(|c| truthy(Some(c)))
        {
            candidate.insert("cloud".into(), cloud.clone());
        }
        self.commit(candidate)
    }
}

/// A string that JavaScript would treat as true (non-empty).
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|s| !s.is_empty())
}

/// A name argument read with `.trim()`: a missing or null one throws the same `TypeError` the
/// TypeScript code throws.
fn name_field(command: &Map<String, Value>, key: &str) -> Result<String> {
    match command.get(key) {
        None => Err(CoreError::new(
            "Cannot read properties of undefined (reading 'trim')",
        )),
        Some(Value::Null) => Err(CoreError::new(
            "Cannot read properties of null (reading 'trim')",
        )),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(CoreError::new(format!(
            "command.{key}.trim is not a function"
        ))),
    }
}

/// `studyCommand(command, now)`: one change to the study library, stored on success.
fn study_command(db: &Connection, command: &Value, now: f64) -> Result<Value> {
    let command = object(command, "study command")?;
    let op = command
        .get("op")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let field = |key: &str| command.get(key).cloned().unwrap_or(Value::Null);
    // `find(command.id)` takes any id; `command.id ? … : …` only a non-empty one.
    let id_arg = command.get("id").and_then(Value::as_str);

    let mut studies = Studies {
        items: read_studies(db)?,
        now,
    };
    let mut result = Map::new();
    match op {
        "saveChapters" => {
            let id =
                studies.save_chapters(&name_field(command, "name")?, &field("chapters"), id_arg)?;
            result.insert("id".into(), json!(id));
        }
        "save" => {
            let name = clean_name(&name_field(command, "name")?);
            if name.is_empty() {
                return Err(CoreError::new("Name the study."));
            }
            let Some(old) = non_empty(id_arg).and_then(|id| studies.find(id).cloned()) else {
                let id = studies.save_chapters(
                    &name,
                    &json!([{ "name": name, "pgn": field("pgn") }]),
                    id_arg,
                )?;
                result.insert("id".into(), json!(id));
                return finish_studies(db, studies, result);
            };
            let old = object(&old, "study")?.clone();
            let chapters = old
                .get("chapters")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let index: i64 = match non_empty(command.get("chapterId").and_then(Value::as_str)) {
                Some(chapter_id) => chapters
                    .iter()
                    .position(|c| c.get("id").and_then(Value::as_str) == Some(chapter_id))
                    .map_or(-1, |i| i as i64),
                None => 0,
            };
            let pgn = field("pgn");
            if index < 0 || !pgn.as_str().is_some_and(kchess_domain::tree::valid_pgn) {
                return Err(CoreError::new("That chapter cannot be saved."));
            }
            let mut updated_chapters = Vec::with_capacity(chapters.len());
            for (i, chapter) in chapters.into_iter().enumerate() {
                if i as i64 == index {
                    let mut changed = object(&chapter, "chapter")?.clone();
                    changed.insert("pgn".into(), pgn.clone());
                    updated_chapters.push(Value::Object(changed));
                } else {
                    updated_chapters.push(chapter);
                }
            }
            let first = updated_chapters
                .first()
                .and_then(|c| c.get("pgn"))
                .cloned()
                .unwrap_or(Value::Null);
            let candidate = spread(
                &old,
                vec![
                    ("name", json!(name)),
                    ("chapters", Value::Array(updated_chapters)),
                    ("pgn", first),
                    ("updatedAt", num(now)),
                ],
            );
            let id = studies.commit(candidate)?;
            result.insert("id".into(), json!(id));
        }
        "addChapter" => {
            let Some(study) = id_arg.and_then(|id| studies.find(id).cloned()) else {
                return Err(CoreError::new("Name the new chapter."));
            };
            let raw_name = name_field(command, "name")?;
            let chapter_name = js::trim(&raw_name).to_string();
            if chapter_name.is_empty() {
                return Err(CoreError::new("Name the new chapter."));
            }
            let study = object(&study, "study")?.clone();
            let mut chapters = study
                .get("chapters")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            chapters.push(json!({ "name": chapter_name, "pgn": "*" }));
            let study_name = study
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let study_id = id_of(&Value::Object(study.clone()))
                .unwrap_or_default()
                .to_string();
            studies.save_chapters(study_name, &Value::Array(chapters), Some(&study_id))?;
            let last = studies
                .find(&study_id)
                .and_then(|saved| saved.get("chapters"))
                .and_then(Value::as_array)
                .and_then(|c| c.last())
                .and_then(|c| c.get("id"))
                .cloned()
                .unwrap_or(Value::Null);
            result.insert("id".into(), last);
        }
        "renameChapter" => {
            let study = id_arg.and_then(|id| studies.find(id).cloned());
            if let Some(study) = study {
                let raw_name = name_field(command, "name")?;
                if js::trim(&raw_name).is_empty() {
                    return finish_studies(db, studies, result);
                }
                let study = object(&study, "study")?.clone();
                let chapter_id = field("chapterId");
                let new_name = clean_name(&raw_name);
                let chapters: Vec<Value> = study
                    .get("chapters")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|c| {
                        if c.get("id") == Some(&chapter_id) {
                            let mut renamed = c.as_object().cloned().unwrap_or_default();
                            renamed.insert("name".into(), json!(new_name));
                            Value::Object(renamed)
                        } else {
                            c
                        }
                    })
                    .collect();
                studies.commit(spread(
                    &study,
                    vec![
                        ("chapters", Value::Array(chapters)),
                        ("updatedAt", num(now)),
                    ],
                ))?;
            }
        }
        "duplicateChapter" => {
            let chapter_id = field("chapterId");
            let study = id_arg.and_then(|id| studies.find(id).cloned());
            let original = study
                .as_ref()
                .and_then(|s| s.get("chapters").and_then(Value::as_array))
                .and_then(|chapters| chapters.iter().find(|c| c.get("id") == Some(&chapter_id)))
                .cloned();
            let (Some(study), Some(original)) = (study, original) else {
                return Err(CoreError::new("That chapter no longer exists."));
            };
            let study = object(&study, "study")?.clone();
            let mut chapters = study
                .get("chapters")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if chapters.len() >= MAX_CHAPTERS {
                return Err(CoreError::new("A study can contain up to 64 chapters."));
            }
            let original_name = original
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let copy_id = random_uuid()?;
            let mut copy = object(&original, "chapter")?.clone();
            copy.insert("id".into(), json!(copy_id));
            copy.insert(
                "name".into(),
                json!(format!("{} (copy)", js_slice(original_name, 113))),
            );
            chapters.push(Value::Object(copy));
            studies.commit(spread(
                &study,
                vec![
                    ("chapters", Value::Array(chapters)),
                    ("updatedAt", num(now)),
                ],
            ))?;
            result.insert("id".into(), json!(copy_id));
        }
        "removeChapter" => {
            let chapter_id = field("chapterId");
            let study = id_arg.and_then(|id| studies.find(id).cloned());
            let index = study
                .as_ref()
                .and_then(|s| s.get("chapters").and_then(Value::as_array))
                .and_then(|chapters| {
                    chapters
                        .iter()
                        .position(|c| c.get("id") == Some(&chapter_id))
                });
            let (Some(study), Some(index)) = (study, index) else {
                return Err(CoreError::new("That chapter no longer exists."));
            };
            let study = object(&study, "study")?.clone();
            let all = study
                .get("chapters")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if all.len() == 1 {
                return Err(CoreError::new("A study must keep at least one chapter."));
            }
            let chapters: Vec<Value> = all
                .into_iter()
                .filter(|c| c.get("id") != Some(&chapter_id))
                .collect();
            let first = chapters
                .first()
                .and_then(|c| c.get("pgn"))
                .cloned()
                .unwrap_or(Value::Null);
            let mut updates = vec![
                ("chapters", Value::Array(chapters.clone())),
                ("pgn", first),
                ("updatedAt", num(now)),
            ];
            if let Some(cloud) = study.get("cloud").filter(|c| truthy(Some(c))) {
                let mut changed = object(cloud, "cloud")?.clone();
                changed.insert("structureChanged".into(), Value::Bool(true));
                updates.push(("cloud", Value::Object(changed)));
            }
            studies.commit(spread(&study, updates))?;
            let pick = index.min(chapters.len() - 1);
            result.insert(
                "id".into(),
                chapters[pick].get("id").cloned().unwrap_or(Value::Null),
            );
        }
        "markCloud" => {
            if let Some(index) = id_arg.and_then(|id| studies.find_index(id)) {
                let cloud = json!({
                    "account": field("account"),
                    "id": field("remoteId"),
                    "downloadedPgn": field("baseline"),
                });
                let study = object(&studies.items[index], "study")?.clone();
                studies.items[index] = Value::Object(spread(&study, vec![("cloud", cloud)]));
            }
        }
        "offline" => offline(&mut studies, command, &mut result)?,
        "remove" => {
            let removed = id_arg.and_then(|id| studies.find(id).cloned());
            let id = field("id");
            studies.items.retain(|item| item.get("id") != Some(&id));
            if let Some(removed) = removed {
                result.insert("removed".into(), removed);
            }
        }
        "restore" => {
            let study = field("study");
            let present = study
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| studies.find(id).is_some());
            if !present {
                let candidate = object(&study, "study")?.clone();
                studies.commit(candidate)?;
            }
        }
        "rename" => {
            let study = id_arg.and_then(|id| studies.find(id).cloned());
            if let Some(study) = study {
                let raw_name = name_field(command, "name")?;
                if !js::trim(&raw_name).is_empty() {
                    let study = object(&study, "study")?.clone();
                    let candidate = spread(
                        &study,
                        vec![
                            ("name", json!(clean_name(&raw_name))),
                            ("updatedAt", num(now)),
                        ],
                    );
                    studies.commit(candidate)?;
                }
            }
        }
        "duplicate" => {
            if let Some(study) = id_arg.and_then(|id| studies.find(id).cloned()) {
                let study_name = study
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let chapters = study.get("chapters").cloned().unwrap_or(Value::Null);
                let id = studies.save_chapters(
                    &format!("{} (copy)", js_slice(study_name, 113)),
                    &chapters,
                    None,
                )?;
                result.insert("id".into(), json!(id));
            }
        }
        // Any other operation changes nothing; the list is still written, as before.
        _ => {}
    }
    finish_studies(db, studies, result)
}

/// `offline`: a cloud copy of a study, kept beside a changed local one.
fn offline(
    studies: &mut Studies,
    command: &Map<String, Value>,
    result: &mut Map<String, Value>,
) -> Result<()> {
    let account = command
        .get("account")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let remote = command.get("remote").cloned().unwrap_or(Value::Null);
    let remote_id = remote.get("id").cloned().unwrap_or(Value::Null);
    let remote_name = remote
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let chapters = command.get("chapters").cloned().unwrap_or(Value::Null);
    let old = studies
        .items
        .iter()
        .find(|s| {
            s.get("cloud").is_some_and(|cloud| {
                cloud
                    .get("account")
                    .and_then(Value::as_str)
                    .map(str::to_lowercase)
                    == Some(account.to_lowercase())
                    && cloud.get("id") == Some(&remote_id)
            })
        })
        .cloned();
    let conflict = match &old {
        Some(study) => {
            let downloaded = study
                .get("cloud")
                .and_then(|c| c.get("downloadedPgn"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            !study_matches(study, downloaded)?
        }
        None => false,
    };
    let old_id = old.as_ref().and_then(id_of).map(str::to_string);
    let name = if conflict {
        format!("{remote_name} (cloud copy)")
    } else {
        remote_name
    };
    let id = studies.save_chapters(
        &name,
        &chapters,
        if conflict { None } else { old_id.as_deref() },
    )?;
    let downloaded = chapters
        .as_array()
        .map(|list| {
            list.iter()
                .map(|c| c.get("pgn").and_then(Value::as_str).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default();
    let cloud = json!({ "account": account, "id": remote_id, "downloadedPgn": downloaded });
    for item in studies.items.iter_mut() {
        if id_of(item) == Some(id.as_str()) {
            let study = object(item, "study")?.clone();
            *item = Value::Object(spread(&study, vec![("cloud", cloud.clone())]));
        }
    }
    result.insert("id".into(), json!(id));
    result.insert("conflict".into(), Value::Bool(conflict));
    Ok(())
}

/// `studyMatchesCloud` for a stored study; a chapter that holds no game is an error.
fn study_matches(study: &Value, downloaded: &str) -> Result<bool> {
    let pairs: Vec<(String, String)> = study
        .get("chapters")
        .and_then(Value::as_array)
        .map(|chapters| {
            chapters
                .iter()
                .map(|c| {
                    (
                        c.get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        c.get("pgn")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    kchess_domain::pgn::study_matches(&pairs, downloaded)
        .ok_or_else(|| CoreError::new("A study chapter holds no game."))
}

/// `writeStudies(items)`, then the `{ studies, ...result }` a command returns.
fn finish_studies(db: &Connection, studies: Studies, result: Map<String, Value>) -> Result<Value> {
    write_document(
        db,
        STUDIES,
        &json!({ "version": 2, "items": studies.items }),
        "This study is too large to save automatically. Export a PGN copy.",
    )?;
    let mut out = Map::new();
    out.insert("studies".into(), Value::Array(studies.items));
    for (key, value) in result {
        out.insert(key, value);
    }
    Ok(Value::Object(out))
}

/* ── Played games: one row each, so a move rewrites only its own game ── */

/// The size the archive had as one document, so its limit stays what it was.
fn archive_overhead() -> i64 {
    utf16_len(&js_json(&json!({ "version": 1, "games": [] }))) as i64
}

/// Add or replace a game; it becomes the most recent.
fn write_game(db: &Connection, game: &Value) -> Result<()> {
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
    if count + 1 > MAX_ARCHIVED_GAMES || size > MAX_DOCUMENT as i64 {
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
fn add_mistakes(db: &Connection, review_key: &str, color: Option<&str>, now: f64) -> Result<Value> {
    let mut items = read_mistakes(db)?;
    let review = match super::reviews::read_review(db, review_key)? {
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

fn joined_tournaments(db: &Connection, now: f64) -> Result<Value> {
    Ok(Value::Array(
        read_joined(db)?
            .into_iter()
            .filter(|entry| until_after(entry, now))
            .collect(),
    ))
}

fn remember_tournament(db: &Connection, entry: &Value, now: f64) -> Result<Value> {
    let mut entries: Vec<Value> = read_joined(db)?
        .into_iter()
        .filter(|e| {
            until_after(e, now)
                && !(e.get("system") == entry.get("system") && e.get("id") == entry.get("id"))
        })
        .collect();
    entries.push(entry.clone());
    write_joined(db, entries)?;
    Ok(Value::Null)
}

fn forget_tournament(db: &Connection, system: &str, id: &str) -> Result<Value> {
    let kept: Vec<Value> = read_joined(db)?
        .into_iter()
        .filter(|e| {
            !(e.get("system").and_then(Value::as_str) == Some(system)
                && e.get("id").and_then(Value::as_str) == Some(id))
        })
        .collect();
    write_joined(db, kept)?;
    Ok(Value::Null)
}

/// Signed-out accounts must not reopen event streams for their tournaments.
fn forget_tournaments_of(db: &Connection, accounts: Option<&Value>) -> Result<Value> {
    let names: Vec<String> = string_list(accounts)
        .iter()
        .map(|name| name.to_lowercase())
        .collect();
    let entries = read_joined(db)?;
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
        write_joined(db, kept)?;
    }
    Ok(Value::Null)
}

/* ── Repertoire training ── */

fn record_repertoire_miss(db: &Connection, key: &str, fen: &str) -> Result<Value> {
    let mut misses = object(&read_misses(db)?, "repertoire misses")?.clone();
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

fn clear_repertoire_misses(db: &Connection, key: &str) -> Result<Value> {
    let mut misses = object(&read_misses(db)?, "repertoire misses")?.clone();
    misses.shift_remove(key);
    let misses = Value::Object(misses);
    write_document(db, MISSES, &misses, "Too many repertoire notes are saved.")?;
    Ok(misses)
}

#[cfg(test)]
mod tests {
    use super::*;

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
