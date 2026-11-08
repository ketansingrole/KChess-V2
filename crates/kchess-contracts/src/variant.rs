//! The playable variants and the start of a game.

use serde::{Deserialize, Serialize};

/// Lichess variant keys KChess can play.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    #[serde(rename = "standard")]
    Standard,
    #[serde(rename = "chess960")]
    Chess960,
    #[serde(rename = "kingOfTheHill")]
    KingOfTheHill,
    #[serde(rename = "threeCheck")]
    ThreeCheck,
    #[serde(rename = "antichess")]
    Antichess,
    #[serde(rename = "atomic")]
    Atomic,
    #[serde(rename = "horde")]
    Horde,
    #[serde(rename = "racingKings")]
    RacingKings,
    #[serde(rename = "crazyhouse")]
    Crazyhouse,
}

/// The variant keys in `Variant`'s order.
pub use kchess_domain::misc::constants::VARIANTS;

/// A game's starting point.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GameSetup {
    pub variant: Variant,
    /// FEN before the first move; for standard games, the usual start.
    pub fen: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_enum_names_the_domain_variants_in_order() {
        let all = [
            Variant::Standard,
            Variant::Chess960,
            Variant::KingOfTheHill,
            Variant::ThreeCheck,
            Variant::Antichess,
            Variant::Atomic,
            Variant::Horde,
            Variant::RacingKings,
            Variant::Crazyhouse,
        ];
        let keys: Vec<serde_json::Value> = all
            .iter()
            .map(|variant| serde_json::to_value(variant).expect("a key"))
            .collect();
        assert_eq!(keys, VARIANTS);
    }
}
