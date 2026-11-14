//! The reads of an online game: server events, connection reports, and the game-state
//! transitions they share with `finish` and `reset_game`.

use serde_json::Number;

use super::connection::{Lane, OnlineConnection, OnlinePhase};
use super::model::{
    ChatEntry, ChatRoom, ClockConfig, Effect, FinishedGame, GamePhase, Offer, Run, Setup, turn_of,
};
use super::server::{ChatLine, GameFull, GameRef, GameState, OpponentGone, Player, ServerEvent};
use crate::color::{self, Color};
use crate::js;
use crate::misc::constants::NotificationKind;
use crate::variant::Variant;

/// The days per move Lichess allows.
const CORRESPONDENCE: [f64; 7] = [1.0, 2.0, 3.0, 5.0, 7.0, 10.0, 14.0];
/// The chat a game keeps: the latest lines.
const CHAT_KEPT: usize = 300;

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
    pub fn read_connection(&mut self, report: &OnlineConnection) {
        let session = report.epoch();
        if session < self.m.state_epoch {
            return;
        }
        if session != self.m.state_epoch {
            self.m.state_epoch = session;
            self.effect(Effect::ConnectionChanged);
        }
        self.m.online_connection = Some(report.clone());
        if !report.account.is_empty() {
            self.m.online_account = report.account.clone();
        }
        if !report.game_id.is_empty() {
            self.m.online_id = report.game_id.clone();
        }
        if report.lane == Lane::Events && !self.m.online_id.is_empty() {
            return;
        }
        if report.lane == Lane::Game && report.phase == OnlinePhase::Idle {
            self.m.online_phase = GamePhase::Idle;
            self.m.online_id.clear();
            self.m.online_status = "No game in progress.".into();
            self.clock_pause();
        }
        if report.unreachable() {
            self.m.online_phase = GamePhase::Disconnected;
            self.m.online_status = report.message.clone().unwrap_or_else(|| {
                "Connection interrupted. Reconnect to recover your game.".into()
            });
            self.clock_pause();
        }
    }

    /// The game is unreachable while Lichess is asked about it.
    pub fn checking(&mut self) {
        self.m.online_phase = GamePhase::Disconnected;
        self.m.online_status = "Checking Lichess for an ongoing game…".into();
        self.m.online_connection = Some(OnlineConnection {
            // An epoch is always a number: it starts at -1 and takes each report's session.
            session: Number::from_f64(self.m.state_epoch).unwrap_or_else(|| Number::from(-1)),
            account: self.m.online_account.clone(),
            game_id: self.m.online_id.clone(),
            lane: Lane::Game,
            phase: OnlinePhase::Checking,
            message: Some(self.m.online_status.clone()),
            kind: None,
        });
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
        m.draw_offer = Offer::None;
        m.takeback_offer = Offer::None;
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
        let previous = self.m.online_phase;
        self.m.online_phase = GamePhase::Finished;
        self.m.online_status = outcome_line(winner, reason, self.m.online_color);
        self.m.online_result = match winner {
            Some("white") => "1-0",
            Some("black") => "0-1",
            // An aborted game has no result; any other undecided finish is a draw.
            _ if matches!(reason, "aborted" | "noStart") => "*",
            _ => "1/2-1/2",
        }
        .into();
        self.m.draw_offer = Offer::None;
        self.m.takeback_offer = Offer::None;
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
                color: self.m.online_color,
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
        if notify && previous == GamePhase::Playing {
            let outcome = if reason == "aborted" {
                "Game aborted"
            } else if winner.is_none() {
                "Draw"
            } else if winner == Some(color::name(self.m.online_color)) {
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
        self.m.online_color = game
            .color
            .as_deref()
            .and_then(color::from_name)
            .unwrap_or(Color::White);
        let opponent = game.opponent.clone().unwrap_or_default();
        self.m.online_opponent = opponent.username.unwrap_or_else(|| "Opponent".into());
        self.m.online_game_rated = game.rated.unwrap_or(false);
        self.m.opponent_id = opponent.id.unwrap_or_default();
        self.m.online_phase = GamePhase::Playing;
        self.m.online_status = "Game in progress".into();
        if !game.correspondence() {
            let body = format!(
                "You are playing {} as {}.",
                self.m.online_opponent,
                color::name(self.m.online_color)
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
        if self.m.online_phase != GamePhase::Playing {
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
        self.m.chat.push(ChatEntry {
            user: event.username.clone(),
            text: event.text.clone(),
            room: event.room.unwrap_or(ChatRoom::Player),
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
                self.m.takeback_offer = Offer::None;
            }
        }
        let white = self.m.online_color == Color::White;
        let their_draw = state.draw(!white);
        let my_draw = state.draw(white);
        if their_draw && self.m.draw_offer != Offer::Theirs && !full {
            let body = format!("{} offers a draw.", self.m.online_opponent);
            self.notify(NotificationKind::GameEvents, "Draw offered", body);
        }
        self.m.draw_offer = if their_draw {
            Offer::Theirs
        } else if my_draw {
            Offer::Mine
        } else {
            Offer::None
        };
        self.m.takeback_offer = if state.takeback(!white) {
            Offer::Theirs
        } else if state.takeback(white) {
            Offer::Mine
        } else {
            Offer::None
        };
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
        let ticking =
            (running && self.m.online_moves.len() >= 2).then(|| color::name(self.m.turn()));
        let initial = self.m.online_initial;
        self.effect(Effect::ClockSet {
            white: white_left,
            black: black_left,
            ticking,
            initial_seconds: (initial != 0.0 && !initial.is_nan()).then_some(initial),
        });
        if running {
            self.m.online_phase = GamePhase::Playing;
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
            self.m.online_color = Color::White;
        } else if is_ours(&black) {
            self.m.online_color = Color::Black;
        }
        let (own, opponent) = if self.m.online_color == Color::White {
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
pub(crate) fn outcome_line(winner: Option<&str>, reason: &str, ours: Color) -> String {
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
        Some(side) if side == color::name(ours) => "You won",
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::model::{Ctx, Model};
    use super::*;

    fn event(value: serde_json::Value) -> ServerEvent {
        ServerEvent::read(&value)
    }

    /// A game state for another game than the one followed is ignored; one for it is read.
    #[test]
    fn a_state_of_another_game_is_ignored() {
        let ctx = Ctx::default();
        let mut model = Model::idle();
        model.online_id = "AbCd1234".into();
        let mut run = Run::new(&mut model, &ctx);
        run.read_event(&event(
            json!({ "type": "gameState", "id": "Other123", "moves": "e2e4" }),
        ));
        assert!(run.m.online_moves.is_empty());
        run.read_event(&event(
            json!({ "type": "gameState", "id": "AbCd1234", "moves": "e2e4" }),
        ));
        assert_eq!(run.m.online_moves, ["e2e4"]);
    }

    /// Berserking in a tournament halves the player's clock: a first state with at most half the
    /// time (and a second's grace) left before the player's first move marks it; a full clock
    /// does not.
    #[test]
    fn a_halved_clock_before_the_first_move_is_a_berserk() {
        let berserked = |wtime: f64| {
            let ctx = Ctx::default();
            let mut model = Model::idle();
            model.online_id = "AbCd1234".into();
            model.online_tournament = "arena".into();
            model.online_clock_config = Some(ClockConfig {
                initial: 180.0,
                increment: 0.0,
            });
            let mut run = Run::new(&mut model, &ctx);
            run.read_event(&event(json!({
                "type": "gameState", "id": "AbCd1234", "moves": "",
                "wtime": wtime, "btime": 180_000, "status": "started",
            })));
            model.berserked
        };
        assert!(berserked(90_000.0));
        assert!(berserked(91_000.0));
        assert!(!berserked(91_001.0));
        assert!(!berserked(180_000.0));
    }

    /// The board's own checking report carries the epoch it holds, and -1 when it holds none.
    #[test]
    fn a_checking_report_carries_the_epoch() {
        let ctx = Ctx::default();
        let mut model = Model::idle();
        model.state_epoch = f64::NAN;
        Run::new(&mut model, &ctx).checking();
        let report = model.online_connection.clone().unwrap();
        assert_eq!(report.session, serde_json::Number::from(-1));
        assert_eq!(model.online_phase, GamePhase::Disconnected);
        model.state_epoch = 7.0;
        Run::new(&mut model, &ctx).checking();
        assert_eq!(model.online_connection.unwrap().epoch(), 7.0);
    }

    /// A chat line is the players' unless it was said in the spectators' room.
    #[test]
    fn a_chat_line_names_its_room() {
        let line = |room| ChatEntry {
            user: None,
            text: None,
            room,
        };
        assert!(line(ChatRoom::Player).players());
        assert!(!line(ChatRoom::Spectator).players());
    }
}
