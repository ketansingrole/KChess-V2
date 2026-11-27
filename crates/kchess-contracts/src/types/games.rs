//! Synced games: a page of them, their record and the library's overview.

use super::*;

union!(
    /// Filter games by result. A refusal names it `GameResult`, as the recorded checks did.
    #[serde(expecting = "enum GameResult")]
    GameResultFilter {
        Win = "win",
        Loss = "loss",
        Draw = "draw",
    }
);

/// A synced game, seen from its account's side.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LichessGame {
    pub id: String,
    pub account: String,
    pub created_at: i64,
    pub last_move_at: i64,
    pub rated: bool,
    pub speed: String,
    pub perf: String,
    pub status: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "kchess_domain::color::option_by_name"
    )]
    pub winner: Option<Color>,
    #[serde(with = "kchess_domain::color::by_name")]
    pub color: Color,
    pub opponent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opponent_rating: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_rating: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_diff: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening: Option<String>,
    /// SAN moves, separated by spaces.
    pub moves: String,
    /// Only `gamePgn` reads it: list rows leave it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pgn: Option<String>,
}

impl LichessGame {
    /// The game as a rating history counts it: rated, with the player's rating and its change.
    pub fn rated_game(&self) -> Option<kchess_domain::records::ratings::RatedGame<'_>> {
        (self.rated).then_some(())?;
        Some(kchess_domain::records::ratings::RatedGame {
            created_at: self.created_at,
            perf: &self.perf,
            rating: self.player_rating? + self.rating_diff?,
        })
    }
}

/// A page of the synced games, filtered.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(expecting = "an object")]
pub struct GamePageQuery {
    /// One account's games (a Lichess username or `name@chess.com`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<GameResultFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rated: Option<bool>,
    /// Part of an opponent's name or an opening's; blank is no search.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    pub offset: u64,
    pub limit: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GamePage {
    pub games: Vec<LichessGame>,
    /// The games the filters match, on every page.
    pub total: i64,
}

/// Win, loss and draw counts.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameRecord {
    pub total: i64,
    pub win: i64,
    pub loss: i64,
    pub draw: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameLibraryOverview {
    /// By account, lower-cased.
    pub by_account: BTreeMap<String, GameRecord>,
    /// The connected accounts' games against each added player, lower-cased.
    pub versus: BTreeMap<String, GameRecord>,
}

/// A game uploaded to Lichess, with the address of its page.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedGame {
    pub id: String,
    pub url: String,
}
