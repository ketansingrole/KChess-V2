//! The training constants (`crates/kchess-domain/data/training.json`), parsed once.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::endgames::EndgameDrill;
use super::engine_levels::EngineLevelInfo;
use super::rush::RushConfig;

const TRAINING_JSON: &str = include_str!("../../data/training.json");

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Data {
    pub rush_configs: BTreeMap<String, RushConfig>,
    pub rush_choices: Vec<String>,
    pub endgame_drills: Vec<EndgameDrill>,
    pub engine_ladder: Vec<EngineLevelInfo>,
    pub default_engine_levels: Vec<String>,
}

/// The constants; an error only if the embedded file is malformed.
pub fn data() -> &'static Data {
    static DATA: OnceLock<Data> = OnceLock::new();
    DATA.get_or_init(|| crate::embedded::parse("training tables", TRAINING_JSON))
}
