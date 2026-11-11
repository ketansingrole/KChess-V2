//! The reads of an online game: server events, connection reports, and the game-state
//! transitions they share with `finish` and `reset_game`.

use serde_json::{Value, json};

use super::model::{ChatEntry, ClockConfig, Effect, FinishedGame, Run, Setup, turn_of};
use super::server::{
    ChatLine, ConnectionReport, GameFull, GameRef, GameState, OpponentGone, Player, ServerEvent,
};
use crate::js;
use crate::misc::constants::NotificationKind;
use crate::variant::Variant;

/// The days per move Lichess allows.
const CORRESPONDENCE: [f64; 7] = [1.0, 2.0, 3.0, 5.0, 7.0, 10.0, 14.0];
/// The chat a game keeps: the latest lines.
const CHAT_KEPT: usize = 300;
/// The connection phases that mean the game is not reachable right now.
const UNREACHABLE: [&str; 4] = ["checking", "reconnecting", "disconnected", "auth-required"];

/// The moves of a game state, split on whitespace.
fn moves_of(text: Option<&str>) -> Vec<String> {
    js::trim(text.unwrap_or_default())
        .split(js::is_space)
        .filter(|m| !m.is_empty())
        .map(str::to_owned)
        .collect()
}

impl Run<'_> {
    /// One server event. Events for other games, and types that carry no
    /// game state, are ignored.
    pub fn read_event(&mut self, event: &ServerEvent) {
        let default = GameRef::default();
        match event {
            ServerEvent::GameStart { game } => self.game_start(game.as_ref().unwrap_or(&default)),
            ServerEvent::GameFinish { game } => self.game_finish(game.as_ref().unwrap_or(&default)),
            ServerEvent::OpponentGone(gone) => self.opponent_gone(gone),
            ServerEvent::ChatLine(line) => self.chat_line(line),
            ServerEvent::GameFull(full) => self.game_full(full),
            ServerEvent::GameState(state) => self.game_state(state),
            ServerEvent::Other => {}
        }
    }

    /// A connection report, which the epoch makes authoritative. The report is kept as it came.
    pub fn read_connection(&mut self, connection: &Value) {
        let report = ConnectionReport::read(connection);
        let session = report.session.unwrap_or(f64::NAN);
        if session < self.m.state_epoch {
            return;
        }
        if session != self.m.state_epoch {
            self.m.state_epoch = session;
            self.effect(Effect::ConnectionChanged);
        }
        self.m.online_connection = Some(connection.clone());
        if let Some(account) = report.account.filter(|account| !account.is_empty()) {
            self.m.online_account = account;
        }
        if let Some(id) = report.game_id.filter(|id| !id.is_empty()) {
            self.m.online_id = id;
        }
        let lane = report.lane.as_deref();
        if lane == Some("events") && !self.m.online_id.is_empty() {
            return;
        }
        if lane == Some("game") && report.phase.as_deref() == Some("idle") {
            self.m.online_phase = "idle".into();
            self.m.online_id.clear();
            self.m.online_status = "No game in progress.".into();
            self.clock_pause();
        }
        if report
            .phase
            .as_deref()
            .is_some_and(|phase| UNREACHABLE.contains(&phase))
        {
            self.m.online_phase = "disconnected".into();
            self.m.online_status = report.message.unwrap_or_else(|| {
                "Connection interrupted. Reconnect to recover your game.".into()
            });
            self.clock_pause();
        }
    }

    /// The game is unreachable while Lichess is asked about it.
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

    /// Forget the board, keeping the game's identity.
    pub fn reset_game(&mut self) {
        self.effect(Effect::ResetView);
        let m = &mut self.m;
        m.online_moves = Vec::new();
        m.online_result = String::new();
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
        m.online_rating = None;
    }

    /// The game is over. A rated or casual finish is remembered for a rematch, and a
    /// game that was being played announces its result.
    pub fn finish(&mut self, winner: Option<&str>, reason: &str, notify: bool) {
        let previous = self.m.online_phase.clone();
        self.m.online_phase = "finished".into();
        self.m.online_status = outcome_line(winner, reason, &self.m.online_color);
        self.m.online_result = match winner {
            Some("white") => "1-0",
            Some("black") => "0-1",
            // An aborted game has no result; any other undecided finish is a draw.
            _ if matches!(reason, "aborted" | "noStart") => "*",
            _ => "1/2-1/2",
        }
        .into();
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
                    .map(|c| js::max(1.0, js::round(c.initial / 60.0))),
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
            self.notify(NotificationKind::GameEvents, outcome, body);
        }
    }

    fn game_start(&mut self, game: &GameRef) {
        let game_id = game.game_id.as_deref();
        if game.correspondence() && game_id != Some(self.m.online_id.as_str()) {
            return;
        }
        let id = game_id.unwrap_or("").to_string();
        if self.m.shown_game != id {
            self.reset_game();
        }
        self.m.shown_game = id.clone();
        self.m.online_id = id;
        self.m.online_color = game.color.clone().unwrap_or_else(|| "white".into());
        let opponent = game.opponent.clone().unwrap_or_default();
        self.m.online_opponent = opponent.username.unwrap_or_else(|| "Opponent".into());
        self.m.online_game_rated = game.rated.unwrap_or(false);
        self.m.opponent_id = opponent.id.unwrap_or_default();
        self.m.online_phase = "playing".into();
        self.m.online_status = "Game in progress".into();
        if !game.correspondence() {
            let body = format!(
                "You are playing {} as {}.",
                self.m.online_opponent, self.m.online_color
            );
            self.notify(NotificationKind::GameEvents, "Game started", body);
        }
    }

    fn game_finish(&mut self, game: &GameRef) {
        if game.game_id.as_deref() != Some(self.m.online_id.as_str()) {
            return;
        }
        let reason = game
            .status
            .as_ref()
            .and_then(|status| status.name.clone())
            .unwrap_or_else(|| "finished".into());
        self.finish(game.winner.as_deref(), &reason, true);
    }

    /// Whether an event naming game `id` is about another game than this one.
    fn other_game(&self, id: Option<&str>) -> bool {
        id.is_some_and(|id| id != self.m.online_id)
    }

    fn opponent_gone(&mut self, event: &OpponentGone) {
        if self.other_game(event.id.as_deref()) {
            return;
        }
        if self.m.online_phase != "playing" {
            return;
        }
        let gone = event.gone.unwrap_or(false);
        self.m.opponent_gone = gone;
        let claim = event
            .claim_win_in_seconds
            .filter(|_| gone)
            .map(|seconds| self.ctx.now + seconds * 1000.0);
        self.m.claim_at = claim;
        self.m.online_status = if gone {
            "Opponent disconnected…".into()
        } else {
            "Game in progress".into()
        };
    }

    fn chat_line(&mut self, event: &ChatLine) {
        if self.other_game(event.id.as_deref()) {
            return;
        }
        if !self.ctx.chat_enabled {
            return;
        }
        let room = if event.room.as_deref() == Some("spectator") {
            "spectator"
        } else {
            "player"
        };
        self.m.chat.push(ChatEntry {
            user: event.username.clone(),
            text: event.text.clone(),
            room: room.into(),
        });
        let excess = self.m.chat.len().saturating_sub(CHAT_KEPT);
        self.m.chat.drain(..excess);
    }

    /// A `gameFull`: the game, then its state.
    fn game_full(&mut self, full: &GameFull) {
        if !self.m.online_id.is_empty() && self.other_game(full.id.as_deref()) {
            return;
        }
        let previous = self.m.online_moves.len();
        self.read_full(full);
        let state = full.state.clone().unwrap_or_default();
        self.read_state(&state, previous, true);
    }

    /// A `gameState`.
    fn game_state(&mut self, state: &GameState) {
        if !self.m.online_id.is_empty() && self.other_game(state.id.as_deref()) {
            return;
        }
        let previous = self.m.online_moves.len();
        self.read_state(state, previous, false);
    }

    /// The board, the clocks, the offers and the outcome of a game's state.
    fn read_state(&mut self, state: &GameState, previous: usize, full: bool) {
        self.m.online_moves = moves_of(state.moves.as_deref());
        if previous > 0 && self.m.online_moves.len() > previous {
            let replayed = self.m.replayed();
            let san = replayed.played.last().cloned();
            self.effect(Effect::Moved { san: san.clone() });
            // A move by the other colour than ours is the opponent's.
            if turn_of(&replayed.setup.fen) == self.m.online_color {
                let body = match &san {
                    Some(san) => format!("{san}. Your move."),
                    None => "Your move.".to_string(),
                };
                let title = format!("{} moved", self.m.online_opponent);
                self.notify(NotificationKind::OpponentMove, &title, body);
                // A new move answers any takeback request.
                self.m.takeback_offer = "none".into();
            }
        }
        let white = self.m.online_color == "white";
        let their_draw = state.draw(!white);
        let my_draw = state.draw(white);
        if their_draw && self.m.draw_offer != "theirs" && !full {
            let body = format!("{} offers a draw.", self.m.online_opponent);
            self.notify(NotificationKind::GameEvents, "Draw offered", body);
        }
        self.m.draw_offer = if their_draw {
            "theirs"
        } else if my_draw {
            "mine"
        } else {
            "none"
        }
        .into();
        self.m.takeback_offer = if state.takeback(!white) {
            "theirs"
        } else if state.takeback(white) {
            "mine"
        } else {
            "none"
        }
        .into();
        self.m.first_move_by = state.expiration.as_ref().map(|expiration| {
            let wait = expiration.millis_to_move.unwrap_or(f64::NAN)
                - expiration.idle_millis.unwrap_or(f64::NAN);
            // Math.max(0, wait): NaN stays NaN.
            let clamped = if wait.is_nan() || wait > 0.0 {
                wait
            } else {
                0.0
            };
            self.ctx.now + clamped
        });
        let running = state.running();
        let white_left = state.wtime.unwrap_or(0.0);
        let black_left = state.btime.unwrap_or(0.0);
        // Berserking halves the clock; the next state shows it.
        if !self.m.online_tournament.is_empty()
            && let Some(config) = self.m.online_clock_config.clone()
        {
            let half = config.initial * 500.0;
            let mine_left = if white { white_left } else { black_left };
            if self.m.own_moves() == 0 && mine_left <= half + 1000.0 && mine_left > 0.0 {
                self.m.berserked = true;
            }
        }
        // Lichess starts a clock only once both sides have moved; until then each side has a
        // first-move timer instead, and the clocks hold at their starting time.
        let ticking = (running && self.m.online_moves.len() >= 2).then(|| self.m.turn());
        let initial = self.m.online_initial;
        self.effect(Effect::ClockSet {
            white: white_left,
            black: black_left,
            ticking,
            initial_seconds: (initial != 0.0 && !initial.is_nan()).then_some(initial),
        });
        if running {
            self.m.online_phase = "playing".into();
            if !self.m.opponent_gone {
                self.m.online_status = "Game in progress".into();
            }
        } else if let Some(reason) = state.status.as_deref() {
            self.finish(state.winner.as_deref(), reason, true);
        }
    }

    /// The fields only a full game carries, and the board they set up.
    fn read_full(&mut self, event: &GameFull) {
        let id = event.id.clone().unwrap_or_default();
        if self.m.shown_game != id {
            self.reset_game();
            self.m.shown_game = id.clone();
            self.effect(Effect::LoadChat { id: id.clone() });
        }
        self.m.online_id = id;
        let clock = event.clock.clone().unwrap_or_default();
        self.m.online_clock_config = clock.initial.map(|initial| ClockConfig {
            initial: initial / 1000.0,
            increment: clock.increment.unwrap_or(0.0) / 1000.0,
        });
        self.m.online_initial = clock.initial.unwrap_or(0.0) / 1000.0;
        self.m.online_game_rated = event.rated.unwrap_or(false);
        self.m.online_speed = event.speed.clone().unwrap_or_default();
        self.m.online_days_per_turn = event.days_per_turn;
        self.m.online_tournament = event.tournament_id.clone().unwrap_or_default();
        let key = event
            .variant
            .as_ref()
            .and_then(|variant| variant.key.as_deref());
        let variant = Variant::from_lichess(key);
        self.m.online_unsupported = if variant.is_some() {
            String::new()
        } else {
            key.unwrap_or("unknown").to_string()
        };
        let variant = variant.unwrap_or_default();
        let fen = match event.initial_fen.as_deref() {
            None | Some("") | Some("startpos") => variant.default_fen(),
            Some(fen) => fen.to_string(),
        };
        self.m.online_setup = Setup { variant, fen };

        // Match by account; if neither side does, keep the colour `gameStart` announced.
        let ours = if self.m.online_account.is_empty() {
            self.ctx.active_account.clone()
        } else {
            self.m.online_account.clone()
        }
        .to_lowercase();
        let (white, black) = (
            event.white.clone().unwrap_or_default(),
            event.black.clone().unwrap_or_default(),
        );
        let is_ours = |player: &Player| {
            player
                .id
                .as_ref()
                .is_some_and(|id| id.to_lowercase() == ours)
        };
        if is_ours(&white) {
            self.m.online_color = "white".into();
        } else if is_ours(&black) {
            self.m.online_color = "black".into();
        }
        let (own, opponent) = if self.m.online_color == "white" {
            (white, black)
        } else {
            (black, white)
        };
        self.m.online_rating = own.rating;
        self.m.online_opponent = opponent.shown_name();
        self.m.online_opponent_rating = opponent.rating;
        self.m.opponent_id = opponent.id.unwrap_or_default();
    }
}

/// One line for a finished game: who won and how, from Lichess's status name (`mate`,
/// `resign`, `outoftime`, ...). The "how" is kept only when it fits the result, so a status that
/// contradicts the winner (or an unfamiliar one) still says who won and nothing false.
pub(crate) fn outcome_line(winner: Option<&str>, reason: &str, ours: &str) -> String {
    if matches!(reason, "aborted" | "noStart") {
        return "Game aborted".into();
    }
    let decided = winner.is_some();
    let how = match reason {
        "mate" if decided => "by checkmate",
        "resign" if decided => "by resignation",
        "timeout" if decided => "by abandonment",
        "cheat" if decided => "by cheat detection",
        "variantEnd" if decided => "by the variant's rules",
        "stalemate" if !decided => "by stalemate",
        "draw" if !decided => "by agreement",
        "outoftime" => "on time",
        _ => "",
    };
    let result = match winner {
        Some(side) if side == ours => "You won",
        Some(_) => "You lost",
        None if how.is_empty() => return "Game over".into(),
        None => "Draw",
    };
    if how.is_empty() {
        result.into()
    } else {
        format!("{result} {how}")
    }
}
