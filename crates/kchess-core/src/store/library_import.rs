//! The library snapshot (`library`), its readers, and the studies-table migration.
//! Named `library_import` because `store::import` already owns the JSON-backup/legacy-database profile migration.

use kchess_contracts::types::{
    Drill, JoinedTournament, Library, PlayedGame, RepertoireMisses, RepertoireSchedule,
    SessionKind, Study,
};
use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};

use super::StoreContext;
use super::documents::{
    DAMAGED_STUDIES, decode_archived_game_text, decode_mistakes_text, parse_text, read_document,
    read_document_text, stored_studies, write_document,
};
use super::library::{JOINED, MISSES, MISTAKES, SCHEDULE, STUDIES};
use super::sql::{Sql as _, transaction};
use super::studies::{Stored, study_text};
use crate::host::{Host, Level};
use kchess_contracts::error::{CoreError, Result};
use kchess_domain::js::{truthy, utf16_len};
use kchess_domain::library::MAX_STUDIES;
use kchess_domain::misc::value::J;

/// Migrate an earlier release's document atomically, retaining it unchanged on any failure.
pub(crate) fn migrate_studies(db: &Connection) -> Result<()> {
    let Some(body) = read_document_text(db, STUDIES)? else {
        return Ok(());
    };
    // A migrated marker is any document whose `version` is 3; extra keys,
    // whitespace and key order must not un-migrate a healthy library.
    let migrated = parse_text(&body)
        .and_then(|doc| doc.get("version").cloned())
        .is_some_and(|version| match &version {
            Value::Number(n) => {
                n.as_u64() == Some(3) || n.as_i64() == Some(3) || n.as_f64() == Some(3.0)
            }
            _ => false,
        });
    if migrated {
        return Ok(());
    }
    let items = stored_studies(&body)?;
    let work = || {
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM studies", [], |row| row.get(0))
            .sql()?;
        if count != 0 {
            return Err(CoreError::new(DAMAGED_STUDIES));
        }
        for (position, study) in items.iter().enumerate() {
            let body = study_text(study);
            let units = utf16_len(&body) as i64;
            db.execute(
                "INSERT INTO studies (id, body, position, textUnits) VALUES (?1, ?2, ?3, ?4)",
                params![study.id, body, position as i64, units],
            )
            .sql()?;
        }
        write_document(db, STUDIES, &json!({ "version": 3 }), "")?;
        Ok(())
    };
    if db.is_autocommit() {
        transaction(db, work)
    } else {
        work()
    }
}

pub(crate) fn read_studies(db: &Connection) -> Result<Vec<Study>> {
    migrate_studies(db)?;
    Ok(read_study_rows(db, StudyBodies::All)?
        .into_iter()
        .filter_map(Stored::into_study)
        .collect())
}

/// Which stored studies a read decodes; the rest are their id alone.
pub(crate) enum StudyBodies<'a> {
    All,
    /// Only the study with this id (none: no study at all).
    Only(Option<&'a str>),
}

/// The `studies` table in order. A damaged row, one stored under another id, or more studies
/// than the library holds refuses the whole read, so no edit can drop what it could not read.
pub(crate) fn read_study_rows(db: &Connection, bodies: StudyBodies) -> Result<Vec<Stored>> {
    let (all, target) = match bodies {
        StudyBodies::All => (true, None),
        StudyBodies::Only(target) => (false, target),
    };
    let mut statement = db
        .prepare(
            "SELECT id, CASE WHEN ?1 OR id = ?2 THEN body ELSE NULL END FROM studies
             ORDER BY position",
        )
        .sql()?;
    let rows = statement
        .query_map(params![all, target], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .sql()?;
    let mut items = Vec::new();
    for row in rows {
        let (id, body) = row.sql()?;
        items.push(match body {
            Some(body) => Stored::Read(decode_study_row(&id, &body)?),
            None => Stored::Unread(id),
        });
    }
    if items.len() > MAX_STUDIES {
        return Err(CoreError::new(DAMAGED_STUDIES));
    }
    Ok(items)
}

fn decode_study_row(id: &str, body: &str) -> Result<Study> {
    let damaged = || CoreError::new(DAMAGED_STUDIES);
    let raw = parse_text(body).ok_or_else(damaged)?;
    let study = kchess_domain::library::decode_study(&raw).ok_or_else(damaged)?;
    if study.id != id {
        return Err(damaged());
    }
    Ok(study)
}

pub(crate) fn read_mistakes(db: &Connection) -> Result<Vec<Drill>> {
    Ok(read_document_text(db, MISTAKES)?
        .and_then(|body| decode_mistakes_text(&body))
        .unwrap_or_default())
}

pub(crate) fn read_misses(host: &dyn Host, db: &Connection) -> Result<RepertoireMisses> {
    let raw = read_document(host, db, MISSES)?.unwrap_or(Value::Null);
    Ok(kchess_domain::misc::documents::decode_repertoire_misses(
        &J::from(&raw),
    ))
}

/// The repertoire schedule's lines, `key` → line → card; none in a profile from before it.
pub(crate) fn read_schedule_lines(host: &dyn Host, db: &Connection) -> Result<RepertoireSchedule> {
    let raw = read_document(host, db, SCHEDULE)?.unwrap_or(Value::Null);
    Ok(kchess_domain::training::repetition::decode_schedule(&raw))
}

pub(crate) fn read_joined(host: &dyn Host, db: &Connection) -> Result<Vec<JoinedTournament>> {
    let raw = read_document(host, db, JOINED)?.unwrap_or(Value::Null);
    let decoded = kchess_domain::misc::documents::decode_joined_tournaments(&J::from(&raw));
    Ok(serde_json::from_value(decoded.into_value()).unwrap_or_default())
}

/// The decoded session, or null.
pub(crate) fn read_session(host: &dyn Host, db: &Connection, kind: &str) -> Result<Value> {
    let raw = read_document(host, db, &format!("session:{kind}"))?.unwrap_or(Value::Null);
    let session = kchess_domain::misc::documents::decode_stored(kind, &J::from(&raw))
        .map_err(CoreError::new)?;
    Ok(session.map_or(Value::Null, J::into_value))
}

/// The played games, newest first; a game that no longer replays is skipped.
pub(crate) fn read_games(host: &dyn Host, db: &Connection) -> Result<Vec<PlayedGame>> {
    let mut statement = db
        .prepare("SELECT id, body FROM archived_games ORDER BY seq DESC")
        .sql()?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .sql()?;
    let mut games = Vec::new();
    for row in rows {
        let (id, body) = row.sql()?;
        match decode_archived_game_text(&body) {
            Some(game) => games.push(game),
            None => host.log(
                Level::Debug,
                "library",
                &format!("Skipped an archived game that does not replay: id={id}"),
            ),
        }
    }
    Ok(games)
}

/// `library(now)` as the store answers it: the snapshot read last, when no row has changed since,
/// else read afresh. Every frontend reads the whole library to start, and pages read it again to
/// resume their sessions; reading it decodes every study and replays every played game.
pub fn cached_library(ctx: &StoreContext, now: f64) -> Result<Library> {
    let changes = ctx.db.total_changes();
    let mut snapshot = match ctx.cache.library(changes) {
        Some(snapshot) => snapshot,
        None => {
            let snapshot = library(ctx.host, ctx.db, f64::NEG_INFINITY)?;
            // Reading may migrate an earlier release's studies: kept as of after that write.
            ctx.cache
                .remember_library(ctx.db.total_changes(), &snapshot);
            snapshot
        }
    };
    snapshot
        .joined_tournaments
        .retain(|entry| entry.until > now);
    Ok(snapshot)
}

/// `library(now)`: the whole snapshot.
pub fn library(host: &dyn Host, db: &Connection, now: f64) -> Result<Library> {
    let mut sessions = Map::new();
    for kind in SessionKind::ALL {
        let kind = kind.as_str();
        let value = read_session(host, db, kind)?;
        if truthy(Some(&value)) {
            sessions.insert(kind.to_string(), value);
        }
    }
    let mut joined = read_joined(host, db)?;
    joined.retain(|entry| entry.until > now);
    let (studies, study_error) = match read_studies(db) {
        Ok(studies) => (studies, None),
        Err(error) if error.message == DAMAGED_STUDIES => {
            host.log(Level::Warn, "library", DAMAGED_STUDIES);
            (Vec::new(), Some(error.message))
        }
        Err(error) => return Err(error),
    };
    Ok(Library {
        studies,
        games: read_games(host, db)?,
        mistakes: read_mistakes(db)?,
        sessions,
        joined_tournaments: joined,
        repertoire_misses: read_misses(host, db)?,
        repertoire_schedule: read_schedule_lines(host, db)?,
        study_error,
    })
}
