//! Online play: challenges, the lobby, chat and the games in progress.

use super::*;

wire_enum!(#[doc = "Reasons Lichess accepts when declining a challenge (it translates them for the challenger)."] DeclineReason, DECLINE_REASONS {
    Generic = "generic",
    Later = "later",
    TooFast = "tooFast",
    TooSlow = "tooSlow",
    TimeControl = "timeControl",
    Rated = "rated",
    Casual = "casual",
    Standard = "standard",
    Variant = "variant",
});

/// A private message went through.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageSent {
    pub sent: bool,
}

/// Health of the idle event stream that waits for challenges and pairings.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LobbyState {
    pub account: String,
    pub phase: OnlinePhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatLine {
    pub user: String,
    pub text: String,
    pub room: ChatRoom,
}

/// What `startOnline` began: a challenge (its id and address), a public seek, or a
/// correspondence game.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StartResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seeking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correspondence: Option<bool>,
}

/// A game a connected account is playing, real-time or correspondence.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OngoingGame {
    pub game_id: String,
    pub account: String,
    pub opponent: PlayerRef,
    /// Your side, when Lichess says.
    #[serde(
        default,
        with = "kchess_domain::color::option_by_name",
        skip_serializing_if = "Option::is_none"
    )]
    pub color: Option<Color>,
    pub fen: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_move: Option<String>,
    pub is_my_turn: bool,
    /// Seconds left on your clock (or for your correspondence move), when Lichess says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_left: Option<Number>,
    pub variant: String,
    pub speed: String,
    pub rated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tournament_id: Option<String>,
    /// Whether the Board API can play it (`compat.board`); a game it cannot is watched instead.
    #[serde(default = "playable_by_default")]
    pub playable: bool,
}

fn playable_by_default() -> bool {
    true
}

impl OngoingGame {
    pub fn correspondence(&self) -> bool {
        self.speed == kchess_domain::perf::Speed::Correspondence.key()
    }
}
