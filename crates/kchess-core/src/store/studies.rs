//! Studies: the `Studies` working set, `studyCommand` and its storage (`studies` table).
//! Split from the library store; bodies are unchanged.

use kchess_contracts::types::{ChapterInput, RemoteStudy, StudyCommand, StudyCommandResult};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use super::documents::{DAMAGED_STUDIES, read_document_text, write_document};
use super::json_codec::{clean_name, js_json};
use super::library::{STUDIES, STUDY_LIBRARY_MAX_TEXT, object};
use super::library_import::{StudyBodies, migrate_studies, read_studies, read_study_rows};
use super::sql::{Sql as _, transaction};
use crate::error::{CoreError, Result};
use crate::ids::random_uuid;
use kchess_domain::js;
use kchess_domain::js::{number_to_json as num, truthy, utf16_len};
use kchess_domain::library::{MAX_CHAPTERS, MAX_STUDIES};
use kchess_domain::misc::MAX_DOCUMENT;

/* ── Studies ── */

pub(crate) fn id_of(value: &Value) -> Option<&str> {
    value.get("id").and_then(Value::as_str)
}

/// `{ ...base, ...updates }`: keys keep their place; new keys follow in the order given.
pub(crate) fn spread(base: &Map<String, Value>, updates: Vec<(&str, Value)>) -> Map<String, Value> {
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
    sizes: std::collections::HashMap<String, usize>,
}

impl Studies {
    /// Unloaded studies contain only their ID; their persisted size avoids reading their PGNs.
    fn text_size(&self, items: &[Value]) -> usize {
        2 + items.len().saturating_sub(1)
            + items
                .iter()
                .map(|study| {
                    if study.as_object().is_some_and(|item| item.len() == 1) {
                        self.sizes
                            .get(id_of(study).unwrap_or_default())
                            .copied()
                            .unwrap_or(0)
                    } else {
                        utf16_len(&js_json(study))
                    }
                })
                .sum::<usize>()
    }
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
        if next.len() > MAX_STUDIES || self.text_size(&next) > STUDY_LIBRARY_MAX_TEXT {
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
                .map(|n| js::slice_utf16(n, 120))
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

/// An incremental command loads only its target; other studies carry their order and size.
fn read_command_studies(db: &Connection, command: &StudyCommand) -> Result<Vec<Value>> {
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

/// `studyCommand(command, now)`: one change to the study library, stored on success.
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
        .find(|study| study.get("id").and_then(Value::as_str) == Some(id));
    let chapters = study
        .as_ref()
        .and_then(|study| study.get("chapters"))
        .and_then(Value::as_array);
    let chapter = match non_empty(chapter_id) {
        Some(chapter_id) => chapters.and_then(|chapters| {
            chapters
                .iter()
                .find(|c| c.get("id").and_then(Value::as_str) == Some(chapter_id))
        }),
        None => chapters.and_then(|chapters| chapters.first()),
    };
    match chapter
        .and_then(|chapter| chapter.get("pgn"))
        .and_then(Value::as_str)
    {
        Some(current) if current == based_on => Ok(study
            .as_ref()
            .and_then(|study| study.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string)),
        _ => Err(CoreError::new(STALE_STUDY)),
    }
}

/// Chapters as `saveChapters` reads them.
fn chapter_list(chapters: &[ChapterInput]) -> Result<Value> {
    serde_json::to_value(chapters).map_err(|cause| CoreError::new(cause.to_string()))
}

/// `{ ...study, ...updates }` with the chapters changed: the first chapter's PGN is the study's,
/// and a cloud copy is marked as changed in structure.
fn restructured(
    study: &Map<String, Value>,
    chapters: Vec<Value>,
    now: f64,
) -> Result<Map<String, Value>> {
    let first = chapters
        .first()
        .and_then(|c| c.get("pgn"))
        .cloned()
        .unwrap_or(Value::Null);
    let mut updates = vec![
        ("chapters", Value::Array(chapters)),
        ("pgn", first),
        ("updatedAt", num(now)),
    ];
    if let Some(cloud) = study.get("cloud").filter(|c| truthy(Some(c))) {
        let mut changed = object(cloud, "cloud")?.clone();
        changed.insert("structureChanged".into(), Value::Bool(true));
        updates.push(("cloud", Value::Object(changed)));
    }
    Ok(spread(study, updates))
}

/// A study's chapters, or none.
fn chapters_of(study: &Map<String, Value>) -> Vec<Value> {
    study
        .get("chapters")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn study_command(
    db: &Connection,
    command: &StudyCommand,
    now: f64,
    incremental: bool,
) -> Result<StudyCommandResult> {
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
        .collect::<Result<std::collections::HashMap<String, usize>>>()?;
    let original = if incremental {
        read_command_studies(db, command)?
    } else {
        read_studies(db)?
    };
    let mut studies = Studies {
        items: original.clone(),
        now,
        sizes,
    };
    let mut result = StudyCommandResult::default();
    let find = |studies: &Studies, id: &str| studies.find(id).cloned();
    match command {
        StudyCommand::SaveChapters { name, chapters, id } => {
            let id = studies.save_chapters(name, &chapter_list(chapters)?, id.as_deref())?;
            result.id = Some(id);
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
            let Some(old) = non_empty(id.as_deref()).and_then(|id| find(&studies, id)) else {
                let id = studies.save_chapters(
                    &name,
                    &json!([{ "name": name, "pgn": pgn }]),
                    id.as_deref(),
                )?;
                result.id = Some(id);
                return finish_studies(db, studies, result, &original, incremental);
            };
            let old = object(&old, "study")?.clone();
            let chapters = chapters_of(&old);
            let index = match non_empty(chapter_id.as_deref()) {
                Some(chapter_id) => chapters
                    .iter()
                    .position(|c| c.get("id").and_then(Value::as_str) == Some(chapter_id)),
                None => Some(0),
            };
            let Some(index) = index.filter(|_| kchess_domain::tree::valid_pgn(pgn)) else {
                return Err(CoreError::new("That chapter cannot be saved."));
            };
            let mut updated_chapters = Vec::with_capacity(chapters.len());
            for (i, chapter) in chapters.into_iter().enumerate() {
                if i == index {
                    let mut changed = object(&chapter, "chapter")?.clone();
                    changed.insert("pgn".into(), json!(pgn));
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
            result.id = Some(studies.commit(candidate)?);
        }
        StudyCommand::AddChapter { id, name } => {
            let Some(study) = find(&studies, id) else {
                return Err(CoreError::new("Name the new chapter."));
            };
            let chapter_name = js::trim(name).to_string();
            if chapter_name.is_empty() {
                return Err(CoreError::new("Name the new chapter."));
            }
            let study = object(&study, "study")?.clone();
            let mut chapters = chapters_of(&study);
            chapters.push(json!({ "name": chapter_name, "pgn": "*" }));
            let study_name = study
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let study_id = study
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            studies.save_chapters(study_name, &Value::Array(chapters), Some(&study_id))?;
            result.id = studies
                .find(&study_id)
                .and_then(|saved| saved.get("chapters"))
                .and_then(Value::as_array)
                .and_then(|c| c.last())
                .and_then(id_of)
                .map(str::to_string);
        }
        StudyCommand::RenameChapter {
            id,
            chapter_id,
            name,
        } => {
            if let Some(study) = find(&studies, id) {
                if js::trim(name).is_empty() {
                    return finish_studies(db, studies, result, &original, incremental);
                }
                let study = object(&study, "study")?.clone();
                let new_name = clean_name(name);
                let chapters: Vec<Value> = chapters_of(&study)
                    .into_iter()
                    .map(|c| {
                        if id_of(&c) == Some(chapter_id) {
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
        StudyCommand::DuplicateChapter { id, chapter_id } => {
            let study = find(&studies, id);
            let original = study
                .as_ref()
                .and_then(|s| s.get("chapters").and_then(Value::as_array))
                .and_then(|chapters| chapters.iter().find(|c| id_of(c) == Some(chapter_id)))
                .cloned();
            let (Some(study), Some(original)) = (study, original) else {
                return Err(CoreError::new("That chapter no longer exists."));
            };
            let study = object(&study, "study")?.clone();
            let mut chapters = chapters_of(&study);
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
                json!(format!("{} (copy)", js::slice_utf16(original_name, 113))),
            );
            chapters.push(Value::Object(copy));
            studies.commit(spread(
                &study,
                vec![
                    ("chapters", Value::Array(chapters)),
                    ("updatedAt", num(now)),
                ],
            ))?;
            result.id = Some(copy_id);
        }
        StudyCommand::RemoveChapter { id, chapter_id } => {
            let study = find(&studies, id);
            let index = study
                .as_ref()
                .and_then(|s| s.get("chapters").and_then(Value::as_array))
                .and_then(|chapters| chapters.iter().position(|c| id_of(c) == Some(chapter_id)));
            let (Some(study), Some(index)) = (study, index) else {
                return Err(CoreError::new("That chapter no longer exists."));
            };
            let study = object(&study, "study")?.clone();
            let all = chapters_of(&study);
            if all.len() == 1 {
                return Err(CoreError::new("A study must keep at least one chapter."));
            }
            let chapters: Vec<Value> = all
                .into_iter()
                .filter(|c| id_of(c) != Some(chapter_id))
                .collect();
            let pick = index.min(chapters.len() - 1);
            result.id = id_of(&chapters[pick]).map(str::to_string);
            studies.commit(restructured(&study, chapters, now)?)?;
        }
        StudyCommand::MoveChapter {
            id,
            chapter_id,
            index,
        } => {
            let Some(study) = find(&studies, id) else {
                return Err(CoreError::new("That study no longer exists."));
            };
            let study = object(&study, "study")?.clone();
            let mut chapters = chapters_of(&study);
            let Some(from) = chapters.iter().position(|c| id_of(c) == Some(chapter_id)) else {
                return Err(CoreError::new("That chapter no longer exists."));
            };
            let to = (*index as usize).min(chapters.len() - 1);
            if from != to {
                let moved = chapters.remove(from);
                chapters.insert(to, moved);
                studies.commit(restructured(&study, chapters, now)?)?;
            }
            result.id = Some(chapter_id.clone());
        }
        StudyCommand::RestoreChapter { id, chapter, index } => {
            let Some(study) = find(&studies, id) else {
                return Err(CoreError::new("That study no longer exists."));
            };
            let study = object(&study, "study")?.clone();
            let mut chapters = chapters_of(&study);
            if !chapters.iter().any(|c| id_of(c) == Some(&chapter.id)) {
                let restored = serde_json::to_value(chapter)
                    .map_err(|cause| CoreError::new(cause.to_string()))?;
                chapters.insert((*index as usize).min(chapters.len()), restored);
                let candidate = restructured(&study, chapters, now)?;
                // The chapter comes back from the app: it must still read as a study's chapter.
                if kchess_domain::library::decode_study(&Value::Object(candidate.clone())).is_none()
                {
                    return Err(CoreError::new("That chapter cannot be restored."));
                }
                studies.commit(candidate)?;
            }
            result.id = Some(chapter.id.clone());
        }
        StudyCommand::MarkCloud {
            id,
            account,
            remote_id,
            baseline,
        } => {
            if let Some(index) = studies.find_index(id) {
                let cloud = json!({
                    "account": account,
                    "id": remote_id,
                    "downloadedPgn": baseline,
                });
                let study = object(&studies.items[index], "study")?.clone();
                studies.items[index] = Value::Object(spread(&study, vec![("cloud", cloud)]));
            }
        }
        StudyCommand::Offline {
            account,
            remote,
            chapters,
        } => offline(&mut studies, account, remote, chapters, &mut result)?,
        StudyCommand::Remove { id } => {
            result.removed = find(&studies, id);
            studies.items.retain(|item| id_of(item) != Some(id));
        }
        StudyCommand::Restore { study } => {
            let present = id_of(study).is_some_and(|id| studies.find(id).is_some());
            if !present {
                let candidate = object(study, "study")?.clone();
                studies.commit(candidate)?;
            }
        }
        StudyCommand::Rename { id, name } => {
            if let Some(study) = find(&studies, id)
                && !js::trim(name).is_empty()
            {
                let study = object(&study, "study")?.clone();
                let candidate = spread(
                    &study,
                    vec![("name", json!(clean_name(name))), ("updatedAt", num(now))],
                );
                studies.commit(candidate)?;
            }
        }
        StudyCommand::Duplicate { id } => {
            if let Some(study) = find(&studies, id) {
                let study_name = study
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let chapters = study.get("chapters").cloned().unwrap_or(Value::Null);
                let id = studies.save_chapters(
                    &format!("{} (copy)", js::slice_utf16(study_name, 113)),
                    &chapters,
                    None,
                )?;
                result.id = Some(id);
            }
        }
    }
    finish_studies(db, studies, result, &original, incremental)
}

/// `offline`: a cloud copy of a study, kept beside a changed local one.
fn offline(
    studies: &mut Studies,
    account: &str,
    remote: &RemoteStudy,
    chapters: &[ChapterInput],
    result: &mut StudyCommandResult,
) -> Result<()> {
    let remote_name = remote.name.clone();
    let chapters = chapter_list(chapters)?;
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
                    && cloud.get("id").and_then(Value::as_str) == Some(remote.id.as_str())
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
    let cloud = json!({ "account": account, "id": remote.id, "downloadedPgn": downloaded });
    for item in studies.items.iter_mut() {
        if id_of(item) == Some(id.as_str()) {
            let study = object(item, "study")?.clone();
            *item = Value::Object(spread(&study, vec![("cloud", cloud.clone())]));
        }
    }
    result.id = Some(id);
    result.conflict = Some(conflict);
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

/// Atomically update changed rows and ordering, then return a snapshot or an incremental response.
fn finish_studies(
    db: &Connection,
    studies: Studies,
    mut result: StudyCommandResult,
    original: &[Value],
    incremental: bool,
) -> Result<StudyCommandResult> {
    // Serialize only changed studies; unrelated documents keep their bodies and row identities.
    let old: std::collections::HashMap<_, _> =
        original.iter().map(|study| (id_of(study), study)).collect();
    let changed: Vec<Value> = studies
        .items
        .iter()
        .filter(|study| old.get(&id_of(study)).copied() != Some(*study))
        .cloned()
        .collect();
    let order: Vec<String> = studies
        .items
        .iter()
        .map(|study| id_of(study).unwrap_or_default().to_string())
        .collect();
    // A command that changed nothing (unknown op, empty rename, missing target)
    // must not take the write lock: skip sizing work and the transaction entirely.
    // `migrate_studies` already ensured the marker, so there is nothing to write.
    if studies.items.as_slice() == original {
        result.studies = if incremental { changed } else { studies.items };
        result.order = incremental.then_some(order);
        return Ok(result);
    }
    // Encode each full study once; placeholders reuse their persisted size.
    // The map backs both the size guard and the row writes below.
    let mut encoded: std::collections::HashMap<String, (String, i64)> =
        std::collections::HashMap::new();
    for study in studies.items.iter() {
        if study.as_object().is_some_and(|item| item.len() != 1)
            && let Some(id) = id_of(study)
        {
            let body = js_json(study);
            let units = utf16_len(&body) as i64;
            encoded.insert(id.to_string(), (body, units));
        }
    }
    let mut text_units: usize = 2 + studies.items.len().saturating_sub(1);
    for study in studies.items.iter() {
        if study.as_object().is_some_and(|item| item.len() == 1) {
            text_units += studies
                .sizes
                .get(id_of(study).unwrap_or_default())
                .copied()
                .unwrap_or(0);
        } else if let Some((_, units)) = id_of(study).and_then(|id| encoded.get(id)) {
            text_units += *units as usize;
        } else {
            // No id (or unexpectedly unencoded): measure directly rather than undercount.
            text_units += utf16_len(&js_json(study));
        }
    }
    let overhead = utf16_len("{\"version\":2,\"items\":}");
    if overhead + text_units > MAX_DOCUMENT {
        return Err(CoreError::new(
            "This study is too large to save automatically. Export a PGN copy.",
        ));
    }
    let changed_ids: std::collections::HashSet<&str> = changed.iter().filter_map(id_of).collect();
    transaction(db, || {
        if read_document_text(db, STUDIES)?.is_none() {
            write_document(db, STUDIES, &json!({ "version": 3 }), "")?;
        }
        for study in original {
            if !studies.items.iter().any(|next| id_of(next) == id_of(study)) {
                db.execute("DELETE FROM studies WHERE id = ?1", params![id_of(study)])
                    .sql()?;
            }
        }
        for (position, study) in studies.items.iter().enumerate() {
            let changed = id_of(study).is_some_and(|id| changed_ids.contains(id));
            if changed {
                let (body, units) = id_of(study)
                    .and_then(|id| encoded.get(id))
                    .cloned()
                    .unwrap_or_else(|| {
                        let body = js_json(study);
                        let units = utf16_len(&body) as i64;
                        (body, units)
                    });
                db.execute("INSERT INTO studies (id, body, position, textUnits) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(id) DO UPDATE SET body = excluded.body, position = excluded.position, textUnits = excluded.textUnits",
                    params![id_of(study), body, position as i64, units]).sql()?;
            } else {
                db.execute(
                    "UPDATE studies SET position = ?2 WHERE id = ?1 AND position != ?2",
                    params![id_of(study), position as i64],
                )
                .sql()?;
            }
        }
        Ok(())
    })?;
    result.studies = if incremental { changed } else { studies.items };
    result.order = incremental.then_some(order);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        read_studies(db).expect("studies")
    }

    fn study(db: &Connection, id: &str) -> Value {
        all(db)
            .into_iter()
            .find(|study| id_of(study) == Some(id))
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
}
