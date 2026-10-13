use serde_json::Value;

use super::super::challenges::PlayerRef;
use super::super::client::Failure;
use super::state::{OngoingGame, State};

/// `checking` of a resume: the account being checked, whether it lacked a login, and the failure
/// to report if no account has a game.
#[derive(Default)]
pub(crate) struct Checking {
    pub(crate) account: String,
    pub(crate) auth_required: bool,
    pub(crate) error: Option<Failure>,
}

/// `finished()` of a game controller: the stream it belongs to is still the current one.
pub(crate) fn finished_check(st: &State, handle_id: u64) -> bool {
    st.game_controller
        .as_ref()
        .is_some_and(|controller| controller.id == handle_id)
}

pub(crate) fn ongoing_game(game: &Value, account: &str) -> OngoingGame {
    let opponent = &game["opponent"];
    let ai = opponent["ai"]
        .as_f64()
        .filter(|level| *level != 0.0)
        .is_some();
    let name = if ai {
        format!("Stockfish level {}", js_value_text(&opponent["ai"]))
    } else {
        opponent["username"]
            .as_str()
            .unwrap_or("Opponent")
            .to_string()
    };
    OngoingGame {
        game_id: game["gameId"].as_str().unwrap_or("").to_string(),
        account: account.to_string(),
        opponent: PlayerRef {
            name,
            rating: opponent["rating"].as_number().cloned(),
            title: None,
            provisional: None,
            online: None,
        },
        color: game["color"].as_str().unwrap_or("").to_string(),
        fen: game["fen"].as_str().unwrap_or("").to_string(),
        last_move: game["lastMove"]
            .as_str()
            .filter(|mv| !mv.is_empty())
            .map(str::to_string),
        is_my_turn: game["isMyTurn"].as_bool().unwrap_or(false),
        seconds_left: game["secondsLeft"].as_number().cloned(),
        variant: game["variant"]["key"]
            .as_str()
            .unwrap_or("standard")
            .to_string(),
        speed: game["speed"].as_str().unwrap_or("").to_string(),
        rated: game["rated"].as_bool().unwrap_or(false),
        tournament_id: game["tournamentId"].as_str().map(str::to_string),
    }
}

pub(crate) fn js_value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}
