//! The shapes every frontend exchanges with the core.
//!
//! Conventions:
//! - Field names are camelCase on the wire. `Option` fields are omitted when absent unless a
//!   field says otherwise with its serde attributes.
//! - A string-literal union is a Rust enum (`kchess_domain::union!`). `wire_enum!` also emits the
//!   array of wire values under the constant's name (`APPEARANCES`, ...); `union!` does not. The
//!   unions domain rules read are declared in the domain and re-exported here.
//! - A flattened parent is `#[serde(flatten)]`.
//!
//! One file per area (settings, games, players, puzzles, …), each re-exported here: a type's path
//! is `types::Name` whichever file holds it. What every area shares (the reply of a call made as
//! an account, the profile's areas) stays in this file.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use crate::Variant;
use kchess_domain::color::Color;

use kchess_domain::{union, wire_enum};

pub use kchess_domain::games::session::{
    AnalysisSession, ComputerSession, LocalSession, SessionDocument, Sessions,
};
pub use kchess_domain::js::number as js_number;
pub use kchess_domain::keyed::Keyed;
pub use kchess_domain::library::{Drill, PlayedGame, Study, StudyChapter, StudyCloud};
pub use kchess_domain::misc::constants::{
    APPEARANCES, Appearance, ENGINE_LEVELS, EngineLevel, HistoryKind, NotificationKind,
    SESSION_KINDS, SessionKind, TournamentSystem,
};
pub use kchess_domain::online_game::connection::{Lane, OnlineConnection, OnlinePhase};
pub use kchess_domain::online_game::model::{
    CHAT_ROOMS, ChatRoom, ONLINE_ACTIONS, OnlineAction, Resumed,
};
pub use kchess_domain::records::challenge::{
    CHALLENGE_COLORS, ChallengeColor, ChallengeDirection, ChallengeInfo, OnlineOptions, PlayerRef,
    TimeControl as ChallengeTimeControl,
};
pub use kchess_domain::records::ratings::RatingSeries;
pub use kchess_domain::records::review::{
    Judgment, ReviewAccuracy, ReviewEval, ReviewSide, ReviewSource, ReviewSummary, StoredReview,
};
pub use kchess_domain::training::puzzle::Puzzle;
pub use kchess_domain::training::repetition::{RepertoireMisses, RepertoireSchedule};

mod accounts;
mod ai;
mod engine;
mod games;
mod insights;
mod library;
mod lookups;
mod online;
mod players;
mod puzzles;
mod review;
mod runs;
mod settings;
mod studies;
mod tournaments;
mod voice;
mod watch;

pub use accounts::*;
pub use ai::*;
pub use engine::*;
pub use games::*;
pub use insights::*;
pub use library::*;
pub use lookups::*;
pub use online::*;
pub use players::*;
pub use puzzles::*;
pub use review::*;
pub use runs::*;
pub use settings::*;
pub use studies::*;
pub use tournaments::*;
pub use voice::*;
pub use watch::*;

/// Returned instead of data when the account's Lichess login lacks (or lost) the puzzle permission.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NeedsReconnect {
    pub needs_reconnect: bool,
}

/// What a call made as a Lichess account answers: its result, or `{ needsReconnect: true }` when
/// the account has no login or Lichess refused it (it must connect again).
#[derive(Debug, Clone, PartialEq)]
pub enum Reply<T> {
    Done(T),
    NeedsReconnect,
}

impl<T: Serialize> Serialize for Reply<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Reply::Done(value) => value.serialize(serializer),
            Reply::NeedsReconnect => NeedsReconnect {
                needs_reconnect: true,
            }
            .serialize(serializer),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Reply<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire<T> {
            Reconnect(NeedsReconnect),
            Done(T),
        }
        match Wire::deserialize(deserializer)? {
            Wire::Reconnect(NeedsReconnect {
                needs_reconnect: true,
            }) => Ok(Reply::NeedsReconnect),
            Wire::Reconnect(_) => Err(serde::de::Error::custom("needsReconnect is only ever true")),
            Wire::Done(value) => Ok(Reply::Done(value)),
        }
    }
}

/// What part of the profile a write changed (`profile:changed`), so other windows refetch it.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProfileArea {
    Studies,
    PlayedGames,
    Drills,
    Sessions,
    Tournaments,
    Repertoire,
    LichessGames,
    Accounts,
    DesktopSettings,
    Puzzles,
    VoiceHistory,
    Usage,
    Engine,
    ViewPreferences,
}

/// `alerts:owner`: the connection whose window raises system notifications now (the window
/// focused last), or none. Every other window updates silently (`docs/shared-core.md`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlertOwner {
    pub connection: Option<u64>,
}

/// `profile:changed`: a write to the profile, from connection `origin` (`docs/shared-core.md`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileChange {
    pub areas: Vec<ProfileArea>,
    /// The connection that wrote it; a window ignores its own writes.
    pub origin: u64,
}

/// A field that is absent (`None`), null (`Some(None)`) or a value.
fn present_or_null<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests;
