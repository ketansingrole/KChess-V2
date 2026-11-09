//! The local library's documents, as the core keeps them. Only the shapes live here.

use serde::{Deserialize, Serialize};

use crate::types::TournamentSystem;

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
pub struct MistakeExercise {
    pub id: String,
    pub fen: String,
    pub solution: Vec<String>,
    pub judgment: String,
    pub due_at: f64,
    pub streak: f64,
    pub attempts: f64,
    /// The Lichess key of the variant of the game it is from; none for standard chess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Times {
    pub white: f64,
    pub black: f64,
}

/// Which archive entry an unfinished game updates, across reloads.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveIdentity {
    pub id: String,
    pub started_at: f64,
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
