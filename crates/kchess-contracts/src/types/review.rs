//! Game review: what is reviewed, how far it got and why it paused.

use super::*;

union!(#[doc = "Why automatic review is paused."] ReviewPause { Battery = "battery", Engine = "engine", Online = "online", Off = "off" });

/// Review a game: its moves from its position, and the Lichess game it is when it is one.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase", expecting = "an object")]
pub struct ReviewRequest {
    pub fen: String,
    pub moves: Vec<String>,
    /// The Lichess key of a variant game's variant (`crazyhouse`, `atomic`…), which
    /// Fairy-Stockfish reviews.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// A Lichess game: its own analysis is fetched first when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCurrent {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    pub done: usize,
    pub total: usize,
    pub background: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFailure {
    pub key: String,
    pub message: String,
}

/// Whether the core lends engine, review and explorer help now, and why not. Reported on
/// `online:assistance` whenever it changes, and answered by `assistanceStatus`.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Assistance {
    Allowed,
    /// A live Lichess game is played, or one may still be (its end not seen).
    LiveGame,
    /// Lichess has not confirmed that no game is in progress: the check is running, or failed.
    Unverified,
}

/// Progress of the review queue, sent whenever it changes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStatus {
    /// The review being worked on, with how many positions are done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<ReviewCurrent>,
    /// Reviews waiting, asked for and automatic.
    pub waiting: usize,
    /// Why automatic reviews are on hold, when they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<ReviewPause>,
    /// The last review asked for that could not be done, and why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<ReviewFailure>,
    /// The key of the last review stopped before it was done, by whichever client stopped it:
    /// another waiting for the same review learns it will not come.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled: Option<String>,
}

/// The payload of `review:update`: the review as stored, and its summary.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUpdate {
    pub review: StoredReview,
    pub summary: ReviewSummary,
}
