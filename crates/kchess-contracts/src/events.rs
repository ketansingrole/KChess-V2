//! Everything the core reports without being asked: one `CoreEvent` per kind, crossing as its
//! name and its JSON payload (`docs/shared-core.md`, "Frames"). The payload's shape is the type
//! each line names; it travels as JSON, as the core wrote it.

use serde_json::Value;

#[allow(unused_imports)]
use crate::types::*;

macro_rules! core_events {
    ($(
        $(#[doc = $doc:literal])*
        $variant:ident = $name:literal ($payload:ty),
    )*) => {
        /// A core event: its kind and its payload.
        #[derive(Clone, Debug, PartialEq)]
        pub enum CoreEvent {
            $(
                $(#[doc = $doc])*
                #[doc = concat!("`", $name, "`: `", stringify!($payload), "`.")]
                $variant(Value),
            )*
        }

        impl CoreEvent {
            /// Every event's name, in declaration order.
            pub const NAMES: &'static [&'static str] = &[$($name),*];

            /// The name it crosses with.
            pub fn name(&self) -> &'static str {
                match self {
                    $(CoreEvent::$variant(_) => $name,)*
                }
            }

            pub fn payload(&self) -> &Value {
                match self {
                    $(CoreEvent::$variant(payload) => payload,)*
                }
            }

            pub fn into_payload(self) -> Value {
                match self {
                    $(CoreEvent::$variant(payload) => payload,)*
                }
            }

            /// The event a name and a payload make; `None` for a name no event has.
            pub fn from_wire(name: &str, payload: Value) -> Option<CoreEvent> {
                Some(match name {
                    $($name => CoreEvent::$variant(payload),)*
                    _ => return None,
                })
            }
        }
    };
}

core_events! {
    OnlineState = "online:state" (OnlineConnection),
    OnlineEvent = "online:event" (Value),
    OnlineError = "online:error" (String),
    PuzzleDbProgress = "puzzledb:progress" (PuzzleDbProgress),
    VoiceModelProgress = "voicemodel:progress" (crate::voice::Progress),
    /// To the window that started the analysis.
    Analysis = "engine:analysis" (AnalysisUpdate),
    ReviewUpdate = "review:update" (ReviewUpdate),
    ReviewStatus = "review:status" (ReviewStatus),
    ChallengesUpdate = "challenges:update" (Vec<ChallengeInfo>),
    OnlineLobby = "online:lobby" (LobbyState),
    OngoingChanged = "online:ongoing-changed" (()),
    /// To the window that watches.
    WatchState = "watch:state" (WatchState),
    WatchFrame = "watch:frame" (WatchFrame),
    WatchBroadcast = "watch:broadcast" (BroadcastUpdate),
    /// A challenge that was not pending before; frontends decide how to alert.
    ChallengeReceived = "challenge:received" (ChallengeInfo),
    SettingsSaved = "settings:saved" (CoreSettings),
    /// Another window (or this one) changed part of the profile; refetch what is shown.
    ProfileChanged = "profile:changed" (ProfileChange),
    /// Which window raises system notifications now; the others stay quiet.
    AlertsOwner = "alerts:owner" (AlertOwner),
    /// The core asks its host for a capability (`crate::host`); answered with `host.reply`.
    HostRequest = "host:request" (crate::host::HostAsk),
}

impl CoreEvent {
    /// A progress update a slow client may miss: the next one carries the whole state.
    pub fn coalesced(&self) -> bool {
        matches!(
            self,
            CoreEvent::Analysis(_)
                | CoreEvent::WatchFrame(_)
                | CoreEvent::PuzzleDbProgress(_)
                | CoreEvent::VoiceModelProgress(_)
        ) && !self.is_terminal()
    }

    /// The last update of a progress event: no later one follows to carry the state. The voice
    /// model's download has none: its method's answer ends it.
    pub fn is_terminal(&self) -> bool {
        match self {
            CoreEvent::Analysis(update) => update["done"].as_bool().unwrap_or(false),
            CoreEvent::PuzzleDbProgress(progress) => matches!(
                progress["phase"].as_str(),
                Some("done" | "cancelled" | "failed")
            ),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CoreEvent;
    use serde_json::json;

    #[test]
    fn an_event_crosses_as_its_name_and_payload() {
        let event = CoreEvent::ProfileChanged(json!({ "areas": ["studies"], "origin": 3 }));
        assert_eq!(event.name(), "profile:changed");
        let back = CoreEvent::from_wire(event.name(), event.payload().clone());
        assert_eq!(back, Some(event));
        assert_eq!(CoreEvent::from_wire("nothing:here", json!(null)), None);
    }

    #[test]
    fn profile_changes_and_alert_owners_cross_as_before() {
        use crate::types::{AlertOwner, ProfileArea, ProfileChange};
        let change = ProfileChange {
            areas: vec![ProfileArea::Studies, ProfileArea::ViewPreferences],
            origin: 3,
        };
        assert_eq!(
            serde_json::to_string(&change).unwrap(),
            r#"{"areas":["studies","viewPreferences"],"origin":3}"#
        );
        let none = AlertOwner { connection: None };
        assert_eq!(
            serde_json::to_string(&none).unwrap(),
            r#"{"connection":null}"#
        );
        let some = AlertOwner {
            connection: Some(7),
        };
        assert_eq!(serde_json::to_string(&some).unwrap(), r#"{"connection":7}"#);
    }

    #[test]
    fn only_a_progress_update_before_the_last_is_coalesced() {
        assert!(CoreEvent::Analysis(json!({ "done": false })).coalesced());
        let last = CoreEvent::Analysis(json!({ "done": true }));
        assert!(last.is_terminal() && !last.coalesced());
        assert!(CoreEvent::PuzzleDbProgress(json!({ "phase": "downloading" })).coalesced());
        for phase in ["done", "cancelled", "failed"] {
            assert!(CoreEvent::PuzzleDbProgress(json!({ "phase": phase })).is_terminal());
        }
        assert!(CoreEvent::WatchFrame(json!({})).coalesced());
        assert!(CoreEvent::VoiceModelProgress(json!({})).coalesced());
        assert!(!CoreEvent::ProfileChanged(json!({})).coalesced());
    }
}
