//! Validators of library documents: studies and their chapters, repertoire notes, view
//! preferences, sessions and the documents the Electron app left.

use super::*;

/// A study's name.
pub(super) fn study_name(name: &str, path: &str) -> Checked<()> {
    max_chars(name, 200, None, path)
}

/// A local study or chapter id: 1 to 80 characters.
pub(super) fn local_id(id: &str, path: &str) -> Checked<()> {
    min_chars(id, 1, None, path)?;
    max_chars(id, 80, None, path)
}

pub(super) fn study_pgn(pgn: &str, path: &str) -> Checked<()> {
    max_chars(pgn, MAX_DOCUMENT, None, path)
}

pub(super) fn study_account(account: &str) -> Checked<()> {
    max_chars(account, 40, None, "account")
}

pub(super) fn study_lichess_id(id: &str, path: &str) -> Checked<()> {
    matches(id, patterns::lichess_id, None, path, "a Lichess id")
}

/// The study commands, by their `op`.
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase", expecting = "a study command")]
pub(super) enum StudyOp {
    Save,
    SaveChapters,
    AddChapter,
    RenameChapter,
    DuplicateChapter,
    RemoveChapter,
    MarkCloud,
    Offline,
    Remove,
    Restore,
    Rename,
    Duplicate,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct ChapterInput {
    name: String,
    pgn: String,
}

/// 1 to `MAX_CHAPTERS` chapters, each with a name and a PGN.
pub(super) fn chapters(list: &[ChapterInput]) -> Checked<()> {
    for chapter in list {
        study_name(&chapter.name, "chapters.name")?;
        study_pgn(&chapter.pgn, "chapters.pgn")?;
    }
    min_items(list, 1, None, "chapters")?;
    max_items(list, MAX_CHAPTERS, None, "chapters")
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
pub(super) struct SaveStudy {
    op: String,
    name: String,
    pgn: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chapter_id: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct SaveChapters {
    op: String,
    name: String,
    chapters: Vec<ChapterInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
}

/// `addChapter`, `rename`: a study and a name.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct NamedStudy {
    op: String,
    id: String,
    name: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
pub(super) struct RenameChapter {
    op: String,
    id: String,
    chapter_id: String,
    name: String,
}

/// `duplicateChapter`, `removeChapter`: a study's chapter.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
pub(super) struct StudyChapter {
    op: String,
    id: String,
    chapter_id: String,
}

/// `restoreChapter`: a removed chapter (its id, name and PGN) back at its place in the study.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct RestoreChapter {
    op: String,
    id: String,
    chapter: ChapterEntry,
    index: u32,
}

/// `moveChapter`: a study's chapter to its new place.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
pub(super) struct MoveChapter {
    op: String,
    id: String,
    chapter_id: String,
    index: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct ChapterEntry {
    id: String,
    name: String,
    pgn: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
pub(super) struct MarkCloud {
    op: String,
    id: String,
    account: String,
    remote_id: String,
    baseline: String,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct Offline {
    op: String,
    account: String,
    remote: RemoteStudy,
    chapters: Vec<ChapterInput>,
}

#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct RemoteStudy {
    id: String,
    name: String,
}

/// `remove`, `duplicate`: a study.
#[derive(Deserialize, Serialize)]
#[serde(expecting = "an object")]
pub(super) struct StudyRef {
    op: String,
    id: String,
}

/// `studyCommandShape`: one study command, chosen by its `op`, with the fields that command
/// takes (others are dropped). A study to restore is passed on as it is; the core checks it.
/// A document's revision as the client read it: its `updatedAt`, milliseconds since the epoch.
pub(super) fn assert_study_command_shape(value: J) -> Checked<Value> {
    if !value.is_object() {
        let error = decode::<SaveStudy>(value, plain).err();
        return Err(error.unwrap_or_else(|| "invalid type, expected an object".into()));
    }
    // Commands newer than the recorded ones (Undo's restore, moving a chapter): told apart
    // before them, so their answers (an unknown command lists theirs) stay as recorded.
    if value.get("op").and_then(J::str) == Some("restoreChapter") {
        let restore: RestoreChapter = decode(value, plain)?;
        local_id(&restore.id, "id")?;
        local_id(&restore.chapter.id, "chapter.id")?;
        study_name(&restore.chapter.name, "chapter.name")?;
        study_pgn(&restore.chapter.pgn, "chapter.pgn")?;
        return out(&restore);
    }
    if value.get("op").and_then(J::str) == Some("moveChapter") {
        let moved: MoveChapter = decode(value, plain)?;
        local_id(&moved.id, "id")?;
        local_id(&moved.chapter_id, "chapterId")?;
        return out(&moved);
    }
    let tag = value.get("op").cloned().unwrap_or(J::Undef);
    let op: StudyOp = decode(tag, plain).map_err(|error| format!("op: {error}"))?;
    match op {
        StudyOp::Save => {
            let save: SaveStudy = decode(value, plain)?;
            study_name(&save.name, "name")?;
            study_pgn(&save.pgn, "pgn")?;
            for id in save.id.iter().chain(&save.chapter_id) {
                local_id(id, "id")?;
            }
            out(&save)
        }
        StudyOp::SaveChapters => {
            let save: SaveChapters = decode(value, plain)?;
            study_name(&save.name, "name")?;
            chapters(&save.chapters)?;
            if let Some(id) = &save.id {
                local_id(id, "id")?;
            }
            out(&save)
        }
        StudyOp::AddChapter | StudyOp::Rename => {
            let named: NamedStudy = decode(value, plain)?;
            local_id(&named.id, "id")?;
            study_name(&named.name, "name")?;
            out(&named)
        }
        StudyOp::RenameChapter => {
            let rename: RenameChapter = decode(value, plain)?;
            local_id(&rename.id, "id")?;
            local_id(&rename.chapter_id, "chapterId")?;
            study_name(&rename.name, "name")?;
            out(&rename)
        }
        StudyOp::DuplicateChapter | StudyOp::RemoveChapter => {
            let chapter: StudyChapter = decode(value, plain)?;
            local_id(&chapter.id, "id")?;
            local_id(&chapter.chapter_id, "chapterId")?;
            out(&chapter)
        }
        StudyOp::MarkCloud => {
            let cloud: MarkCloud = decode(value, plain)?;
            local_id(&cloud.id, "id")?;
            study_account(&cloud.account)?;
            study_lichess_id(&cloud.remote_id, "remoteId")?;
            study_pgn(&cloud.baseline, "baseline")?;
            out(&cloud)
        }
        StudyOp::Offline => {
            let offline: Offline = decode(value, plain)?;
            study_account(&offline.account)?;
            study_lichess_id(&offline.remote.id, "remote.id")?;
            study_name(&offline.remote.name, "remote.name")?;
            chapters(&offline.chapters)?;
            out(&offline)
        }
        StudyOp::Remove | StudyOp::Duplicate => {
            let study: StudyRef = decode(value, plain)?;
            local_id(&study.id, "id")?;
            out(&study)
        }
        StudyOp::Restore => {
            let study = value.get("study").cloned().ok_or("missing field `study`")?;
            let mut command = serde_json::Map::new();
            command.insert("op".into(), Value::from("restore"));
            if study != J::Undef {
                command.insert("study".into(), study.into_value());
            }
            Ok(Value::Object(command))
        }
    }
}

/// A library entry's id: 1 to 80 characters.
pub(super) fn assert_library_id(value: J) -> Checked<String> {
    let id: String = decode(value, plain)?;
    local_id(&id, "")?;
    Ok(id)
}

/// A repertoire's key (`studyId:color`): 1 to 200 characters.
pub(super) fn assert_repertoire_key(value: J) -> Checked<String> {
    let key: String = decode(value, plain)?;
    min_chars(&key, 1, None, "")?;
    max_chars(&key, 200, None, "")?;
    Ok(key)
}

/// A repertoire line: its UCI moves, as the spaced-repetition schedule names lines.
pub(super) fn assert_repertoire_line(value: J) -> Checked<String> {
    let line: String = decode(value, plain)?;
    if !crate::training::repetition::valid_line(&line) {
        return Err("A repertoire line must be its UCI moves.".into());
    }
    Ok(line)
}

/// A view preference's name: `kchess:` and lowercase letters, digits, `-` and `:`, as the Electron
/// the Electron app named its `localStorage` keys; at most 100 characters.
pub(super) fn assert_view_preference_key(value: J) -> Checked<String> {
    let key: String = decode(value, plain)?;
    let valid = key.len() <= 100
        && key.strip_prefix("kchess:").is_some_and(|rest| {
            !rest.is_empty()
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b':')
        });
    ensure(valid, None, "", || "a view preference name".into())?;
    Ok(key)
}

pub(super) fn assert_session_kind(value: J) -> Checked<String> {
    picked(value, SESSION_KINDS, None)
}

pub(super) fn assert_side(value: J) -> Checked<String> {
    picked(value, &["white", "black"], None)
}
