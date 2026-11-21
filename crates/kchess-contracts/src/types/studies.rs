//! Studies: Lichess's, a linked study's sync and the edits a study takes.

use super::*;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LichessStudy {
    pub id: String,
    pub name: String,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudySyncRequest {
    pub account: String,
    pub study_id: String,
    pub baseline: String,
    pub pgn: String,
}

/// Where `exportToLichessStudy` added the game: the study's id.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StudyExport {
    pub id: String,
}

/// A study chapter, as Lichess sends it or to save: its name and PGN.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(expecting = "an object")]
pub struct ChapterInput {
    pub name: String,
    pub pgn: String,
}

/// A Lichess study a local one is a copy of.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(expecting = "an object")]
pub struct RemoteStudy {
    pub id: String,
    pub name: String,
}

/// One change to the local study library, by its `op`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum StudyCommand {
    /// A study of one chapter, or (with `id`) one chapter's moves of a stored study.
    Save {
        name: String,
        pgn: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chapter_id: Option<String>,
    },
    /// A study of these chapters, new or (with `id`) replacing a stored one.
    SaveChapters {
        name: String,
        chapters: Vec<ChapterInput>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    AddChapter {
        id: String,
        name: String,
    },
    RenameChapter {
        id: String,
        chapter_id: String,
        name: String,
    },
    DuplicateChapter {
        id: String,
        chapter_id: String,
    },
    RemoveChapter {
        id: String,
        chapter_id: String,
    },
    /// A chapter to its new place.
    MoveChapter {
        id: String,
        chapter_id: String,
        index: u32,
    },
    /// Undo of a removed chapter: it goes back at its place.
    RestoreChapter {
        id: String,
        chapter: StudyChapter,
        index: u32,
    },
    /// The study is a copy of a Lichess study, as downloaded (`baseline`). `chapters` are the
    /// ids of the chapters uploaded, in order: a study moved, added to or cut since then keeps its
    /// structure marked changed.
    MarkCloud {
        id: String,
        account: String,
        remote_id: String,
        baseline: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        chapters: Vec<String>,
    },
    /// A Lichess study kept as a local one.
    Offline {
        account: String,
        remote: RemoteStudy,
        chapters: Vec<ChapterInput>,
    },
    Remove {
        id: String,
    },
    /// Undo of a removed study: the study as `remove` answered it.
    Restore {
        study: Value,
    },
    Rename {
        id: String,
        name: String,
    },
    Duplicate {
        id: String,
    },
}

impl StudyCommand {
    /// The stored study the command changes, when it names one.
    pub fn study_id(&self) -> Option<&str> {
        match self {
            StudyCommand::Save { id, .. } | StudyCommand::SaveChapters { id, .. } => id.as_deref(),
            StudyCommand::AddChapter { id, .. }
            | StudyCommand::RenameChapter { id, .. }
            | StudyCommand::DuplicateChapter { id, .. }
            | StudyCommand::RemoveChapter { id, .. }
            | StudyCommand::MoveChapter { id, .. }
            | StudyCommand::RestoreChapter { id, .. }
            | StudyCommand::MarkCloud { id, .. }
            | StudyCommand::Remove { id }
            | StudyCommand::Rename { id, .. }
            | StudyCommand::Duplicate { id } => Some(id),
            StudyCommand::Restore { study } => study.get("id").and_then(Value::as_str),
            StudyCommand::Offline { .. } => None,
        }
    }

    /// The chapter the command changes, when it names one.
    pub fn chapter_id(&self) -> Option<&str> {
        match self {
            StudyCommand::Save { chapter_id, .. } => chapter_id.as_deref(),
            StudyCommand::RenameChapter { chapter_id, .. }
            | StudyCommand::DuplicateChapter { chapter_id, .. }
            | StudyCommand::RemoveChapter { chapter_id, .. }
            | StudyCommand::MoveChapter { chapter_id, .. } => Some(chapter_id),
            StudyCommand::RestoreChapter { chapter, .. } => Some(&chapter.id),
            _ => None,
        }
    }
}

/// What a study command did: the study or chapter it made or kept (`id`), a cloud copy kept
/// apart (`conflict`), the study removed, and the studies it changed (all of them, unless the
/// command was incremental: then only the changed ones, with every study's id in `order`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct StudyCommandResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<Study>,
    pub studies: Vec<Study>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,
}
