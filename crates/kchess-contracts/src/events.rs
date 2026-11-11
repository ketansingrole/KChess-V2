//! Everything the core reports without being asked: one `CoreEvent` per kind, crossing as its
//! name and its JSON payload (`docs/shared-core.md`, "Frames"). The payload's shape is the type
//! each line names. A line marked `typed` holds that type; the others hold the JSON as the core
//! wrote it.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

#[allow(unused_imports)]
use crate::types::*;

/// What an event holds: its contract type, or the JSON of an event not yet typed.
pub trait EventPayload: Serialize + DeserializeOwned {
    /// The JSON, when the event keeps its payload as JSON.
    fn as_json(&self) -> Option<&Value> {
        None
    }

    fn into_json(self) -> Value {
        serde_json::to_value(self).expect("an event payload is JSON")
    }

    fn from_json(payload: Value) -> serde_json::Result<Self> {
        serde_json::from_value(payload)
    }
}

impl EventPayload for Value {
    fn as_json(&self) -> Option<&Value> {
        Some(self)
    }

    fn into_json(self) -> Value {
        self
    }

    fn from_json(payload: Value) -> serde_json::Result<Self> {
        Ok(payload)
    }
}

impl EventPayload for AnalysisUpdate {}
impl EventPayload for Box<ReviewUpdate> {}
impl EventPayload for ReviewStatus {}

/// What a variant holds: its payload type when the line is `typed`, else JSON.
macro_rules! held {
    ($payload:ty) => {
        Value
    };
    ($payload:ty, typed) => {
        $payload
    };
}

macro_rules! core_events {
    ($(
        $(#[doc = $doc:literal])*
        $variant:ident = $name:literal ($payload:ty) $($typed:ident)?,
    )*) => {
        /// A core event: its kind and its payload.
        #[derive(Clone, Debug, PartialEq)]
        pub enum CoreEvent {
            $(
                $(#[doc = $doc])*
                #[doc = concat!("`", $name, "`: `", stringify!($payload), "`.")]
                $variant(held!($payload $(, $typed)?)),
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

            /// The payload of an event kept as JSON; `None` for a typed one.
            pub fn payload(&self) -> Option<&Value> {
                match self {
                    $(CoreEvent::$variant(payload) => payload.as_json(),)*
                }
            }

            /// The JSON it crosses with.
            pub fn into_payload(self) -> Value {
                match self {
                    $(CoreEvent::$variant(payload) => payload.into_json(),)*
                }
            }

            /// The event a name and a payload make; `None` for a name no event has, or for a
            /// payload that is not its event's shape.
            pub fn from_wire(name: &str, payload: Value) -> Option<CoreEvent> {
                Some(match name {
                    $($name => CoreEvent::$variant(EventPayload::from_json(payload).ok()?),)*
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
    Analysis = "engine:analysis" (AnalysisUpdate) typed,
    ReviewUpdate = "review:update" (Box<ReviewUpdate>) typed,
    ReviewStatus = "review:status" (ReviewStatus) typed,
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
            CoreEvent::Analysis(update) => update.done,
            CoreEvent::PuzzleDbProgress(progress) => progress["phase"]
                .as_str()
                .and_then(PuzzleDbPhase::from_key)
                .is_some_and(PuzzleDbPhase::is_terminal),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CoreEvent;
    use crate::types::AnalysisUpdate;
    use serde_json::json;

    fn analysis(done: bool) -> CoreEvent {
        CoreEvent::Analysis(AnalysisUpdate {
            id: 1,
            fen: "8/8/8/8/8/8/8/8 w - - 0 1".into(),
            depth: 10.,
            nps: None,
            lines: Vec::new(),
            done,
            context: String::new(),
            client_id: None,
            reason: None,
            engine: "stockfish".into(),
            error: None,
        })
    }

    #[test]
    fn an_event_crosses_as_its_name_and_payload() {
        let event = CoreEvent::ProfileChanged(json!({ "areas": ["studies"], "origin": 3 }));
        assert_eq!(event.name(), "profile:changed");
        let back = CoreEvent::from_wire(event.name(), event.payload().unwrap().clone());
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
    fn progress_and_connection_states_cross_as_before() {
        use crate::types::{
            LobbyState, OnlineConnection, OnlinePhase, PuzzleDbPhase, PuzzleDbProgress,
        };
        let mut progress = PuzzleDbProgress {
            phase: PuzzleDbPhase::Downloading,
            received: 10,
            kept: 2,
            total: None,
            message: None,
        };
        assert_eq!(
            serde_json::to_string(&progress).unwrap(),
            r#"{"phase":"downloading","received":10,"kept":2}"#
        );
        progress.phase = PuzzleDbPhase::Failed;
        progress.total = Some(99);
        progress.message = Some("No".into());
        assert_eq!(
            serde_json::to_string(&progress).unwrap(),
            r#"{"phase":"failed","received":10,"kept":2,"total":99,"message":"No"}"#
        );
        let state = OnlineConnection {
            session: 4.into(),
            account: "me".into(),
            game_id: "abcdefgh".into(),
            lane: crate::types::Lane::Game,
            phase: OnlinePhase::Idle,
            message: None,
            kind: Some(crate::ErrorKind::Offline),
        };
        assert_eq!(
            serde_json::to_string(&state).unwrap(),
            r#"{"session":4,"account":"me","gameId":"abcdefgh","lane":"game","phase":"idle","kind":"offline"}"#
        );
        let lobby = LobbyState {
            account: "me".into(),
            phase: OnlinePhase::Idle,
            message: Some("Hi".into()),
        };
        assert_eq!(
            serde_json::to_string(&lobby).unwrap(),
            r#"{"account":"me","phase":"idle","message":"Hi"}"#
        );
    }

    #[test]
    fn a_typed_event_crosses_as_its_json() {
        let event = analysis(true);
        assert_eq!(event.payload(), None);
        let sent = event.clone().into_payload();
        assert_eq!(sent["done"], json!(true));
        assert_eq!(CoreEvent::from_wire("engine:analysis", sent), Some(event));
        assert_eq!(
            CoreEvent::from_wire("engine:analysis", json!({ "done": true })),
            None,
            "a payload not of its event's shape is no event"
        );
    }

    #[test]
    fn only_a_progress_update_before_the_last_is_coalesced() {
        assert!(analysis(false).coalesced());
        let last = analysis(true);
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
