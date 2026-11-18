//! Timed runs as the core keeps them (Puzzle Rush, the practice trainers): a run to save, and the
//! best score saved.

use kchess_contracts::types::{RunInput, RunKind, RunSummary};
use serde_json::Value;

/// The best score of a variant in a summary.
pub fn best(summary: Option<&RunSummary>, variant: &str) -> Option<f64> {
    Some(summary?.best.get(variant)?.score)
}

/// A finished run of `kind` to save; `detail` is an object of numbers, words and flags.
pub fn input(kind: RunKind, variant: impl Into<String>, score: f64, detail: Value) -> RunInput {
    RunInput {
        kind,
        variant: variant.into(),
        score,
        detail: match detail {
            Value::Object(detail) => detail,
            _ => serde_json::Map::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A run's detail is kept when it is an object of fields, and dropped otherwise.
    #[test]
    fn a_run_keeps_its_detail_when_it_has_fields() {
        let run = input(RunKind::Rush, "standard", 12., json!({ "mistakes": 2 }));
        assert_eq!(run.variant, "standard");
        assert_eq!(run.score, 12.);
        assert_eq!(run.detail.get("mistakes"), Some(&json!(2)));
        assert!(
            input(RunKind::Rush, "standard", 1., json!(3))
                .detail
                .is_empty()
        );
    }

    #[test]
    fn the_best_score_is_the_variants() {
        let summary: RunSummary = serde_json::from_value(json!({
            "kind": "rush",
            "best": { "standard": { "score": 9.0, "playedAt": 0 } },
            "recent": [],
            "total": 1,
        }))
        .expect("a summary");
        assert_eq!(best(Some(&summary), "standard"), Some(9.));
        assert_eq!(best(Some(&summary), "chess960"), None);
        assert_eq!(best(None, "standard"), None);
    }
}
