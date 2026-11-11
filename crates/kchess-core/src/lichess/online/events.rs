use serde_json::Value;

use super::state::CHALLENGE_TYPES;

pub(crate) fn is_challenge_event(kind: &str) -> bool {
    CHALLENGE_TYPES.contains(&kind)
}

pub(crate) use kchess_domain::records::online::validate as validate_online_event;

/// Whether a game with this Lichess `status` is still being played.
pub(crate) fn game_in_progress(status: &str) -> bool {
    kchess_domain::records::games::is_game_in_progress(Some(&Value::from(status)))
}
