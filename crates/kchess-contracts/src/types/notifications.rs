//! The notification centre: the notifications KChess showed, kept so a missed one can be found
//! again (`notifications`, `recordNotification`).

use super::*;

/// Where a notification leads when it is opened again.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NotificationLink {
    /// Nowhere in particular.
    #[default]
    None,
    /// A challenge received: the challenges on Play on Lichess, where it can still be answered
    /// while it stands.
    Challenge { id: String },
    /// A correspondence game of one of the accounts.
    Correspondence { account: String, game: String },
    /// The real-time game on Play on Lichess.
    LiveGame,
    /// The game against the computer.
    ComputerGame,
    /// A broadcast round on Watch, at one of its games when `game` is not empty.
    BroadcastRound {
        round: String,
        #[serde(default)]
        game: String,
    },
    /// A new version of KChess, in Settings › Updates.
    Update { version: String },
}

/// A notification to keep, as the app showed it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct NotificationInput {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub link: NotificationLink,
    /// Seen as it arrived (shown inside the window), so it does not count as unread.
    #[serde(default)]
    pub read: bool,
    /// The same notification announced again (a challenge, an update) has the same key: it
    /// replaces the one kept, or, saying the same, leaves it as it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// A notification kept on this profile.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationEntry {
    pub id: String,
    /// When it arrived, in milliseconds since 1970.
    #[serde(with = "js_number")]
    pub at: f64,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub link: NotificationLink,
    #[serde(default)]
    pub read: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

impl NotificationEntry {
    /// Whether `input` says what this entry says, so recording it again changes nothing.
    pub fn says(&self, input: &NotificationInput) -> bool {
        self.title == input.title && self.body == input.body && self.link == input.link
    }
}
