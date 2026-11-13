//! A page's unfinished session as the library keeps it: one document type per `SessionKind`,
//! written as the stored documents are (whole numbers without a fraction).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use shakmaty::Color;

use super::archive::Identity;
use super::local::LocalClock;
use super::{Clock, Setup, Times};
use crate::misc::constants::{EngineLevel, HistoryKind, SessionKind};
use crate::misc::value::{J, whole};
use crate::records::games::GameEnd;

/// The analysis board's: its PGN, the moves to the position shown, and the study chapter it edits
/// (empty when none).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AnalysisSession {
    pub pgn: String,
    /// UCI moves from the start, separated by spaces.
    pub path: String,
    #[serde(with = "crate::color::by_name")]
    pub orientation: Color,
    pub study: String,
    pub chapter: String,
}

/// Over the board's game.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LocalSession {
    pub setup: Setup,
    pub moves: Vec<String>,
    #[serde(with = "whole")]
    pub clock: Option<LocalClock>,
    #[serde(with = "whole")]
    pub times: Option<Times>,
    pub result: Option<GameEnd>,
}

/// The game against the computer.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComputerSession {
    pub moves: Vec<String>,
    /// The move shown.
    pub ply: usize,
    pub level: EngineLevel,
    #[serde(with = "crate::color::by_name")]
    pub color: Color,
    pub resigned: bool,
    pub setup: Setup,
    #[serde(with = "whole")]
    pub clock: Option<Clock>,
    #[serde(with = "whole")]
    pub times: Option<Times>,
    #[serde(with = "crate::color::option_by_name")]
    pub flagged: Option<Color>,
}

/// One session, of whichever kind: what a page saves and what `session` answers.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum SessionDocument {
    Analysis(AnalysisSession),
    Local(LocalSession),
    Computer(ComputerSession),
    /// A Game history place's identity (the `archive:` kinds).
    Archive(#[serde(with = "whole")] Identity),
}

impl SessionDocument {
    /// A stored session of `kind`, checked and typed; none when it is not one.
    pub fn decode(kind: SessionKind, stored: &J) -> Option<SessionDocument> {
        let decoded = crate::misc::documents::decode_stored(kind.as_str(), stored).ok()??;
        Self::read(kind, decoded.into_value())
    }

    /// A decoded document of `kind` as its type; none when it is not one.
    pub fn read(kind: SessionKind, decoded: Value) -> Option<SessionDocument> {
        let read = match kind {
            SessionKind::Analysis => serde_json::from_value(decoded).map(Self::Analysis),
            SessionKind::Local => serde_json::from_value(decoded).map(Self::Local),
            SessionKind::Computer => serde_json::from_value(decoded).map(Self::Computer),
            SessionKind::ArchiveComputer
            | SessionKind::ArchiveBoard
            | SessionKind::ArchiveClock => serde_json::from_value(decoded).map(Self::Archive),
        };
        read.ok()
    }

    /// Whether it is a document of `kind`.
    pub fn is(&self, kind: SessionKind) -> bool {
        matches!(
            (self, kind),
            (Self::Analysis(_), SessionKind::Analysis)
                | (Self::Local(_), SessionKind::Local)
                | (Self::Computer(_), SessionKind::Computer)
                | (
                    Self::Archive(_),
                    SessionKind::ArchiveComputer
                        | SessionKind::ArchiveBoard
                        | SessionKind::ArchiveClock
                )
        )
    }
}

/// The sessions kept, by kind; a kind with none is left out.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Sessions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<AnalysisSession>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<LocalSession>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub computer: Option<ComputerSession>,
    #[serde(
        rename = "archive:computer",
        default,
        skip_serializing_if = "Option::is_none",
        with = "whole"
    )]
    pub archive_computer: Option<Identity>,
    #[serde(
        rename = "archive:board",
        default,
        skip_serializing_if = "Option::is_none",
        with = "whole"
    )]
    pub archive_board: Option<Identity>,
    #[serde(
        rename = "archive:clock",
        default,
        skip_serializing_if = "Option::is_none",
        with = "whole"
    )]
    pub archive_clock: Option<Identity>,
}

impl Sessions {
    /// Keep `document` as the `kind` session; one of another kind is left out.
    pub fn insert(&mut self, kind: SessionKind, document: SessionDocument) {
        match (kind, document) {
            (SessionKind::Analysis, SessionDocument::Analysis(session)) => {
                self.analysis = Some(session);
            }
            (SessionKind::Local, SessionDocument::Local(session)) => self.local = Some(session),
            (SessionKind::Computer, SessionDocument::Computer(session)) => {
                self.computer = Some(session);
            }
            (SessionKind::ArchiveComputer, SessionDocument::Archive(identity)) => {
                self.archive_computer = Some(identity);
            }
            (SessionKind::ArchiveBoard, SessionDocument::Archive(identity)) => {
                self.archive_board = Some(identity);
            }
            (SessionKind::ArchiveClock, SessionDocument::Archive(identity)) => {
                self.archive_clock = Some(identity);
            }
            _ => {}
        }
    }

    /// The identity of the `kind` place in Game history, taken out.
    pub fn take_identity(&mut self, kind: HistoryKind) -> Option<Identity> {
        match kind {
            HistoryKind::Computer => self.archive_computer.take(),
            HistoryKind::Board => self.archive_board.take(),
            HistoryKind::Clock => self.archive_clock.take(),
        }
    }
}

impl From<ComputerSession> for super::computer::Saved {
    fn from(session: ComputerSession) -> Self {
        super::computer::Saved {
            moves: Some(session.moves),
            ply: Some(session.ply),
            level: Some(session.level.as_str().to_string()),
            color: Some(session.color),
            resigned: Some(session.resigned),
            setup: Some(session.setup),
            clock: session.clock,
            times: session.times,
            flagged: session.flagged,
        }
    }
}

impl From<LocalSession> for super::local::Saved {
    fn from(session: LocalSession) -> Self {
        super::local::Saved {
            setup: Some(session.setup),
            moves: Some(session.moves),
            clock: session.clock,
            times: session.times,
            result: session.result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::misc::documents::{decode_stored, encode_session};

    use serde_json::json;

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    /// A session as a page sends it, stored, read back and decoded: its type writes the decoded
    /// document's bytes, and reads back from them.
    fn round_trip(kind: SessionKind, sent: Value) -> SessionDocument {
        let stored = encode_session(kind.as_str(), &J::from(&sent));
        let decoded = decode_stored(kind.as_str(), &stored)
            .expect("a known kind")
            .expect("a document of its kind")
            .into_value();
        let typed = SessionDocument::read(kind, decoded.clone()).expect("typed");
        assert!(typed.is(kind), "{typed:?}");
        let written = serde_json::to_string(&typed).expect("JSON");
        assert_eq!(written, decoded.to_string(), "{kind:?}");
        let again: SessionDocument = serde_json::from_str(&written).expect("reads back");
        assert_eq!(again, typed, "{kind:?} reads back as its own kind");
        typed
    }

    #[test]
    fn every_kind_writes_its_decoded_document() {
        round_trip(
            SessionKind::Analysis,
            json!({ "pgn": "1. e4 e5 *", "path": "e2e4 c7c5" }),
        );
        let local = round_trip(
            SessionKind::Local,
            json!({
                "setup": { "variant": "standard", "fen": START }, "moves": ["e2e4"],
                "clock": { "white": { "minutes": 5, "increment": 3 },
                           "black": { "minutes": 5, "increment": 3 } },
                "times": { "white": 300000, "black": 299876.5 },
                "result": { "winner": "black", "reason": "White resigned" },
            }),
        );
        assert!(matches!(
            local,
            SessionDocument::Local(LocalSession { clock: Some(_), .. })
        ));
        round_trip(
            SessionKind::Local,
            json!({ "setup": { "variant": "standard", "fen": START }, "moves": [] }),
        );
        let computer = round_trip(
            SessionKind::Computer,
            json!({
                "moves": ["e2e4", "e7e5"], "ply": 1, "level": "club", "color": "black",
                "resigned": false, "setup": { "variant": "standard", "fen": START },
                "clock": { "minutes": 3, "increment": 2 }, "times": { "white": 1, "black": 2.5 },
                "flagged": "white",
            }),
        );
        assert!(matches!(
            computer,
            SessionDocument::Computer(ComputerSession { ply: 1, .. })
        ));
        round_trip(
            SessionKind::Computer,
            json!({ "version": 1, "moves": [], "level": "max", "color": "white" }),
        );
        for kind in [
            SessionKind::ArchiveComputer,
            SessionKind::ArchiveBoard,
            SessionKind::ArchiveClock,
        ] {
            round_trip(kind, json!({ "id": "game-1", "startedAt": 5 }));
        }
    }

    /// The library's sessions are written in the kinds' order, as the stored documents were.
    #[test]
    fn the_sessions_keep_the_kinds_order() {
        let mut sessions = Sessions::default();
        for kind in SessionKind::ALL.iter().rev() {
            let document = match kind {
                SessionKind::Analysis => json!({ "pgn": "*", "path": "", "orientation": "white",
                    "study": "", "chapter": "" }),
                SessionKind::Local => json!({ "setup": { "variant": "standard", "fen": START },
                    "moves": [], "clock": null, "times": null, "result": null }),
                SessionKind::Computer => json!({ "moves": [], "ply": 0, "level": "club",
                    "color": "white", "resigned": false,
                    "setup": { "variant": "standard", "fen": START },
                    "clock": null, "times": null, "flagged": null }),
                _ => json!({ "id": kind.as_str(), "startedAt": 5 }),
            };
            sessions.insert(
                *kind,
                SessionDocument::read(*kind, document).expect("typed"),
            );
        }
        let keys: Vec<String> = match serde_json::to_value(&sessions).expect("JSON") {
            Value::Object(fields) => fields.keys().cloned().collect(),
            other => panic!("{other}"),
        };
        let kinds: Vec<&str> = SessionKind::ALL.iter().map(|kind| kind.as_str()).collect();
        assert_eq!(keys, kinds);
        assert_eq!(
            sessions
                .take_identity(HistoryKind::Clock)
                .map(|identity| identity.id),
            Some("archive:clock".into())
        );
        assert_eq!(sessions.archive_clock, None);
    }

    /// A stored computer session decodes to its type, and resumes as the game it saved.
    #[test]
    fn a_stored_computer_session_resumes_its_game() {
        let sent = json!({
            "moves": ["e2e4", "e7e5"], "ply": 1, "level": "club", "color": "black",
            "resigned": true, "setup": { "variant": "standard", "fen": START },
            "clock": { "minutes": 3, "increment": 2 }, "times": { "white": 1, "black": 2.5 },
            "flagged": "white",
        });
        let stored = encode_session("computer", &J::from(&sent));
        let Some(SessionDocument::Computer(session)) =
            SessionDocument::decode(SessionKind::Computer, &stored)
        else {
            panic!("a computer session");
        };
        let saved = super::super::computer::Saved::from(session);
        let expected: super::super::computer::Saved = serde_json::from_value(sent).unwrap();
        assert_eq!(saved, expected);
        assert_eq!(
            SessionDocument::decode(SessionKind::Computer, &J::from(&json!("no"))),
            None
        );
    }

    /// A document of another kind is not kept as this one.
    #[test]
    fn a_document_is_kept_only_as_its_kind() {
        let identity = SessionDocument::Archive(Identity {
            id: "g".into(),
            started_at: 1.0,
        });
        let mut sessions = Sessions::default();
        sessions.insert(SessionKind::Computer, identity.clone());
        assert_eq!(sessions, Sessions::default());
        assert!(!identity.is(SessionKind::Local));
        assert_eq!(
            SessionDocument::read(SessionKind::Computer, json!({ "id": "g", "startedAt": 1 })),
            None
        );
    }
}
