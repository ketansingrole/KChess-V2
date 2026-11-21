//! The engine: the one KChess manages, a search's options and its analysis.

use super::*;

union!(#[doc = "Why engine analysis stopped."] AnalysisReason { Completed = "completed", Interrupted = "interrupted", Failed = "failed" });

/// What the managed engine directory holds.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineManaged {
    pub installed: bool,
    pub path: String,
    /// The release tag recorded when it was downloaded, e.g. `sf_17.1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// The engine that would run: ready when its executable is.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub ready: bool,
    /// The executable in use (a downloaded or chosen one), or empty when the bundled engine is.
    pub path: String,
    pub bundled: bool,
    /// The Stockfish KChess downloaded, whether or not it is the one in use.
    pub managed: EngineManaged,
    /// An official native download exists for this platform and architecture.
    pub can_download: bool,
    /// Identity of the executable currently selected, including native file changes; only a
    /// ready engine has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
}

/// The downloaded Stockfish. `updated` is false when it already was the latest release.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineInstalled {
    pub path: String,
    pub version: String,
    pub updated: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", expecting = "an object")]
pub struct BestMoveOptions {
    /// Start from this position instead of the standard one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fen: Option<String>,
    /// Think for this many milliseconds instead of the level's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movetime: Option<u64>,
    /// Chess960 rules: castling is king-takes-rook in UCI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chess960: Option<bool>,
    /// The Lichess key of a variant with rules of its own; Fairy-Stockfish plays it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

impl From<&kchess_domain::games::computer::SearchOptions> for BestMoveOptions {
    fn from(options: &kchess_domain::games::computer::SearchOptions) -> BestMoveOptions {
        BestMoveOptions {
            fen: options.fen.clone(),
            movetime: options.movetime.map(|ms| ms as u64),
            chess960: options.chess960.then_some(true),
            variant: options.variant.map(|variant| variant.key().to_string()),
        }
    }
}

/// Ask the analysis engine to study one position until stopped or a limit is reached.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase", expecting = "an object")]
pub struct AnalysisRequest {
    pub fen: String,
    /// How many of the best lines to report (MultiPV).
    pub lines: u32,
    /// Keep searching until stopped instead of stopping at a sensible depth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infinite: Option<bool>,
    /// Repetition context: root plus moves must produce fen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_fen: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moves: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<u64>,
    /// The Lichess variant key (`crazyhouse`, `atomic`…) when the position is not standard chess;
    /// those go to Fairy-Stockfish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

/// One engine line. Scores are from White's point of view, like Lichess shows them.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineLine {
    /// 1 for the best line, 2 for the next best…
    pub rank: u32,
    pub depth: f64,
    /// Centipawns, when there is no forced mate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cp: Option<f64>,
    /// Moves to mate: positive when White mates, negative when Black does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mate: Option<f64>,
    /// Principal variation as UCI moves.
    pub pv: Vec<String>,
}

/// The payload of `engine:analysis`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisUpdate {
    /// The request this belongs to; a frontend drops updates of superseded ones.
    pub id: u64,
    pub fen: String,
    pub depth: f64,
    /// Nodes per second, for the engine's status line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nps: Option<f64>,
    pub lines: Vec<EngineLine>,
    /// The search finished (limit reached, or stopped) and no more updates follow.
    pub done: bool,
    pub context: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<AnalysisReason>,
    pub engine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
