//! The core's report on an online connection (`online:state`), as the core writes it and the
//! online game reads it.

use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::misc::constants::ErrorKind;

crate::union!(
    /// Which online stream a message arrived on.
    Lane {
        Events = "events",
        Game = "game",
        Seek = "seek",
    }
);

crate::union!(
    /// Connection state of the online play session.
    OnlinePhase {
        Checking = "checking",
        Connecting = "connecting",
        Connected = "connected",
        Reconnecting = "reconnecting",
        Disconnected = "disconnected",
        AuthRequired = "auth-required",
        Idle = "idle",
    }
);

/// One report on a connection. `session` is the connection's epoch: a report from an older
/// session than the board follows is stale.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnlineConnection {
    /// A whole number as the core counts it; the board's own report while it checks for a game
    /// (`Run::checking`) writes the epoch it holds, which stays the number it was.
    pub session: Number,
    pub account: String,
    pub game_id: String,
    pub lane: Lane,
    pub phase: OnlinePhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The kind of failure that caused this state, so the page offers what can help.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ErrorKind>,
}

impl OnlineConnection {
    /// The epoch, as the board compares it.
    pub fn epoch(&self) -> f64 {
        self.session.as_f64().unwrap_or(f64::NAN)
    }

    /// Lichess is asked for a game in progress.
    pub fn checking(&self) -> bool {
        self.lane == Lane::Game && self.phase == OnlinePhase::Checking
    }

    /// The phases that mean the game is not reachable right now.
    pub fn unreachable(&self) -> bool {
        matches!(
            self.phase,
            OnlinePhase::Checking
                | OnlinePhase::Reconnecting
                | OnlinePhase::Disconnected
                | OnlinePhase::AuthRequired
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The board's check report keeps its epoch's number, and a core report its whole number:
    /// both read back to the bytes they came as.
    #[test]
    fn a_report_reads_back_to_the_same_json() {
        for report in [
            json!({ "session": 3, "account": "a", "gameId": "g", "lane": "game", "phase": "idle" }),
            json!({ "session": -1.0, "account": "", "gameId": "", "lane": "game",
                    "phase": "checking", "message": "Checking" }),
            json!({ "session": 0, "account": "a", "gameId": "", "lane": "events",
                    "phase": "auth-required", "kind": "login" }),
        ] {
            let read: OnlineConnection = serde_json::from_value(report.clone()).unwrap();
            assert_eq!(serde_json::to_string(&read).unwrap(), report.to_string());
        }
    }

    #[test]
    fn a_check_for_a_game_is_on_the_game_lane() {
        let mut report: OnlineConnection = serde_json::from_value(json!({
            "session": 0, "account": "", "gameId": "", "lane": "game", "phase": "checking",
        }))
        .unwrap();
        assert!(report.checking() && report.unreachable());
        report.lane = Lane::Seek;
        assert!(!report.checking());
        report.phase = OnlinePhase::Connected;
        assert!(!report.unreachable());
    }
}
