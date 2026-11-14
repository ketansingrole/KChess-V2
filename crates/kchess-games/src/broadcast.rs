//! A Lichess broadcast as every frontend opens it.

use kchess_contracts::types::{BroadcastRoundRef, BroadcastTourDetail};

/// The round a broadcast tournament (`broadcastTour`'s answer) opens at: the one Lichess names
/// (`defaultRoundId`), else the one being played, else the last.
pub fn opening_round(tour: &BroadcastTourDetail) -> Option<&BroadcastRoundRef> {
    let rounds = &tour.rounds;
    rounds
        .iter()
        .find(|round| tour.default_round_id.as_deref() == Some(round.id.as_str()))
        .or_else(|| rounds.iter().find(|round| round.ongoing))
        .or(rounds.last())
}

#[cfg(test)]
mod tests {
    use super::opening_round;
    use kchess_contracts::types::{BroadcastRoundRef, BroadcastTourDetail};

    fn round(id: &str, ongoing: bool) -> BroadcastRoundRef {
        BroadcastRoundRef {
            id: id.into(),
            name: id.into(),
            ongoing,
            finished: false,
            starts_at: None,
        }
    }

    fn tour(rounds: Vec<BroadcastRoundRef>, default_round_id: Option<&str>) -> BroadcastTourDetail {
        BroadcastTourDetail {
            id: "t".into(),
            name: "T".into(),
            description: None,
            rounds,
            default_round_id: default_round_id.map(String::from),
        }
    }

    #[test]
    fn a_broadcast_opens_at_its_named_round_else_the_live_one_else_the_last() {
        let id = |tour: BroadcastTourDetail| opening_round(&tour).map(|round| round.id.clone());
        let rounds = || vec![round("r1", false), round("r2", true), round("r3", false)];
        assert_eq!(id(tour(rounds(), Some("r3"))).as_deref(), Some("r3"));
        assert_eq!(id(tour(rounds(), None)).as_deref(), Some("r2"));
        assert_eq!(id(tour(rounds(), Some("gone"))).as_deref(), Some("r2"));
        let quiet = vec![round("r1", false), round("r2", false)];
        assert_eq!(id(tour(quiet, None)).as_deref(), Some("r2"));
        assert_eq!(id(tour(Vec::new(), None)), None);
    }
}
