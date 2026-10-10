//! `crates/kchess-wasm/js/library.ts`: the local library's documents, as the core keeps them.
//!
//! Bounds and decoders stay in TypeScript (`domain/library.ts`); only the shapes live here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::{Config, TS};

use crate::generate::Item;
use crate::strs;
use crate::types::{EngineLevel, TournamentSystem};
use crate::variant::GameSetup;

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
pub enum Color {
    #[serde(rename = "white")]
    White,
    #[serde(rename = "black")]
    Black,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudyChapter {
    pub id: String,
    pub name: String,
    pub pgn: String,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct StudyCloud {
    pub account: String,
    pub id: String,
    pub downloaded_pgn: String,
    pub structure_changed: Option<bool>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct SavedStudy {
    pub id: String,
    pub name: String,
    /// First chapter, retained for existing repertoire and preview consumers.
    pub pgn: String,
    pub chapters: Vec<StudyChapter>,
    pub updated_at: f64,
    pub cloud: Option<StudyCloud>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ArchivedSource {
    Computer,
    Board,
    Clock,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
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

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
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

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
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

/// A tournament joined from KChess; its pairings arrive on the account's event stream.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JoinedTournament {
    pub system: TournamentSystem,
    pub id: String,
    pub account: String,
    pub name: String,
    /// Stop keeping the event stream open for it after this time.
    pub until: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClockSide {
    pub minutes: f64,
    pub increment: f64,
}

/// Each side's clock: minutes and seconds added after every move (sides may differ, for odds).
#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalClock {
    pub white: ClockSide,
    pub black: ClockSide,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisSession {
    pub pgn: String,
    pub path: String,
    pub orientation: Color,
    pub study: String,
    pub chapter: String,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Times {
    pub white: f64,
    pub black: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct LocalResult {
    pub winner: Option<Color>,
    pub reason: String,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalSession {
    pub setup: GameSetup,
    pub moves: Vec<String>,
    /// `null` when the game has no clock.
    #[ts(optional = false)]
    pub clock: Option<LocalClock>,
    #[ts(optional = false)]
    pub times: Option<Times>,
    #[ts(optional = false)]
    pub result: Option<LocalResult>,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComputerClock {
    pub minutes: f64,
    pub increment: f64,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComputerSession {
    pub moves: Vec<String>,
    pub ply: f64,
    pub level: EngineLevel,
    pub color: Color,
    pub resigned: bool,
    pub setup: GameSetup,
    #[ts(optional = false)]
    pub clock: Option<ComputerClock>,
    #[ts(optional = false)]
    pub times: Option<Times>,
    #[ts(optional = false)]
    pub flagged: Option<Color>,
}

/// Which archive entry an unfinished game updates, across reloads.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveIdentity {
    pub id: String,
    pub started_at: f64,
}

/// One unfinished document per session kind (`SessionKind` is `keyof SessionDocuments`).
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
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

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySnapshot {
    pub studies: Vec<SavedStudy>,
    pub games: Vec<ArchivedGame>,
    pub mistakes: Vec<MistakeExercise>,
    /// `Partial<SessionDocuments>`: a session kind is present only when one was saved.
    #[ts(type = "Partial<SessionDocuments>")]
    pub sessions: SessionDocuments,
    pub joined_tournaments: Vec<JoinedTournament>,
    #[ts(type = "RepertoireMisses")]
    pub repertoire_misses: BTreeMap<String, BTreeMap<String, f64>>,
    /// False until documents saved by an earlier release have been offered for import.
    pub imported: bool,
}

/* ── Study commands a frontend sends to the core ── */

/// A study command's shape, as a frontend sends it. A study to restore is checked by the core
/// service, whose rules replay every chapter (`assertStudyCommand` in `services/rules.ts`).
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(tag = "op")]
pub enum StudyCommandInput {
    #[serde(rename = "save")]
    #[serde(rename_all = "camelCase")]
    Save {
        name: String,
        pgn: String,
        #[ts(optional)]
        id: Option<String>,
        #[ts(optional)]
        chapter_id: Option<String>,
    },
    #[serde(rename = "saveChapters")]
    #[serde(rename_all = "camelCase")]
    SaveChapters {
        name: String,
        chapters: Vec<StudyChapterDraft>,
        #[ts(optional)]
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
    Restore {
        #[ts(type = "unknown")]
        study: Value,
    },
    #[serde(rename = "rename")]
    Rename { id: String, name: String },
    #[serde(rename = "duplicate")]
    Duplicate { id: String },
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
pub struct StudyChapterDraft {
    pub name: String,
    pub pgn: String,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
pub struct StudyRemote {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct StudyCommandResult {
    pub studies: Vec<SavedStudy>,
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

pub fn items(cfg: &Config) -> Vec<Item> {
    let mut out = vec![
        Item::Const { name: "SESSION_KINDS".into(), values: strs(SESSION_KINDS), suffix: " satisfies readonly SessionKind[]".into() },
        Item::Const { name: "LEGACY_DOCUMENT_KEYS".into(), values: strs(LEGACY_DOCUMENT_KEYS), suffix: String::new() },
        Item::Raw { name: "SessionKind".into(), text: "export type SessionKind = keyof SessionDocuments;".into() },
        Item::Raw { name: "RepertoireMisses".into(), text: "export type RepertoireMisses = Record<string, Record<string, number>>;".into() },
        Item::Raw { name: "LegacyDocuments".into(), text: "export type LegacyDocuments = Partial<Record<(typeof LEGACY_DOCUMENT_KEYS)[number], string>>;".into() },
        Item::Raw { name: "GameSnapshot".into(), text: "export type GameSnapshot = Omit<ArchivedGame, 'id' | 'startedAt' | 'updatedAt' | 'finished'>;".into() },
        Item::Raw { name: "StudyCommand".into(), text: "export type StudyCommand =\n  Exclude<StudyCommandInput, { op: 'restore' }> | { op: 'restore'; study: SavedStudy };".into() },
    ];
    macro_rules! decl {
        ($($t:ty),+ $(,)?) => { $(out.push(Item::of::<$t>(cfg));)+ };
    }
    decl!(
        Color,
        StudyChapter,
        StudyCloud,
        SavedStudy,
        ArchivedSource,
        ArchivedResult,
        ArchivedGame,
        MistakeExercise,
        JoinedTournament,
        ClockSide,
        LocalClock,
        AnalysisSession,
        Times,
        LocalResult,
        LocalSession,
        ComputerClock,
        ComputerSession,
        ArchiveIdentity,
        SessionDocuments,
        LibrarySnapshot,
        StudyCommandInput,
        StudyChapterDraft,
        StudyRemote,
        StudyCommandResult,
    );
    out
}
