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

/// The level of `levels` whose rating is nearest `rating`, the weaker on a tie; full strength
/// (no rating) only when nothing else is offered.
pub fn nearest(levels: &[String], rating: u32) -> Option<String> {
    let rated = levels.iter().filter_map(|id| Some((id, info(id)?.elo?)));
    rated
        .min_by(|(_, a), (_, b)| {
            let (a, b) = ((a - f64::from(rating)).abs(), (b - f64::from(rating)).abs());
            a.total_cmp(&b)
        })
        .map(|(id, _)| id.clone())
        .or_else(|| levels.first().cloned())
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

    #[test]
    fn the_nearest_offered_level_to_a_rating() {
        let levels: Vec<String> = ["beginner", "club", "expert", "im", "max"]
            .map(String::from)
            .to_vec();
        assert_eq!(nearest(&levels, 2150).as_deref(), Some("expert"));
        assert_eq!(nearest(&levels, 2250).as_deref(), Some("im"));
        assert_eq!(nearest(&levels, 300).as_deref(), Some("beginner"));
        assert_eq!(nearest(&levels, 3200).as_deref(), Some("im"));
        // A tie goes to the weaker: club (1600) and expert (2000) are both 200 from 1800.
        assert_eq!(nearest(&levels, 1800).as_deref(), Some("club"));
        assert_eq!(nearest(&["max".to_string()], 1500).as_deref(), Some("max"));
        assert_eq!(nearest(&[], 1500), None);
    }
}
