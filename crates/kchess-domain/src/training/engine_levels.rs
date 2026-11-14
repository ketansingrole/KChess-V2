//! Engine strength levels: the ladder from weakest to
//! strongest, and how a chosen set of levels is kept and substituted.

use serde::{Deserialize, Serialize};

use super::data::data;
use crate::js;

/// One rung of the ladder (`EngineLevelInfo`), from `data/training.json`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineLevelInfo {
    pub id: String,
    pub label: String,
    pub elo: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uci_elo: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub random_move: Option<f64>,
    pub time: f64,
}

/// The ladder from the weakest level to the strongest.
pub fn ladder() -> &'static [EngineLevelInfo] {
    &data().engine_ladder
}

/// The levels a new profile enables.
pub fn default_levels() -> &'static [String] {
    &data().default_engine_levels
}

/// The ladder entry for a level, none for an unknown one.
pub fn info(id: &str) -> Option<&'static EngineLevelInfo> {
    ladder().iter().find(|entry| entry.id == id)
}

/// "Grandmaster · ~2550", "Stockfish Max · Full strength".
pub fn label(id: &str) -> Result<String, String> {
    let entry = info(id).ok_or_else(|| format!("unknown engine level {id}"))?;
    let strength = match entry.elo {
        Some(elo) => format!("~{}", js::number_to_string(elo)),
        None => "Full strength".into(),
    };
    Ok(format!("{} · {strength}", entry.label))
}

/// The known levels of `levels`, in ladder order; the default levels when none is known.
pub fn normalize(levels: &[String]) -> Vec<String> {
    let kept: Vec<String> = ladder()
        .iter()
        .filter(|entry| levels.contains(&entry.id))
        .map(|entry| entry.id.clone())
        .collect();
    if kept.is_empty() {
        default_levels().to_vec()
    } else {
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_levels() {
        assert_eq!(label("im").as_deref(), Ok("International Master · ~2400"));
        assert_eq!(label("max").as_deref(), Ok("Stockfish Max · Full strength"));
        assert_eq!(
            normalize(&["gm".into(), "bogus".into(), "beginner".into()]),
            vec!["beginner".to_string(), "gm".into()]
        );
        assert_eq!(normalize(&[]), default_levels());
    }
}
