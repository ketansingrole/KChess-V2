//! The playable variants and the start of a game.

use serde::{Deserialize, Serialize};

pub use kchess_domain::variant::{VARIANTS, Variant};

/// A game's starting point.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GameSetup {
    pub variant: Variant,
    /// FEN before the first move; for standard games, the usual start.
    pub fen: String,
}
