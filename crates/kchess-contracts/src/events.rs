//! Everything the core reports without being asked: one `CoreEvent` per kind, crossing as its
//! name and its JSON payload (`docs/shared-core.md`, "Frames"). Each line names the type its
//! payload holds; only `online:event`, Lichess's own event as it arrived, stays JSON.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::types::*;

/// What an event holds: its contract type, read from and written to the JSON it crosses as.
pub trait EventPayload: Serialize + DeserializeOwned {
    fn into_json(self) -> Value {
        serde_json::to_value(self).expect("an event payload is JSON")
    }

    fn from_json(payload: Value) -> serde_json::Result<Self> {
        serde_json::from_value(payload)
    }
}

impl EventPayload for Value {
    fn into_json(self) -> Value {
        self
    }

    fn from_json(payload: Value) -> serde_json::Result<Self> {
        Ok(payload)
    }
}

impl EventPayload for () {}
impl EventPayload for String {}
impl EventPayload for OnlineConnection {}
impl EventPayload for PuzzleDbProgress {}
impl EventPayload for crate::voice::Progress {}
impl EventPayload for AnalysisUpdate {}
impl EventPayload for Box<ReviewUpdate> {}
impl EventPayload for ReviewStatus {}
impl EventPayload for Vec<ChallengeInfo> {}
impl EventPayload for LobbyState {}
impl EventPayload for WatchState {}
impl EventPayload for Box<WatchFrame> {}
impl EventPayload for BroadcastUpdate {}
impl EventPayload for Box<ChallengeInfo> {}
impl EventPayload for CoreSettings {}
impl EventPayload for ProfileChange {}
impl EventPayload for AlertOwner {}
impl EventPayload for crate::host::HostAsk {}

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
                #[doc = concat!("`", $name, "`.")]
                $variant($payload),
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
    /// Lichess's own stream event, read by the online game (`ServerEvent::read`).
    OnlineEvent = "online:event" (Value),
    OnlineError = "online:error" (String),
    PuzzleDbProgress = "puzzledb:progress" (PuzzleDbProgress),
    VoiceModelProgress = "voicemodel:progress" (crate::voice::Progress),
    /// To the window that started the analysis.
    Analysis = "engine:analysis" (AnalysisUpdate),
    ReviewUpdate = "review:update" (Box<ReviewUpdate>),
    ReviewStatus = "review:status" (ReviewStatus),
    ChallengesUpdate = "challenges:update" (Vec<ChallengeInfo>),
    OnlineLobby = "online:lobby" (LobbyState),
    OngoingChanged = "online:ongoing-changed" (()),
    /// To the window that watches.
    WatchState = "watch:state" (WatchState),
    WatchFrame = "watch:frame" (Box<WatchFrame>),
    WatchBroadcast = "watch:broadcast" (BroadcastUpdate),
    /// A challenge that was not pending before; frontends decide how to alert.
    ChallengeReceived = "challenge:received" (Box<ChallengeInfo>),
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
            CoreEvent::PuzzleDbProgress(progress) => progress.phase.is_terminal(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CoreEvent;
    use crate::types::*;
    use kchess_domain::color::Color;
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

    fn puzzle_db(phase: PuzzleDbPhase) -> CoreEvent {
        CoreEvent::PuzzleDbProgress(PuzzleDbProgress {
            phase,
            received: 0,
            kept: 0,
            total: None,
            message: None,
        })
    }

    /// `event` crosses as `expected`, and reading those bytes back gives the event again.
    fn crosses_as(event: CoreEvent, expected: &str) {
        let name = event.name();
        let payload = event.clone().into_payload();
        assert_eq!(serde_json::to_string(&payload).unwrap(), sorted(expected));
        assert_eq!(CoreEvent::from_wire(name, payload), Some(event));
    }

    /// The JSON text with its keys in the order a `Value` keeps them.
    fn sorted(text: &str) -> String {
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        serde_json::to_string(&value).unwrap()
    }

    #[test]
    fn an_event_crosses_as_its_name_and_payload() {
        let event = CoreEvent::ProfileChanged(ProfileChange {
            areas: vec![ProfileArea::Studies],
            origin: 3,
        });
        assert_eq!(event.name(), "profile:changed");
        crosses_as(event, r#"{"areas":["studies"],"origin":3}"#);
        assert_eq!(CoreEvent::from_wire("nothing:here", json!(null)), None);
        assert_eq!(
            CoreEvent::from_wire("profile:changed", json!({ "areas": "studies" })),
            None,
            "a payload not of its event's shape is no event"
        );
    }

    #[test]
    fn lichess_s_own_event_stays_json() {
        let raw = json!({ "type": "gameStart", "game": { "id": "abcdefgh", "extra": [1] } });
        let event = CoreEvent::from_wire("online:event", raw.clone()).unwrap();
        assert_eq!(event, CoreEvent::OnlineEvent(raw.clone()));
        assert_eq!(event.into_payload(), raw);
    }

    #[test]
    fn small_events_cross_as_before() {
        crosses_as(CoreEvent::OngoingChanged(()), "null");
        crosses_as(CoreEvent::OnlineError("Gone".into()), r#""Gone""#);
        crosses_as(
            CoreEvent::AlertsOwner(AlertOwner { connection: None }),
            r#"{"connection":null}"#,
        );
        crosses_as(
            CoreEvent::AlertsOwner(AlertOwner {
                connection: Some(7),
            }),
            r#"{"connection":7}"#,
        );
        crosses_as(
            CoreEvent::ProfileChanged(ProfileChange {
                areas: vec![ProfileArea::Studies, ProfileArea::ViewPreferences],
                origin: 3,
            }),
            r#"{"areas":["studies","viewPreferences"],"origin":3}"#,
        );
        crosses_as(
            CoreEvent::VoiceModelProgress(crate::voice::Progress {
                phase: crate::voice::Phase::Downloading,
                received: 3,
                total: Some(9),
            }),
            r#"{"phase":"downloading","received":3,"total":9}"#,
        );
        crosses_as(
            CoreEvent::HostRequest(crate::host::HostAsk {
                id: 4,
                request: crate::host::HostRequest::OnBattery,
            }),
            r#"{"id":4,"kind":"onBattery","payload":null}"#,
        );
    }

    #[test]
    fn progress_and_connection_states_cross_as_before() {
        let mut progress = PuzzleDbProgress {
            phase: PuzzleDbPhase::Downloading,
            received: 10,
            kept: 2,
            total: None,
            message: None,
        };
        crosses_as(
            CoreEvent::PuzzleDbProgress(progress.clone()),
            r#"{"phase":"downloading","received":10,"kept":2}"#,
        );
        progress.phase = PuzzleDbPhase::Failed;
        progress.total = Some(99);
        progress.message = Some("No".into());
        crosses_as(
            CoreEvent::PuzzleDbProgress(progress),
            r#"{"phase":"failed","received":10,"kept":2,"total":99,"message":"No"}"#,
        );
        let state = OnlineConnection {
            session: 4,
            account: "me".into(),
            game_id: "abcdefgh".into(),
            lane: Lane::Game,
            phase: OnlinePhase::Idle,
            message: None,
            kind: Some(crate::ErrorKind::Offline),
        };
        crosses_as(
            CoreEvent::OnlineState(state),
            r#"{"session":4,"account":"me","gameId":"abcdefgh","lane":"game","phase":"idle","kind":"offline"}"#,
        );
        let lobby = LobbyState {
            account: "me".into(),
            phase: OnlinePhase::Idle,
            message: Some("Hi".into()),
        };
        crosses_as(
            CoreEvent::OnlineLobby(lobby),
            r#"{"account":"me","phase":"idle","message":"Hi"}"#,
        );
    }

    fn player(name: &str, title: Option<&str>, rating: Option<f64>) -> WatchPlayer {
        WatchPlayer {
            name: name.into(),
            title: title.map(Into::into),
            rating,
        }
    }

    /// The bytes below were written by the core's former serialise-only copies of the watch
    /// types; the contract types write the same.
    #[test]
    fn watch_events_cross_as_the_core_wrote_them() {
        let past = TvPastGame {
            game_id: "g1".into(),
            white: player("A", Some("GM"), Some(2700.)),
            black: player("B", None, None),
            fen: "8/8/8/8/8/8/8/8 w - - 0 1".into(),
            last_move: Some("e2e4".into()),
            orientation: Color::Black,
            status: Some("mate".into()),
            winner: Some(Color::White),
        };
        let bare_past = TvPastGame {
            last_move: None,
            status: None,
            winner: None,
            orientation: Color::White,
            ..past.clone()
        };
        let full = WatchFrame {
            session: 3,
            source: WatchSource::Tv,
            channel: Some("rapid".into()),
            game_id: "g2".into(),
            white: player("A", Some("GM"), Some(2700.5)),
            black: player("B", None, Some(1500.)),
            orientation: Color::Black,
            fen: "f".into(),
            last_move: Some("e2e4".into()),
            white_clock: Some(599.),
            black_clock: Some(12.5),
            variant: "standard".into(),
            speed: Some("rapid".into()),
            rated: Some(true),
            clock: Some(WatchClock {
                initial: 600.,
                increment: 2.,
            }),
            status: Some("mate".into()),
            winner: Some(Color::Black),
            finished: true,
            start_fen: Some("s".into()),
            moves: Some(vec!["e2e4".into()]),
            previous: Some(vec![past, bare_past]),
        };
        let full_text = r#"{"session":3,"source":"tv","channel":"rapid","gameId":"g2","white":{"name":"A","title":"GM","rating":2700.5},"black":{"name":"B","rating":1500.0},"orientation":"black","fen":"f","lastMove":"e2e4","whiteClock":599.0,"blackClock":12.5,"variant":"standard","speed":"rapid","rated":true,"clock":{"initial":600.0,"increment":2.0},"status":"mate","winner":"black","finished":true,"startFen":"s","moves":["e2e4"],"previous":[{"gameId":"g1","white":{"name":"A","title":"GM","rating":2700.0},"black":{"name":"B"},"fen":"8/8/8/8/8/8/8/8 w - - 0 1","lastMove":"e2e4","orientation":"black","status":"mate","winner":"white"},{"gameId":"g1","white":{"name":"A","title":"GM","rating":2700.0},"black":{"name":"B"},"fen":"8/8/8/8/8/8/8/8 w - - 0 1","orientation":"white"}]}"#;
        assert_eq!(serde_json::to_string(&full).unwrap(), full_text);
        crosses_as(CoreEvent::WatchFrame(Box::new(full)), full_text);

        let bare = WatchFrame {
            session: 1,
            source: WatchSource::Game,
            channel: None,
            game_id: "g3".into(),
            white: player("A", None, None),
            black: player("B", None, None),
            orientation: Color::White,
            fen: "f".into(),
            last_move: None,
            white_clock: None,
            black_clock: None,
            variant: "standard".into(),
            speed: None,
            rated: None,
            clock: None,
            status: None,
            winner: None,
            finished: false,
            start_fen: None,
            moves: None,
            previous: None,
        };
        let bare_text = r#"{"session":1,"source":"game","gameId":"g3","white":{"name":"A"},"black":{"name":"B"},"orientation":"white","fen":"f","variant":"standard","finished":false}"#;
        assert_eq!(serde_json::to_string(&bare).unwrap(), bare_text);
        crosses_as(CoreEvent::WatchFrame(Box::new(bare)), bare_text);

        let game = BroadcastGame {
            id: "c1".into(),
            name: "R1".into(),
            white: player("A", Some("IM"), Some(2400.)),
            black: player("B", None, None),
            result: "*".into(),
            start_fen: "s".into(),
            moves: vec!["e2e4".into()],
            fen: "f".into(),
            last_move: Some("e2e4".into()),
            white_clock: Some(5400.),
            black_clock: Some(5399.5),
            ongoing: true,
            pgn: "1. e4 *".into(),
        };
        let bare_game = BroadcastGame {
            last_move: None,
            white_clock: None,
            black_clock: None,
            ongoing: false,
            result: "1-0".into(),
            moves: vec![],
            ..game.clone()
        };
        let update = BroadcastUpdate {
            session: 7,
            round_id: "r1".into(),
            games: vec![game, bare_game],
            ended: Some(true),
            error: Some("x".into()),
        };
        let update_text = r#"{"session":7,"roundId":"r1","games":[{"id":"c1","name":"R1","white":{"name":"A","title":"IM","rating":2400.0},"black":{"name":"B"},"result":"*","startFen":"s","moves":["e2e4"],"fen":"f","lastMove":"e2e4","whiteClock":5400.0,"blackClock":5399.5,"ongoing":true,"pgn":"1. e4 *"},{"id":"c1","name":"R1","white":{"name":"A","title":"IM","rating":2400.0},"black":{"name":"B"},"result":"1-0","startFen":"s","moves":[],"fen":"f","ongoing":false,"pgn":"1. e4 *"}],"ended":true,"error":"x"}"#;
        assert_eq!(serde_json::to_string(&update).unwrap(), update_text);
        crosses_as(CoreEvent::WatchBroadcast(update), update_text);

        let bare_update = BroadcastUpdate {
            session: 8,
            round_id: "r1".into(),
            games: vec![],
            ended: None,
            error: None,
        };
        let bare_update_text = r#"{"session":8,"roundId":"r1","games":[]}"#;
        assert_eq!(
            serde_json::to_string(&bare_update).unwrap(),
            bare_update_text
        );
        crosses_as(CoreEvent::WatchBroadcast(bare_update), bare_update_text);

        let failed = WatchState {
            session: 2,
            phase: WatchPhase::Error,
            message: Some("m".into()),
            kind: Some(crate::ErrorKind::NotFound),
        };
        let failed_text = r#"{"session":2,"phase":"error","message":"m","kind":"notFound"}"#;
        assert_eq!(serde_json::to_string(&failed).unwrap(), failed_text);
        crosses_as(CoreEvent::WatchState(failed), failed_text);

        let connected = WatchState {
            session: 2,
            phase: WatchPhase::Connected,
            message: None,
            kind: None,
        };
        let connected_text = r#"{"session":2,"phase":"connected"}"#;
        assert_eq!(serde_json::to_string(&connected).unwrap(), connected_text);
        crosses_as(CoreEvent::WatchState(connected), connected_text);
    }

    #[test]
    fn a_typed_event_crosses_as_its_json() {
        let event = analysis(true);
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
        assert!(puzzle_db(PuzzleDbPhase::Downloading).coalesced());
        assert!(puzzle_db(PuzzleDbPhase::Importing).coalesced());
        for phase in [
            PuzzleDbPhase::Done,
            PuzzleDbPhase::Cancelled,
            PuzzleDbPhase::Failed,
        ] {
            assert!(puzzle_db(phase).is_terminal());
            assert!(!puzzle_db(phase).coalesced());
        }
        let frame: WatchFrame = serde_json::from_str(
            r#"{"session":1,"source":"game","gameId":"g","white":{"name":"A"},"black":{"name":"B"},"orientation":"white","fen":"f","variant":"standard","finished":false}"#,
        )
        .unwrap();
        assert!(CoreEvent::WatchFrame(Box::new(frame)).coalesced());
        assert!(
            CoreEvent::VoiceModelProgress(crate::voice::Progress {
                phase: crate::voice::Phase::Checking,
                received: 0,
                total: None,
            })
            .coalesced()
        );
        assert!(!CoreEvent::OngoingChanged(()).coalesced());
    }
}
