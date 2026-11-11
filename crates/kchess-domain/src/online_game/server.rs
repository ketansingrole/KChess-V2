//! What Lichess's board streams send about a game, as the online game reads it. A field of an
//! unexpected type reads as absent, so one odd field never loses the rest of the event.

use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::model::ChatRoom;
use crate::js;

/// A field read as `T`, absent when it is missing, null, or of another type.
fn lenient<'de, D: Deserializer<'de>, T: serde::de::DeserializeOwned>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Ok(serde_json::from_value(Value::deserialize(deserializer)?).ok())
}

/// One event of the game or event stream; a type the game has no use for is `Other`.
#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerEvent {
    GameStart {
        #[serde(default, deserialize_with = "lenient")]
        game: Option<GameRef>,
    },
    GameFinish {
        #[serde(default, deserialize_with = "lenient")]
        game: Option<GameRef>,
    },
    OpponentGone(OpponentGone),
    ChatLine(ChatLine),
    GameFull(Box<GameFull>),
    GameState(GameState),
    #[default]
    #[serde(other)]
    Other,
}

impl ServerEvent {
    /// The event a stream's JSON is; `Other` when it is not one the game reads.
    pub fn read(event: &Value) -> ServerEvent {
        ServerEvent::deserialize(event).unwrap_or_default()
    }
}

/// The game a `gameStart` or `gameFinish` names.
#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameRef {
    #[serde(default, deserialize_with = "lenient")]
    pub game_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub speed: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub color: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub rated: Option<bool>,
    #[serde(default, deserialize_with = "lenient")]
    pub opponent: Option<Opponent>,
    #[serde(default, deserialize_with = "lenient")]
    pub winner: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub status: Option<Status>,
    #[serde(default, deserialize_with = "lenient")]
    pub compat: Option<Compat>,
}

impl GameRef {
    pub fn correspondence(&self) -> bool {
        self.speed.as_deref() == Some("correspondence")
    }

    /// Whether the Board API can play it; a game started on another device may not be.
    pub fn board_compatible(&self) -> bool {
        self.compat.as_ref().and_then(|compat| compat.board) != Some(false)
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Opponent {
    #[serde(default, deserialize_with = "lenient")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub username: Option<String>,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Status {
    #[serde(default, deserialize_with = "lenient")]
    pub name: Option<String>,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Compat {
    #[serde(default, deserialize_with = "lenient")]
    pub board: Option<bool>,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct OpponentGone {
    #[serde(default, deserialize_with = "lenient")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub gone: Option<bool>,
    #[serde(default, deserialize_with = "lenient")]
    pub claim_win_in_seconds: Option<f64>,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
pub struct ChatLine {
    #[serde(default, deserialize_with = "lenient")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub room: Option<ChatRoom>,
    #[serde(default, deserialize_with = "lenient")]
    pub username: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub text: Option<String>,
}

/// A `gameState`, or the `state` of a `gameFull`.
#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameState {
    #[serde(default, deserialize_with = "lenient")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub moves: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub wtime: Option<f64>,
    #[serde(default, deserialize_with = "lenient")]
    pub btime: Option<f64>,
    #[serde(default, deserialize_with = "lenient")]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub winner: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub wdraw: Option<bool>,
    #[serde(default, deserialize_with = "lenient")]
    pub bdraw: Option<bool>,
    #[serde(default, deserialize_with = "lenient")]
    pub wtakeback: Option<bool>,
    #[serde(default, deserialize_with = "lenient")]
    pub btakeback: Option<bool>,
    #[serde(default, deserialize_with = "lenient")]
    pub expiration: Option<Expiration>,
}

impl GameState {
    /// The side (`white` or `black`) offers a draw.
    pub fn draw(&self, white: bool) -> bool {
        if white { self.wdraw } else { self.bdraw }.unwrap_or(false)
    }

    /// The side (`white` or `black`) asks to take back a move.
    pub fn takeback(&self, white: bool) -> bool {
        if white {
            self.wtakeback
        } else {
            self.btakeback
        }
        .unwrap_or(false)
    }

    /// The game goes on: no status yet, or `created` or `started`.
    pub fn running(&self) -> bool {
        matches!(
            self.status.as_deref(),
            None | Some("" | "started" | "created")
        )
    }
}

/// The first-move timer.
#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Expiration {
    #[serde(default, deserialize_with = "lenient")]
    pub millis_to_move: Option<f64>,
    #[serde(default, deserialize_with = "lenient")]
    pub idle_millis: Option<f64>,
}

/// The first event of a game's stream: the game and its state.
#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameFull {
    #[serde(default, deserialize_with = "lenient")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub clock: Option<Clock>,
    #[serde(default, deserialize_with = "lenient")]
    pub rated: Option<bool>,
    #[serde(default, deserialize_with = "lenient")]
    pub speed: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub days_per_turn: Option<f64>,
    #[serde(default, deserialize_with = "lenient")]
    pub tournament_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub variant: Option<VariantKey>,
    #[serde(default, deserialize_with = "lenient")]
    pub initial_fen: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub white: Option<Player>,
    #[serde(default, deserialize_with = "lenient")]
    pub black: Option<Player>,
    #[serde(default, deserialize_with = "lenient")]
    pub state: Option<GameState>,
}

/// A real-time game's clock, in milliseconds.
#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Clock {
    #[serde(default, deserialize_with = "lenient")]
    pub initial: Option<f64>,
    #[serde(default, deserialize_with = "lenient")]
    pub increment: Option<f64>,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
pub struct VariantKey {
    #[serde(default, deserialize_with = "lenient")]
    pub key: Option<String>,
}

/// A side of a `gameFull`: a Lichess account, or Stockfish at a level.
#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Player {
    #[serde(default, deserialize_with = "lenient")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub rating: Option<f64>,
    #[serde(default, deserialize_with = "lenient")]
    pub ai_level: Option<f64>,
}

impl Player {
    /// How the board names the player: the account, Stockfish's level, or "Opponent".
    pub fn shown_name(&self) -> String {
        match (&self.name, self.ai_level) {
            (Some(name), _) => name.clone(),
            (None, Some(level)) if level != 0.0 && !level.is_nan() => {
                format!("Stockfish level {}", js::number_to_string(level))
            }
            _ => "Opponent".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_odd_field_reads_as_absent_and_keeps_the_rest() {
        let event = ServerEvent::read(&json!({
            "type": "gameState", "moves": "e2e4", "wtime": "soon", "btime": 1000, "wdraw": 1,
        }));
        let ServerEvent::GameState(state) = event else {
            panic!("a game state: {event:?}");
        };
        assert_eq!(state.moves.as_deref(), Some("e2e4"));
        assert_eq!((state.wtime, state.btime), (None, Some(1000.)));
        assert!(!state.draw(true));
    }

    #[test]
    fn an_unknown_or_missing_type_is_other() {
        assert_eq!(
            ServerEvent::read(&json!({ "type": "challenge" })),
            ServerEvent::Other
        );
        assert_eq!(
            ServerEvent::read(&json!({ "moves": "e2e4" })),
            ServerEvent::Other
        );
        assert_eq!(ServerEvent::read(&json!("gameState")), ServerEvent::Other);
    }

    #[test]
    fn a_game_start_says_whether_the_board_can_play_it() {
        let start = |game| match ServerEvent::read(&json!({ "type": "gameStart", "game": game })) {
            ServerEvent::GameStart { game } => game.unwrap_or_default(),
            other => panic!("a game start: {other:?}"),
        };
        assert!(start(json!({ "gameId": "a" })).board_compatible());
        assert!(!start(json!({ "compat": { "board": false } })).board_compatible());
        assert!(start(json!({ "speed": "correspondence" })).correspondence());
    }

    #[test]
    fn a_player_is_named_by_account_or_stockfish_level() {
        let player = |value| serde_json::from_value::<Player>(value).unwrap();
        assert_eq!(
            player(json!({ "name": "Bob", "aiLevel": 3 })).shown_name(),
            "Bob"
        );
        assert_eq!(
            player(json!({ "aiLevel": 3 })).shown_name(),
            "Stockfish level 3"
        );
        assert_eq!(player(json!({ "aiLevel": 0 })).shown_name(), "Opponent");
        assert_eq!(player(json!({})).shown_name(), "Opponent");
    }
}
