//! Engine strength levels (`core/src/domain/engineLevels.ts`): the ladder from weakest to
//! strongest, and how a chosen set of levels is kept and substituted.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::data::data;
use super::{Out, text_arg, text_of, to_value, value_arg};

/// One rung of the ladder (`EngineLevelInfo`); the data is shared with TypeScript.
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

fn ladder() -> Result<&'static [EngineLevelInfo], String> {
    Ok(&data()?.engine_ladder)
}

fn find(id: &str) -> Result<Option<&'static EngineLevelInfo>, String> {
    Ok(ladder()?.iter().find(|entry| entry.id == id))
}

/// `engineLevelLabel`: "Grandmaster · ~2550", "Stockfish Max · Full strength".
fn label(id: &str) -> Result<String, String> {
    let entry = find(id)?.ok_or_else(|| format!("unknown engine level {id}"))?;
    let strength = match entry.elo {
        Some(elo) if elo != 0.0 => format!("~{}", text_of(elo)),
        _ => "Full strength".into(),
    };
    Ok(format!("{} · {strength}", entry.label))
}

/// `normalizeEngineLevels`: the known levels, in ladder order; never empty.
fn normalize(levels: &[String]) -> Result<Vec<String>, String> {
    let kept: Vec<String> = ladder()?
        .iter()
        .filter(|entry| levels.contains(&entry.id))
        .map(|entry| entry.id.clone())
        .collect();
    if kept.is_empty() {
        Ok(data()?.default_engine_levels.clone())
    } else {
        Ok(kept)
    }
}

/// `nearestEngineLevel`: the enabled level closest in strength to `wanted`.
fn nearest(wanted: &str, enabled: &[String]) -> Result<String, String> {
    if enabled.iter().any(|id| id == wanted) || enabled.is_empty() {
        return Ok(wanted.to_string());
    }
    let rank = |id: &str| {
        ladder()
            .ok()
            .and_then(|ladder| ladder.iter().position(|entry| entry.id == id))
            .map_or(-1, |index| index as i64)
    };
    let target = rank(wanted);
    let mut sorted: Vec<&String> = enabled.iter().collect();
    // A stable sort, as `Array.prototype.sort` is, so equally close levels keep their order.
    sorted.sort_by_key(|id| (rank(id) - target).abs());
    sorted
        .first()
        .map(|id| (*id).clone())
        .ok_or_else(|| "no engine levels are enabled".into())
}

pub fn call(method: &str, args: &[Value]) -> Option<Out> {
    let result = match method {
        "engineLadder" => ladder().and_then(to_value),
        "defaultEngineLevels" => data().and_then(|d| to_value(&d.default_engine_levels)),
        "engineLevelInfo" => text_arg(args, 0, "id")
            .and_then(find)
            .and_then(|entry| to_value(entry.cloned())),
        "engineLevelLabel" => text_arg(args, 0, "id").and_then(label).map(Value::String),
        "normalizeEngineLevels" => value_arg::<Vec<String>>(args, 0, "levels")
            .and_then(|levels| normalize(&levels))
            .and_then(to_value),
        "nearestEngineLevel" => text_arg(args, 0, "wanted").and_then(|wanted| {
            let enabled: Vec<String> = value_arg(args, 1, "enabled")?;
            nearest(wanted, &enabled).map(Value::String)
        }),
        _ => return None,
    };
    Some(result)
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
            Ok(vec!["beginner".into(), "gm".into()])
        );
        assert_eq!(
            normalize(&[]).ok(),
            data().ok().map(|d| d.default_engine_levels.clone())
        );
    }

    #[test]
    fn nearest_level_is_the_closest_enabled_one() {
        let set = |ids: &[&str]| ids.iter().map(|id| (*id).to_string()).collect::<Vec<_>>();
        assert_eq!(nearest("fm", &set(&["fm", "gm"])).as_deref(), Ok("fm"));
        assert_eq!(
            nearest("cm", &set(&["beginner", "im", "max"])).as_deref(),
            Ok("im")
        );
        assert_eq!(
            nearest("casual", &set(&["beginner", "gm"])).as_deref(),
            Ok("beginner")
        );
    }
}
