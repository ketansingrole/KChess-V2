//! Voice input: what was said, where, and what became of it.

use super::*;

wire_enum!(#[doc = "Where a spoken phrase was said."] VoiceSource, VOICE_SOURCES {
    Computer = "computer",
    Coordinates = "coordinates",
    SquareColor = "squareColor",
    KnightPath = "knightPath",
    Analysis = "analysis",
    Editor = "editor",
    Local = "local",
    Puzzles = "puzzles",
});

wire_enum!(#[doc = "What became of a spoken phrase. Moves: `played` at once, `pending` a choice that then was"] #[doc = "`confirmed` (by voice), `picked` (clicked), `cancelled`, `replaced` by another phrase or"] #[doc = "`abandoned` (the position moved on). `invalid` matched no legal move, `unclear` was too uncertain"] #[doc = "to use, `command` was “confirm”/“cancel”. Coordinates and square colours: `correct` or `wrong`;"] #[doc = "knight paths: `played` for a jump, `invalid` for a square the knight cannot reach."] VoiceOutcome, VOICE_OUTCOMES {
    Played = "played",
    Pending = "pending",
    Confirmed = "confirmed",
    Picked = "picked",
    Cancelled = "cancelled",
    Replaced = "replaced",
    Abandoned = "abandoned",
    Invalid = "invalid",
    Unclear = "unclear",
    Command = "command",
    Correct = "correct",
    Wrong = "wrong",
});

union!(#[doc = "OS permission state of the microphone."] MicrophoneStatus {
    Granted = "granted",
    Denied = "denied",
    Restricted = "restricted",
    NotDetermined = "not-determined",
    Unknown = "unknown",
});

/// One recognized word and the recognizer's confidence in it (0–1).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceWord {
    pub word: String,
    #[serde(with = "js_number")]
    pub conf: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceAttemptInput {
    pub source: VoiceSource,
    /// The recognizer's text, as heard.
    pub heard: String,
    /// Average word confidence, 0–1.
    pub confidence: f64,
    pub words: Vec<VoiceWord>,
    pub outcome: VoiceOutcome,
    /// How KChess read it: SAN choices joined by `|`, a square, or `confirm`/`cancel`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsed: Option<String>,
    /// What the player meant, when known: the square asked for, or the move they went on to play.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    /// The position it was said in (FEN), so the phrase can be parsed again later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fen: Option<String>,
    /// An earlier attempt at the same move or question that this one repeats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_of: Option<i64>,
}

/// A logged phrase: its id and time, then the phrase as saved.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceAttempt {
    pub id: i64,
    pub at: i64,
    #[serde(flatten)]
    pub input: VoiceAttemptInput,
}

/// A logged phrase's later outcome, and what the player meant once known.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct VoiceUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<VoiceOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
}
