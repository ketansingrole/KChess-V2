//! `core/src/domain/variant.ts`: the playable variants and the start of a game.

use serde::{Deserialize, Serialize};
use ts_rs::{Config, TS};

use crate::generate::Item;
use crate::strs;

/// Lichess variant keys KChess can play. Crazyhouse is missing: its pocket needs a drop UI.
#[derive(Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// The variant keys in `Variant`'s order.
pub const VARIANTS: &[&str] = &[
    "standard",
    "chess960",
    "kingOfTheHill",
    "threeCheck",
    "antichess",
    "atomic",
    "horde",
    "racingKings",
];

/// A game's starting point.
#[derive(Serialize, Deserialize, TS, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GameSetup {
    pub variant: Variant,
    /// FEN before the first move; for standard games, the usual start.
    pub fen: String,
}

pub fn items(cfg: &Config) -> Vec<Item> {
    vec![
        Item::Const {
            name: "VARIANTS".into(),
            values: strs(VARIANTS),
            suffix: String::new(),
        },
        Item::of::<Variant>(cfg),
        Item::of::<GameSetup>(cfg),
    ]
}
