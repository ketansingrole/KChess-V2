//! Accounts: Lichess and Chess.com players, what was synced and what Lichess was asked.

use super::*;

union!(#[doc = "Which app area a metered action belongs to."] UsageKind {
    Games = "games",
    Profile = "profile",
    Play = "play",
    Presence = "presence",
    Puzzles = "puzzles",
    Database = "database",
    Watch = "watch",
    Tournament = "tournament",
    Analysis = "analysis",
    Study = "study",
    Other = "other",
});

/// A Lichess account or added player, with what was synced.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub username: String,
    /// Signed in (a Lichess login is kept), rather than a player added to follow.
    pub connected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<i64>,
    /// The player's Lichess play time when their games were last synced, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_play_time: Option<i64>,
}

/// The core's settings, the accounts and how many games are synced.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppData {
    pub settings: CoreSettings,
    pub accounts: Vec<Account>,
    pub game_count: i64,
}

/// A Lichess account signed in through the browser, and the data with it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ConnectedLichess {
    pub data: AppData,
    /// The account Lichess authorised.
    pub username: String,
}

/// A Chess.com player whose games are synced.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChesscomAccount {
    /// As Chess.com spells it.
    pub username: String,
    pub added_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<i64>,
}

/// A player's profile and rating history as last saved, with no network.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CachedProfile {
    /// Null when never fetched.
    pub profile: Option<LichessUser>,
    /// Null when never fetched.
    pub rating_history: Option<Vec<RatingSeries>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_fetched_at: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageCell {
    pub requests: i64,
    /// Bytes received, after decompression.
    pub bytes_in: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsage {
    pub total: UsageCell,
    /// By the kind's wire name (`UsageKind`), in the order counted.
    pub by_kind: Keyed<UsageCell>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AccountStorage {
    pub games: i64,
    /// Approximate size of the stored games.
    pub bytes: i64,
    /// Profile and rating data cached for the account.
    pub cache_bytes: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageReport {
    /// Username → downloads from Lichess; the empty key is traffic with no account (login, etc.).
    pub accounts: Keyed<AccountUsage>,
    /// Lower-cased username → what is kept on this computer.
    pub storage: Keyed<AccountStorage>,
    /// When counting started; absent when nothing has been counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<f64>,
    /// Size of the profile's own database (games, studies, accounts) on disk; the puzzles and
    /// game databases report theirs.
    pub db_bytes: u64,
}
