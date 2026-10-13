//! Studies: the `Studies` working set, `studyCommand` and its storage (`studies` table).
//! Split from the library store; bodies are unchanged.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use super::documents::{DAMAGED_STUDIES, parse_text, read_document_text, write_document};
use super::json_codec::{clean_name, js_json, js_slice, utf16_len};
use super::library::{
    STUDIES, STUDY_LIBRARY_MAX_TEXT, db_err, num, object, random_uuid, transaction, truthy,
};
use super::library_import::{migrate_studies, read_studies};
use crate::error::{CoreError, Result};
use kchess_domain::js;
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

/// An incremental command loads only its target; other studies carry their order and size.
fn read_command_studies(db: &Connection, command: &Map<String, Value>) -> Result<Vec<Value>> {
    let op = command
        .get("op")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let target = if op == "restore" {
        command.get("study").and_then(id_of)
    } else {
        command.get("id").and_then(Value::as_str)
    };
    let offline_id: Option<String> = if op == "offline" {
        let account = command
            .get("account")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let remote = command
            .get("remote")
            .and_then(|remote| remote.get("id"))
            .and_then(Value::as_str);
        db.query_row("SELECT id FROM studies WHERE lower(json_extract(body, '$.cloud.account')) = lower(?1) AND json_extract(body, '$.cloud.id') = ?2 ORDER BY position LIMIT 1",
            params![account, remote], |row| row.get(0)).optional().map_err(db_err)?
    } else {
        None
    };
    let target = offline_id.as_deref().or(target);
    let mut statement = db
        .prepare(
            "SELECT id, CASE WHEN id = ?1 THEN body ELSE NULL END FROM studies ORDER BY position",
        )
        .map_err(db_err)?;
    let rows = statement
        .query_map(params![target], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(db_err)?;
    let mut items = Vec::new();
    for row in rows {
        let (id, body) = row.map_err(db_err)?;
        let study = match body {
            Some(body) => {
                let raw = parse_text(&body).ok_or_else(|| CoreError::new(DAMAGED_STUDIES))?;
                let study = kchess_domain::library::decode_study(&raw)
                    .ok_or_else(|| CoreError::new(DAMAGED_STUDIES))?;
                if id_of(&study) != Some(id.as_str()) {
                    return Err(CoreError::new(DAMAGED_STUDIES));
                }
                study
            }
            None => json!({ "id": id }),
        };
        items.push(study);
    }
    if items.len() > MAX_STUDIES {
        return Err(CoreError::new(DAMAGED_STUDIES));
    }
    Ok(items)
}

/// `studyCommand(command, now)`: one change to the study library, stored on success.
pub(crate) fn study_command(
    db: &Connection,
    command: &Value,
    now: f64,
    incremental: bool,
) -> Result<Value> {
    let command = object(command, "study command")?;
    let op = command
        .get("op")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let field = |key: &str| command.get(key).cloned().unwrap_or(Value::Null);
    // `find(command.id)` takes any id; `command.id ? … : …` only a non-empty one.
    let id_arg = command.get("id").and_then(Value::as_str);

    migrate_studies(db)?;
    let sizes: std::collections::HashMap<String, usize> = db
        .prepare("SELECT id, textUnits FROM studies")
        .map_err(db_err)?
        .query_map([], |row| Ok((row.get(0)?, row.get::<_, i64>(1)? as usize)))
        .map_err(db_err)?
        .collect::<std::result::Result<_, _>>()
        .map_err(db_err)?;
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
                return finish_studies(db, studies, result, &original, incremental);
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
                    return finish_studies(db, studies, result, &original, incremental);
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
    finish_studies(db, studies, result, &original, incremental)
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

/// Atomically update changed rows and ordering, then return a snapshot or an incremental response.
fn finish_studies(
    db: &Connection,
    studies: Studies,
    result: Map<String, Value>,
    original: &[Value],
    incremental: bool,
) -> Result<Value> {
    // Serialize only changed studies; unrelated documents keep their bodies and row identities.
    let old: std::collections::HashMap<_, _> =
        original.iter().map(|study| (id_of(study), study)).collect();
    let changed: Vec<Value> = studies
        .items
        .iter()
        .filter(|study| old.get(&id_of(study)).copied() != Some(*study))
        .cloned()
        .collect();
    let order: Vec<Value> = studies
        .items
        .iter()
        .map(|study| study["id"].clone())
        .collect();
    // A command that changed nothing (unknown op, empty rename, missing target)
    // must not take the write lock: skip sizing work and the transaction entirely.
    // `migrate_studies` already ensured the marker, so there is nothing to write.
    if studies.items.as_slice() == original {
        let mut out = result;
        out.insert(
            "studies".into(),
            Value::Array(if incremental { changed } else { studies.items }),
        );
        if incremental {
            out.insert("order".into(), Value::Array(order));
        }
        return Ok(Value::Object(out));
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
                    .map_err(db_err)?;
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
                    params![id_of(study), body, position as i64, units]).map_err(db_err)?;
            } else {
                db.execute(
                    "UPDATE studies SET position = ?2 WHERE id = ?1 AND position != ?2",
                    params![id_of(study), position as i64],
                )
                .map_err(db_err)?;
            }
        }
        Ok(())
    })?;
    let mut out = result;
    out.insert(
        "studies".into(),
        Value::Array(if incremental { changed } else { studies.items }),
    );
    if incremental {
        out.insert("order".into(), Value::Array(order));
    }
    Ok(Value::Object(out))
}
