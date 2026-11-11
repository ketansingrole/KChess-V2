//! The live Lichess game, app-wide (over `OnlineGame`): every decision is the rules' (`onlineGame*`); this
//! keeps their state, makes the calls they ask for, and runs their effects (clock, sounds,
//! notices). Beside it: the challenges, the lobby stream and the games in progress.
//! Engine help stays off while a game is live or being looked for, and
//! until the core has said whether one was left running (`assistanceAllowed`).

use kchess_contracts::calls;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, Task};
use kchess_contracts::Level;
use kchess_domain::training::clock::{ClockState, SetData, format_clock};
use serde_json::{Value, json};

use crate::core::{AppCore, CoreEvent};
use crate::game::rules;

/// Clocks and the low-time alert are read this often during a game.
const TICK: Duration = Duration::from_millis(250);
/// Lichess's view of both players' connections, asked this often during a real-time game.
pub const PRESENCE_EVERY: Duration = Duration::from_secs(5);
/// Round trips kept for the connection panel.
const LATENCY_KEPT: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Recovering a live game after startup.
    Checking,
    Idle,
    Seeking,
    Playing,
    Finished,
    Disconnected,
}

/// The find-a-game form (`onlineMinutes` and the rest).
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub minutes: f64,
    pub increment: f64,
    pub target: String,
    /// `random`, `white` or `black`.
    pub color: String,
    pub rated: bool,
    pub variant: String,
    pub correspondence: bool,
    pub days: f64,
}

impl Default for Form {
    fn default() -> Form {
        Form {
            minutes: 15.,
            increment: 10.,
            target: String::new(),
            color: "random".into(),
            rated: false,
            variant: "standard".into(),
            correspondence: false,
            days: 3.,
        }
    }
}

/// `performance.now()`: milliseconds on a clock that only goes forward.
pub fn now() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.
}

fn log(message: &str) {
    crate::log::write(Level::Warn, "online-game", message);
}

/// Show a notice in the app's window.
pub fn notice(cx: &mut App, text: impl Into<gpui_kit::SharedString>) {
    let text = text.into();
    if let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) {
        let _ = window.update(cx, |_, window, cx| window.push_notification(text, cx));
    }
}

pub struct OnlineState {
    pub phase: Phase,
    /// The rules' state: `onlineGameState()` and the class's private fields.
    model: Value,
    /// `ClockState`.
    clock: ClockState,
    /// The position looked at while reviewing; `None` follows the game.
    pub view_ply: Option<usize>,
    pub flipped: bool,
    pub form: Form,
    /// `PresenceReport` for both players, and recent round trips.
    pub presence: Option<Value>,
    pub latency: Vec<f64>,
    pub move_ack: Option<f64>,
    presence_epoch: u64,
    presence_pending: bool,
    /// Something the rules started is waiting for Lichess (`busy`).
    pub busy: bool,
    pub challenges: Vec<Value>,
    pub lobby: Option<Value>,
    pub ongoing: Vec<Value>,
    pub ongoing_loaded: bool,
    pub ongoing_error: Option<String>,
    /// Each games-in-progress request; an answer to an earlier one is dropped.
    ongoing_request: u64,
    /// The kind of failure that ended the game's connection (`ErrorKind`), until it recovers.
    pub failure: Option<String>,
    /// The game connection's state that ended it for good, kept until the game connects again.
    failed_state: Option<Value>,
    pub busy_challenge: String,
    /// Turn flags seen at the last check, to say only when it becomes your turn.
    turns: std::collections::HashMap<String, bool>,
    _ticker: Option<Task<()>>,
    /// What the game showed by the clock at the last tick (`tick`).
    ticked: Option<([String; 2], [Option<i64>; 2])>,
    _presence: Option<Task<()>>,
    _ongoing_poll: Option<Task<()>>,
    /// Which `resumeOnline` answer is current: a later one (after a reconnect) replaces it.
    resumes: u64,
}

impl OnlineState {
    /// The core session whose reports the board follows.
    #[cfg(test)]
    pub(crate) fn state_epoch(&self) -> f64 {
        self.model["stateEpoch"].as_f64().unwrap_or_default()
    }

    /// The engine may help: no live game, and none still being looked for.
    pub fn assistance_allowed(&self) -> bool {
        matches!(self.phase, Phase::Idle | Phase::Finished)
    }

    /// A game of the account's is using the connection (`playingOwnGame`).
    pub fn busy_with_game(&self) -> bool {
        match self.phase {
            Phase::Playing | Phase::Seeking => true,
            Phase::Disconnected => !self.id().is_empty(),
            _ => false,
        }
    }

    pub fn model(&self) -> &Value {
        &self.model
    }

    pub fn id(&self) -> &str {
        self.model["onlineId"].as_str().unwrap_or_default()
    }

    pub fn account(&self) -> &str {
        self.model["onlineAccount"].as_str().unwrap_or_default()
    }

    pub fn moves(&self) -> Vec<String> {
        serde_json::from_value(self.model["onlineMoves"].clone()).unwrap_or_default()
    }

    pub fn setup(&self) -> Value {
        self.model["onlineSetup"].clone()
    }

    pub fn color(&self) -> shakmaty::Color {
        if self.model["onlineColor"] == json!("black") {
            shakmaty::Color::Black
        } else {
            shakmaty::Color::White
        }
    }

    pub fn orientation(&self) -> shakmaty::Color {
        if self.flipped {
            !self.color()
        } else {
            self.color()
        }
    }

    /// The ply looked at.
    pub fn ply(&self) -> usize {
        let count = self.moves().len();
        self.view_ply.unwrap_or(count).min(count)
    }

    pub fn view_ply(&mut self, ply: usize, cx: &mut Context<Self>) {
        let count = self.moves().len();
        let ply = ply.min(count);
        self.view_ply = (ply < count).then_some(ply);
        cx.notify();
    }

    /// `onlineGameView`: the live position, the SAN history, the moves this side made, whether
    /// it is a correspondence game, and the rematch's options.
    /// A real-time game of the account's is on, its clock running on Lichess whether or not
    /// KChess stays open (`beforeunload` on lichess.org asks first too).
    pub fn live_game_on(&self) -> bool {
        let on = match self.phase {
            Phase::Playing => true,
            Phase::Disconnected => !self.id().is_empty(),
            _ => false,
        };
        on && !self.correspondence()
    }

    pub fn view(&self) -> Value {
        rules("onlineGameView", std::slice::from_ref(&self.model)).unwrap_or(Value::Null)
    }

    pub fn correspondence(&self) -> bool {
        self.view()["correspondence"] == json!(true)
    }

    /// Milliseconds left on `color`'s clock.
    pub fn remaining(&self, color: &str) -> f64 {
        self.clock.remaining(color, now())
    }

    pub fn clock_running(&self) -> Option<&str> {
        self.clock.ticking.as_deref()
    }

    pub fn clock_text(&self, color: &str) -> String {
        format_clock(self.remaining(color))
    }

    fn context(cx: &App) -> Value {
        let chat = cx
            .try_global::<crate::settings::SettingsHandle>()
            .is_none_or(|settings| settings.0.read(cx).settings.core.online_chat);
        json!({
            "now": now(),
            "activeAccount": crate::accounts::handle(cx).read(cx).active(cx).unwrap_or_default(),
            "chatEnabled": chat,
        })
    }

    /// Asks the rules for the next state, keeps it, then runs its effects in order.
    fn transition(
        &mut self,
        method: &str,
        args: Vec<Value>,
        cause: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Value {
        let mut all = vec![self.model.clone()];
        all.extend(args);
        all.push(Self::context(cx));
        let next = match rules(method, &all) {
            Ok(next) => next,
            Err(error) => {
                log(&format!("{method} failed: {error}"));
                return Value::Null;
            }
        };
        if next["state"].is_object() {
            self.model = next["state"].clone();
            self.sync_phase(cx);
        }
        for effect in next["effects"].as_array().cloned().unwrap_or_default() {
            self.run(&effect, cause, cx);
        }
        cx.notify();
        next
    }

    /// Put the state in a phase as the rules would (tests): the model too, which the phase is
    /// derived from again whenever a late answer lands.
    #[cfg(test)]
    pub(crate) fn set_phase(&mut self, phase: &str, cx: &mut Context<Self>) {
        self.model["onlinePhase"] = json!(phase);
        self.sync_phase(cx);
        cx.notify();
    }

    fn sync_phase(&mut self, cx: &mut Context<Self>) {
        let before = self.phase;
        self.phase = match self.model["onlinePhase"].as_str() {
            Some("seeking") => Phase::Seeking,
            Some("playing") => Phase::Playing,
            Some("finished") => Phase::Finished,
            Some("disconnected") => Phase::Disconnected,
            _ if self.phase == Phase::Checking => Phase::Checking,
            _ => Phase::Idle,
        };
        if before == self.phase {
            return;
        }
        self.presence_epoch += 1;
        if self.phase == Phase::Playing {
            self.start_ticking(cx);
            self.poll_presence(cx);
        } else {
            self._ticker = None;
            self._presence = None;
            self.presence = None;
            self.latency.clear();
            self.move_ack = None;
        }
        // A game that ends while it is played sounds, as on Lichess.
        if before == Phase::Playing && self.phase == Phase::Finished {
            crate::sound::play(crate::sound::Sound::GenericNotify, cx);
        }
    }

    fn run(&mut self, effect: &Value, cause: Option<&str>, cx: &mut Context<Self>) {
        match effect["effect"].as_str().unwrap_or_default() {
            "warn" => log(&format!(
                "{} {}",
                effect["message"].as_str().unwrap_or_default(),
                cause.unwrap_or_default()
            )),
            "notify" => {
                let title = effect["title"].as_str().unwrap_or_default().to_string();
                let body = effect["body"].as_str().unwrap_or_default().to_string();
                crate::notifications::desktop(
                    effect["kind"].as_str().unwrap_or_default(),
                    &title,
                    &body,
                    cx,
                );
            }
            "moved" => {
                let capture = effect["san"].as_str().is_some_and(|san| san.contains('x'));
                crate::sound::play(
                    if capture {
                        crate::sound::Sound::Capture
                    } else {
                        crate::sound::Sound::Move
                    },
                    cx,
                );
            }
            "resetView" => {
                self.view_ply = None;
                self.flipped = false;
                self.presence = None;
            }
            "connectionChanged" => self.presence_epoch += 1,
            "moveAcknowledged" => self.move_ack = effect["ms"].as_f64(),
            "failed" => {
                if let Some(cause) = cause {
                    notice(cx, cause.to_string());
                }
            }
            "clockPause" => self.clock.pause(now()),
            "clockSet" => match (effect["white"].as_f64(), effect["black"].as_f64()) {
                (Some(white), Some(black)) => {
                    let data = SetData {
                        white,
                        black,
                        ticking: effect["ticking"].as_str().map(str::to_string),
                        initial_seconds: effect["initialSeconds"].as_f64(),
                        delay_centis: None,
                    };
                    self.clock.set(&data, now());
                }
                _ => crate::log::write(
                    Level::Warn,
                    "online",
                    &format!("A clock without both times: {effect}"),
                ),
            },
            "loadChat" => {
                let id = effect["id"].clone();
                self.perform("chat", id, cx).detach();
            }
            _ => {}
        }
    }

    /// A network-backed operation: the rules start it, the core makes the call, the rules settle
    /// it. The task gives what the rules return.
    pub fn perform(
        &mut self,
        operation: &str,
        input: Value,
        cx: &mut Context<Self>,
    ) -> Task<Value> {
        let begun = self.transition("onlineGameBegin", vec![json!(operation), input], None, cx);
        let Some(method) = begun["call"]["method"].as_str() else {
            return Task::ready(begun["ret"].clone());
        };
        let args = begun["call"]["args"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let call = cx.global::<AppCore>().call(method, args);
        let pending = begun["pending"].clone();
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            this.update(cx, |this, cx| {
                let (outcome, cause) = match answer {
                    Ok(value) => (json!({ "ok": true, "value": value }), None),
                    Err(error) => (json!({ "ok": false }), Some(error.message)),
                };
                let settled = this.transition(
                    "onlineGameSettle",
                    vec![pending, outcome],
                    cause.as_deref(),
                    cx,
                );
                if settled["rethrow"] == json!(true)
                    && let Some(cause) = cause
                {
                    notice(cx, cause);
                }
                settled["ret"].clone()
            })
            .unwrap_or(Value::Null)
        })
    }

    /// `run`: an operation that shows the app busy until it settles.
    fn run_busy(&mut self, operation: &str, input: Value, cx: &mut Context<Self>) -> Task<Value> {
        self.busy = true;
        cx.notify();
        let task = self.perform(operation, input, cx);
        cx.spawn(async move |this, cx| {
            let value = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                cx.notify();
            });
            value
        })
    }

    /// `startOnline`: the form's game, or a rematch's options over it.
    pub fn start(&mut self, rematch: Option<Value>, cx: &mut Context<Self>) {
        let form = self.form.clone();
        let correspondence = match &rematch {
            Some(options) => !options["days"].is_null(),
            None => form.correspondence,
        };
        let account = if rematch.is_some() && !self.account().is_empty() {
            self.account().to_string()
        } else {
            crate::accounts::handle(cx)
                .read(cx)
                .active(cx)
                .unwrap_or_default()
        };
        let mut wanted = json!({
            "minutes": form.minutes,
            "increment": form.increment,
            "color": form.color,
            "rated": form.rated,
        });
        // Unset options are left out: the validators reject null.
        if !form.target.trim().is_empty() {
            wanted["target"] = json!(form.target.trim());
        }
        if !account.is_empty() {
            wanted["account"] = json!(account);
        }
        if form.variant != "standard" {
            wanted["variant"] = json!(form.variant);
        }
        if correspondence && rematch.is_none() {
            wanted["days"] = json!(form.days);
        }
        if let Some(options) = rematch.as_ref().and_then(Value::as_object) {
            for (key, value) in options {
                if !value.is_null() {
                    wanted[key] = value.clone();
                }
            }
        }
        let target = wanted["target"].as_str().map(str::to_string);
        let started = self.run_busy("start", wanted, cx);
        cx.spawn(async move |_, cx| {
            let started = started.await;
            if started == json!(true) && correspondence {
                cx.update(|cx| {
                    notice(
                        cx,
                        match target {
                            Some(target) => format!(
                                "Correspondence challenge sent to @{target}. It appears under your games once accepted."
                            ),
                            None => "Correspondence seek created on Lichess. It stays open there until someone joins.".into(),
                        },
                    )
                });
            }
        })
        .detach();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.view_ply = None;
        self.perform("stop", Value::Null, cx).detach();
    }

    /// Offer the last opponent the same game again, colours swapped.
    pub fn rematch(&mut self, cx: &mut Context<Self>) {
        let options = self.view()["rematch"].clone();
        if options.is_object() {
            self.start(Some(options), cx);
        }
    }

    /// The last game's kind as a seek against anyone (Lichess's _New opponent_): its clock,
    /// rating and variant, either colour. None for one a seek cannot repeat: correspondence, or
    /// from a set-up position.
    pub fn new_opponent_form(&self) -> Option<Form> {
        let options = self.view()["rematch"].clone();
        if !options.is_object() || !options["days"].is_null() || !options["fen"].is_null() {
            return None;
        }
        Some(Form {
            minutes: options["minutes"].as_f64()?,
            increment: options["increment"].as_f64().unwrap_or_default(),
            target: String::new(),
            color: "random".into(),
            rated: options["rated"] == json!(true),
            variant: options["variant"]
                .as_str()
                .unwrap_or("standard")
                .to_string(),
            correspondence: false,
            days: self.form.days,
        })
    }

    /// Seek the last game's kind again; the form is left as it was.
    pub fn new_opponent(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.new_opponent_form() else {
            return;
        };
        let kept = std::mem::replace(&mut self.form, form);
        self.start(None, cx);
        self.form = kept;
    }

    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        self.run_busy("reconnect", Value::Null, cx).detach();
    }

    /// `openOngoing`: one of the account's games in progress on the board.
    pub fn open(&mut self, account: &str, id: &str, cx: &mut Context<Self>) {
        self.run_busy("open", json!({ "account": account, "id": id }), cx)
            .detach();
    }

    pub fn play(&mut self, uci: String, cx: &mut Context<Self>) {
        self.perform("move", json!(uci), cx).detach();
    }

    /// `onlineAction`: resign, abort, draw and takeback offers, claiming the win, berserk.
    pub fn action(&mut self, action: &str, cx: &mut Context<Self>) {
        self.perform("action", json!(action), cx).detach();
    }

    pub fn send_chat(&mut self, text: String, room: &str, cx: &mut Context<Self>) -> Task<bool> {
        let id = self.id().to_string();
        if id.is_empty() || text.trim().is_empty() {
            return Task::ready(false);
        }
        let call = cx.global::<AppCore>().request(calls::SendChat {
            id: json!(id),
            room: json!(room),
            text: json!(text.trim()),
        });
        cx.spawn(async move |_, cx| match call.await {
            Ok(_) => true,
            Err(cause) => {
                cx.update(|cx| notice(cx, cause.message));
                false
            }
        })
    }

    /// `prepareForGame`: an accepted challenge's game is about to start.
    fn prepare_for_game(&mut self, account: &str, id: &str) {
        self.model["onlineAccount"] = json!(account);
        self.model["onlineId"] = json!(id);
        self.model["onlineStatus"] = json!("Starting the game…");
    }

    fn start_ticking(&mut self, cx: &mut Context<Self>) {
        self._ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = this.update(cx, |this, cx| this.tick(cx)).is_ok();
                if !alive {
                    return;
                }
            }
        }));
        self._presence = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PRESENCE_EVERY).await;
                let alive = this.update(cx, |this, cx| this.poll_presence(cx)).is_ok();
                if !alive {
                    return;
                }
            }
        }));
    }

    /// Redraw the clocks; the low-time alert sounds once a side.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.phase != Phase::Playing {
            return;
        }
        if self.model["onlineClockConfig"].is_object()
            && let Some(turn) = self.clock_running().map(str::to_string)
            && self.clock.low_time_alert(&turn, now())
        {
            crate::sound::play(crate::sound::Sound::LowTime, cx);
            let mine = self.model["onlineColor"] == json!(turn);
            if mine {
                let left = self.clock_text(&turn);
                let opponent = self.model["onlineOpponent"].as_str().unwrap_or_default();
                crate::notifications::desktop(
                    "lowTime",
                    "Low on time",
                    &format!("{left} left against {opponent}"),
                    cx,
                );
            }
        }
        // Redraw only when something the game shows by the clock changes: a clock face (each
        // second, each tenth in the last ten) or a countdown to abort or claim.
        let now = now();
        let seconds_until = |key: &str| {
            self.model[key]
                .as_f64()
                .map(|at| ((at - now) / 1000.).ceil().max(0.) as i64)
        };
        let shown = (
            [self.clock_text("white"), self.clock_text("black")],
            [seconds_until("firstMoveBy"), seconds_until("claimAt")],
        );
        if self.ticked.as_ref() != Some(&shown) {
            self.ticked = Some(shown);
            cx.notify();
        }
    }

    /// Both players' connections, as Lichess sees them, and the round trip there.
    fn poll_presence(&mut self, cx: &mut Context<Self>) {
        let me = if self.account().is_empty() {
            crate::accounts::handle(cx)
                .read(cx)
                .active(cx)
                .unwrap_or_default()
        } else {
            self.account().to_string()
        };
        // A correspondence game moves once a day; its presence is not worth asking for.
        if self.presence_pending
            || self.phase != Phase::Playing
            || me.is_empty()
            || self.correspondence()
        {
            return;
        }
        let opponent = self.model["opponentId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let ids = if opponent.is_empty() {
            json!([me])
        } else {
            json!([me, opponent])
        };
        let epoch = self.presence_epoch;
        self.presence_pending = true;
        let call = cx
            .global::<AppCore>()
            .request(calls::Presence { usernames: ids });
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            let _ = this.update(cx, |this, cx| {
                this.presence_pending = false;
                if epoch != this.presence_epoch || this.phase != Phase::Playing {
                    return;
                }
                // A missed poll leaves the last reading; a later one recovers.
                if let Ok(report) = answer {
                    if let Some(ms) = report["latencyMs"].as_f64() {
                        this.latency.push(ms);
                        if this.latency.len() > LATENCY_KEPT {
                            this.latency.remove(0);
                        }
                    }
                    this.presence = Some(report);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /* ── Challenges and games in progress ── */

    pub fn refresh_ongoing(&mut self, cx: &mut Context<Self>) {
        // A newer request either way: an answer still on its way (a poll overtaken by a change,
        // or from before signing out) must not bring back older games, or their notifications.
        self.ongoing_request += 1;
        let request = self.ongoing_request;
        if crate::accounts::handle(cx).read(cx).connected().is_empty() {
            self.ongoing.clear();
            self.ongoing_error = None;
            cx.notify();
            return;
        }
        let call = cx.global::<AppCore>().request(calls::OngoingGames {});
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            let _ = this.update(cx, |this, cx| {
                if request != this.ongoing_request {
                    return;
                }
                match answer {
                    Ok(games) => {
                        let games = games.as_array().cloned().unwrap_or_default();
                        for game in &games {
                            let id = game["gameId"].as_str().unwrap_or_default().to_string();
                            let mine = game["isMyTurn"] == json!(true);
                            let before = this.turns.get(&id).copied();
                            if game["speed"] == json!("correspondence")
                                && mine
                                && before == Some(false)
                                && id != this.id()
                            {
                                crate::notifications::your_move(game, cx);
                            }
                            this.turns.insert(id, mine);
                        }
                        this.ongoing = games;
                        this.ongoing_error = None;
                    }
                    Err(cause) => this.ongoing_error = Some(cause.message),
                }
                this.ongoing_loaded = true;
                cx.notify();
            });
        })
        .detach();
    }

    /// The account whose event stream stays open while idle: one taking part in a tournament,
    /// else the active one when challenges are wanted.
    pub fn stay_connected(&mut self, cx: &mut Context<Self>) {
        let wanted = cx
            .try_global::<crate::settings::SettingsHandle>()
            .is_none_or(|settings| settings.0.read(cx).settings.core.receive_challenges);
        let joined = cx.global::<AppCore>().request(calls::JoinedTournaments {});
        let active = crate::accounts::handle(cx)
            .read(cx)
            .active(cx)
            .unwrap_or_default();
        let core = cx.global::<AppCore>().clone();
        cx.spawn(async move |this, cx| {
            let joined = joined.await.unwrap_or(json!([]));
            let now = chrono::Utc::now().timestamp_millis() as f64;
            let current: Vec<&Value> = joined
                .as_array()
                .into_iter()
                .flatten()
                .filter(|entry| entry["until"].as_f64().is_some_and(|until| until > now))
                .collect();
            let participant = current
                .iter()
                .find(|entry| entry["account"].as_str() == Some(active.as_str()))
                .or(current.last())
                .and_then(|entry| entry["account"].as_str().map(str::to_string));
            let account = participant.unwrap_or(if wanted { active } else { String::new() });
            if let Err(cause) = core
                .request(calls::StayConnected {
                    account: json!(account),
                })
                .await
            {
                log(&format!("staying connected failed: {}", cause.message));
            }
            if account.is_empty() {
                let _ = this.update(cx, |this, cx| {
                    this.lobby = None;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn load_challenges(&mut self, cx: &mut Context<Self>) {
        let call = cx.global::<AppCore>().request(calls::Challenges {});
        cx.spawn(async move |this, cx| {
            if let Ok(list) = call.await {
                let _ = this.update(cx, |this, cx| {
                    this.challenges = list.as_array().cloned().unwrap_or_default();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn challenge_act(
        &mut self,
        id: String,
        method: &'static str,
        args: Vec<Value>,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        self.busy_challenge = id;
        cx.notify();
        let call = cx.global::<AppCore>().call(method, args);
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            this.update(cx, |this, cx| {
                this.busy_challenge.clear();
                cx.notify();
                match answer {
                    Ok(_) => true,
                    Err(cause) => {
                        notice(cx, cause.message);
                        false
                    }
                }
            })
            .unwrap_or(false)
        })
    }

    /// Accept a challenge: a live one opens on the board; a correspondence one joins the games.
    pub fn accept(&mut self, challenge: &Value, cx: &mut Context<Self>) -> Task<bool> {
        let id = challenge["id"].as_str().unwrap_or_default().to_string();
        let live = challenge["timeControl"]["type"] == json!("clock");
        if live {
            self.prepare_for_game(challenge["account"].as_str().unwrap_or_default(), &id);
        }
        let opponent = challenge["opponent"]["name"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let done = self.challenge_act(id.clone(), "acceptChallenge", vec![json!(id)], cx);
        cx.spawn(async move |this, cx| {
            let done = done.await;
            if done && !live {
                let _ = this.update(cx, |this, cx| {
                    notice(
                        cx,
                        format!("Accepted. Your game against {opponent} is under Correspondence."),
                    );
                    this.refresh_ongoing(cx);
                });
            }
            done && live
        })
    }

    pub fn decline(&mut self, challenge: &Value, reason: &str, cx: &mut Context<Self>) {
        let id = challenge["id"].as_str().unwrap_or_default().to_string();
        self.challenge_act(
            id.clone(),
            "declineChallenge",
            vec![json!(id), json!(reason)],
            cx,
        )
        .detach();
    }

    pub fn withdraw(&mut self, challenge: &Value, cx: &mut Context<Self>) {
        let id = challenge["id"].as_str().unwrap_or_default().to_string();
        self.challenge_act(id.clone(), "cancelChallenge", vec![json!(id)], cx)
            .detach();
    }

    /// The opponent's rematch offer for the game just finished.
    pub fn rematch_offer(&self) -> Option<Value> {
        let last = self.model["lastGame"]["id"].as_str()?;
        self.challenges
            .iter()
            .find(|c| c["direction"] == json!("in") && c["rematchOf"].as_str() == Some(last))
            .cloned()
    }

    fn receive(&mut self, event: &CoreEvent, cx: &mut Context<Self>) {
        match event.name.as_str() {
            "online:event" => {
                // Not for a game the Board API cannot play, such as one started on another device.
                let starts = event.payload["type"] == json!("gameStart")
                    && event.payload["game"]["speed"] != json!("correspondence")
                    && event.payload["game"]["compat"]["board"] != json!(false);
                self.transition("onlineGameEvent", vec![event.payload.clone()], None, cx);
                // The event stream announces a game whose board stream already failed for good
                // (on launch the two race): the failure stands, as the core opens no new stream.
                if event.payload["type"] == json!("gameStart")
                    && let Some(failed) = self.failed_state.clone()
                    && failed["gameId"] == event.payload["game"]["gameId"]
                {
                    self.transition("onlineGameConnection", vec![failed], None, cx);
                }
                // A pairing or an accepted challenge takes you to the board, as Lichess does.
                if starts {
                    cx.defer(|cx| {
                        if let Some(window) =
                            cx.active_window().or_else(|| cx.windows().first().copied())
                        {
                            let _ = window.update(cx, |_, window, cx| {
                                crate::shell::open(crate::pages::Page::Online, window, cx)
                            });
                        }
                    });
                }
            }
            "online:state" => {
                let kind = event.payload["kind"].as_str().map(str::to_string);
                if event.payload["lane"] == json!("game") {
                    // Only a failure that ended the stream carries a kind.
                    self.failed_state = kind.is_some().then(|| event.payload.clone());
                    self.failure = kind;
                } else if kind.is_some() {
                    // The event stream's own states never clear the game's failure.
                    self.failure = kind;
                }
                self.transition(
                    "onlineGameConnection",
                    vec![event.payload.clone()],
                    None,
                    cx,
                );
            }
            "online:error" => {
                if let Some(message) = event.payload.as_str() {
                    notice(cx, message.to_string());
                }
            }
            "online:lobby" => {
                self.lobby = Some(event.payload.clone()).filter(|lobby| !lobby.is_null());
                cx.notify();
            }
            "online:ongoing-changed" => self.refresh_ongoing(cx),
            "challenges:update" => {
                self.challenges = event.payload.as_array().cloned().unwrap_or_default();
                cx.notify();
            }
            // A new challenge for one of the accounts: Accept and Decline from its notification.
            "challenge:received" if event.payload["direction"] == json!("in") => {
                let challenge = &event.payload;
                let (title, body) = challenge_alert(challenge);
                // The notification's buttons find it here, before the inbox's next update.
                let id = challenge["id"].as_str().unwrap_or_default().to_string();
                if !self
                    .challenges
                    .iter()
                    .any(|known| known["id"] == challenge["id"])
                {
                    self.challenges.push(challenge.clone());
                }
                crate::notifications::challenge(&id, &title, &body, cx);
            }
            _ => {}
        }
    }

    fn start_polling_ongoing(&mut self, cx: &mut Context<Self>) {
        let minutes = cx
            .try_global::<crate::settings::SettingsHandle>()
            .map_or(5., |settings| {
                settings.0.read(cx).settings.core.correspondence_poll
            });
        if minutes <= 0. {
            self._ongoing_poll = None;
            return;
        }
        let every = Duration::from_secs_f64(minutes.max(1.) * 60.);
        self._ongoing_poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(every).await;
                if this
                    .update(cx, |this, cx| this.refresh_ongoing(cx))
                    .is_err()
                {
                    return;
                }
            }
        }));
    }
}

#[derive(Clone)]
pub struct OnlineHandle(pub Entity<OnlineState>);

impl Global for OnlineHandle {}

pub fn handle(cx: &App) -> Entity<OnlineState> {
    cx.global::<OnlineHandle>().0.clone()
}

/// The account Lichess actions use (`activeOnlineAccount`): the one chosen for online play when it
/// is connected, else the first connected one.
pub fn active_account(accounts: &[Value], cx: &App) -> Option<String> {
    let preferred = crate::prefs::get::<String>(cx, crate::accounts::ONLINE_ACCOUNT, String::new());
    let listed: Vec<Value> = accounts
        .iter()
        .map(
            |account| json!({ "username": account["username"], "connected": account["connected"] }),
        )
        .collect();
    let index = crate::game::rules("connectedAccountIndex", &[json!(listed), json!(preferred)])
        .ok()
        .filter(|index| !index.is_null())?;
    accounts.get(crate::game::whole(&index) as usize)?["username"]
        .as_str()
        .map(str::to_string)
}

/// Which connected account plays (`setOnlineAccount`).
pub fn set_active_account(account: &str, cx: &mut App) {
    crate::prefs::set(cx, crate::accounts::ONLINE_ACCOUNT, account.to_string());
}

pub fn init(cx: &mut App) {
    let state = cx.new(|cx| {
        let mut state = OnlineState {
            phase: Phase::Checking,
            model: kchess_domain::online_game::idle_state(),
            clock: ClockState::initial(),
            view_ply: None,
            flipped: false,
            form: Form::default(),
            presence: None,
            latency: Vec::new(),
            move_ack: None,
            presence_epoch: 0,
            presence_pending: false,
            busy: false,
            challenges: Vec::new(),
            lobby: None,
            ongoing: Vec::new(),
            ongoing_loaded: false,
            ongoing_error: None,
            ongoing_request: 0,
            failure: None,
            failed_state: None,
            busy_challenge: String::new(),
            turns: std::collections::HashMap::new(),
            _ticker: None,
            ticked: None,
            _presence: None,
            _ongoing_poll: None,
            resumes: 0,
        };
        state.transition("onlineGameChecking", Vec::new(), None, cx);
        state
    });
    cx.set_global(OnlineHandle(state.clone()));
    let events = cx.global::<AppCore>().events.clone();
    cx.subscribe(&events, {
        let state = state.clone();
        move |_, event: &CoreEvent, cx| {
            if event.name.starts_with("online:")
                || event.name == "challenges:update"
                || event.name == "challenge:received"
            {
                state.update(cx, |state, cx| state.receive(event, cx));
            }
        }
    })
    .detach();
    // A restarted core attached no game, joined no stream: ask Lichess again, as on startup,
    // never replaying a move that was pending on the old core.
    cx.subscribe(&events, {
        let state = state.clone();
        move |_, event: &CoreEvent, cx| {
            if event.name != crate::core::RECONNECTED {
                return;
            }
            state.update(cx, |state, cx| {
                // The new core counts its sessions from the start: the old core's epoch would
                // drop every report it sends. The old connection's events all came before this.
                state.model["stateEpoch"] = json!(0);
                let phase = state.model["onlinePhase"].as_str().unwrap_or_default();
                if matches!(phase, "playing" | "seeking" | "disconnected") {
                    state.phase = Phase::Checking;
                    state.transition("onlineGameChecking", Vec::new(), None, cx);
                }
            });
            resume(state.clone(), cx);
        }
    })
    .detach();
    resume(state, cx);
}

/// An account was connected: its game and its event stream, as at startup. A game on the board
/// stays; the connection follows it.
pub fn account_connected(cx: &mut App) {
    let state = handle(cx);
    let phase = state.read(cx).model["onlinePhase"].as_str();
    if !matches!(phase, Some("playing" | "seeking")) {
        // Checking from now, not from the core's first report: a seek begun before that report
        // cut the check short, and the check's end put the page back to its form while Lichess
        // still held the seek.
        state.update(cx, |state, cx| {
            state.transition("onlineGameChecking", Vec::new(), None, cx);
        });
        resume_quietly(state, true, cx);
    }
}

/// Attach the game Lichess says is in progress, then stay connected for challenges and games.
fn resume(state: Entity<OnlineState>, cx: &mut App) {
    resume_quietly(state, false, cx);
}

/// `resume`, with `quiet_first` closing the idle event stream before it. A resume ends by opening
/// the stream of the account the core last kept it for: after a connect that is the previous
/// account, whose stream then opened once more only to be replaced by the new account's.
fn resume_quietly(state: Entity<OnlineState>, quiet_first: bool, cx: &mut App) {
    let attempt = state.update(cx, |state, _| {
        state.resumes += 1;
        state.resumes
    });
    let core = cx.global::<AppCore>().clone();
    let resume = async move {
        if quiet_first {
            let _ = core
                .request(calls::StayConnected { account: json!("") })
                .await;
        }
        core.request(calls::ResumeOnline {}).await
    };
    cx.spawn(async move |cx| {
        let resumed = resume.await;
        state.update(cx, |state, cx| {
            if state.resumes != attempt {
                return;
            }
            match resumed {
                Ok(game) if game.is_object() => {
                    // The game may belong to any connected account; it becomes the active one.
                    let account = game["account"].as_str().unwrap_or_default().to_string();
                    set_active_account(&account, cx);
                    state.model["onlineAccount"] = json!(account);
                    state.model["onlineId"] = game["id"].clone();
                    if state.model["onlinePhase"] != json!("playing") {
                        state.model["onlinePhase"] = json!("disconnected");
                    }
                }
                Ok(_) => state.model["onlinePhase"] = json!("idle"),
                Err(cause) => {
                    log(&format!("resuming online game failed: {}", cause.message));
                    if state.model["onlineConnection"]["phase"] == json!("checking") {
                        state.model["onlineConnection"]["phase"] = json!("disconnected");
                    }
                    if state.model["onlinePhase"] != json!("playing") {
                        state.model["onlinePhase"] = json!("idle");
                    }
                }
            }
            // The check is over: the phase is the rules' from now on.
            state.phase = Phase::Idle;
            state.sync_phase(cx);
            state.load_challenges(cx);
            state.stay_connected(cx);
            state.refresh_ongoing(cx);
            state.start_polling_ongoing(cx);
            cx.notify();
        });
    })
    .detach();
}

/// A challenge's notification: who sent it, then its clock, variant and whether it is rated
/// ("Alice (1800) challenges you", "5+3 · Standard · Rated").
pub fn challenge_alert(challenge: &Value) -> (String, String) {
    let opponent = &challenge["opponent"];
    let name = opponent["name"].as_str().unwrap_or("Someone");
    let title = match opponent["rating"].as_f64() {
        Some(rating) => format!("{name} ({rating}) challenges you"),
        None => format!("{name} challenges you"),
    };
    let variant = challenge["variantName"].as_str().unwrap_or("Standard");
    let rated = if challenge["rated"] == json!(true) {
        "Rated"
    } else {
        "Casual"
    };
    let control = crate::pages::online::lists::describe_control(challenge);
    (title, format!("{control} · {variant} · {rated}"))
}

#[cfg(test)]
mod tests {
    use super::Form;
    use crate::e2e::Harness;

    /// Signing out while the games in progress loaded kept that request current: its answer
    /// brought the account's games back, and an earlier failure stayed on the empty list.
    #[gpui_kit::test]
    fn games_in_progress_from_before_signing_out_do_not_come_back(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let mut app = Harness::start(cx);
        let (stale, error) = app.with_window(|_, cx| {
            super::handle(cx).update(cx, |online, cx| {
                online.ongoing_error = Some("Lichess did not answer".into());
                let asked = online.ongoing_request;
                // No account is connected in a fresh profile.
                online.refresh_ongoing(cx);
                (
                    online.ongoing_request == asked,
                    online.ongoing_error.clone(),
                )
            })
        });
        assert!(!stale);
        assert_eq!(error, None);
    }

    #[test]
    fn the_form_starts_at_15_plus_10() {
        let form = Form::default();
        assert_eq!((form.minutes, form.increment), (15., 10.));
        assert_eq!(form.color, "random");
        assert!(!form.rated && !form.correspondence);
    }
}
