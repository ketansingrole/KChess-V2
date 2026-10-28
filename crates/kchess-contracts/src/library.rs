//! The local library's documents, as the core keeps them. Only the shapes live here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{EngineLevel, TournamentSystem};
use crate::variant::GameSetup;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub enum Color {
    #[serde(rename = "white")]
    White,
    #[serde(rename = "black")]
    Black,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudyChapter {
    pub id: String,
    pub name: String,
    pub pgn: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudyCloud {
    pub account: String,
    pub id: String,
    pub downloaded_pgn: String,
    pub structure_changed: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedStudy {
    pub id: String,
    pub name: String,
    /// First chapter, retained for existing repertoire and preview consumers.
    pub pgn: String,
    pub chapters: Vec<StudyChapter>,
    pub updated_at: f64,
    pub cloud: Option<StudyCloud>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ArchivedSource {
    Computer,
    Board,
    Clock,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub enum ArchivedResult {
    #[serde(rename = "*")]
    Unfinished,
    #[serde(rename = "1-0")]
    WhiteWins,
    #[serde(rename = "0-1")]
    BlackWins,
    #[serde(rename = "1/2-1/2")]
    Draw,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedGame {
    pub id: String,
    pub source: ArchivedSource,
    pub started_at: f64,
    pub updated_at: f64,
    pub white: String,
    pub black: String,
    pub result: ArchivedResult,
    pub reason: String,
    pub finished: bool,
    pub setup: GameSetup,
    pub moves: Vec<String>,
    pub time_control: String,
    pub clock_summary: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MistakeExercise {
    pub id: String,
    pub fen: String,
    pub solution: Vec<String>,
    pub judgment: String,
    pub due_at: f64,
    pub streak: f64,
    pub attempts: f64,
}

/// A reviewed game that has positions in practice, and where it came from when that is known.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MistakeGame {
    /// The review's key: its positions' ids start with it.
    pub key: String,
    /// The reviewed line: its start position and UCI moves.
    pub fen: String,
    pub moves: Vec<String>,
    /// When the game was played, or else when it was reviewed.
    pub played_at: f64,
    pub white: Option<String>,
    pub black: Option<String>,
    /// The side you played.
    pub player: Option<Color>,
    /// The synced Lichess game, and the account it belongs to.
    pub game_id: Option<String>,
    pub account: Option<String>,
}

/// A tournament joined from KChess; its pairings arrive on the account's event stream.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JoinedTournament {
    pub system: TournamentSystem,
    pub id: String,
    pub account: String,
    pub name: String,
    /// Stop keeping the event stream open for it after this time.
    pub until: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClockSide {
    pub minutes: f64,
    pub increment: f64,
}

/// Each side's clock: minutes and seconds added after every move (sides may differ, for odds).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalClock {
    pub white: ClockSide,
    pub black: ClockSide,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisSession {
    pub pgn: String,
    pub path: String,
    pub orientation: Color,
    pub study: String,
    pub chapter: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Times {
    pub white: f64,
    pub black: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalResult {
    pub winner: Option<Color>,
    pub reason: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalSession {
    pub setup: GameSetup,
    pub moves: Vec<String>,
    /// `null` when the game has no clock.
    pub clock: Option<LocalClock>,
    pub times: Option<Times>,
    pub result: Option<LocalResult>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComputerClock {
    pub minutes: f64,
    pub increment: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComputerSession {
    pub moves: Vec<String>,
    pub ply: f64,
    pub level: EngineLevel,
    pub color: Color,
    pub resigned: bool,
    pub setup: GameSetup,
    pub clock: Option<ComputerClock>,
    pub times: Option<Times>,
    pub flagged: Option<Color>,
}

/// Which archive entry an unfinished game updates, across reloads.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveIdentity {
    pub id: String,
    pub started_at: f64,
}

/// One unfinished document per session kind (the fields are the session kinds).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SessionDocuments {
    pub analysis: AnalysisSession,
    pub local: LocalSession,
    pub computer: ComputerSession,
    #[serde(rename = "archive:computer")]
    pub archive_computer: ArchiveIdentity,
    #[serde(rename = "archive:board")]
    pub archive_board: ArchiveIdentity,
    #[serde(rename = "archive:clock")]
    pub archive_clock: ArchiveIdentity,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySnapshot {
    pub studies: Vec<SavedStudy>,
    /// Stored studies could not be decoded; other library documents still load normally.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study_error: Option<String>,
    pub games: Vec<ArchivedGame>,
    pub mistakes: Vec<MistakeExercise>,
    /// A session kind is present only when one was saved.
    pub sessions: SessionDocuments,
    pub joined_tournaments: Vec<JoinedTournament>,
    pub repertoire_misses: BTreeMap<String, BTreeMap<String, f64>>,
    /// False until documents saved by an earlier release have been offered for import.
    pub imported: bool,
}

/* ── Study commands a frontend sends to the core ── */

/// A study command's shape, as a frontend sends it. A study to restore is checked by the core
/// service, whose rules replay every chapter.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "op")]
pub enum StudyCommandInput {
    #[serde(rename = "save")]
    #[serde(rename_all = "camelCase")]
    Save {
        name: String,
        pgn: String,
        id: Option<String>,
        chapter_id: Option<String>,
    },
    #[serde(rename = "saveChapters")]
    #[serde(rename_all = "camelCase")]
    SaveChapters {
        name: String,
        chapters: Vec<StudyChapterDraft>,
        id: Option<String>,
    },
    #[serde(rename = "addChapter")]
    AddChapter { id: String, name: String },
    #[serde(rename = "renameChapter")]
    #[serde(rename_all = "camelCase")]
    RenameChapter {
        id: String,
        chapter_id: String,
        name: String,
    },
    #[serde(rename = "duplicateChapter")]
    #[serde(rename_all = "camelCase")]
    DuplicateChapter { id: String, chapter_id: String },
    #[serde(rename = "removeChapter")]
    #[serde(rename_all = "camelCase")]
    RemoveChapter { id: String, chapter_id: String },
    #[serde(rename = "markCloud")]
    #[serde(rename_all = "camelCase")]
    MarkCloud {
        id: String,
        account: String,
        remote_id: String,
        baseline: String,
    },
    #[serde(rename = "offline")]
    #[serde(rename_all = "camelCase")]
    Offline {
        account: String,
        remote: StudyRemote,
        chapters: Vec<StudyChapterDraft>,
    },
    #[serde(rename = "remove")]
    Remove { id: String },
    #[serde(rename = "restore")]
    Restore { study: Value },
    #[serde(rename = "rename")]
    Rename { id: String, name: String },
    #[serde(rename = "duplicate")]
    Duplicate { id: String },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StudyChapterDraft {
    pub name: String,
    pub pgn: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StudyRemote {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudyCommandResult {
    pub studies: Vec<SavedStudy>,
    /// Incremental responses contain only changed studies; order lists every remaining study ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,
    /// The study or chapter the command created or selected.
    pub id: Option<String>,
    /// `offline`: the device copy had local edits, so the cloud version became a new copy.
    pub conflict: Option<bool>,
    /// `remove`: what was removed, for undo.
    pub removed: Option<SavedStudy>,
}

/// Each `SESSION_KINDS` entry, in order.
pub const SESSION_KINDS: &[&str] = &[
    "analysis",
    "local",
    "computer",
    "archive:computer",
    "archive:board",
    "archive:clock",
];

/// Where earlier releases kept each document in the renderer's local storage.
pub const LEGACY_DOCUMENT_KEYS: &[&str] = &[
    "kchess:studies:v1",
    "kchess:game-history:v1",
    "kchess:mistakes:v1",
    "kchess:analysis:v1",
    "kchess:local:v1",
    "kchess:computer:v1",
    "kchess:history-session:computer",
    "kchess:history-session:board",
    "kchess:history-session:clock",
    "kchess:tournaments-joined",
    "kchess:repertoire-misses",
];
