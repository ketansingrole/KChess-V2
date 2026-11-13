//! Studies: the `Studies` working set, `studyCommand` and its storage (`studies` table).

use std::collections::{HashMap, HashSet};

use kchess_contracts::types::{
    ChapterInput, RemoteStudy, Study, StudyChapter, StudyCloud, StudyCommand, StudyCommandResult,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use super::documents::{DAMAGED_STUDIES, read_document_text, write_document};
use super::json_codec::{clean_name, js_json};
use super::library::{STUDIES, STUDY_LIBRARY_MAX_TEXT};
use super::library_import::{StudyBodies, migrate_studies, read_studies, read_study_rows};
use super::sql::{Sql as _, transaction};
use crate::ids::random_uuid;
use kchess_contracts::error::{CoreError, ErrorKind, Result};
use kchess_domain::js;
use kchess_domain::js::utf16_len;
use kchess_domain::library::{MAX_CHAPTERS, MAX_STUDIES, decode_study};
use kchess_domain::misc::MAX_DOCUMENT;

/* ── Studies ── */

/// A stored study as one command reads it: whole, or only its id when an incremental command
/// leaves it unread (its persisted size stands for its text).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Stored {
    Read(Study),
    Unread(String),
}

impl Stored {
    fn id(&self) -> &str {
        match self {
            Stored::Read(study) => &study.id,
            Stored::Unread(id) => id,
        }
    }

    fn study(&self) -> Option<&Study> {
        match self {
            Stored::Read(study) => Some(study),
            Stored::Unread(_) => None,
        }
    }

    pub(crate) fn into_study(self) -> Option<Study> {
        match self {
            Stored::Read(study) => Some(study),
            Stored::Unread(_) => None,
        }
    }
}

/// A study as its row stores it: compact, whole numbers without `.0`.
pub(crate) fn study_text(study: &Study) -> String {
    js_json(&serde_json::to_value(study).unwrap_or_default())
}

/// The time a command stamps on a study: a whole number of milliseconds without a fraction.
fn stamp(now: f64) -> serde_json::Number {
    match js::number_to_json(now) {
        serde_json::Value::Number(n) => n,
        _ => 0.into(),
    }
}

/// The chapters as they are saved again: their names and PGNs.
fn inputs(chapters: &[StudyChapter]) -> Vec<ChapterInput> {
    chapters
        .iter()
        .map(|chapter| ChapterInput {
            name: chapter.name.clone(),
            pgn: chapter.pgn.clone(),
        })
        .collect()
}

/// The study list one command works on; `finish_studies` stores it.
struct Studies {
    items: Vec<Stored>,
    now: f64,
    sizes: HashMap<String, usize>,
}

impl Studies {
    fn text_size(&self, items: &[Stored]) -> usize {
        2 + items.len().saturating_sub(1)
            + items
                .iter()
                .map(|item| match item {
                    Stored::Read(study) => utf16_len(&study_text(study)),
                    Stored::Unread(id) => self.sizes.get(id).copied().unwrap_or(0),
                })
                .sum::<usize>()
    }

    fn find(&self, id: &str) -> Option<&Study> {
        self.items
            .iter()
            .filter_map(Stored::study)
            .find(|study| study.id == id)
    }

    /// Put `candidate` first, replacing any study with its id; the list is unchanged on failure.
    fn commit(&mut self, candidate: Study) -> Result<String> {
        let id = candidate.id.clone();
        let mut next = vec![Stored::Read(candidate)];
        next.extend(self.items.iter().filter(|item| item.id() != id).cloned());
        if next.len() > MAX_STUDIES || self.text_size(&next) > STUDY_LIBRARY_MAX_TEXT {
            return Err(CoreError::new(
                "The study library is full. Export and remove a study before saving another.",
            ));
        }
        self.items = next;
        Ok(id)
    }

    /// A study from named PGN chapters; a stored study's chapters keep their ids by place.
    fn save_chapters(
        &mut self,
        raw_name: &str,
        chapters: &[ChapterInput],
        id: Option<&str>,
    ) -> Result<String> {
        let name = clean_name(raw_name);
        let valid = !chapters.is_empty()
            && chapters.len() <= MAX_CHAPTERS
            && chapters
                .iter()
                .all(|c| kchess_domain::tree::valid_pgn(&c.pgn));
        if name.is_empty() || !valid {
            return Err(CoreError::new(
                "Name the study and use up to 64 valid chapters.",
            ));
        }
        let old = id
            .filter(|id| !id.is_empty())
            .and_then(|id| self.find(id).cloned());
        let mut saved = Vec::with_capacity(chapters.len());
        for (index, chapter) in chapters.iter().enumerate() {
            let reused = old
                .as_ref()
                .and_then(|study| study.chapters.get(index))
                .map(|c| c.id.clone());
            saved.push(StudyChapter {
                id: match reused {
                    Some(id) => id,
                    None => random_uuid()?,
                },
                name: js::slice_utf16(&chapter.name, 120).to_string(),
                pgn: chapter.pgn.clone(),
            });
        }
        let study_id = match id {
            Some(id) => id.to_string(),
            None => random_uuid()?,
        };
        self.commit(Study {
            id: study_id,
            name,
            pgn: saved[0].pgn.clone(),
            chapters: saved,
            updated_at: stamp(self.now),
            cloud: old.and_then(|study| study.cloud),
        })
    }
}

/// A non-empty string.
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|s| !s.is_empty())
}

/// An incremental command loads only its target; other studies carry their order and size.
fn read_command_studies(db: &Connection, command: &StudyCommand) -> Result<Vec<Stored>> {
    let offline_id: Option<String> = match command {
        StudyCommand::Offline {
            account, remote, ..
        } => db
            .query_row("SELECT id FROM studies WHERE lower(json_extract(body, '$.cloud.account')) = lower(?1) AND json_extract(body, '$.cloud.id') = ?2 ORDER BY position LIMIT 1",
                params![account, remote.id], |row| row.get(0)).optional().sql()?,
        _ => None,
    };
    let target = offline_id.as_deref().or(command.study_id());
    read_study_rows(db, StudyBodies::Only(target))
}

/// One change to the study library, stored on success.
/// Another window saved the study after this one read it (`docs/shared-core.md`).
pub const STALE_STUDY: &str =
    "This study was changed in another KChess window. Reload it to see the latest version.";

/// A chapter save based on the chapter as the window last read or wrote it (`based_on`, its PGN)
/// is refused once the chapter holds other moves, so two windows never overwrite each other's
/// work. Renames and other chapters do not conflict. A study or chapter removed since then is
/// refused the same way rather than created again: the window keeps its moves as a copy.
/// Answers the study's stored name, which a content save keeps (another window may have renamed
/// it); `None` for a command without a study id.
pub(crate) fn check_based_on(
    db: &Connection,
    command: &StudyCommand,
    based_on: &str,
) -> Result<Option<String>> {
    // A restored study and a restored chapter name theirs inside what they bring back.
    let (id, chapter_id) = match command {
        StudyCommand::Restore { .. } => (None, None),
        StudyCommand::RestoreChapter { id, .. } => (Some(id.as_str()), None),
        _ => (command.study_id(), command.chapter_id()),
    };
    let Some(id) = id.filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    migrate_studies(db)?;
    let study = read_command_studies(db, command)?
        .into_iter()
        .filter_map(Stored::into_study)
        .find(|study| study.id == id);
    let chapter = study
        .as_ref()
        .and_then(|study| match non_empty(chapter_id) {
            Some(chapter_id) => study.chapter(chapter_id),
            None => study.chapters.first(),
        });
    match (&study, chapter) {
        (Some(study), Some(chapter)) if chapter.pgn == based_on => Ok(Some(study.name.clone())),
        _ => Err(CoreError::new(STALE_STUDY).with_kind(ErrorKind::Conflict)),
    }
}

/// The study with its chapters changed in number or order: its PGN is its first chapter's, and
/// a cloud copy is marked as changed in structure.
fn restructured(study: Study, chapters: Vec<StudyChapter>, now: f64) -> Study {
    Study {
        pgn: chapters.first().map(|c| c.pgn.clone()).unwrap_or_default(),
        chapters,
        updated_at: stamp(now),
        cloud: study.cloud.map(|cloud| StudyCloud {
            structure_changed: true,
            ..cloud
        }),
        ..study
    }
}

pub(crate) fn study_command(
    db: &Connection,
    command: &StudyCommand,
    now: f64,
    incremental: bool,
) -> Result<StudyCommandResult> {
    // A restored study is read as a stored one is.
    let restored = match command {
        StudyCommand::Restore { study } => Some(
            decode_study(study).ok_or_else(|| CoreError::new("That study cannot be restored."))?,
        ),
        _ => None,
    };
    migrate_studies(db)?;
    let stored_sizes: Vec<(String, i64)> = db
        .prepare("SELECT id, textUnits FROM studies")
        .sql()?
        .query_map([], |row| Ok((row.get(0)?, row.get::<_, i64>(1)?)))
        .sql()?
        .collect::<std::result::Result<_, _>>()
        .sql()?;
    // A size no study can have (negative, or past one document) is damage: as `usize` it
    // overflowed the sums below, a panic with the database held.
    let sizes = stored_sizes
        .into_iter()
        .map(|(id, units)| {
            usize::try_from(units)
                .ok()
                .filter(|units| *units <= MAX_DOCUMENT)
                .map(|units| (id, units))
                .ok_or_else(|| CoreError::new(DAMAGED_STUDIES))
        })
        .collect::<Result<HashMap<String, usize>>>()?;
    let original: Vec<Stored> = if incremental {
        read_command_studies(db, command)?
    } else {
        read_studies(db)?.into_iter().map(Stored::Read).collect()
    };
    let mut studies = Studies {
        items: original.clone(),
        now,
        sizes,
    };
    let mut result = StudyCommandResult::default();
    match command {
        StudyCommand::SaveChapters { name, chapters, id } => {
            result.id = Some(studies.save_chapters(name, chapters, id.as_deref())?);
        }
        StudyCommand::Save {
            name,
            pgn,
            id,
            chapter_id,
        } => {
            let name = clean_name(name);
            if name.is_empty() {
                return Err(CoreError::new("Name the study."));
            }
            let Some(old) = non_empty(id.as_deref()).and_then(|id| studies.find(id).cloned())
            else {
                let chapter = ChapterInput {
                    name: name.clone(),
                    pgn: pgn.clone(),
                };
                result.id = Some(studies.save_chapters(&name, &[chapter], id.as_deref())?);
                return finish_studies(db, studies, result, &original, incremental);
            };
            let index = match non_empty(chapter_id.as_deref()) {
                Some(chapter_id) => old.chapters.iter().position(|c| c.id == chapter_id),
                None => Some(0),
            };
            let Some(index) = index.filter(|_| kchess_domain::tree::valid_pgn(pgn)) else {
                return Err(CoreError::new("That chapter cannot be saved."));
            };
            let mut chapters = old.chapters.clone();
            chapters[index].pgn = pgn.clone();
            let candidate = Study {
                name,
                pgn: chapters[0].pgn.clone(),
                chapters,
                updated_at: stamp(now),
                ..old
            };
            result.id = Some(studies.commit(candidate)?);
        }
        StudyCommand::AddChapter { id, name } => {
            let chapter_name = js::trim(name).to_string();
            let Some(study) = studies.find(id).filter(|_| !chapter_name.is_empty()) else {
                return Err(CoreError::new("Name the new chapter."));
            };
            let (study_id, study_name) = (study.id.clone(), study.name.clone());
            let mut chapters = inputs(&study.chapters);
            chapters.push(ChapterInput {
                name: chapter_name,
                pgn: "*".into(),
            });
            studies.save_chapters(&study_name, &chapters, Some(&study_id))?;
            result.id = studies
                .find(&study_id)
                .and_then(|saved| saved.chapters.last())
                .map(|chapter| chapter.id.clone());
        }
        StudyCommand::RenameChapter {
            id,
            chapter_id,
            name,
        } => {
            if let Some(study) = studies.find(id).cloned()
                && !js::trim(name).is_empty()
            {
                let new_name = clean_name(name);
                let mut chapters = study.chapters.clone();
                for chapter in chapters.iter_mut().filter(|c| c.id == *chapter_id) {
                    chapter.name = new_name.clone();
                }
                studies.commit(Study {
                    chapters,
                    updated_at: stamp(now),
                    ..study
                })?;
            }
        }
        StudyCommand::DuplicateChapter { id, chapter_id } => {
            let Some((study, original)) = studies
                .find(id)
                .and_then(|study| Some((study.clone(), study.chapter(chapter_id)?.clone())))
            else {
                return Err(CoreError::new("That chapter no longer exists."));
            };
            if study.chapters.len() >= MAX_CHAPTERS {
                return Err(CoreError::new("A study can contain up to 64 chapters."));
            }
            let copy_id = random_uuid()?;
            let mut chapters = study.chapters.clone();
            chapters.push(StudyChapter {
                id: copy_id.clone(),
                name: format!("{} (copy)", js::slice_utf16(&original.name, 113)),
                pgn: original.pgn,
            });
            studies.commit(Study {
                chapters,
                updated_at: stamp(now),
                ..study
            })?;
            result.id = Some(copy_id);
        }
        StudyCommand::RemoveChapter { id, chapter_id } => {
            let Some((study, index)) = studies.find(id).and_then(|study| {
                let index = study.chapters.iter().position(|c| c.id == *chapter_id)?;
                Some((study.clone(), index))
            }) else {
                return Err(CoreError::new("That chapter no longer exists."));
            };
            if study.chapters.len() == 1 {
                return Err(CoreError::new("A study must keep at least one chapter."));
            }
            let mut chapters = study.chapters.clone();
            chapters.retain(|c| c.id != *chapter_id);
            let pick = index.min(chapters.len() - 1);
            result.id = Some(chapters[pick].id.clone());
            studies.commit(restructured(study, chapters, now))?;
        }
        StudyCommand::MoveChapter {
            id,
            chapter_id,
            index,
        } => {
            let Some(study) = studies.find(id).cloned() else {
                return Err(CoreError::new("That study no longer exists."));
            };
            let mut chapters = study.chapters.clone();
            let Some(from) = chapters.iter().position(|c| c.id == *chapter_id) else {
                return Err(CoreError::new("That chapter no longer exists."));
            };
            let to = (*index as usize).min(chapters.len() - 1);
            if from != to {
                let moved = chapters.remove(from);
                chapters.insert(to, moved);
                studies.commit(restructured(study, chapters, now))?;
            }
            result.id = Some(chapter_id.clone());
        }
        StudyCommand::RestoreChapter { id, chapter, index } => {
            let Some(study) = studies.find(id).cloned() else {
                return Err(CoreError::new("That study no longer exists."));
            };
            if study.chapter(&chapter.id).is_none() {
                // The chapter comes back from the app: it must still read as a study's chapter.
                if study.chapters.len() >= MAX_CHAPTERS
                    || !kchess_domain::tree::valid_pgn(&chapter.pgn)
                {
                    return Err(CoreError::new("That chapter cannot be restored."));
                }
                let mut chapters = study.chapters.clone();
                chapters.insert((*index as usize).min(chapters.len()), chapter.clone());
                studies.commit(restructured(study, chapters, now))?;
            }
            result.id = Some(chapter.id.clone());
        }
        StudyCommand::MarkCloud {
            id,
            account,
            remote_id,
            baseline,
        } => {
            let cloud = StudyCloud {
                account: account.clone(),
                id: remote_id.clone(),
                downloaded_pgn: baseline.clone(),
                structure_changed: false,
            };
            mark_cloud(&mut studies, id, cloud);
        }
        StudyCommand::Offline {
            account,
            remote,
            chapters,
        } => offline(&mut studies, account, remote, chapters, &mut result)?,
        StudyCommand::Remove { id } => {
            result.removed = studies.find(id).cloned();
            studies.items.retain(|item| item.id() != id);
        }
        StudyCommand::Restore { .. } => {
            if let Some(study) = restored
                && !studies.items.iter().any(|item| item.id() == study.id)
            {
                studies.commit(study)?;
            }
        }
        StudyCommand::Rename { id, name } => {
            if let Some(study) = studies.find(id).cloned()
                && !js::trim(name).is_empty()
            {
                studies.commit(Study {
                    name: clean_name(name),
                    updated_at: stamp(now),
                    ..study
                })?;
            }
        }
        StudyCommand::Duplicate { id } => {
            if let Some(study) = studies.find(id) {
                let name = format!("{} (copy)", js::slice_utf16(&study.name, 113));
                let chapters = inputs(&study.chapters);
                result.id = Some(studies.save_chapters(&name, &chapters, None)?);
            }
        }
    }
    finish_studies(db, studies, result, &original, incremental)
}

/// The stored study `id` linked to a Lichess study, in its place.
fn mark_cloud(studies: &mut Studies, id: &str, cloud: StudyCloud) {
    for item in studies.items.iter_mut() {
        if let Stored::Read(study) = item
            && study.id == id
        {
            study.cloud = Some(cloud.clone());
        }
    }
}

/// A cloud copy of a study, kept beside a changed local one.
fn offline(
    studies: &mut Studies,
    account: &str,
    remote: &RemoteStudy,
    chapters: &[ChapterInput],
    result: &mut StudyCommandResult,
) -> Result<()> {
    let old = studies
        .items
        .iter()
        .filter_map(Stored::study)
        .find(|study| {
            study.cloud.as_ref().is_some_and(|cloud| {
                cloud.account.to_lowercase() == account.to_lowercase() && cloud.id == remote.id
            })
        })
        .cloned();
    let conflict = match &old {
        Some(study) => {
            let downloaded = study
                .cloud
                .as_ref()
                .map(|cloud| cloud.downloaded_pgn.as_str())
                .unwrap_or_default();
            !study_matches(study, downloaded)?
        }
        None => false,
    };
    let name = if conflict {
        format!("{} (cloud copy)", remote.name)
    } else {
        remote.name.clone()
    };
    let old_id = old.filter(|_| !conflict).map(|study| study.id);
    let id = studies.save_chapters(&name, chapters, old_id.as_deref())?;
    let downloaded = chapters
        .iter()
        .map(|c| c.pgn.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let cloud = StudyCloud {
        account: account.to_string(),
        id: remote.id.clone(),
        downloaded_pgn: downloaded,
        structure_changed: false,
    };
    mark_cloud(studies, &id, cloud);
    result.id = Some(id);
    result.conflict = Some(conflict);
    Ok(())
}

/// Whether a stored study's chapters match the downloaded PGN; a chapter that holds no game is an error.
fn study_matches(study: &Study, downloaded: &str) -> Result<bool> {
    let pairs: Vec<(&str, &str)> = study
        .chapters
        .iter()
        .map(|c| (c.name.as_str(), c.pgn.as_str()))
        .collect();
    kchess_domain::pgn::study_matches(&pairs, downloaded)
        .ok_or_else(|| CoreError::new("A study chapter holds no game."))
}

/// Atomically update changed rows and ordering, then return a snapshot or an incremental response.
fn finish_studies(
    db: &Connection,
    studies: Studies,
    mut result: StudyCommandResult,
    original: &[Stored],
    incremental: bool,
) -> Result<StudyCommandResult> {
    // Serialize only changed studies; unrelated documents keep their bodies and row identities.
    let old: HashMap<&str, &Stored> = original.iter().map(|item| (item.id(), item)).collect();
    let changed: Vec<&Study> = studies
        .items
        .iter()
        .filter(|item| old.get(item.id()).copied() != Some(*item))
        .filter_map(Stored::study)
        .collect();
    let order: Vec<String> = studies
        .items
        .iter()
        .map(|item| item.id().to_string())
        .collect();
    let answer = |changed: Vec<&Study>, items: &[Stored]| -> Vec<Study> {
        if incremental {
            changed.into_iter().cloned().collect()
        } else {
            items.iter().filter_map(Stored::study).cloned().collect()
        }
    };
    // A command that changed nothing (empty rename, missing target) must not take the write
    // lock: skip sizing work and the transaction entirely. `migrate_studies` already ensured the
    // marker, so there is nothing to write.
    if studies.items.as_slice() == original {
        result.studies = answer(changed, &studies.items);
        result.order = incremental.then_some(order);
        return Ok(result);
    }
    // Encode each full study once; unread ones reuse their persisted size. The map backs both
    // the size guard and the row writes below.
    let encoded: HashMap<&str, (String, i64)> = studies
        .items
        .iter()
        .filter_map(Stored::study)
        .map(|study| {
            let body = study_text(study);
            let units = utf16_len(&body) as i64;
            (study.id.as_str(), (body, units))
        })
        .collect();
    let mut text_units: usize = 2 + studies.items.len().saturating_sub(1);
    for item in studies.items.iter() {
        text_units += match item {
            Stored::Read(study) => encoded
                .get(study.id.as_str())
                .map_or(0, |(_, units)| *units as usize),
            Stored::Unread(id) => studies.sizes.get(id).copied().unwrap_or(0),
        };
    }
    let overhead = utf16_len("{\"version\":2,\"items\":}");
    if overhead + text_units > MAX_DOCUMENT {
        return Err(CoreError::new(
            "This study is too large to save automatically. Export a PGN copy.",
        ));
    }
    let changed_ids: HashSet<&str> = changed.iter().map(|study| study.id.as_str()).collect();
    transaction(db, || {
        if read_document_text(db, STUDIES)?.is_none() {
            write_document(db, STUDIES, &json!({ "version": 3 }), "")?;
        }
        for item in original {
            if !studies.items.iter().any(|next| next.id() == item.id()) {
                db.execute("DELETE FROM studies WHERE id = ?1", params![item.id()])
                    .sql()?;
            }
        }
        for (position, item) in studies.items.iter().enumerate() {
            match encoded.get(item.id()) {
                Some((body, units)) if changed_ids.contains(item.id()) => {
                    db.execute("INSERT INTO studies (id, body, position, textUnits) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(id) DO UPDATE SET body = excluded.body, position = excluded.position, textUnits = excluded.textUnits",
                        params![item.id(), body, position as i64, units]).sql()?;
                }
                _ => {
                    db.execute(
                        "UPDATE studies SET position = ?2 WHERE id = ?1 AND position != ?2",
                        params![item.id(), position as i64],
                    )
                    .sql()?;
                }
            }
        }
        Ok(())
    })?;
    result.studies = answer(changed, &studies.items);
    result.order = incremental.then_some(order);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn database() -> Connection {
        let db = Connection::open_in_memory().expect("database");
        super::super::migrations::migrate(&db).expect("migrate");
        db
    }

    /// A command as the window sends it, and the answer as the window reads it.
    fn run(db: &Connection, command: Value) -> Result<Value> {
        let command: StudyCommand = serde_json::from_value(command).expect("command");
        study_command(db, &command, 5.0, true).map(|answer| json!(answer))
    }

    /// Every stored study, read whole.
    fn all(db: &Connection) -> Vec<Value> {
        migrate_studies(db).expect("migrate");
        read_studies(db)
            .expect("studies")
            .into_iter()
            .map(|study| json!(study))
            .collect()
    }

    fn study(db: &Connection, id: &str) -> Value {
        all(db)
            .into_iter()
            .find(|study| study["id"] == id)
            .expect("study")
    }

    fn chapter_names(study: &Value) -> Vec<String> {
        study["chapters"]
            .as_array()
            .expect("chapters")
            .iter()
            .map(|c| c["name"].as_str().expect("name").to_string())
            .collect()
    }

    fn create(db: &Connection, name: &str, pgn: &str) -> String {
        run(db, json!({ "op": "save", "name": name, "pgn": pgn })).expect("create")["id"]
            .as_str()
            .expect("id")
            .to_string()
    }

    /// A stored size of -1 became 2^64 - 1 as `usize`, and adding it to the library's size
    /// overflowed: a panic (release builds check overflow) with the database lock held.
    #[test]
    fn an_impossible_stored_size_is_damage_not_a_panic() {
        let db = database();
        let id = create(&db, "Openings", "1. e4 *");
        create(&db, "Endgames", "1. d4 *");
        for units in [-1_i64, i64::MAX] {
            db.execute(
                "UPDATE studies SET textUnits = ?1 WHERE id = ?2",
                params![units, id],
            )
            .expect("damage");
            let error = run(
                &db,
                json!({ "op": "save", "name": "Third", "pgn": "1. c4 *" }),
            )
            .expect_err("refused");
            assert_eq!(error.message, DAMAGED_STUDIES, "{units}");
        }
    }

    #[test]
    fn saving_creates_then_updates_the_chosen_chapter() {
        let db = database();
        let id = create(&db, "  Openings  ", "1. e4 *");
        let saved = study(&db, &id);
        assert_eq!(saved["name"], json!("Openings"));
        assert_eq!(saved["pgn"], json!("1. e4 *"));
        let second = run(
            &db,
            json!({ "op": "addChapter", "id": id, "name": " Sicilian " }),
        )
        .expect("add")["id"]
            .clone();
        run(
            &db,
            json!({ "op": "save", "id": id, "name": "Repertoire", "chapterId": second, "pgn": "1. e4 c5 *" }),
        )
        .expect("save the second chapter");
        let saved = study(&db, &id);
        assert_eq!(saved["name"], json!("Repertoire"));
        assert_eq!(chapter_names(&saved), ["Openings", "Sicilian"]);
        assert_eq!(saved["chapters"][1]["pgn"], json!("1. e4 c5 *"));
        // The study's own PGN is its first chapter's.
        assert_eq!(saved["pgn"], json!("1. e4 *"));
        assert_eq!(saved["updatedAt"].as_f64(), Some(5.0));

        for (command, message) in [
            (
                json!({ "op": "save", "id": id, "name": "  ", "pgn": "*" }),
                "Name the study.",
            ),
            (
                json!({ "op": "save", "id": id, "name": "R", "chapterId": "missing", "pgn": "*" }),
                "That chapter cannot be saved.",
            ),
            (
                json!({ "op": "save", "id": id, "name": "R", "pgn": "1. e5 *" }),
                "That chapter cannot be saved.",
            ),
            (
                json!({ "op": "addChapter", "id": "missing", "name": "X" }),
                "Name the new chapter.",
            ),
            (
                json!({ "op": "addChapter", "id": id, "name": "  " }),
                "Name the new chapter.",
            ),
        ] {
            assert_eq!(run(&db, command).expect_err("refused").message, message);
        }
    }

    /// Undo puts a removed chapter back where it was, once, and the study's PGN is its first
    /// chapter's again.
    #[test]
    fn a_removed_chapter_is_restored_at_its_place() {
        let db = database();
        let id = create(&db, "Endgames", "1. e4 *");
        let first = study(&db, &id)["chapters"][0]["id"].clone();
        let copy = run(
            &db,
            json!({ "op": "duplicateChapter", "id": id, "chapterId": first }),
        )
        .expect("duplicate")["id"]
            .clone();
        let removed = study(&db, &id)["chapters"][0].clone();
        run(
            &db,
            json!({ "op": "removeChapter", "id": id, "chapterId": first }),
        )
        .expect("remove");
        assert_eq!(study(&db, &id)["chapters"][0]["id"], copy);

        let restore = json!({ "op": "restoreChapter", "id": id, "chapter": removed, "index": 0 });
        let result = run(&db, restore.clone()).expect("restore");
        assert_eq!(result["id"], first);
        let restored = study(&db, &id);
        assert_eq!(restored["chapters"][0], removed);
        assert_eq!(restored["chapters"][1]["id"], copy);
        assert_eq!(restored["pgn"], removed["pgn"]);
        // Restoring again changes nothing.
        run(&db, restore).expect("restore again");
        assert_eq!(
            study(&db, &id)["chapters"].as_array().map(Vec::len),
            Some(2)
        );

        let mut broken = removed.clone();
        broken["id"] = json!("other");
        broken["pgn"] = json!("1. e5 *");
        assert_eq!(
            run(
                &db,
                json!({ "op": "restoreChapter", "id": id, "chapter": broken, "index": 0 })
            )
            .expect_err("refused")
            .message,
            "That chapter cannot be restored."
        );
        let gone =
            json!({ "op": "restoreChapter", "id": "missing", "chapter": removed, "index": 0 });
        assert_eq!(
            run(&db, gone).expect_err("refused").message,
            "That study no longer exists."
        );
    }

    /// A chapter moves to another place; the study's PGN is its first chapter's.
    #[test]
    fn a_chapter_moves_to_another_place() {
        let db = database();
        let id = create(&db, "Endgames", "1. e4 *");
        let first = study(&db, &id)["chapters"][0]["id"].clone();
        let copy = run(
            &db,
            json!({ "op": "duplicateChapter", "id": id, "chapterId": first }),
        )
        .expect("duplicate")["id"]
            .clone();
        run(
            &db,
            json!({ "op": "renameChapter", "id": id, "chapterId": copy, "name": "Second" }),
        )
        .expect("rename");
        let result = run(
            &db,
            json!({ "op": "moveChapter", "id": id, "chapterId": copy, "index": 0 }),
        )
        .expect("move");
        assert_eq!(result["id"], copy);
        let moved = study(&db, &id);
        assert_eq!(moved["chapters"][0]["id"], copy);
        assert_eq!(moved["chapters"][1]["id"], first);
        assert_eq!(moved["pgn"], moved["chapters"][0]["pgn"]);
        // A place past the end is the last one.
        run(
            &db,
            json!({ "op": "moveChapter", "id": id, "chapterId": copy, "index": 9 }),
        )
        .expect("move to the end");
        assert_eq!(study(&db, &id)["chapters"][1]["id"], copy);
        assert_eq!(
            run(
                &db,
                json!({ "op": "moveChapter", "id": id, "chapterId": "gone", "index": 0 })
            )
            .expect_err("refused")
            .message,
            "That chapter no longer exists."
        );
    }

    #[test]
    fn chapters_are_renamed_duplicated_and_removed() {
        let db = database();
        let id = create(&db, "Endgames", "1. e4 *");
        let first = study(&db, &id)["chapters"][0]["id"].clone();
        let copy = run(
            &db,
            json!({ "op": "duplicateChapter", "id": id, "chapterId": first }),
        )
        .expect("duplicate")["id"]
            .clone();
        run(
            &db,
            json!({ "op": "renameChapter", "id": id, "chapterId": copy, "name": " Rook endings " }),
        )
        .expect("rename");
        // An empty name changes nothing.
        run(
            &db,
            json!({ "op": "renameChapter", "id": id, "chapterId": copy, "name": " " }),
        )
        .expect("empty rename");
        let saved = study(&db, &id);
        assert_eq!(chapter_names(&saved), ["Endgames", "Rook endings"]);
        assert_eq!(saved["chapters"][1]["pgn"], json!("1. e4 *"));

        // Removing the first chapter selects the one now in its place, and the study's PGN follows.
        let next = run(
            &db,
            json!({ "op": "removeChapter", "id": id, "chapterId": first }),
        )
        .expect("remove")["id"]
            .clone();
        assert_eq!(next, copy);
        assert_eq!(chapter_names(&study(&db, &id)), ["Rook endings"]);
        for (command, message) in [
            (
                json!({ "op": "removeChapter", "id": id, "chapterId": copy }),
                "A study must keep at least one chapter.",
            ),
            (
                json!({ "op": "removeChapter", "id": id, "chapterId": "gone" }),
                "That chapter no longer exists.",
            ),
            (
                json!({ "op": "duplicateChapter", "id": id, "chapterId": "gone" }),
                "That chapter no longer exists.",
            ),
        ] {
            assert_eq!(run(&db, command).expect_err("refused").message, message);
        }
    }

    #[test]
    fn a_study_holds_at_most_its_chapter_limit() {
        let db = database();
        let chapters: Vec<Value> = (0..MAX_CHAPTERS)
            .map(|i| json!({ "name": format!("C{i}"), "pgn": "*" }))
            .collect();
        let id = run(
            &db,
            json!({ "op": "saveChapters", "name": "Full", "chapters": chapters }),
        )
        .expect("save chapters")["id"]
            .as_str()
            .expect("id")
            .to_string();
        let first = study(&db, &id)["chapters"][0]["id"].clone();
        assert_eq!(
            run(
                &db,
                json!({ "op": "duplicateChapter", "id": id, "chapterId": first })
            )
            .expect_err("full")
            .message,
            "A study can contain up to 64 chapters."
        );
    }

    #[test]
    fn studies_are_renamed_duplicated_removed_and_restored() {
        let db = database();
        let a = create(&db, "A", "1. e4 *");
        let b = create(&db, "B", "1. d4 *");
        run(&db, json!({ "op": "rename", "id": a, "name": " Alpha " })).expect("rename");
        run(&db, json!({ "op": "rename", "id": a, "name": "" })).expect("empty rename");
        run(&db, json!({ "op": "rename", "id": "missing", "name": "X" })).expect("missing");
        assert_eq!(study(&db, &a)["name"], json!("Alpha"));

        let copy = run(&db, json!({ "op": "duplicate", "id": b })).expect("duplicate")["id"]
            .as_str()
            .expect("id")
            .to_string();
        assert_eq!(study(&db, &copy)["name"], json!("B (copy)"));
        assert_eq!(study(&db, &copy)["pgn"], json!("1. d4 *"));
        assert!(
            run(&db, json!({ "op": "duplicate", "id": "missing" }))
                .expect("missing")
                .get("id")
                .is_none()
        );

        let removed =
            run(&db, json!({ "op": "remove", "id": b })).expect("remove")["removed"].clone();
        assert_eq!(removed["name"], json!("B"));
        assert_eq!(all(&db).len(), 2);
        // Restoring brings it back; restoring a study that is present changes nothing.
        run(&db, json!({ "op": "restore", "study": removed })).expect("restore");
        run(&db, json!({ "op": "restore", "study": study(&db, &a) })).expect("present");
        let ids: Vec<Value> = all(&db).iter().map(|s| s["id"].clone()).collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&json!(b)));

        // An incremental answer carries only what changed, and the whole order.
        let answer = run(&db, json!({ "op": "rename", "id": a, "name": "Again" })).expect("rename");
        assert_eq!(answer["studies"].as_array().map(Vec::len), Some(1));
        assert_eq!(answer["order"].as_array().map(Vec::len), Some(3));
        let unchanged = run(&db, json!({ "op": "rename", "id": "missing", "name": "X" }))
            .expect("a study no longer there");
        assert_eq!(unchanged["studies"], json!([]));
    }

    #[test]
    fn a_cloud_copy_replaces_an_unchanged_study_and_sits_beside_a_changed_one() {
        let db = database();
        let chapters = json!([{ "name": "One", "pgn": "1. e4 *" }]);
        let remote = json!({ "id": "abcdEFGH", "name": "Cloud" });
        let first = run(
            &db,
            json!({ "op": "offline", "account": "Alice", "remote": remote, "chapters": chapters }),
        )
        .expect("offline");
        assert_eq!(first["conflict"], json!(false));
        let id = first["id"].as_str().expect("id").to_string();
        let stored = study(&db, &id);
        assert_eq!(stored["cloud"]["account"], json!("Alice"));
        assert_eq!(stored["cloud"]["downloadedPgn"], json!("1. e4 *"));

        // Unchanged since the download: the next download replaces it in place.
        let newer = json!([{ "name": "One", "pgn": "1. e4 e5 *" }]);
        let again = run(
            &db,
            json!({ "op": "offline", "account": "alice", "remote": remote, "chapters": newer }),
        )
        .expect("offline again");
        assert_eq!(
            (again["id"].as_str(), again["conflict"].clone()),
            (Some(id.as_str()), json!(false))
        );

        // Changed locally: the download is kept as a copy beside it.
        run(
            &db,
            json!({ "op": "save", "id": id, "name": "Cloud", "pgn": "1. d4 *" }),
        )
        .expect("edit");
        let conflict = run(
            &db,
            json!({ "op": "offline", "account": "Alice", "remote": remote, "chapters": newer }),
        )
        .expect("offline conflict");
        assert_eq!(conflict["conflict"], json!(true));
        assert_ne!(conflict["id"], json!(id));
        assert_eq!(
            study(&db, conflict["id"].as_str().expect("id"))["name"],
            json!("Cloud (cloud copy)")
        );

        // Marking a local study links it to a cloud study; removing a chapter flags the change.
        let local = create(&db, "Local", "1. c4 *");
        run(
            &db,
            json!({ "op": "markCloud", "id": local, "account": "Alice", "remoteId": "wxyzWXYZ", "baseline": "1. c4 *" }),
        )
        .expect("mark");
        assert_eq!(study(&db, &local)["cloud"]["id"], json!("wxyzWXYZ"));
        run(
            &db,
            json!({ "op": "addChapter", "id": local, "name": "Two" }),
        )
        .expect("add");
        let first_chapter = study(&db, &local)["chapters"][0]["id"].clone();
        run(
            &db,
            json!({ "op": "removeChapter", "id": local, "chapterId": first_chapter }),
        )
        .expect("remove");
        assert_eq!(study(&db, &local)["cloud"]["structureChanged"], json!(true));
    }

    /// A download replaces only the study of the same account and the same cloud study: another
    /// cloud study of the account, or the same one of another account, is a study of its own.
    #[test]
    fn a_cloud_copy_belongs_to_its_account_and_cloud_study() {
        let db = database();
        let chapters = json!([{ "name": "One", "pgn": "1. e4 *" }]);
        let download = |account: &str, remote: &str| -> String {
            let remote = json!({ "id": remote, "name": format!("{account} {remote}") });
            run(
                &db,
                json!({ "op": "offline", "account": account, "remote": remote, "chapters": chapters }),
            )
            .expect("offline")["id"]
                .as_str()
                .expect("id")
                .to_string()
        };
        let first = download("Alice", "abcdEFGH");
        let other_study = download("Alice", "wxyzWXYZ");
        let other_account = download("Bob", "abcdEFGH");
        assert_ne!(other_study, first);
        assert_ne!(other_account, first);
        assert_ne!(other_account, other_study);
        let mut names: Vec<Value> = all(&db).iter().map(|s| s["name"].clone()).collect();
        names.sort_by_key(|name| name.to_string());
        assert_eq!(
            names,
            [
                json!("Alice abcdEFGH"),
                json!("Alice wxyzWXYZ"),
                json!("Bob abcdEFGH")
            ]
        );
    }

    /// A command that reads every study writes back only the studies it changed: another
    /// study's row keeps its text as it was stored.
    #[test]
    fn a_full_read_writes_back_only_the_changed_study() {
        let db = database();
        let kept = create(&db, "Kept", "1. e4 *");
        let renamed = create(&db, "Renamed", "1. d4 *");
        let body = |id: &str| -> String {
            db.query_row(
                "SELECT body FROM studies WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("body")
        };
        // The same study, stored with other spacing than the core writes.
        let parsed: Value = serde_json::from_str(&body(&kept)).expect("json");
        let spaced = serde_json::to_string_pretty(&parsed).expect("text");
        db.execute(
            "UPDATE studies SET body = ?1 WHERE id = ?2",
            params![spaced, kept],
        )
        .expect("respace");
        let command: StudyCommand =
            serde_json::from_value(json!({ "op": "rename", "id": renamed, "name": "Other" }))
                .expect("command");
        study_command(&db, &command, 5.0, false).expect("rename");
        assert_eq!(body(&kept), spaced);
        assert_eq!(study(&db, &renamed)["name"], json!("Other"));
    }

    /// The studies are one document of at most `MAX_DOCUMENT` text units, their separators and
    /// brackets included: a save that fills it exactly is kept, one unit more is refused.
    #[test]
    fn the_library_fills_its_document_to_the_last_unit() {
        let db = database();
        let big = create(&db, "Big", "1. e4 *");
        let small = create(&db, "Small", "1. d4 *");
        let units = |id: &str| -> usize {
            db.query_row(
                "SELECT textUnits FROM studies WHERE id = ?1",
                params![id],
                |row| row.get::<_, i64>(0),
            )
            .expect("units") as usize
        };
        // Linking a study to the cloud is the one change the library's own (lower) limit does
        // not check first; each link here stores a baseline of the same length.
        let link = |baseline: &str| {
            run(
                &db,
                json!({ "op": "markCloud", "id": small, "account": "Alice", "remoteId": "wxyzWXYZ", "baseline": baseline }),
            )
        };
        link("Small 1").expect("link");
        // The big study is not read by a change of the other: its stored size counts.
        let overhead = utf16_len("{\"version\":2,\"items\":}");
        let fits = MAX_DOCUMENT - overhead - 3 - units(&small);
        for (size, kept) in [(fits, true), (fits + 1, false)] {
            db.execute(
                "UPDATE studies SET textUnits = ?1 WHERE id = ?2",
                params![size as i64, big],
            )
            .expect("size");
            let answer = link(if kept { "Small 2" } else { "Small 3" });
            match answer {
                Ok(_) => assert!(kept, "{size} units are past the document"),
                Err(cause) => {
                    assert!(!kept, "{size} units fit: {}", cause.message);
                    assert_eq!(
                        cause.message,
                        "This study is too large to save automatically. Export a PGN copy."
                    );
                }
            }
        }
    }
}
