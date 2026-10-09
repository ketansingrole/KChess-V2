//! The reads of `OnlineGame`: server events (`readOnlineEvent`), connection reports
//! (`readOnlineState`, `checkingOnline`), and the game-state transitions they share with `finish`
//! and `resetGame`.

use serde_json::{Value, json};

use super::model::{
    ClockConfig, Effect, FinishedGame, Run, Setup, at, default_fen, is_running, js_max, js_round,
    nullish, number_or_zero, string_or, text_of, truthy, turn_of, variant_from_lichess,
};
use crate::js;

/// `CORRESPONDENCE`: the days per move Lichess allows.
const CORRESPONDENCE: [f64; 7] = [1.0, 2.0, 3.0, 5.0, 7.0, 10.0, 14.0];
/// The chat a game keeps: the latest lines.
const CHAT_KEPT: usize = 300;
/// The connection phases that mean the game is not reachable right now.
const UNREACHABLE: [&str; 4] = ["checking", "reconnecting", "disconnected", "auth-required"];

/// The moves of a game state, split on whitespace.
fn moves_of(v: Option<&Value>) -> Vec<String> {
    let text = string_or(v, "");
    js::trim(&text)
        .split(js::is_space)
        .filter(|m| !m.is_empty())
        .map(str::to_owned)
        .collect()
}

impl Run {
    /// `readOnlineEvent`: one server event. Events for other games, and types that carry no
    /// game state, are ignored.
    pub fn read_event(&mut self, event: &Value) {
        match text_of(event.get("type")) {
            Some("gameStart") => self.game_start(event),
            Some("gameFinish") => self.game_finish(event),
            Some("opponentGone") => self.opponent_gone(event),
            Some("chatLine") => self.chat_line(event),
            Some(kind @ ("gameFull" | "gameState")) => self.game_state(event, kind == "gameFull"),
            _ => {}
        }
    }

    /// `readOnlineState`: a connection report, which the epoch makes authoritative.
    pub fn read_connection(&mut self, connection: &Value) {
        let session = js::to_number(connection.get("session"));
        if session < self.m.state_epoch {
            return;
        }
        if session != self.m.state_epoch {
            self.m.state_epoch = session;
            self.effect(Effect::ConnectionChanged);
        }
        self.m.online_connection = Some(connection.clone());
        if truthy(connection.get("account")) {
            self.m.online_account = js::to_string(connection.get("account"));
        }
        if truthy(connection.get("gameId")) {
            self.m.online_id = js::to_string(connection.get("gameId"));
        }
        let lane = text_of(connection.get("lane"));
        if lane == Some("events") && !self.m.online_id.is_empty() {
            return;
        }
        if lane == Some("game") && text_of(connection.get("phase")) == Some("idle") {
            self.m.online_phase = "idle".into();
            self.m.online_id.clear();
            self.m.online_status = "No game in progress.".into();
            self.clock_pause();
        }
        let phase = text_of(connection.get("phase"));
        if phase.is_some_and(|phase| UNREACHABLE.contains(&phase)) {
            self.m.online_phase = "disconnected".into();
            self.m.online_status = string_or(
                connection.get("message"),
                "Connection interrupted. Reconnect to recover your game.",
            );
            self.clock_pause();
        }
    }

    /// `checkingOnline`: the game is unreachable while Lichess is asked about it.
    pub fn checking(&mut self) {
        self.m.online_phase = "disconnected".into();
        self.m.online_status = "Checking Lichess for an ongoing game…".into();
        self.m.online_connection = Some(json!({
            "session": self.m.state_epoch,
            "account": self.m.online_account,
            "gameId": self.m.online_id,
            "lane": "game",
            "phase": "checking",
            "message": self.m.online_status,
        }));
        self.clock_pause();
    }

    /// `resetGame`: forget the board, keeping the game's identity.
    pub fn reset_game(&mut self) {
        self.effect(Effect::ResetView);
        let m = &mut self.m;
        m.online_moves = Vec::new();
        m.opponent_gone = false;
        m.claim_at = None;
        m.draw_offer = "none".into();
        m.takeback_offer = "none".into();
        m.first_move_by = None;
        m.berserked = false;
        m.chat = Vec::new();
        m.online_initial = 0.0;
        m.online_setup = Setup::standard();
        m.online_unsupported = String::new();
        m.online_speed = String::new();
        m.online_clock_config = None;
        m.online_days_per_turn = None;
        m.online_tournament = String::new();
        m.online_opponent_rating = None;
    }

    /// `finish`: the game is over. A rated or casual finish is remembered for a rematch, and a
    /// game that was being played announces its result.
    pub fn finish(&mut self, winner: Option<&str>, reason: &str, notify: bool) {
        let previous = self.m.online_phase.clone();
        self.m.online_phase = "finished".into();
        self.m.online_status = format!("Game ended: {reason}");
        self.m.draw_offer = "none".into();
        self.m.takeback_offer = "none".into();
        self.m.claim_at = None;
        self.m.first_move_by = None;
        self.clock_pause();
        if !self.m.online_id.is_empty() && !self.m.opponent_id.is_empty() {
            let account = if self.m.online_account.is_empty() {
                self.ctx.active_account.clone()
            } else {
                self.m.online_account.clone()
            };
            let config = self.m.online_clock_config.clone();
            let days = self.m.online_days_per_turn.unwrap_or(0.0);
            self.m.last_game = Some(FinishedGame {
                id: self.m.online_id.clone(),
                account,
                opponent: self.m.opponent_id.clone(),
                color: self.m.online_color.clone(),
                rated: self.m.online_game_rated,
                setup: self.m.online_setup.clone(),
                minutes: config
                    .as_ref()
                    .map(|c| js_max(1.0, js_round(c.initial / 60.0))),
                increment: config.as_ref().map(|c| c.increment),
                days: self
                    .m
                    .online_days_per_turn
                    .filter(|_| CORRESPONDENCE.contains(&days)),
            });
        }
        if notify && previous == "playing" {
            let outcome = if reason == "aborted" {
                "Game aborted"
            } else if winner.is_none() {
                "Draw"
            } else if winner == Some(self.m.online_color.as_str()) {
                "You won"
            } else {
                "You lost"
            };
            let body = format!("Against {} · {reason}.", self.m.online_opponent);
            self.notify("gameEvents", outcome, body);
        }
    }

    fn game_start(&mut self, event: &Value) {
        let game = event.get("game");
        let game_id = text_of(at(game, "gameId"));
        if text_of(at(game, "speed")) == Some("correspondence")
            && game_id != Some(self.m.online_id.as_str())
        {
            return;
        }
        let id = game_id.unwrap_or("").to_string();
        if self.m.shown_game != id {
            self.reset_game();
        }
        self.m.shown_game = id.clone();
        self.m.online_id = id;
        self.m.online_color = string_or(at(game, "color"), "white");
        let opponent = nullish(at(game, "opponent"));
        self.m.online_opponent = string_or(at(opponent, "username"), "Opponent");
        self.m.online_game_rated = at(game, "rated").and_then(Value::as_bool).unwrap_or(false);
        self.m.opponent_id = string_or(at(opponent, "id"), "");
        self.m.online_phase = "playing".into();
        self.m.online_status = "Game in progress".into();
        if text_of(at(game, "speed")) != Some("correspondence") {
            let body = format!(
                "You are playing {} as {}.",
                self.m.online_opponent, self.m.online_color
            );
            self.notify("gameEvents", "Game started", body);
        }
    }

    fn game_finish(&mut self, event: &Value) {
        let game = event.get("game");
        if text_of(at(game, "gameId")) != Some(self.m.online_id.as_str()) {
            return;
        }
        let winner = text_of(at(game, "winner"));
        let reason = string_or(at(at(game, "status"), "name"), "finished");
        self.finish(winner, &reason, true);
    }

    fn opponent_gone(&mut self, event: &Value) {
        if event.get("id").is_some() && text_of(event.get("id")) != Some(self.m.online_id.as_str())
        {
            return;
        }
        if self.m.online_phase != "playing" {
            return;
        }
        let gone = truthy(event.get("gone"));
        self.m.opponent_gone = gone;
        let claim = (gone && event.get("claimWinInSeconds").is_some())
            .then(|| self.ctx.now + js::to_number(event.get("claimWinInSeconds")) * 1000.0);
        self.m.claim_at = claim;
        self.m.online_status = if gone {
            "Opponent disconnected…".into()
        } else {
            "Game in progress".into()
        };
    }

    fn chat_line(&mut self, event: &Value) {
        if event.get("id").is_some() && text_of(event.get("id")) != Some(self.m.online_id.as_str())
        {
            return;
        }
        if !self.ctx.chat_enabled {
            return;
        }
        let room = if text_of(event.get("room")) == Some("spectator") {
            "spectator"
        } else {
            "player"
        };
        self.m.chat.push(json!({
            "user": event.get("username").cloned().unwrap_or(Value::Null),
            "text": event.get("text").cloned().unwrap_or(Value::Null),
            "room": room,
        }));
        let excess = self.m.chat.len().saturating_sub(CHAT_KEPT);
        self.m.chat.drain(..excess);
    }

    /// A `gameFull` or `gameState` event: the board, the clocks, the offers and the outcome.
    fn game_state(&mut self, event: &Value, full: bool) {
        if event.get("id").is_some()
            && !self.m.online_id.is_empty()
            && text_of(event.get("id")) != Some(self.m.online_id.as_str())
        {
            return;
        }
        let state = if full {
            event.get("state")
        } else {
            Some(event)
        };
        let previous = self.m.online_moves.len();
        if full {
            self.read_full(event);
        }
        self.m.online_moves = moves_of(at(state, "moves"));
        if previous > 0 && self.m.online_moves.len() > previous {
            let replayed = self.m.replayed();
            let san = replayed.played.last().cloned();
            self.effect(Effect::Moved { san: san.clone() });
            // A move by the other colour than ours is the opponent's.
            if turn_of(&replayed.setup.fen) == self.m.online_color {
                let body = format!("{}. Your move.", san.as_deref().unwrap_or("undefined"));
                let title = format!("{} moved", self.m.online_opponent);
                self.notify("opponentMove", &title, body);
                // A new move answers any takeback request.
                self.m.takeback_offer = "none".into();
            }
        }
        let (theirs, mine) = if self.m.online_color == "white" {
            ("b", "w")
        } else {
            ("w", "b")
        };
        let flag = |key: String| truthy(at(state, &key));
        let their_draw = flag(format!("{theirs}draw"));
        let my_draw = flag(format!("{mine}draw"));
        if their_draw && self.m.draw_offer != "theirs" && !full {
            let body = format!("{} offers a draw.", self.m.online_opponent);
            self.notify("gameEvents", "Draw offered", body);
        }
        self.m.draw_offer = if their_draw {
            "theirs"
        } else if my_draw {
            "mine"
        } else {
            "none"
        }
        .into();
        self.m.takeback_offer = if flag(format!("{theirs}takeback")) {
            "theirs"
        } else if flag(format!("{mine}takeback")) {
            "mine"
        } else {
            "none"
        }
        .into();
        let expiration = at(state, "expiration");
        self.m.first_move_by = truthy(expiration).then(|| {
            let wait = js::to_number(at(expiration, "millisToMove"))
                - js::to_number(at(expiration, "idleMillis"));
            // Math.max(0, wait): NaN stays NaN.
            let clamped = if wait.is_nan() || wait > 0.0 {
                wait
            } else {
                0.0
            };
            self.ctx.now + clamped
        });
        let running = is_running(at(state, "status"));
        let white = number_or_zero(at(state, "wtime"));
        let black = number_or_zero(at(state, "btime"));
        // Berserking halves the clock; the next state shows it.
        if !self.m.online_tournament.is_empty()
            && let Some(config) = self.m.online_clock_config.clone()
        {
            let half = config.initial * 500.0;
            let mine_left = if self.m.online_color == "white" {
                white
            } else {
                black
            };
            if self.m.own_moves() == 0 && mine_left <= half + 1000.0 && mine_left > 0.0 {
                self.m.berserked = true;
            }
        }
        let ticking = running.then(|| self.m.turn());
        let initial = self.m.online_initial;
        self.effect(Effect::ClockSet {
            white,
            black,
            ticking,
            initial_seconds: (initial != 0.0 && !initial.is_nan()).then_some(initial),
        });
        if running {
            self.m.online_phase = "playing".into();
            if !self.m.opponent_gone {
                self.m.online_status = "Game in progress".into();
            }
        } else {
            let winner = text_of(at(state, "winner"));
            let reason = js::to_string(at(state, "status"));
            self.finish(winner, &reason, true);
        }
    }

    /// The fields only a full game carries, and the board they set up.
    fn read_full(&mut self, event: &Value) {
        let id = text_of(event.get("id")).unwrap_or("").to_string();
        if self.m.shown_game != id {
            self.reset_game();
            self.m.shown_game = id.clone();
            self.effect(Effect::LoadChat { id: id.clone() });
        }
        self.m.online_id = id;
        let clock = nullish(event.get("clock"));
        self.m.online_clock_config = match clock {
            Some(clock) if clock.get("initial").is_some() => Some(ClockConfig {
                initial: js::to_number(clock.get("initial")) / 1000.0,
                increment: number_or_zero(clock.get("increment")) / 1000.0,
            }),
            _ => None,
        };
        self.m.online_initial = number_or_zero(at(clock, "initial")) / 1000.0;
        self.m.online_game_rated = event.get("rated").and_then(Value::as_bool).unwrap_or(false);
        self.m.online_speed = string_or(event.get("speed"), "");
        self.m.online_days_per_turn =
            nullish(event.get("daysPerTurn")).map(|v| js::to_number(Some(v)));
        self.m.online_tournament = string_or(event.get("tournamentId"), "");
        let key = text_of(at(event.get("variant"), "key"));
        let variant = variant_from_lichess(key);
        self.m.online_unsupported = if variant.is_some() {
            String::new()
        } else {
            key.unwrap_or("unknown").to_string()
        };
        let variant = variant.unwrap_or("standard");
        let fen = match text_of(event.get("initialFen")) {
            None | Some("") | Some("startpos") => default_fen(variant),
            Some(fen) => fen.to_string(),
        };
        self.m.online_setup = Setup {
            variant: variant.to_string(),
            fen,
        };
        // Match by account; if neither side does, keep the colour `gameStart` announced.
        let ours = if self.m.online_account.is_empty() {
            self.ctx.active_account.clone()
        } else {
            self.m.online_account.clone()
        }
        .to_lowercase();
        let is_ours = |player: Option<&Value>| {
            text_of(at(player, "id")).is_some_and(|id| id.to_lowercase() == ours)
        };
        if is_ours(event.get("white")) {
            self.m.online_color = "white".into();
        } else if is_ours(event.get("black")) {
            self.m.online_color = "black".into();
        }
        let opponent = if self.m.online_color == "white" {
            event.get("black")
        } else {
            event.get("white")
        };
        self.m.online_opponent = match nullish(at(opponent, "name")) {
            Some(name) => js::to_string(Some(name)),
            None => {
                let ai = at(opponent, "aiLevel");
                if truthy(ai) {
                    format!("Stockfish level {}", js::to_string(ai))
                } else {
                    "Opponent".into()
                }
            }
        };
        self.m.online_opponent_rating = at(opponent, "rating").and_then(Value::as_f64);
        self.m.opponent_id = string_or(at(opponent, "id"), "");
    }
}
