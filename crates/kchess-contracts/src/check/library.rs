//! The arguments of the library methods (studies, played games, drills, sessions, repertoire
//! notes) and of the view preferences.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use kchess_domain::color::{self, Color};
use kchess_domain::library::MAX_CHAPTERS;
use kchess_domain::misc::MAX_DOCUMENT;
use kchess_domain::misc::patterns;
use kchess_domain::misc::value::J;
use kchess_domain::training::repetition;

use super::*;
use crate::types::{
    ChapterInput, Keyed, RemoteStudy, SESSION_KINDS, SessionKind, StudyChapter, StudyCommand,
};

/* ── Single values ── */

/// A side of the board, as `"white"` or `"black"`.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct Side(#[serde(with = "kchess_domain::color::by_name")] pub Color);

impl Validate for Side {
    fn validate(value: J) -> Result<Self, Refusal> {
        let name: String = picked(value, &["white", "black"], None)?;
        color::from_name(&name)
            .map(Side)
            .ok_or_else(|| "invalid value, expected one of white, black".into())
    }
}

impl Validate for SessionKind {
    fn validate(value: J) -> Result<Self, Refusal> {
        picked(value, SESSION_KINDS, None)
    }
}

/// A page's unfinished session as sent: any object, which the core reads by its kind.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(transparent)]
pub struct SessionObject(pub Value);

impl SessionObject {
    /// A session, to send.
    pub fn of(session: &crate::types::SessionDocument) -> SessionObject {
        SessionObject(serde_json::to_value(session).unwrap_or(Value::Null))
    }
}

impl Validate for SessionObject {
    fn validate(value: J) -> Result<Self, Refusal> {
        ensure(value.is_object(), None, "", || "an object".into())?;
        Ok(SessionObject(value.into_value()))
    }
}

/// A played game to save: an object, whose fields the core checks.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(transparent)]
pub struct ArchivedGame(pub Value);

impl ArchivedGame {
    /// A played game, to send.
    pub fn of(game: &kchess_domain::library::PlayedGame) -> ArchivedGame {
        ArchivedGame(serde_json::to_value(game).unwrap_or(Value::Null))
    }
}

impl Validate for ArchivedGame {
    fn validate(value: J) -> Result<Self, Refusal> {
        if !value.truthy() || !value.is_object() {
            return Err("This game cannot be saved.".into());
        }
        Ok(ArchivedGame(value.into_value()))
    }
}

/// A repertoire's key (`studyId:color`): 1 to 200 characters.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct RepertoireKey(pub String);

impl Validate for RepertoireKey {
    fn validate(value: J) -> Result<Self, Refusal> {
        let key: String = decode(value, plain)?;
        min_chars(&key, 1, None, "")?;
        max_chars(&key, 200, None, "")?;
        Ok(RepertoireKey(key))
    }
}

/// A repertoire line: its UCI moves, as the spaced-repetition schedule names lines.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct RepertoireLine(pub String);

impl Validate for RepertoireLine {
    fn validate(value: J) -> Result<Self, Refusal> {
        let line: String = decode(value, plain)?;
        if !repetition::valid_line(&line) {
            return Err("A repertoire line must be its UCI moves.".into());
        }
        Ok(RepertoireLine(line))
    }
}

/// A repertoire's missed positions to put back (Undo): how often each was missed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(transparent)]
pub struct MissCounts(pub Keyed<f64>);

impl Validate for MissCounts {
    fn validate(value: J) -> Result<Self, Refusal> {
        ensure(value.is_object(), None, "", || "an object".into())?;
        const COUNT: &str = "a miss count must be a positive number";
        let counts: Keyed<f64> = decode(value, |_| Some(COUNT))?;
        for (_, count) in counts.iter() {
            ensure(
                count.is_finite() && *count > 0.0,
                Some(COUNT),
                "",
                String::new,
            )?;
        }
        Ok(MissCounts(counts))
    }
}

/// A view preference's name: `kchess:` and lowercase letters, digits, `-` and `:`; at most 100
/// characters.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct ViewPreferenceKey(pub String);

impl Validate for ViewPreferenceKey {
    fn validate(value: J) -> Result<Self, Refusal> {
        let key: String = decode(value, plain)?;
        let valid = key.len() <= 100
            && key.strip_prefix("kchess:").is_some_and(|rest| {
                !rest.is_empty()
                    && rest.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b':'
                    })
            });
        ensure(valid, None, "", || "a view preference name".into())?;
        Ok(ViewPreferenceKey(key))
    }
}

/* ── Study commands ── */

fn study_name(name: &str, path: &str) -> Result<(), Refusal> {
    max_chars(name, 200, None, path)
}

/// A local study or chapter id: 1 to 80 characters.
fn local_id(id: &str, path: &str) -> Result<(), Refusal> {
    min_chars(id, 1, None, path)?;
    max_chars(id, 80, None, path)
}

fn study_pgn(pgn: &str, path: &str) -> Result<(), Refusal> {
    max_chars(pgn, MAX_DOCUMENT, None, path)
}

fn study_account(account: &str) -> Result<(), Refusal> {
    max_chars(account, 40, None, "account")
}

fn study_lichess_id(id: &str, path: &str) -> Result<(), Refusal> {
    matches(id, patterns::lichess_id, None, path, "a Lichess id")
}

/// 1 to `MAX_CHAPTERS` chapters, each with a name and a PGN.
fn chapters(list: &[ChapterInput]) -> Result<(), Refusal> {
    for chapter in list {
        study_name(&chapter.name, "chapters.name")?;
        study_pgn(&chapter.pgn, "chapters.pgn")?;
    }
    ensure(!list.is_empty(), None, "chapters", || {
        "at least 1 item".into()
    })?;
    max_items(list, MAX_CHAPTERS, None, "chapters")
}

/// The commands whose answers the recorded suites hold, by their `op`: an unknown one lists
/// these (the commands added since are told apart before them).
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase", expecting = "a study command")]
enum StudyOp {
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
struct SentSave {
    name: String,
    pgn: String,
    id: Option<String>,
    chapter_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentSaveChapters {
    name: String,
    chapters: Vec<ChapterInput>,
    id: Option<String>,
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentNamed {
    id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
struct SentRenameChapter {
    id: String,
    chapter_id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
struct SentChapter {
    id: String,
    chapter_id: String,
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentRestoreChapter {
    id: String,
    chapter: StudyChapter,
    index: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
struct SentMoveChapter {
    id: String,
    chapter_id: String,
    index: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", expecting = "an object")]
struct SentMarkCloud {
    id: String,
    account: String,
    remote_id: String,
    baseline: String,
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentOffline {
    account: String,
    remote: RemoteStudy,
    chapters: Vec<ChapterInput>,
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentStudy {
    id: String,
}

/// One study command, chosen by its `op`, with the fields that command takes (others are
/// dropped). A study to restore is passed on as it is; the core checks it.
impl Validate for StudyCommand {
    fn validate(value: J) -> Result<Self, Refusal> {
        if !value.is_object() {
            let error = decode::<SentSave>(value, plain).err();
            return Err(error.unwrap_or_else(|| "invalid type, expected an object".into()));
        }
        // Commands newer than the recorded ones (Undo's restore, moving a chapter): told apart
        // before them, so their answers (an unknown command lists theirs) stay as recorded.
        if value.get("op").and_then(J::str) == Some("restoreChapter") {
            let sent: SentRestoreChapter = decode(value, plain)?;
            local_id(&sent.id, "id")?;
            local_id(&sent.chapter.id, "chapter.id")?;
            study_name(&sent.chapter.name, "chapter.name")?;
            study_pgn(&sent.chapter.pgn, "chapter.pgn")?;
            return Ok(StudyCommand::RestoreChapter {
                id: sent.id,
                chapter: sent.chapter,
                index: sent.index,
            });
        }
        if value.get("op").and_then(J::str) == Some("moveChapter") {
            let sent: SentMoveChapter = decode(value, plain)?;
            local_id(&sent.id, "id")?;
            local_id(&sent.chapter_id, "chapterId")?;
            return Ok(StudyCommand::MoveChapter {
                id: sent.id,
                chapter_id: sent.chapter_id,
                index: sent.index,
            });
        }
        let tag = value.get("op").cloned().unwrap_or(J::Undef);
        let op: StudyOp = decode(tag, plain).map_err(|error| format!("op: {error}"))?;
        Ok(match op {
            StudyOp::Save => {
                let sent: SentSave = decode(value, plain)?;
                study_name(&sent.name, "name")?;
                study_pgn(&sent.pgn, "pgn")?;
                for id in sent.id.iter().chain(&sent.chapter_id) {
                    local_id(id, "id")?;
                }
                StudyCommand::Save {
                    name: sent.name,
                    pgn: sent.pgn,
                    id: sent.id,
                    chapter_id: sent.chapter_id,
                }
            }
            StudyOp::SaveChapters => {
                let sent: SentSaveChapters = decode(value, plain)?;
                study_name(&sent.name, "name")?;
                chapters(&sent.chapters)?;
                if let Some(id) = &sent.id {
                    local_id(id, "id")?;
                }
                StudyCommand::SaveChapters {
                    name: sent.name,
                    chapters: sent.chapters,
                    id: sent.id,
                }
            }
            StudyOp::AddChapter | StudyOp::Rename => {
                let sent: SentNamed = decode(value, plain)?;
                local_id(&sent.id, "id")?;
                study_name(&sent.name, "name")?;
                let SentNamed { id, name } = sent;
                match op {
                    StudyOp::AddChapter => StudyCommand::AddChapter { id, name },
                    _ => StudyCommand::Rename { id, name },
                }
            }
            StudyOp::RenameChapter => {
                let sent: SentRenameChapter = decode(value, plain)?;
                local_id(&sent.id, "id")?;
                local_id(&sent.chapter_id, "chapterId")?;
                study_name(&sent.name, "name")?;
                StudyCommand::RenameChapter {
                    id: sent.id,
                    chapter_id: sent.chapter_id,
                    name: sent.name,
                }
            }
            StudyOp::DuplicateChapter | StudyOp::RemoveChapter => {
                let sent: SentChapter = decode(value, plain)?;
                local_id(&sent.id, "id")?;
                local_id(&sent.chapter_id, "chapterId")?;
                let SentChapter { id, chapter_id } = sent;
                match op {
                    StudyOp::DuplicateChapter => StudyCommand::DuplicateChapter { id, chapter_id },
                    _ => StudyCommand::RemoveChapter { id, chapter_id },
                }
            }
            StudyOp::MarkCloud => {
                let sent: SentMarkCloud = decode(value, plain)?;
                local_id(&sent.id, "id")?;
                study_account(&sent.account)?;
                study_lichess_id(&sent.remote_id, "remoteId")?;
                study_pgn(&sent.baseline, "baseline")?;
                StudyCommand::MarkCloud {
                    id: sent.id,
                    account: sent.account,
                    remote_id: sent.remote_id,
                    baseline: sent.baseline,
                }
            }
            StudyOp::Offline => {
                let sent: SentOffline = decode(value, plain)?;
                study_account(&sent.account)?;
                study_lichess_id(&sent.remote.id, "remote.id")?;
                study_name(&sent.remote.name, "remote.name")?;
                chapters(&sent.chapters)?;
                StudyCommand::Offline {
                    account: sent.account,
                    remote: sent.remote,
                    chapters: sent.chapters,
                }
            }
            StudyOp::Remove | StudyOp::Duplicate => {
                let SentStudy { id } = decode(value, plain)?;
                local_id(&id, "id")?;
                match op {
                    StudyOp::Remove => StudyCommand::Remove { id },
                    _ => StudyCommand::Duplicate { id },
                }
            }
            StudyOp::Restore => {
                let study = value.get("study").cloned().ok_or("missing field `study`")?;
                StudyCommand::Restore {
                    study: study.into_value(),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(fields: Vec<(&str, J)>) -> J {
        J::Obj(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    fn sent(command: StudyCommand) -> serde_json::Value {
        serde_json::to_value(command).unwrap()
    }

    /// Undo's `restoreChapter` takes the chapter whole and its place (the core reads its PGN).
    #[test]
    fn a_chapter_to_restore_is_checked() {
        let chapter = obj(vec![
            ("id", J::Str("c1".into())),
            ("name", J::Str("Rook endings".into())),
            ("pgn", J::Str("1. e4 *".into())),
        ]);
        let restore = obj(vec![
            ("op", J::Str("restoreChapter".into())),
            ("id", J::Str("s1".into())),
            ("chapter", chapter),
            ("index", J::Num(1.0)),
            ("extra", J::Str("dropped".into())),
        ]);
        assert_eq!(
            sent(check::<StudyCommand>(restore).unwrap()),
            json!({
                "op": "restoreChapter",
                "id": "s1",
                "chapter": { "id": "c1", "name": "Rook endings", "pgn": "1. e4 *" },
                "index": 1,
            })
        );
        let without_chapter = obj(vec![
            ("op", J::Str("restoreChapter".into())),
            ("id", J::Str("s1".into())),
            ("index", J::Num(0.0)),
        ]);
        assert!(check::<StudyCommand>(without_chapter).is_err());
    }

    /// `moveChapter` takes a chapter and its new place.
    #[test]
    fn a_chapter_to_move_is_checked() {
        let moved = obj(vec![
            ("op", J::Str("moveChapter".into())),
            ("id", J::Str("s1".into())),
            ("chapterId", J::Str("c1".into())),
            ("index", J::Num(2.0)),
            ("extra", J::Str("dropped".into())),
        ]);
        assert_eq!(
            sent(check::<StudyCommand>(moved).unwrap()),
            json!({ "op": "moveChapter", "id": "s1", "chapterId": "c1", "index": 2 })
        );
        let without_place = obj(vec![
            ("op", J::Str("moveChapter".into())),
            ("id", J::Str("s1".into())),
            ("chapterId", J::Str("c1".into())),
        ]);
        assert!(check::<StudyCommand>(without_place).is_err());
    }

    #[test]
    fn single_values_keep_their_messages() {
        assert_eq!(
            check::<RepertoireLine>(J::Str("e4".into())),
            Err("A repertoire line must be its UCI moves.".into())
        );
        assert_eq!(
            check::<ViewPreferenceKey>(J::Str("tab".into())),
            Err("invalid value, expected a view preference name".into())
        );
        assert_eq!(
            check::<SessionObject>(J::Str("x".into())),
            Err("invalid value, expected an object".into())
        );
        let misses = obj(vec![("fen", J::Num(-1.0))]);
        assert_eq!(
            check::<MissCounts>(misses),
            Err("a miss count must be a positive number".into())
        );
        assert_eq!(
            check::<PgnText>(J::Str("  ".into())),
            Err("invalid value, expected some text".into())
        );
    }
}
