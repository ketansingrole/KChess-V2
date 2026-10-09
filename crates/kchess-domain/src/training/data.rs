//! The training constants shared with TypeScript (`core/src/domain/data/training.json`), parsed
//! once. TypeScript imports the same file, so the two cannot drift.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::endgames::EndgameDrill;
use super::engine_levels::EngineLevelInfo;
use super::rush::RushConfig;

const TRAINING_JSON: &str = include_str!("../../../../core/src/domain/data/training.json");

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
pub fn data() -> Result<&'static Data, String> {
    static DATA: OnceLock<Option<Data>> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(TRAINING_JSON).ok())
        .as_ref()
        .ok_or_else(|| "training data is not valid JSON".to_string())
}
