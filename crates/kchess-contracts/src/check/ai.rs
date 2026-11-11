//! The arguments of the ChatGPT methods: the model and effort, and what to explain.

use serde::{Deserialize, Serialize};

use kchess_domain::misc::value::J;

use super::*;
use crate::types::{AI_EFFORTS, AiEffort, AiRequest};

const MODEL: &str = "Invalid model.";
const UNEXPLAINABLE: &str = "That position cannot be explained.";

/// A model's slug: 1 to 100 bytes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct AiModelName(pub String);

impl Validate for AiModelName {
    fn validate(value: J) -> Result<Self, Refusal> {
        let model: String = decode(value, |_| Some(MODEL))?;
        ensure(
            !model.is_empty() && model.len() <= 100,
            Some(MODEL),
            "",
            String::new,
        )?;
        Ok(AiModelName(model))
    }
}

impl Validate for AiEffort {
    fn validate(value: J) -> Result<Self, Refusal> {
        picked(value, AI_EFFORTS, Some("Invalid effort."))
    }
}

/// A position of at most 100 bytes, lines of at most 64 moves and a played move of at most 5.
impl Validate for AiRequest {
    fn validate(value: J) -> Result<Self, Refusal> {
        let request: AiRequest = decode(value, |_| Some(UNEXPLAINABLE))?;
        let lines = [&request.moves, &request.best, &request.refutation];
        let usable = request.fen.len() <= 100
            && lines.iter().all(|line| line.len() <= 64)
            && request
                .played
                .as_ref()
                .is_none_or(|played| played.len() <= 5);
        ensure(usable, Some(UNEXPLAINABLE), "", String::new)?;
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A position of 100 bytes, lines of 64 moves and a played move of 5 are explained; one more
    /// of any is not.
    #[test]
    fn a_request_is_short_enough_to_explain() {
        let line = |n: usize| vec!["e2e4"; n];
        let request = |fen: usize, moves: usize, best: usize, refutation: usize, played: &str| {
            json!({
                "kind": "move",
                "fen": "f".repeat(fen),
                "moves": line(moves),
                "best": line(best),
                "refutation": line(refutation),
                "played": played,
            })
        };
        let explained = request(100, 64, 64, 64, "e7e8q");
        assert_eq!(checked::<AiRequest>(explained.clone()), Ok(explained));
        for refused in [
            request(101, 0, 0, 0, "e2e4"),
            request(10, 65, 0, 0, "e2e4"),
            request(10, 0, 65, 0, "e2e4"),
            request(10, 0, 0, 65, "e2e4"),
            request(10, 0, 0, 0, "e7e8qq"),
        ] {
            assert_eq!(
                checked::<AiRequest>(refused.clone()),
                Err(UNEXPLAINABLE.into()),
                "{refused}"
            );
        }
    }

    #[test]
    fn a_model_is_named_and_short() {
        assert_eq!(check::<Option<AiModelName>>(J::Null), Ok(None));
        assert_eq!(
            check::<AiModelName>(J::Str(String::new())),
            Err(MODEL.into())
        );
        assert_eq!(
            check::<AiModelName>(J::Str("m".repeat(101))),
            Err(MODEL.into())
        );
        assert_eq!(check::<AiEffort>(J::Str("low".into())), Ok(AiEffort::Low));
        assert_eq!(
            check::<AiEffort>(J::Str("max".into())),
            Err("Invalid effort.".into())
        );
    }
}
