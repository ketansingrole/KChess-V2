//! The state of one online game as `OnlineGame` holds it (`core/src/domain/onlineGame.ts`), what a
//! transition reads from its host, and what it returns to the driver: the next state, the effects
//! the host must see in order, and any network call the driver must make.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::js;
use crate::records;
use crate::replay::replay_setup;
use crate::rules::{default_position, lichess_variant, make_fen};

/// The standard start (`STANDARD_SETUP` in `variant.ts`).
pub const INITIAL_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// The variants a Lichess key may name (`VARIANTS` in `variant.ts`).
const VARIANTS: [&str; 8] = [
    "standard",
    "chess960",
    "kingOfTheHill",
    "threeCheck",
    "antichess",
    "atomic",
    "horde",
    "racingKings",
];

/// `GameSetup`: the rules and the position before the first move.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    pub variant: String,
    pub fen: String,
}

impl Setup {
    pub fn standard() -> Setup {
        Setup {
            variant: "standard".into(),
            fen: INITIAL_FEN.into(),
        }
    }
}

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

/// `onlineGameState()` plus the two private fields the class kept: the connection epoch and the
/// request generation (`RequestScope`'s counter). Options are `undefined` in the TypeScript and
/// `null` on the wire.
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
    pub online_status: String,
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

/// What the class read from its host at the moment of a transition.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ctx {
    pub now: f64,
    pub active_account: String,
    pub chat_enabled: bool,
}

/// One effect for the driver, in order. Host callbacks are optional on the host, so the driver
/// calls them with `?.`; `warn` and `failed` take the cause the driver caught.
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

/// A network call the driver makes with the host's API, then reports back with `settle`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Call {
    pub method: &'static str,
    pub args: Vec<Value>,
}

impl Call {
    pub fn new(method: &'static str, args: Vec<Value>) -> Call {
        Call { method, args }
    }
}

/// What a transition returns to its driver.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub state: Model,
    pub effects: Vec<Effect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ret: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call: Option<Call>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<Value>,
    pub rethrow: bool,
}

/// A replay of the moves: the position they reach and the SAN of each move played.
pub struct Replayed {
    pub setup: Setup,
    pub played: Vec<String>,
}

/// One transition's working copy of the state and its effects.
pub struct Run {
    pub m: Model,
    pub fx: Vec<Effect>,
    pub ctx: Ctx,
}

impl Run {
    pub fn new(m: Model, ctx: Ctx) -> Run {
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

    pub fn done(
        self,
        ret: Option<Value>,
        call: Option<Call>,
        pending: Option<Value>,
        rethrow: bool,
    ) -> Result<Value, String> {
        serde_json::to_value(Transition {
            state: self.m,
            effects: self.fx,
            ret,
            call,
            pending,
            rethrow,
        })
        .map_err(|e| e.to_string())
    }
}

impl Model {
    /// `setupPositionAfter` and `setupSanHistory`: the position the moves reach and the SAN of each
    /// move played. An illegal setup falls back to the standard start with no history.
    pub fn replayed(&self) -> Replayed {
        let setup = &self.online_setup;
        match replay_setup(&setup.variant, &setup.fen, &self.online_moves) {
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
                        variant: setup.variant.clone(),
                        fen,
                    },
                    played: replay.played.into_iter().map(|m| m.san).collect(),
                }
            }
        }
    }

    /// Whose turn the position reached by the moves is.
    pub fn turn(&self) -> &'static str {
        turn_of(&self.replayed().setup.fen)
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
        if game.setup.variant != "standard" {
            out.insert("variant".into(), game.setup.variant.clone().into());
        } else if game.setup.fen != INITIAL_FEN {
            out.insert("fen".into(), game.setup.fen.clone().into());
        }
        Some(Value::Object(out))
    }

    #[cfg(test)]
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
            online_status: String::new(),
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

/// `variantFromLichess`: the playable variant for a Lichess key; `fromPosition` is standard rules.
pub fn variant_from_lichess(key: Option<&str>) -> Option<&'static str> {
    match key {
        None | Some("") | Some("fromPosition") => Some("standard"),
        Some(key) => VARIANTS.iter().copied().find(|variant| *variant == key),
    }
}

/// `defaultFen(variant)`: the variant's usual start (Chess960's standard start is the usual one).
pub fn default_fen(variant: &str) -> String {
    match lichess_variant(variant) {
        Some(rules) => make_fen(&default_position(rules)),
        None => INITIAL_FEN.into(),
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

/// A value that is a string, as the TypeScript types say.
pub fn text_of(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

/// `String(x ?? fallback)` for a string the TypeScript keeps as text.
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

/// `!!x`
pub fn truthy(v: Option<&Value>) -> bool {
    !js::falsy(v)
}

/// `isGameInProgress(status ?? null)`: the records rule, reused.
pub fn is_running(status: Option<&Value>) -> bool {
    let status = nullish(status).cloned().unwrap_or(Value::Null);
    matches!(
        records::call("isGameInProgress", &[status]),
        Some(Ok(Value::Bool(true)))
    )
}

/// JavaScript's `Math.round` (halves round up).
pub fn js_round(x: f64) -> f64 {
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// JavaScript's `Math.max` for two numbers, where NaN wins.
pub fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if b > a {
        b
    } else {
        a
    }
}
