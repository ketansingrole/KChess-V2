//! ChatGPT: the connection, its models and an explanation asked of it.

use super::*;

wire_enum!(#[doc = "How hard the model thinks; `auto` leaves it to the model."] AiEffort, AI_EFFORTS {
    Auto = "auto",
    Low = "low",
    Medium = "medium",
    High = "high",
});

union!(#[doc = "Why an AI request came to nothing; each asks something different of the person: `connect`, sign in again (the token was refused for good); `limit`, the allowance ran out; `ineligible`, this plan, workspace or region cannot use it; `unsupported`, the model refuses the effort; `unavailable`, temporary (the credentials stay); `unverified`, the answer named something outside the facts, twice."] AiProblem {
    Connect = "connect",
    Limit = "limit",
    Ineligible = "ineligible",
    Unsupported = "unsupported",
    Unavailable = "unavailable",
    Unverified = "unverified",
});

union!(#[doc = "What an explanation is about: a played move, a hint, a puzzle's solution or a wrong puzzle move."] AiKind {
    Move = "move",
    Nudge = "nudge",
    Solution = "solution",
    Wrong = "wrong",
});

/// The ChatGPT connection, and the model and effort every AI feature uses.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiConnection {
    pub connected: bool,
    pub registered: bool,
    /// `null` when no account is signed in.
    pub email: Option<String>,
    /// `null`: the account's first model.
    pub model: Option<String>,
    pub effort: AiEffort,
    /// The chosen model refused the effort, so requests leave it out.
    pub effort_refused: bool,
}

impl Default for AiConnection {
    fn default() -> AiConnection {
        AiConnection {
            connected: false,
            registered: false,
            email: None,
            model: None,
            effort: AiEffort::Auto,
            effort_refused: false,
        }
    }
}

/// A model on offer, as the picker shows it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AiModel {
    pub slug: String,
    pub name: String,
}

/// The models the account may use, or why they could not be listed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum AiModelList {
    Listed { models: Vec<AiModel> },
    Refused { problem: AiProblem },
}

/// What `aiExplain` is asked to explain.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiRequest {
    pub kind: AiKind,
    pub fen: String,
    /// The move played (UCI): `move` and `wrong` need it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub played: Option<String>,
    /// The line from `fen`: a puzzle's solution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moves: Vec<String>,
    /// The engine's line instead of `played`, and its reply to `played`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub best: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refutation: Vec<String>,
    /// Moves to mate before and after `played`, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mate_before: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mate_after: Option<f64>,
}

/// An explanation, or why there is none.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum AiExplanation {
    Told { text: String },
    Refused { problem: AiProblem },
}
