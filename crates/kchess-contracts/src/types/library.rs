//! The library as one read, and the mistakes drilled from games.

use super::*;

/// The local library as saved. Its entries are the stored documents, read with the domain's
/// codecs: studies, played games, mistake drills, sessions by kind, tournaments joined from
/// KChess, the repertoires' missed positions and their review schedule.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub studies: Vec<Study>,
    pub games: Vec<PlayedGame>,
    pub mistakes: Vec<Drill>,
    pub sessions: Sessions,
    pub joined_tournaments: Vec<JoinedTournament>,
    pub repertoire_misses: RepertoireMisses,
    pub repertoire_schedule: RepertoireSchedule,
    /// Why the studies could not be read (they are kept, and shown as none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub study_error: Option<String>,
}

/// The drills after adding a review's mistakes; `added` counts the new ones.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct MistakesAdded {
    pub added: u32,
    pub items: Vec<Drill>,
}

/// A reviewed game with positions in practice, and its game when known: a synced Lichess game
/// (`gameId`, `account`), or one played here (the side named "You" is `player`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MistakeGame {
    /// The review's key.
    pub key: String,
    pub fen: String,
    pub moves: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub white: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub black: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "kchess_domain::color::option_by_name"
    )]
    pub player: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(with = "js_number")]
    pub played_at: f64,
}
