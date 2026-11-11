//! The local library: studies, played games, mistake drills,
//! unfinished sessions, joined tournaments and repertoire notes, each a bounded, versioned
//! document in `kchess.db`. The decoders and semantic validation are `kchess_domain`'s.

use kchess_contracts::types::{
    Drill, JoinedTournament, Keyed, MistakeGame, MistakesAdded, PlayedGame, RepertoireMisses,
    RepertoireSchedule, SessionKind, StudyCommand, StudyCommandResult, TournamentSystem,
};
use kchess_domain::color::Color;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use super::sql::Sql as _;
use crate::host::{Host, Level};
use kchess_contracts::error::{CoreError, Result};
use kchess_domain::js::{self, utf16_len};
use kchess_domain::library::{MAX_ARCHIVED_GAMES, MAX_MISTAKES};
use kchess_domain::misc::MAX_DOCUMENT;
use kchess_domain::misc::value::J;
use kchess_domain::training::repetition;

pub(crate) const STUDY_LIBRARY_MAX_TEXT: usize = 1_800_000;

pub(crate) const STUDIES: &str = "studies";
pub(crate) const MISTAKES: &str = "mistakes";
pub(crate) const JOINED: &str = "tournaments:joined";
pub(crate) const MISSES: &str = "repertoire:misses";
/// The repertoire lines' spaced-repetition schedule (`kchess_domain::training::repetition`).
pub(crate) const SCHEDULE: &str = "repertoire:schedule";

const GAMES_FULL: &str =
    "Game history is full. Export saved games before making room for new games.";

/// The unfinished session of a page, or null: a page resuming its session reads that alone, as
/// the library replays every played game.
pub fn session(host: &dyn Host, db: &Connection, kind: SessionKind) -> Result<Option<Value>> {
    let session = read_session(host, db, kind.as_str())?;
    Ok(truthy(Some(&session)).then_some(session))
}

/// Runs a study command. `based_on` is the content the editor started from: a save over a study
/// changed since is refused rather than lost. `incremental` names a save of one chapter's moves.
pub fn run_study_command(
    db: &Connection,
    mut command: StudyCommand,
    now: f64,
    incremental: bool,
    based_on: Option<&str>,
) -> Result<StudyCommandResult> {
    if let Some(based_on) = based_on {
        let stored = check_based_on(db, &command, based_on)?;
        // A content save changes the chapter's moves only, never the study's name.
        if let (Some(stored), StudyCommand::Save { name, .. }) = (stored, &mut command) {
            *name = stored;
        }
    }
    study_command(db, &command, now, incremental)
}

pub fn remove_archived_game(db: &Connection, id: &str) -> Result<()> {
    db.execute("DELETE FROM archived_games WHERE id = ?1", params![id])
        .sql()?;
    Ok(())
}

/// Saves a page's unfinished session (`value`, already validated for `kind`).
pub fn save_session(db: &Connection, kind: SessionKind, value: Value) -> Result<()> {
    let kind = kind.as_str();
    let encoded = encode_session(kind, &value);
    write_document(
        db,
        &format!("session:{kind}"),
        &encoded,
        "This session is too large to save automatically. Export a PGN copy.",
    )
    .map(|_| ())
}

/// A JSON number: whole numbers without a fraction.
fn whole(n: f64) -> serde_json::Number {
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        (n as i64).into()
    } else {
        serde_json::Number::from_f64(n).unwrap_or_else(|| 0.into())
    }
}

/// Whether an optional JSON value is set and not false, 0, NaN, an empty string or null.
pub fn truthy(value: Option<&Value>) -> bool {
    !js::falsy(value)
}

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

/// A session of `kind` as stored, with its kind's version.
pub(crate) fn encode_session(kind: &str, session: &Value) -> Value {
    kchess_domain::misc::documents::encode_session(kind, &J::from(session)).into_value()
}

pub use super::documents::read_document;
pub(crate) use super::documents::{assert_replaceable, write_document};
pub use super::json_codec::{clean_name, js_json};
pub use super::library_import::{cached_library, library};
pub(crate) use super::library_import::{
    read_joined, read_misses, read_mistakes, read_schedule_lines, read_session,
};
pub(crate) use super::studies::{check_based_on, study_command};

/* ── Played games: one row each, so a move rewrites only its own game ── */

/// The size the archive had as one document, so its limit stays what it was.
fn archive_overhead() -> i64 {
    utf16_len(&js_json(&json!({ "version": 1, "games": [] }))) as i64
}

/// Add or replace a game; it becomes the most recent.
pub fn write_game(db: &Connection, game: &PlayedGame) -> Result<()> {
    let id = game.id.as_str();
    let body =
        js_json(&serde_json::to_value(game).map_err(|cause| CoreError::new(cause.to_string()))?);
    let (count, bytes): (i64, i64) = db
        .query_row(
            "SELECT COUNT(*) AS count, COALESCE(SUM(length(body)), 0) AS bytes
             FROM archived_games WHERE id != ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .sql()?;
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
    .sql()?;
    Ok(())
}

/* ── Mistake drills ── */

/// The mistakes document as this release stores it: version 1 with a list of items.
fn mistakes_readable(raw: &Value) -> bool {
    raw.get("version").and_then(Value::as_f64) == Some(1.0)
        && raw.get("items").is_some_and(Value::is_array)
}

fn write_mistakes(db: &Connection, items: Vec<Drill>) -> Result<()> {
    assert_replaceable(db, MISTAKES, Some(1), mistakes_readable)?;
    write_document(
        db,
        MISTAKES,
        &json!({ "version": 1, "items": items }),
        "Too many mistakes are saved to keep more.",
    )
    .map(|_| ())
}

/// Drills from a completed review's mistakes.
pub fn add_mistakes(
    host: &dyn Host,
    db: &Connection,
    review_key: &str,
    color: Option<Color>,
    now: f64,
) -> Result<MistakesAdded> {
    let mut items = read_mistakes(db)?;
    let review = match super::reviews::read_review(host, db, review_key)? {
        Some(review) if truthy(review.get("complete")) => review,
        _ => return Ok(MistakesAdded { added: 0, items }),
    };
    let color = color.map(kchess_domain::color::name);
    // A variant game's drills are played by its rules (Fairy-Stockfish reviewed it).
    let variant = review
        .get("variant")
        .and_then(Value::as_str)
        .and_then(kchess_domain::variant::Variant::from_key);
    let replay = |fen: &str, moves: &[String]| match variant {
        Some(variant) => kchess_domain::replay::replay_positions_in(variant, fen, moves),
        None => kchess_domain::replay::replay_positions(fen, moves),
    };
    let fen = review
        .get("fen")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let moves = string_list(review.get("moves"));
    let positions = replay(&fen, &moves);
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
        let playable = replay(&position, &solution).len();
        solution.truncate(playable.saturating_sub(1));
        if solution.len().is_multiple_of(2) {
            solution.pop();
        }
        if solution.is_empty() {
            continue;
        }
        let id = format!("{stored_key}:{index}");
        if items.iter().any(|item| item.id == id) {
            continue;
        }
        items.push(Drill {
            id,
            fen: position,
            solution,
            judgment: judgment.to_string(),
            due_at: whole(now),
            streak: 0.into(),
            attempts: 0.into(),
            variant,
        });
        added += 1;
    }
    let items = keep_last(items, MAX_MISTAKES);
    if added > 0 {
        write_mistakes(db, items.clone())?;
    }
    Ok(MistakesAdded { added, items })
}

/// The review key of a line: that of its legal moves from its normalized start, as a review of
/// it is stored. None when no move replays.
fn line_review_key(fen: &str, moves: &[String]) -> Option<String> {
    let positions = kchess_domain::replay::replay_positions(fen, moves);
    let start = positions.first()?;
    if positions.len() < 2 {
        return None;
    }
    Some(kchess_domain::records::review::review_key(
        &start.fen,
        &moves[..positions.len() - 1],
        None,
    ))
}

/// Each review with positions in practice, most recently played first, with
/// its game when that is known: a synced Lichess game through its review link, or a game played
/// here whose line it reviews (your side there is the one named "You").
pub fn mistake_games(host: &dyn Host, db: &Connection) -> Result<Vec<MistakeGame>> {
    let mut keys: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in read_mistakes(db)? {
        let Some((key, _)) = item.id.rsplit_once(':') else {
            continue;
        };
        if seen.insert(key.to_string()) {
            keys.push(key.to_string());
        }
    }
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let mut played = std::collections::HashMap::new();
    for game in super::library_import::read_games(host, db)? {
        if let Some(key) = line_review_key(&game.setup.fen, &game.moves) {
            played.entry(key).or_insert(game);
        }
    }
    let mut lichess = db
        .prepare(
            "SELECT g.id, g.account, g.color, g.opponent, g.createdAt
             FROM game_reviews gr JOIN games g ON g.id = gr.gameId
             WHERE gr.reviewKey = ?1 ORDER BY g.createdAt DESC LIMIT 1",
        )
        .sql()?;
    let mut games = Vec::new();
    for key in keys {
        let Some(review) = super::reviews::read_review(host, db, &key)? else {
            continue;
        };
        let mut entry = MistakeGame {
            fen: review
                .get("fen")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            moves: string_list(review.get("moves")),
            key,
            white: None,
            black: None,
            player: None,
            game_id: None,
            account: None,
            played_at: 0.0,
        };
        let mut at = js::to_number(review.get("updatedAt"));
        let synced = lichess
            .query_row(params![entry.key], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .optional()
            .sql()?;
        if let Some((id, account, color, opponent, created)) = synced {
            let player = kchess_domain::color::from_name(&color).unwrap_or(Color::Black);
            let (white, black) = if player == Color::White {
                (&account, &opponent)
            } else {
                (&opponent, &account)
            };
            entry.white = Some(white.clone());
            entry.black = Some(black.clone());
            entry.player = Some(player);
            entry.game_id = Some(id);
            entry.account = Some(account);
            #[allow(clippy::cast_precision_loss)]
            {
                at = created as f64;
            }
        } else if let Some(game) = played.get(&entry.key) {
            let (white, black) = (game.white.as_str(), game.black.as_str());
            entry.white = Some(white.to_string());
            entry.black = Some(black.to_string());
            if (white == "You") != (black == "You") {
                entry.player = Some(if white == "You" {
                    Color::White
                } else {
                    Color::Black
                });
            }
            at = game.started_at;
        }
        entry.played_at = at;
        games.push(entry);
    }
    games.sort_by(|a, b| b.played_at.total_cmp(&a.played_at));
    Ok(games)
}

/// Schedules a drill again.
pub fn answer_mistake(db: &Connection, id: &str, solved: bool, now: f64) -> Result<Vec<Drill>> {
    let mut items = read_mistakes(db)?;
    let Some(item) = items.iter_mut().find(|entry| entry.id == id) else {
        return Ok(items);
    };
    let attempts = item.attempts() + 1.0;
    let streak = if solved { item.streak() + 1.0 } else { 0.0 };
    let wait = if solved {
        2f64.powf(streak - 1.0).min(30.0) * 86_400_000.0
    } else {
        10.0 * 60_000.0
    };
    item.attempts = whole(attempts);
    item.streak = whole(streak);
    item.due_at = whole(now + wait);
    write_mistakes(db, items.clone())?;
    Ok(items)
}

/* ── Tournaments joined from KChess ── */

fn write_joined(db: &Connection, entries: Vec<JoinedTournament>) -> Result<()> {
    assert_replaceable(db, JOINED, None, Value::is_array)?;
    write_document(db, JOINED, &json!(entries), "Too many joined tournaments.").map(|_| ())
}

pub fn joined_tournaments(
    host: &dyn Host,
    db: &Connection,
    now: f64,
) -> Result<Vec<JoinedTournament>> {
    let mut entries = read_joined(host, db)?;
    entries.retain(|entry| entry.until > now);
    Ok(entries)
}

/// Memberships belong to an account: the same event joined by another account is another entry.
pub fn remember_tournament(
    host: &dyn Host,
    db: &Connection,
    entry: &JoinedTournament,
    now: f64,
) -> Result<()> {
    let mut entries = read_joined(host, db)?;
    entries.retain(|e| {
        e.until > now
            && !(e.system == entry.system && e.id == entry.id && e.joined_by(&entry.account))
    });
    entries.push(entry.clone());
    write_joined(db, entries)
}

/// Forgets one account's membership, or every account's when `account` is `None` (the event ended).
pub fn forget_tournament(
    host: &dyn Host,
    db: &Connection,
    system: TournamentSystem,
    id: &str,
    account: Option<&str>,
) -> Result<()> {
    let mut kept = read_joined(host, db)?;
    kept.retain(|e| {
        !(e.system == system && e.id == id && account.is_none_or(|name| e.joined_by(name)))
    });
    write_joined(db, kept)
}

/// Signed-out accounts must not reopen event streams for their tournaments.
pub fn forget_tournaments_of(host: &dyn Host, db: &Connection, accounts: &[String]) -> Result<()> {
    let entries = read_joined(host, db)?;
    let total = entries.len();
    let kept: Vec<JoinedTournament> = entries
        .into_iter()
        .filter(|e| !accounts.iter().any(|name| e.joined_by(name)))
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
    Ok(())
}

/* ── Repertoire training ── */

fn write_misses(db: &Connection, misses: &RepertoireMisses) -> Result<()> {
    assert_replaceable(db, MISSES, None, Value::is_object)?;
    let document =
        serde_json::to_value(misses).map_err(|cause| CoreError::new(cause.to_string()))?;
    write_document(
        db,
        MISSES,
        &document,
        "Too many repertoire notes are saved.",
    )
    .map(|_| ())
}

/// A count added to the one a position has: `now` is none for a position never missed.
fn add_count(now: Option<&serde_json::Number>, count: f64) -> serde_json::Number {
    whole(now.and_then(serde_json::Number::as_f64).unwrap_or(0.0) + count)
}

pub fn record_repertoire_miss(
    host: &dyn Host,
    db: &Connection,
    key: &str,
    fen: &str,
) -> Result<RepertoireMisses> {
    let mut misses = read_misses(host, db)?;
    let table = misses.entry(key);
    table.insert(fen.to_string(), add_count(table.get(fen), 1.0));
    write_misses(db, &misses)?;
    Ok(misses)
}

pub fn clear_repertoire_misses(
    host: &dyn Host,
    db: &Connection,
    key: &str,
) -> Result<RepertoireMisses> {
    let mut misses = read_misses(host, db)?;
    misses.0.retain(|(table, _)| table != key);
    write_misses(db, &misses)?;
    Ok(misses)
}

/// Undo of a clear: the cleared table's counts, added to any missed since.
pub fn restore_repertoire_misses(
    host: &dyn Host,
    db: &Connection,
    key: &str,
    restored: &Keyed<f64>,
) -> Result<RepertoireMisses> {
    let mut misses = read_misses(host, db)?;
    let table = misses.entry(key);
    for (fen, count) in restored.iter() {
        table.insert(fen.to_string(), add_count(table.get(fen), *count));
    }
    write_misses(db, &misses)?;
    Ok(misses)
}

/// Answer one line of a repertoire: its schedule moves on, and the whole schedule is returned.
pub fn review_repertoire_line(
    host: &dyn Host,
    db: &Connection,
    key: &str,
    line: &str,
    correct: bool,
    now: f64,
) -> Result<RepertoireSchedule> {
    let mut lines = read_schedule_lines(host, db)?;
    repetition::record(&mut lines, key, line, correct, now).map_err(CoreError::new)?;
    // An unversioned schedule (a development format) is replaced; a newer one is kept.
    assert_replaceable(db, SCHEDULE, Some(repetition::SCHEDULE_VERSION), |_| true)?;
    write_document(
        db,
        SCHEDULE,
        &repetition::encode_schedule(&lines),
        "Too many repertoire lines are scheduled.",
    )?;
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::documents::{DAMAGED_STUDIES, read_document_text};
    use crate::store::library_import::read_studies;
    use crate::store::{StoreContext, library_import};

    use crate::test_host::QUIET;
    use kchess_contracts::check::{MissCounts, Validate as _};

    // The store's calls with what the window sends and reads: JSON in, JSON out.

    fn json_of<T: serde::Serialize>(value: T) -> Value {
        serde_json::to_value(value).expect("json")
    }

    fn typed(command: &Value) -> StudyCommand {
        serde_json::from_value(command.clone()).expect("study command")
    }

    fn study_command(
        db: &Connection,
        command: &Value,
        now: f64,
        incremental: bool,
    ) -> Result<Value> {
        super::study_command(db, &typed(command), now, incremental).map(json_of)
    }

    fn check_based_on(db: &Connection, command: &Value, based_on: &str) -> Result<Option<String>> {
        super::check_based_on(db, &typed(command), based_on)
    }

    fn run_study_command(
        db: &Connection,
        command: Value,
        now: f64,
        incremental: bool,
        based_on: Option<&str>,
    ) -> Result<Value> {
        super::run_study_command(db, typed(&command), now, incremental, based_on).map(json_of)
    }

    fn cached_library(ctx: &StoreContext, now: f64) -> Result<Value> {
        library_import::cached_library(ctx, now).map(json_of)
    }

    fn snapshot_of(host: &dyn Host, db: &Connection, now: f64) -> Result<Value> {
        library_import::library(host, db, now).map(json_of)
    }

    fn add_mistakes(
        host: &dyn Host,
        db: &Connection,
        review_key: &str,
        color: Option<&str>,
        now: f64,
    ) -> Result<Value> {
        let color = color.map(|name| kchess_domain::color::from_name(name).expect("a side"));
        super::add_mistakes(host, db, review_key, color, now).map(json_of)
    }

    fn mistake_games(host: &dyn Host, db: &Connection) -> Result<Value> {
        super::mistake_games(host, db).map(json_of)
    }

    fn answer_mistake(db: &Connection, id: &str, solved: bool, now: f64) -> Result<Value> {
        super::answer_mistake(db, id, solved, now).map(json_of)
    }

    fn joined_tournaments(host: &dyn Host, db: &Connection, now: f64) -> Result<Value> {
        super::joined_tournaments(host, db, now).map(json_of)
    }

    fn kind(name: &str) -> SessionKind {
        serde_json::from_value(json!(name)).expect("a session kind")
    }

    fn session(host: &dyn Host, db: &Connection, name: &str) -> Result<Value> {
        super::session(host, db, kind(name)).map(json_of)
    }

    fn save_session(db: &Connection, name: &str, value: Value) -> Result<()> {
        super::save_session(db, kind(name), value)
    }

    fn record_repertoire_miss(
        host: &dyn Host,
        db: &Connection,
        key: &str,
        fen: &str,
    ) -> Result<Value> {
        super::record_repertoire_miss(host, db, key, fen).map(json_of)
    }

    fn clear_repertoire_misses(host: &dyn Host, db: &Connection, key: &str) -> Result<Value> {
        super::clear_repertoire_misses(host, db, key).map(json_of)
    }

    fn review_repertoire_line(
        host: &dyn Host,
        db: &Connection,
        key: &str,
        line: &str,
        correct: bool,
        now: f64,
    ) -> Result<Value> {
        super::review_repertoire_line(host, db, key, line, correct, now).map(json_of)
    }

    fn restore_repertoire_misses(
        host: &dyn Host,
        db: &Connection,
        key: &str,
        misses: &Value,
    ) -> Result<Value> {
        let misses = MissCounts::validate(J::from(misses)).map_err(CoreError::new)?;
        super::restore_repertoire_misses(host, db, key, &misses.0).map(json_of)
    }

    /// An arena that has ended is dropped when the next one is remembered, and
    /// forgetting an account drops its tournaments whatever the case of its name.
    #[test]
    fn ended_tournaments_and_those_of_signed_out_accounts_are_forgotten() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        crate::store::migrations::migrate(&db).unwrap();
        let now = 1_700_000_000_000.0;
        let entry = |system: &str, id: &str, account: &str, name: &str, until: f64| json!({ "system": system, "id": id, "account": account, "name": name, "until": until });
        let remember = |item: Value| {
            remember_tournament(&QUIET, &db, &serde_json::from_value(item).unwrap(), now).unwrap()
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
        let joined = || json!(joined_tournaments(&QUIET, &db, now).unwrap());
        assert_eq!(ids(joined()), ["live", "other"]);
        forget_tournaments_of(&QUIET, &db, &["alice".to_string()]).unwrap();
        assert_eq!(ids(joined()), ["other"]);
    }

    // Viewing an arena as Bob (not in it) forgot Alice's membership, and Bob joining the same
    // event replaced hers: memberships were keyed without the account.
    #[test]
    fn tournament_memberships_belong_to_their_account() {
        let db = Connection::open_in_memory().expect("database");
        super::super::migrations::migrate(&db).expect("migrate");
        let now = 1_700_000_000_000.0;
        let remember = |account: &str| {
            let entry = json!({ "system": "arena", "id": "live", "account": account, "name": "Live", "until": now + 60_000.0 });
            remember_tournament(&QUIET, &db, &serde_json::from_value(entry).unwrap(), now).unwrap();
        };
        let forget = |account: Option<&str>| {
            forget_tournament(&QUIET, &db, TournamentSystem::Arena, "live", account).unwrap();
        };
        let accounts = || -> Vec<String> {
            joined_tournaments(&QUIET, &db, now)
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["account"].as_str().unwrap().to_string())
                .collect()
        };
        remember("Alice");
        remember("Bob");
        remember("bob");
        assert_eq!(accounts(), ["Alice", "bob"]);
        forget(Some("BOB"));
        assert_eq!(accounts(), ["Alice"]);
        forget(Some("Carol"));
        assert_eq!(accounts(), ["Alice"]);
        remember("Bob");
        // The event ending forgets every account's place.
        forget(None);
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
            let session = encode_session(
                "analysis",
                &json!({"pgn":"1. d4 *","path":"","orientation":"white","study":"","chapter":""}),
            );
            write_document(&db, "session:analysis", &session, "").expect("save session");
            let snapshot = snapshot_of(&QUIET, &db, 1.0).expect("other documents remain readable");
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
        assert!(studies[0].chapters[0].pgn.contains("c5 $1"));
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
            &json!({"op":"save", "id":"removed", "name":"N", "pgn":"1. e4 *"}),
            "1. e4 *",
        )
        .expect_err("removed");
        assert_eq!(removed.message, super::super::studies::STALE_STUDY);
        assert_eq!(
            removed.kind,
            Some(kchess_contracts::error::ErrorKind::Conflict)
        );
        check_based_on(&db, &save("1. d4 *"), "1. e4 *").expect("based on the current chapter");
        study_command(&db, &save("1. d4 *"), 1.0, true).expect("first window saves");
        let stale = check_based_on(&db, &save("1. e4 e5 *"), "1. e4 *").expect_err("second window");
        assert_eq!(stale.message, super::super::studies::STALE_STUDY);
        assert_eq!(
            stale.kind,
            Some(kchess_contracts::error::ErrorKind::Conflict)
        );
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
        let call = |command: Value, now: f64, based_on: Option<&str>| {
            run_study_command(&db, command, now, true, based_on)
        };
        let created = call(
            json!({"op":"save", "name":"Original", "pgn":"1. e4 *"}),
            1.0,
            None,
        )
        .unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        let chapter = created["studies"][0]["chapters"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        call(
            json!({"op":"rename", "id":id, "name":"Remote rename"}),
            2.0,
            None,
        )
        .unwrap();
        let save = |pgn: &str, based_on: &str| {
            call(
                json!({"op":"save", "name":"Original", "pgn":pgn, "id":id, "chapterId":chapter}),
                3.0,
                Some(based_on),
            )
        };
        save("1. e4 e5 *", "1. e4 *").expect("autosave");
        let studies = read_studies(&db).unwrap();
        assert_eq!(studies[0].name, "Remote rename");
        assert_eq!(studies[0].chapters[0].pgn, "1. e4 e5 *");
        call(json!({"op":"remove", "id":id}), 4.0, None).unwrap();
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

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    /// 1. e4 e5 2. Qh5 Nc6 3. Bc4 Nf6?? 4. Qxf7#
    const BLUNDER: [&str; 7] = ["e2e4", "e7e5", "d1h5", "b8c6", "f1c4", "g8f6", "h5f7"];

    fn drills_db() -> Connection {
        let db = Connection::open_in_memory().expect("database");
        crate::store::migrations::migrate(&db).expect("migrate");
        db
    }

    /// A complete review of `BLUNDER`: level until Black's sixth move hands White mate in one.
    fn store_blunder_review(db: &Connection, source: &str, depth: u32) -> String {
        let key = json!(kchess_domain::records::review::review_key(
            START, &BLUNDER, None
        ));
        let mut evals: Vec<Value> = (0..5)
            .map(|_| json!({ "cp": 30, "depth": depth, "best": "a2a3" }))
            .collect();
        evals.push(
            json!({ "cp": 30, "depth": depth, "best": "g7g6", "pv": ["g7g6", "h5f3", "g8f6"] }),
        );
        evals.push(json!({ "mate": 1, "depth": depth, "best": "h5f7", "pv": ["h5f7"] }));
        evals.push(json!({ "mate": 0, "depth": depth }));
        crate::store::reviews::write_review(
            &QUIET,
            db,
            &json!({
                "key": key, "fen": START, "moves": BLUNDER, "source": source,
                "complete": true, "depth": depth, "updatedAt": 7, "evals": evals,
            }),
        )
        .expect("write review");
        key.as_str().expect("key").to_string()
    }

    #[test]
    fn a_reviewed_blunder_becomes_a_drill_scheduled_by_its_answers() {
        let db = drills_db();
        let key = store_blunder_review(&db, "local", 20);
        // White made no mistake, so a drill of White's moves adds nothing.
        let white = add_mistakes(&QUIET, &db, &key, Some("white"), 1.0).expect("add");
        assert_eq!(white["added"], json!(0));
        let added = add_mistakes(&QUIET, &db, &key, Some("black"), 1.0).expect("add");
        assert_eq!(added["added"], json!(1));
        let drill = &added["items"][0];
        assert_eq!(drill["id"], json!(format!("{key}:5")));
        assert_eq!(drill["judgment"], json!("blunder"));
        // The engine's line from the position, ending on a move of the drilled side.
        assert_eq!(drill["solution"], json!(["g7g6", "h5f3", "g8f6"]));
        assert_eq!(
            (drill["dueAt"].clone(), drill["streak"].clone()),
            (json!(1), json!(0))
        );
        // Adding again keeps one drill per position.
        let again = add_mistakes(&QUIET, &db, &key, None, 2.0).expect("add");
        assert_eq!(again["added"], json!(0));
        assert_eq!(again["items"].as_array().map(Vec::len), Some(1));

        let id = drill["id"].as_str().expect("id");
        let day = 86_400_000.0;
        let solved = answer_mistake(&db, id, true, 10.0).expect("answer");
        assert_eq!(solved[0]["streak"], json!(1));
        assert_eq!(solved[0]["attempts"], json!(1));
        assert_eq!(solved[0]["dueAt"].as_f64(), Some(10.0 + day));
        let twice = answer_mistake(&db, id, true, 10.0).expect("answer");
        assert_eq!(twice[0]["dueAt"].as_f64(), Some(10.0 + 2.0 * day));
        let missed = answer_mistake(&db, id, false, 10.0).expect("answer");
        assert_eq!(missed[0]["streak"], json!(0));
        assert_eq!(missed[0]["attempts"], json!(3));
        assert_eq!(missed[0]["dueAt"].as_f64(), Some(10.0 + 600_000.0));
        // An unknown drill changes nothing.
        let unknown = answer_mistake(&db, "nope", true, 10.0).expect("answer");
        assert_eq!(unknown[0]["attempts"].as_f64(), Some(3.0));
        assert_eq!(unknown[0]["dueAt"].as_f64(), Some(10.0 + 600_000.0));
    }

    /// A Crazyhouse game's mistake is drilled by Crazyhouse's rules: its answer is a drop, and
    /// the drill keeps the variant to be played in.
    #[test]
    fn a_variant_games_mistake_becomes_a_drill_in_its_variant() {
        let db = drills_db();
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR[] w KQkq - 0 1";
        let moves = json!(["e2e4", "d7d5", "e4d5", "d8d5", "g1f3"]);
        let key = json!(kchess_domain::records::review::review_key(
            fen,
            &string_list(Some(&moves)),
            Some(kchess_domain::variant::Variant::Crazyhouse)
        ));
        let mut evals: Vec<Value> = (0..4)
            .map(|_| json!({ "cp": 30, "depth": 20, "best": "a2a3" }))
            .collect();
        evals.push(json!({ "cp": 30, "depth": 20, "best": "P@e4", "pv": ["P@e4"] }));
        evals.push(json!({ "cp": -500, "depth": 20 }));
        crate::store::reviews::write_review(
            &QUIET,
            &db,
            &json!({
                "key": key, "fen": fen, "moves": moves, "source": "local", "variant": "crazyhouse",
                "complete": true, "depth": 20, "updatedAt": 7, "evals": evals,
            }),
        )
        .expect("write review");
        let added =
            add_mistakes(&QUIET, &db, key.as_str().unwrap(), Some("white"), 1.0).expect("add");
        assert_eq!(added["added"], json!(1), "{added}");
        let drill = &added["items"][0];
        assert_eq!(drill["solution"], json!(["P@e4"]));
        assert_eq!(drill["variant"], json!("crazyhouse"));
        assert!(
            drill["fen"].as_str().is_some_and(|fen| fen.contains('[')),
            "{drill}"
        );
        // The drill is kept, variant and all, as the document is read back.
        let answered =
            answer_mistake(&db, drill["id"].as_str().unwrap(), true, 2.0).expect("answer");
        assert_eq!(answered[0]["variant"], json!("crazyhouse"), "{answered}");
    }

    #[test]
    fn shallow_local_reviews_and_unfinished_ones_make_no_drills() {
        let db = drills_db();
        let key = store_blunder_review(&db, "local", 8);
        let shallow = add_mistakes(&QUIET, &db, &key, None, 1.0).expect("add");
        assert_eq!(shallow["added"], json!(0));
        let missing = add_mistakes(&QUIET, &db, "no review", None, 1.0).expect("add");
        assert_eq!(missing, json!({ "added": 0, "items": [] }));
    }

    #[test]
    fn drill_games_name_the_game_a_review_came_from() {
        let db = drills_db();
        assert_eq!(mistake_games(&QUIET, &db).expect("games"), json!([]));
        let key = store_blunder_review(&db, "local", 20);
        add_mistakes(&QUIET, &db, &key, None, 1.0).expect("add");
        // Without a game it is dated by its review (when it was written).
        let games = mistake_games(&QUIET, &db).expect("games");
        assert_eq!(games[0]["key"], json!(key));
        assert!(games[0]["playedAt"].as_f64().is_some_and(|at| at > 1.0e12));
        assert!(games[0].get("white").is_none());

        // A game played here whose line it reviews, with "You" as Black.
        let game = json!({
            "id": "local1", "source": "computer", "startedAt": 50, "updatedAt": 60, "white": "Stockfish",
            "black": "You", "result": "1-0", "reason": "Checkmate", "timeControl": "-", "finished": true, "setup": { "variant": "standard", "fen": START },
            "moves": BLUNDER,
        });
        write_game(
            &db,
            &kchess_domain::library::decode_archived_game(&game).expect("a played game"),
        )
        .map(|()| Value::Null)
        .expect("save");
        let games = mistake_games(&QUIET, &db).expect("games");
        assert_eq!(
            (
                games[0]["white"].clone(),
                games[0]["player"].clone(),
                games[0]["playedAt"].clone()
            ),
            (json!("Stockfish"), json!("black"), json!(50))
        );

        // A synced Lichess game linked to the review wins.
        db.execute_batch(&format!(
            "INSERT INTO games (account, id, createdAt, lastMoveAt, rated, speed, perf, status, color,
                                opponent, moves)
               VALUES ('Alice', 'Lichess1', 99, 99, 1, 'blitz', 'blitz', 'mate', 'black', 'Bob', '');
             INSERT INTO game_reviews (gameId, reviewKey) VALUES ('Lichess1', '{key}');"
        ))
        .expect("seed");
        let games = mistake_games(&QUIET, &db).expect("games");
        assert_eq!(
            (
                games[0]["white"].clone(),
                games[0]["black"].clone(),
                games[0]["gameId"].clone()
            ),
            (json!("Bob"), json!("Alice"), json!("Lichess1"))
        );
        assert_eq!(games[0]["playedAt"], json!(99));

        remove_archived_game(&db, "local1")
            .map(|()| Value::Null)
            .expect("remove");
    }

    #[test]
    fn repertoire_misses_are_counted_per_line_and_cleared() {
        let db = drills_db();
        let miss =
            |key: &str, fen: &str| record_repertoire_miss(&QUIET, &db, key, fen).expect("miss");
        miss("white", "fen1");
        miss("white", "fen1");
        let misses = miss("black", "fen2");
        assert_eq!(
            misses,
            json!({ "white": { "fen1": 2 }, "black": { "fen2": 1 } })
        );
        let cleared = clear_repertoire_misses(&QUIET, &db, "white").expect("clear");
        assert_eq!(cleared, json!({ "black": { "fen2": 1 } }));
        // Undo puts the cleared table back, adding to a miss recorded since.
        miss("white", "fen3");
        let restored =
            restore_repertoire_misses(&QUIET, &db, "white", &json!({ "fen1": 2, "fen3": 1 }))
                .expect("restore");
        assert_eq!(
            restored,
            json!({ "black": { "fen2": 1 }, "white": { "fen3": 2, "fen1": 2 } })
        );
        assert!(restore_repertoire_misses(&QUIET, &db, "white", &json!({ "fen1": -1 })).is_err());
    }

    #[test]
    fn a_damaged_or_newer_document_is_kept_instead_of_replaced() {
        use crate::store::documents::DAMAGED_DOCUMENT;
        let now = 1_800_000_000_000.0;
        let seed = |db: &Connection, key: &str, body: &str| {
            db.execute(
                "INSERT INTO documents (key, body, updatedAt) VALUES (?1, ?2, 0)
                 ON CONFLICT(key) DO UPDATE SET body = excluded.body",
                params![key, body],
            )
            .expect("seed");
        };
        type Write = fn(&Connection, &str) -> Result<Value>;
        let cases: Vec<(&str, &str, &str, Write)> = vec![
            // A schedule a newer release wrote: answering one line used to keep only that line.
            (
                SCHEDULE,
                r#"{"version":2,"lines":{"study:white":{"e2e4":{"due":0}}}}"#,
                "reviewRepertoireLine",
                |db, _| {
                    review_repertoire_line(
                        &QUIET,
                        db,
                        "study:white",
                        "d2d4",
                        true,
                        1_800_000_000_000.0,
                    )
                },
            ),
            // One damaged byte in the drills: the next review's drills used to replace them all.
            (
                MISTAKES,
                r#"{"version":1,"items":[{"id":"a"#,
                "addMistakes",
                |db, review| add_mistakes(&QUIET, db, review, Some("black"), 1.0),
            ),
            (
                MISTAKES,
                r#"{"version":2,"items":[]}"#,
                "addMistakes",
                |db, review| add_mistakes(&QUIET, db, review, Some("black"), 1.0),
            ),
            (
                JOINED,
                r#"[{"system":"arena""#,
                "forgetTournament",
                |db, _| {
                    forget_tournament(&QUIET, db, TournamentSystem::Arena, "x", None)
                        .map(|()| Value::Null)
                },
            ),
            (
                MISSES,
                r#"{"study:white":"#,
                "recordRepertoireMiss",
                |db, _| record_repertoire_miss(&QUIET, db, "study:white", "fen"),
            ),
        ];
        for (key, body, method, write) in cases {
            let db = drills_db();
            let review = store_blunder_review(&db, "local", 20);
            seed(&db, key, body);
            let error = write(&db, &review).expect_err(method);
            assert_eq!(error.message, DAMAGED_DOCUMENT, "{method}");
            let kept = read_document_text(&db, key).expect("read").expect("kept");
            assert_eq!(kept, body, "{method} replaced {key}");
            // Reading still works, so the rest of the library opens.
            snapshot_of(&QUIET, &db, now).expect("library");
        }
    }

    #[test]
    fn a_profile_from_before_the_schedule_has_every_line_new() {
        let db = drills_db();
        // What an earlier release left: repertoire misses, and no schedule.
        db.execute(
            "INSERT INTO documents (key, body, updatedAt) VALUES (?1, ?2, 0)",
            params![MISSES, r#"{"study:white":{"fen1":2}}"#],
        )
        .expect("seed");
        let now = 1_800_000_000_000.0;
        let library = snapshot_of(&QUIET, &db, now).expect("library");
        assert_eq!(library["repertoireSchedule"], json!({}));
        assert_eq!(
            library["repertoireMisses"],
            json!({ "study:white": { "fen1": 2 } })
        );

        // A schedule document of another shape (unversioned) is no schedule either.
        db.execute(
            "INSERT INTO documents (key, body, updatedAt) VALUES (?1, ?2, 0)",
            params![SCHEDULE, r#"{"study:white":{"e2e4":{"due":0}}}"#],
        )
        .expect("seed");
        let library = snapshot_of(&QUIET, &db, now).expect("library");
        assert_eq!(library["repertoireSchedule"], json!({}));

        // Answering a line schedules it; the misses stay as they were.
        let review = |line: &str, correct: bool, at: f64| {
            review_repertoire_line(&QUIET, &db, "study:white", line, correct, at)
        };
        let schedule = review("e2e4 e7e5", true, now).expect("review");
        assert_eq!(schedule["study:white"]["e2e4 e7e5"]["interval"], json!(1.0));
        assert_eq!(
            schedule["study:white"]["e2e4 e7e5"]["due"],
            json!(now + repetition::DAY_MS)
        );
        let schedule = review("d2d4", false, now).expect("review");
        assert_eq!(schedule["study:white"]["d2d4"]["due"], json!(now));
        let library = snapshot_of(&QUIET, &db, now).expect("library");
        assert_eq!(library["repertoireSchedule"], schedule);
        assert_eq!(
            library["repertoireMisses"],
            json!({ "study:white": { "fen1": 2 } })
        );
        let stored = read_document_text(&db, SCHEDULE)
            .expect("read")
            .expect("stored");
        assert!(stored.starts_with(r#"{"version":1,"lines":"#), "{stored}");

        // A line that is not moves, or a missing key, is refused and changes nothing.
        assert!(review("not moves", true, now).is_err());
        let after = snapshot_of(&QUIET, &db, now).expect("library");
        assert_eq!(after["repertoireSchedule"], schedule);
    }

    fn played(id: &str) -> Value {
        json!({
            "id": id, "source": "board", "startedAt": 1, "updatedAt": 2, "white": "A",
            "black": "B", "result": "1-0", "reason": "Checkmate", "timeControl": "-",
            "finished": true, "setup": { "variant": "standard", "fen": START }, "moves": BLUNDER,
        })
    }

    fn game_ids(library: &Value) -> Vec<&str> {
        library["games"]
            .as_array()
            .expect("games")
            .iter()
            .map(|game| game["id"].as_str().expect("id"))
            .collect()
    }

    /// Reading the library replays every played game, and the app read it three times to start:
    /// a read again is the snapshot kept from the last, until any write changes a row, whichever
    /// storage method (or none, as here) writes it.
    #[test]
    fn a_library_read_again_is_kept_until_any_row_changes() {
        let db = drills_db();
        let config = crate::host::Config::new(std::path::PathBuf::new());
        let cache = crate::store::StoreCache::default();
        let ctx = crate::store::StoreContext {
            db: &db,
            host: &QUIET,
            config: &config,
            cache: &cache,
        };
        let read = || cached_library(&ctx, 0.0).expect("library");
        write_game(
            &db,
            &kchess_domain::library::decode_archived_game(&played("one")).expect("a played game"),
        )
        .expect("save");
        assert_eq!(game_ids(&read()), ["one"]);
        let changes = db.total_changes();
        assert!(cache.library(changes).is_some(), "the read is kept");
        assert!(
            cache.library(changes + 1).is_none(),
            "and stale after a write"
        );
        assert_eq!(game_ids(&read()), ["one"]);

        write_game(
            &db,
            &kchess_domain::library::decode_archived_game(&played("two")).expect("a played game"),
        )
        .expect("save");
        assert_eq!(game_ids(&read()), ["two", "one"]);
        save_session(&db, "local", json!({ "x": 1 })).ok();
        db.execute("DELETE FROM archived_games WHERE id = 'two'", [])
            .expect("delete");
        assert_eq!(game_ids(&read()), ["one"]);
        // A stored game that no longer replays is left out, as on a first read.
        db.execute(
            "UPDATE archived_games SET body = replace(body, 'h5f7', 'h5h8')",
            [],
        )
        .expect("damage");
        assert!(game_ids(&read()).is_empty());

        cache.remember_library(db.total_changes(), &Default::default());
        cache.forget_library();
        assert!(cache.library(db.total_changes()).is_none());
        cache.remember_library(db.total_changes(), &Default::default());
        cache.clear();
        assert!(cache.library(db.total_changes()).is_none());
    }

    /// The kept snapshot holds every joined tournament: each read leaves out those ended by its
    /// own `now`, so a tournament ends on time without a write.
    #[test]
    fn a_kept_library_still_leaves_out_tournaments_ended_since() {
        let db = drills_db();
        let config = crate::host::Config::new(std::path::PathBuf::new());
        let cache = crate::store::StoreCache::default();
        let ctx = crate::store::StoreContext {
            db: &db,
            host: &QUIET,
            config: &config,
            cache: &cache,
        };
        let now = 1_700_000_000_000.0;
        let entry = json!({ "system": "arena", "id": "live", "account": "Alice", "name": "Live", "until": now + 60_000.0 });
        remember_tournament(&QUIET, &db, &serde_json::from_value(entry).unwrap(), now)
            .expect("remember");
        let joined = |at: f64| {
            cached_library(&ctx, at).expect("library")["joinedTournaments"]
                .as_array()
                .expect("joined")
                .len()
        };
        assert_eq!(joined(now), 1);
        assert_eq!(joined(now + 120_000.0), 0, "ended by then");
        assert_eq!(joined(now), 1, "the kept snapshot still has it");
    }

    /// The pages that resume a session read it, and the drills, alone: each answer is what the
    /// whole library holds for it, without replaying every played game.
    #[test]
    fn a_session_and_the_drills_read_alone_are_the_librarys() {
        let db = drills_db();
        let now = 1_700_000_000_000.0;
        let library = || snapshot_of(&QUIET, &db, now).expect("library");
        let session = |kind: &str| session(&QUIET, &db, kind).expect(kind);
        let mistakes = || {
            read_mistakes(&db)
                .map(|items| json!(items))
                .expect("mistakes")
        };
        assert_eq!(session("analysis"), Value::Null, "none kept yet");
        assert_eq!(mistakes(), json!([]));

        let kept = json!({ "pgn": "1. e4 e5 *", "path": "e2e4", "orientation": "black" });
        save_session(&db, "analysis", kept).expect("save");
        let key = store_blunder_review(&db, "local", 18);
        add_mistakes(&QUIET, &db, &key, None, 1.0).expect("add");
        let library = library();
        assert!(
            !library["sessions"]["analysis"].is_null(),
            "the session is kept"
        );
        assert_eq!(session("analysis"), library["sessions"]["analysis"]);
        assert_eq!(session("local"), Value::Null);
        assert!(mistakes().as_array().is_some_and(|items| !items.is_empty()));
        assert_eq!(mistakes(), library["mistakes"]);
    }
}
