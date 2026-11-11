//! A Lichess broadcast as every frontend opens it.

use serde_json::Value;

/// The round a broadcast tournament (`broadcastTour`'s answer) opens at: the one Lichess names
/// (`defaultRoundId`), else the one being played, else the last.
pub fn opening_round(tour: &Value) -> Option<&Value> {
    let rounds = tour["rounds"].as_array()?;
    rounds
        .iter()
        .find(|round| round["id"] == tour["defaultRoundId"])
        .or_else(|| rounds.iter().find(|round| round["ongoing"] == true))
        .or(rounds.last())
}

#[cfg(test)]
mod tests {
    use super::opening_round;
    use serde_json::json;

    #[test]
    fn a_broadcast_opens_at_its_named_round_else_the_live_one_else_the_last() {
        let id =
            |tour| opening_round(&tour).and_then(|round| round["id"].as_str().map(String::from));
        let rounds = json!([{ "id": "r1" }, { "id": "r2", "ongoing": true }, { "id": "r3" }]);
        assert_eq!(
            id(json!({ "rounds": rounds, "defaultRoundId": "r3" })).as_deref(),
            Some("r3")
        );
        assert_eq!(id(json!({ "rounds": rounds })).as_deref(), Some("r2"));
        assert_eq!(
            id(json!({ "rounds": [{ "id": "r1" }, { "id": "r2" }] })).as_deref(),
            Some("r2")
        );
        assert_eq!(id(json!({ "rounds": [] })), None);
        assert_eq!(id(json!({})), None);
    }
}
