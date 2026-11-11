//! The state of one online game, what a transition reads from its host, and what it returns to
//! the driver: the effects the host must see in order, and any network call the driver must make.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::js;
use crate::records;
use crate::replay::replay_setup;
use crate::rules::INITIAL_FEN;
pub use crate::variant::Setup;

/// `{ initial, increment }` in seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClockConfig {
    pub initial: f64,
    pub increment: f64,
}

/// `FinishedGame`: the last game, kept for a rematch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishedGame {
    pub id: String,
    pub account: String,
    pub opponent: String,
    pub color: String,
    pub rated: bool,
    pub setup: Setup,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub increment: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<f64>,
}

/// The game's state, with the connection epoch and the request generation (a newer request
/// supersedes every earlier one). Absent options are `null` on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub online_phase: String,
    pub online_id: String,
    pub online_account: String,
    pub online_connection: Option<Value>,
    pub online_moves: Vec<String>,
    pub online_color: String,
    pub online_opponent: String,
    pub online_opponent_rating: Option<f64>,
    /// Our own rating in this game, as Lichess sent it with the players.
    #[serde(default)]
    pub online_rating: Option<f64>,
    pub online_status: String,
    /// The finished game's PGN result (`1-0`, `0-1`, `1/2-1/2`); empty while none is known.
    #[serde(default)]
    pub online_result: String,
    pub online_initial: f64,
    pub online_game_rated: bool,
    pub online_setup: Setup,
    pub online_unsupported: String,
    pub online_speed: String,
    pub online_clock_config: Option<ClockConfig>,
    pub online_days_per_turn: Option<f64>,
    pub online_tournament: String,
    pub opponent_id: String,
    pub opponent_gone: bool,
    pub claim_at: Option<f64>,
    pub draw_offer: String,
    pub takeback_offer: String,
    pub first_move_by: Option<f64>,
    pub berserked: bool,
    pub chat: Vec<Value>,
    pub last_game: Option<FinishedGame>,
    pub state_epoch: f64,
    pub shown_game: String,
    pub generation: f64,
}

/// What the host read at the moment of a transition.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ctx {
    pub now: f64,
    pub active_account: String,
    pub chat_enabled: bool,
}

/// One effect for the driver, in order; `warn` and `failed` take the cause the driver caught.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "effect", rename_all = "camelCase")]
pub enum Effect {
    Warn {
        message: &'static str,
    },
    Notify {
        kind: &'static str,
        title: String,
        body: String,
    },
    Moved {
        san: Option<String>,
    },
    ResetView,
    ConnectionChanged,
    MoveAcknowledged {
        ms: f64,
    },
    Failed,
    ClockPause,
    ClockSet {
        white: f64,
        black: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        ticking: Option<&'static str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        initial_seconds: Option<f64>,
    },
    LoadChat {
        id: String,
    },
}

/// A network call the driver makes with the core, then reports back with `settle`.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    /// A seek or challenge with these options.
    StartOnline(Value),
    CancelOnline,
    ResumeOnline,
    OpenGame {
        account: String,
        id: String,
    },
    PlayOnline {
        id: String,
        uci: String,
    },
    OnlineAction {
        id: String,
        action: String,
    },
    OnlineChat {
        id: String,
    },
}

/// The game a real-time seek left: a correspondence game being played, to go back to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Left {
    pub id: String,
    pub account: String,
}

/// What a call `begin` made needs to settle its reply. `generation` is the request's: a newer
/// request supersedes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Pending {
    Start {
        generation: f64,
        correspondence: bool,
        previous: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        left: Option<Left>,
    },
    Stop,
    Reconnect {
        generation: f64,
    },
    Open {
        generation: f64,
        account: String,
        id: String,
    },
    Move {
        generation: f64,
        id: String,
        epoch: f64,
        sent: f64,
    },
    Action {
        generation: f64,
        id: String,
        epoch: f64,
        action: String,
    },
    Chat {
        id: String,
    },
}

/// A network-backed operation the host asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// A seek or challenge with these options.
    Start(Value),
    Stop,
    Reconnect,
    Open {
        account: String,
        id: String,
    },
    Move(String),
    Action(String),
    Chat(String),
}

/// The core's reply to a call: its answer, or a failure whose cause the driver keeps.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Ok(Value),
    Failed,
}

/// What a transition returns to its driver; the state changed in place.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub effects: Vec<Effect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ret: Option<bool>,
    #[serde(skip)]
    pub call: Option<Call>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<Pending>,
    /// The call's failure is the operation's: the host reports its cause.
    pub rethrow: bool,
}

/// The board's view of the state.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    /// The live position.
    pub position: Setup,
    /// The SAN of each move played.
    pub history: Vec<String>,
    /// Moves this side has made.
    pub own_moves: usize,
    pub correspondence: bool,
    /// The challenge that starts a rematch of the last game.
    pub rematch: Option<Value>,
}

/// A replay of the moves: the position they reach and the SAN of each move played.
pub struct Replayed {
    pub setup: Setup,
    pub played: Vec<String>,
}

/// One transition: the state it changes, its effects so far, and what the host read.
pub struct Run<'a> {
    pub m: &'a mut Model,
    pub fx: Vec<Effect>,
    pub ctx: &'a Ctx,
}

impl<'a> Run<'a> {
    pub fn new(m: &'a mut Model, ctx: &'a Ctx) -> Run<'a> {
        Run {
            m,
            fx: Vec::new(),
            ctx,
        }
    }

    pub fn effect(&mut self, effect: Effect) {
        self.fx.push(effect);
    }

    pub fn warn(&mut self, message: &'static str) {
        self.fx.push(Effect::Warn { message });
    }

    pub fn notify(&mut self, kind: &'static str, title: &str, body: String) {
        self.fx.push(Effect::Notify {
            kind,
            title: title.to_string(),
            body,
        });
    }

    pub fn clock_pause(&mut self) {
        self.fx.push(Effect::ClockPause);
    }

    /// `RequestScope.next()`: a new request supersedes every earlier one.
    pub fn next_generation(&mut self) -> f64 {
        self.m.generation += 1.0;
        self.m.generation
    }

    /// The effects so far, and nothing more.
    pub fn done(self) -> Outcome {
        Outcome {
            effects: self.fx,
            ..Outcome::default()
        }
    }

    /// The operation's answer.
    pub fn answer(self, ret: bool) -> Outcome {
        Outcome {
            ret: Some(ret),
            ..self.done()
        }
    }

    /// The call to make, and what settles its reply.
    pub fn call(self, call: Call, pending: Pending) -> Outcome {
        Outcome {
            call: Some(call),
            pending: Some(pending),
            ..self.done()
        }
    }

    /// The call failed, and the host reports why.
    pub fn rethrow(self) -> Outcome {
        Outcome {
            rethrow: true,
            ..self.done()
        }
    }
}

impl Model {
    /// `setupPositionAfter` and `setupSanHistory`: the position the moves reach and the SAN of each
    /// move played. An illegal setup falls back to the standard start with no history.
    pub fn replayed(&self) -> Replayed {
        let setup = &self.online_setup;
        match replay_setup(setup.variant, &setup.fen, &self.online_moves) {
            None => Replayed {
                setup: Setup::standard(),
                played: Vec::new(),
            },
            Some(replay) => {
                // Without a move the position is the setup as given, not its normalized FEN.
                let fen = if replay.played.is_empty() {
                    setup.fen.clone()
                } else {
                    replay.fen
                };
                Replayed {
                    setup: Setup {
                        variant: setup.variant,
                        fen,
                    },
                    played: replay.played.into_iter().map(|m| m.san).collect(),
                }
            }
        }
    }

    /// Whose turn the position reached by the moves is.
    pub fn turn(&self) -> &'static str {
        // `replayed()` without its SAN and FEN: only the side to move is wanted.
        let setup = &self.online_setup;
        match crate::replay::replay_setup_turn(setup.variant, &setup.fen, &self.online_moves) {
            None => turn_of(&Setup::standard().fen),
            // Without a move the position is the setup as given.
            Some((0, _)) => turn_of(&setup.fen),
            Some((_, shakmaty::Color::Black)) => "black",
            Some((_, shakmaty::Color::White)) => "white",
        }
    }

    /// Moves this side has made so far.
    pub fn own_moves(&self) -> usize {
        let white_first = self.online_setup.fen.split(' ').nth(1) != Some("b");
        let mine = usize::from(self.online_color != "white");
        self.online_moves
            .iter()
            .enumerate()
            .filter(|(i, _)| (if white_first { i % 2 } else { (i + 1) % 2 }) == mine)
            .count()
    }

    /// A correspondence game: days per move, or a speed Lichess names so.
    pub fn correspondence(&self) -> bool {
        self.online_speed == "correspondence" || self.online_days_per_turn.is_some()
    }

    /// The challenge that starts a rematch of the last game, colours swapped.
    pub fn rematch(&self) -> Option<Value> {
        let game = self.last_game.as_ref()?;
        let mut out = Map::new();
        out.insert("target".into(), game.opponent.clone().into());
        out.insert("account".into(), game.account.clone().into());
        let color = if game.color == "white" {
            "black"
        } else {
            "white"
        };
        out.insert("color".into(), color.into());
        out.insert("rated".into(), game.rated.into());
        if let Some(minutes) = game.minutes {
            out.insert("minutes".into(), minutes.into());
        }
        if let Some(increment) = game.increment {
            out.insert("increment".into(), increment.into());
        }
        if let Some(days) = game.days {
            out.insert("days".into(), days.into());
        }
        if !game.setup.variant.is_standard() {
            out.insert("variant".into(), game.setup.variant.key().into());
        } else if game.setup.fen != INITIAL_FEN {
            out.insert("fen".into(), game.setup.fen.clone().into());
        }
        Some(Value::Object(out))
    }

    /// A board with no game (`onlineGameState()`).
    pub fn idle() -> Model {
        Model {
            online_phase: "idle".into(),
            online_id: String::new(),
            online_account: String::new(),
            online_connection: None,
            online_moves: Vec::new(),
            online_color: "white".into(),
            online_opponent: "Opponent".into(),
            online_opponent_rating: None,
            online_rating: None,
            online_status: String::new(),
            online_result: String::new(),
            online_initial: 0.0,
            online_game_rated: false,
            online_setup: Setup::standard(),
            online_unsupported: String::new(),
            online_speed: String::new(),
            online_clock_config: None,
            online_days_per_turn: None,
            online_tournament: String::new(),
            opponent_id: String::new(),
            opponent_gone: false,
            claim_at: None,
            draw_offer: "none".into(),
            takeback_offer: "none".into(),
            first_move_by: None,
            berserked: false,
            chat: Vec::new(),
            last_game: None,
            state_epoch: -1.0,
            shown_game: String::new(),
            generation: 0.0,
        }
    }
}

/// The side to move in a FEN: `black` for `b`, otherwise `white`.
pub fn turn_of(fen: &str) -> &'static str {
    if fen.split(' ').nth(1) == Some("b") {
        "black"
    } else {
        "white"
    }
}

/// `obj?.key`: the entry when there is an object holding it.
pub fn at<'a>(v: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    v.and_then(|v| v.get(key))
}

/// `x ?? y`: the value unless it is absent or null.
pub fn nullish(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| !v.is_null())
}

/// `String(x ?? fallback)`: any value as text, `fallback` when absent.
pub fn string_or(v: Option<&Value>, fallback: &str) -> String {
    match nullish(v) {
        Some(v) => js::to_string(Some(v)),
        None => fallback.to_string(),
    }
}

/// `Number(x ?? 0)`.
pub fn number_or_zero(v: Option<&Value>) -> f64 {
    match nullish(v) {
        Some(v) => js::to_number(Some(v)),
        None => 0.0,
    }
}

/// `isGameInProgress(status ?? null)`: the records rule, reused.
pub fn is_running(status: Option<&Value>) -> bool {
    records::games::is_game_in_progress(nullish(status))
}
