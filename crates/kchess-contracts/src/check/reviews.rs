//! The arguments of the review methods: a game to review, a review's key and games' ids.

use serde::{Deserialize, Serialize};

use kchess_domain::misc::patterns;
use kchess_domain::misc::value::J;
use kchess_domain::{js, replay};

use super::*;
use crate::types::ReviewRequest;

const REVIEW: &str = "Invalid review.";
const HISTORY: &str = "Invalid game position or move history.";

/// A review's key (`reviewKey`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct ReviewKey(pub String);

impl Validate for ReviewKey {
    fn validate(value: J) -> Result<Self, Refusal> {
        let key: String = decode(value, |_| Some(REVIEW))?;
        matches(&key, patterns::review_key, Some(REVIEW), "", "")?;
        Ok(ReviewKey(key))
    }
}

/// Up to one history page of Lichess game ids.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(transparent)]
pub struct GameIds(pub Vec<String>);

impl Validate for GameIds {
    fn validate(value: J) -> Result<Self, Refusal> {
        let ids: Vec<String> = decode(value, |keys| match keys {
            [] => Some("Invalid game list."),
            _ => Some("Invalid game id."),
        })?;
        for id in &ids {
            matches(id, patterns::game_id, Some("Invalid game id."), "", "")?;
        }
        max_items(&ids, 100, Some("Too many games."), "")?;
        Ok(GameIds(ids))
    }
}

/// The request's moves replay from its position, by its variant's rules for a variant game; a
/// variant with standard rules is dropped and the account is trimmed.
pub fn review_request(mut request: ReviewRequest) -> Result<ReviewRequest, Refusal> {
    let variant = other_variant(request.variant.as_deref())?;
    match variant {
        Some(_) => max_chars(&request.fen, 120, Some("Invalid position."), "fen")?,
        None => {
            request.variant = None;
            fen(&request.fen, None, Some("Invalid position."), "fen")?;
            engine_position(&request.fen)?;
        }
    }
    move_list(&request.moves)?;
    if let Some(id) = &request.game_id {
        matches(id, patterns::game_id, Some("Invalid game id."), "", "")?;
    }
    if let Some(account) = &request.account {
        let trimmed = js::trim(account);
        matches(
            trimmed,
            patterns::game_account,
            Some(USERNAME),
            "account",
            "",
        )?;
        request.account = Some(trimmed.to_owned());
    }
    let replayed = match variant {
        Some(variant) => {
            replay::replay_setup_count(variant, &request.fen, &request.moves)
                == Some(request.moves.len())
        }
        None => {
            replay::replay_positions_count(&request.fen, &request.moves) == request.moves.len() + 1
        }
    };
    ensure(replayed, Some(HISTORY), "", String::new)?;
    Ok(request)
}

impl Validate for ReviewRequest {
    fn validate(value: J) -> Result<Self, Refusal> {
        let request: ReviewRequest = decode(value, |keys| match keys {
            ["fen"] => Some("Invalid position."),
            ["moves", rest @ ..] => move_list_messages(rest),
            ["gameId"] => Some("Invalid game id."),
            ["account"] => Some(USERNAME),
            ["variant"] => Some("Invalid variant."),
            _ => Some("Invalid review request."),
        })?;
        review_request(request)
    }
}

/// `reviewGet`'s position and moves: without a variant the position alone is checked first, as
/// analysis would take it; then they must make a review.
pub fn review_get(
    (Fen(fen), Moves(moves), variant): &(Fen, Moves, Option<String>),
) -> Result<(), Refusal> {
    if variant.is_none() {
        analysis_position(fen, None)?;
    }
    review_request(ReviewRequest {
        fen: fen.clone(),
        moves: moves.clone(),
        variant: variant.clone(),
        game_id: None,
        account: None,
    })
    .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_review_key_and_game_ids_are_checked() {
        assert_eq!(check::<ReviewKey>(J::Str("x".into())), Err(REVIEW.into()));
        assert_eq!(
            check::<GameIds>(J::Arr(vec![J::Str("not an id!".into())])),
            Err("Invalid game id.".into())
        );
        let many = J::Arr(vec![J::Str("abcdefgh".into()); 101]);
        assert_eq!(check::<GameIds>(many), Err("Too many games.".into()));
    }

    #[test]
    fn a_review_request_is_checked_field_by_field() {
        let obj = |fields: Vec<(&str, J)>| {
            J::Obj(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
        };
        let moves = obj(vec![
            ("fen", J::Str("x".into())),
            ("moves", J::Arr(vec![J::Num(1.0)])),
        ]);
        assert_eq!(check::<ReviewRequest>(moves), Err("Invalid move.".into()));
        // Black has a seventeenth piece: Stockfish quit on it ("More than 32 pieces").
        let crowded = "rnbqkbnr/pppppppp/8/8/3q3N/4K3/1PPPPPPP/RNBQ1BNR w kq - 0 1";
        let review = obj(vec![
            ("fen", J::Str(crowded.into())),
            ("moves", J::Arr(vec![])),
        ]);
        assert_eq!(
            check::<ReviewRequest>(review),
            Err("Each side can have at most 16 pieces.".into())
        );
    }

    #[test]
    fn a_review_of_moves_that_do_not_replay_is_refused() {
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let asked = (Fen(start.into()), Moves(vec!["e2e5".into()]), None);
        assert_eq!(review_get(&asked), Err(HISTORY.into()));
        let asked = (
            Fen(start.into()),
            Moves(vec!["e2e4".into()]),
            Some("chess960".into()),
        );
        assert_eq!(review_get(&asked), Ok(()));
    }
}
