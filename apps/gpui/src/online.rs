//! The live Lichess game, app-wide: every decision is the rules' (`kchess_domain::online_game`); this
//! keeps their state, makes the calls they ask for, and runs their effects (clock, sounds,
//! notices). Beside it: the challenges, the lobby stream and the games in progress.
//! Engine help stays off while a game is live or being looked for, and
//! until the core has said whether one was left running.

use crate::loading::Epoch;
use kchess_contracts::calls;
use kchess_contracts::check::OptionalAccount;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use futures::FutureExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, Task};
use kchess_contracts::check::{GameId, PresenceUsers, Uci, Username, Validate};
use kchess_contracts::types::{
    ChallengeInfo, ChallengeTimeControl, ChatRoom, DeclineReason, LobbyState, NotificationKind,
    OngoingGame, PresenceReport,
};
use kchess_contracts::{CoreError, Level};
use kchess_domain::online_game::model::{Call, Ctx, Effect, Model, Op, Outcome, Reply, View};
use kchess_domain::online_game::server::{ConnectionReport, ServerEvent};
use kchess_domain::records::accounts::{ListedAccount, connected_account_index};
use kchess_domain::training::clock::{ClockState, SetData, format_clock};
use kchess_domain::variant::{Setup, Variant};
use serde_json::{Value, json};

use crate::core::{AppCore, ClientEvent, CoreEvent};
use crate::game::surface::Faces;

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

/// The find-a-game form.
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub minutes: f64,
    pub increment: f64,
    pub target: String,
    /// `random`, `white` or `black`.
    pub color: String,
    pub rated: bool,
    pub variant: Variant,
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
            variant: Variant::Standard,
            correspondence: false,
            days: 3.,
        }
    }
}

/// Milliseconds on a clock that only goes forward.
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
    /// The rules' state of the game.
    model: Model,
    clock: ClockState,
    /// The position looked at while reviewing; `None` follows the game.
    pub view_ply: Option<usize>,
    pub flipped: bool,
    pub form: Form,
    /// `PresenceReport` for both players, and recent round trips.
    pub presence: Option<PresenceReport>,
    pub latency: Vec<f64>,
    pub move_ack: Option<f64>,
    presence_epoch: Epoch,
    presence_pending: bool,
    /// Something the rules started is waiting for Lichess (`busy`).
    pub busy: bool,
    pub challenges: Vec<ChallengeInfo>,
    pub lobby: Option<LobbyState>,
    pub ongoing: Vec<OngoingGame>,
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
    ticked: Faces<([String; 2], [Option<i64>; 2])>,
    _presence: Option<Task<()>>,
    _ongoing_poll: Option<Task<()>>,
    /// Which `resumeOnline` answer is current: a later one (after a reconnect) replaces it.
    resumes: u64,
}

impl OnlineState {
    /// The core session whose reports the board follows.
    #[cfg(test)]
    pub(crate) fn state_epoch(&self) -> f64 {
        self.model.state_epoch
    }

    /// The engine may help: no live game, and none still being looked for.
    pub fn assistance_allowed(&self) -> bool {
        matches!(self.phase, Phase::Idle | Phase::Finished)
    }

    /// A game of the account's is using the connection.
    pub fn busy_with_game(&self) -> bool {
        match self.phase {
            Phase::Playing | Phase::Seeking => true,
            Phase::Disconnected => !self.id().is_empty(),
            _ => false,
        }
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn id(&self) -> &str {
        &self.model.online_id
    }

    pub fn account(&self) -> &str {
        &self.model.online_account
    }

    pub fn moves(&self) -> Vec<String> {
        self.model.online_moves.clone()
    }

    pub fn setup(&self) -> &Setup {
        &self.model.online_setup
    }

    pub fn color(&self) -> shakmaty::Color {
        if self.model.online_color == "black" {
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

    /// The live position, the SAN history, the moves this side made, whether it is a
    /// correspondence game, and the rematch's options.
    pub fn view(&self) -> View {
        self.model.view()
    }

    pub fn correspondence(&self) -> bool {
        self.model.correspondence()
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

    fn context(cx: &App) -> Ctx {
        let chat = cx
            .try_global::<crate::settings::SettingsHandle>()
            .is_none_or(|settings| settings.0.read(cx).settings.core.online_chat);
        Ctx {
            now: now(),
            active_account: crate::accounts::handle(cx)
                .read(cx)
                .active(cx)
                .unwrap_or_default(),
            chat_enabled: chat,
        }
    }

    /// Lets the rules move the state on, then runs their effects in order.
    fn transition(
        &mut self,
        step: impl FnOnce(&mut Model, &Ctx) -> Outcome,
        cause: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Outcome {
        let ctx = Self::context(cx);
        let outcome = step(&mut self.model, &ctx);
        self.sync_phase(cx);
        for effect in &outcome.effects {
            self.run(effect, cause, cx);
        }
        cx.notify();
        outcome
    }

    /// Put the state in a phase as the rules would (tests): the model too, which the phase is
    /// derived from again whenever a late answer lands.
    #[cfg(test)]
    pub(crate) fn set_phase(&mut self, phase: &str, cx: &mut Context<Self>) {
        self.model.online_phase = phase.into();
        self.sync_phase(cx);
        cx.notify();
    }

    fn sync_phase(&mut self, cx: &mut Context<Self>) {
        let before = self.phase;
        self.phase = match self.model.online_phase.as_str() {
            "seeking" => Phase::Seeking,
            "playing" => Phase::Playing,
            "finished" => Phase::Finished,
            "disconnected" => Phase::Disconnected,
            _ if self.phase == Phase::Checking => Phase::Checking,
            _ => Phase::Idle,
        };
        if before == self.phase {
            return;
        }
        self.presence_epoch.invalidate();
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

    fn run(&mut self, effect: &Effect, cause: Option<&str>, cx: &mut Context<Self>) {
        match effect {
            Effect::Warn { message } => log(&format!("{message} {}", cause.unwrap_or_default())),
            Effect::Notify { kind, title, body } => {
                crate::notifications::desktop(*kind, title, body, cx);
            }
            Effect::Moved { san } => {
                crate::sound::move_sound(san.as_deref().unwrap_or_default(), cx)
            }
            Effect::ResetView => {
                self.view_ply = None;
                self.flipped = false;
                self.presence = None;
            }
            Effect::ConnectionChanged => self.presence_epoch.invalidate(),
            Effect::MoveAcknowledged { ms } => self.move_ack = Some(*ms),
            Effect::Failed => {
                if let Some(cause) = cause {
                    notice(cx, cause.to_string());
                }
            }
            Effect::ClockPause => self.clock.pause(now()),
            Effect::ClockSet {
                white,
                black,
                ticking,
                initial_seconds,
            } => {
                let data = SetData {
                    white: *white,
                    black: *black,
                    ticking: ticking.map(str::to_string),
                    initial_seconds: *initial_seconds,
                    delay_centis: None,
                };
                self.clock.set(&data, now());
            }
            Effect::LoadChat { id } => self.perform(Op::Chat(id.clone()), cx).detach(),
        }
    }

    /// A network-backed operation: the rules start it, the core makes the call, the rules settle
    /// it. The task gives what the rules answer.
    pub fn perform(&mut self, op: Op, cx: &mut Context<Self>) -> Task<Option<bool>> {
        let begun = self.transition(|model, ctx| model.begin(op, ctx), None, cx);
        let (Some(call), Some(pending)) = (begun.call, begun.pending) else {
            return Task::ready(begun.ret);
        };
        let core = cx.global::<AppCore>();
        // The rules keep their calls and answers as JSON: each is typed on its way to the core.
        let call = match call {
            Call::StartOnline(options) => match typed(&options) {
                Ok(options) => answer(core.request(calls::StartOnline { options })),
                Err(refused) => futures::future::ready(Err(refused)).boxed_local(),
            },
            Call::CancelOnline => answer(core.request(calls::CancelOnline {})),
            Call::ResumeOnline => answer(core.request(calls::ResumeOnline {})),
            Call::OpenGame { account, id } => answer(core.request(calls::OpenGame {
                account: Username(account),
                id: GameId(id),
            })),
            Call::PlayOnline { id, uci } => answer(core.request(calls::PlayOnline {
                id: GameId(id),
                r#move: Uci(uci),
            })),
            Call::OnlineAction { id, action } => match typed(&json!(action)) {
                Ok(action) => answer(core.request(calls::OnlineAction {
                    id: GameId(id),
                    action,
                })),
                Err(refused) => futures::future::ready(Err(refused)).boxed_local(),
            },
            Call::OnlineChat { id } => answer(core.request(calls::OnlineChat { id: GameId(id) })),
        };
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            this.update(cx, |this, cx| {
                let (reply, cause) = match answer {
                    Ok(value) => (Reply::Ok(value), None),
                    Err(error) => (Reply::Failed, Some(error.message)),
                };
                let settled = this.transition(
                    |model, ctx| model.settle(&pending, reply, ctx),
                    cause.as_deref(),
                    cx,
                );
                if settled.rethrow
                    && let Some(cause) = cause
                {
                    notice(cx, cause);
                }
                settled.ret
            })
            .unwrap_or(None)
        })
    }

    /// An operation that shows the app busy until it settles.
    fn run_busy(&mut self, op: Op, cx: &mut Context<Self>) -> Task<Option<bool>> {
        self.busy = true;
        cx.notify();
        let task = self.perform(op, cx);
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
    /// Seek a real-time game against anyone: this clock, rated or casual, either colour.
    pub fn seek(&mut self, minutes: f64, increment: f64, rated: bool, cx: &mut Context<Self>) {
        let form = &mut self.form;
        form.minutes = minutes;
        form.increment = increment;
        form.rated = rated;
        form.color = "random".into();
        form.target.clear();
        form.correspondence = false;
        self.start(None, cx);
    }

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
        if form.variant != Variant::Standard {
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
        let started = self.run_busy(Op::Start(wanted), cx);
        cx.spawn(async move |_, cx| {
            let started = started.await;
            if started == Some(true) && correspondence {
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
        self.perform(Op::Stop, cx).detach();
    }

    /// Offer the last opponent the same game again, colours swapped.
    pub fn rematch(&mut self, cx: &mut Context<Self>) {
        if let Some(options) = self.view().rematch {
            self.start(Some(options), cx);
        }
    }

    /// The last game's kind as a seek against anyone (Lichess's _New opponent_): its clock,
    /// rating and variant, either colour. None for one a seek cannot repeat: correspondence, or
    /// from a set-up position.
    pub fn new_opponent_form(&self) -> Option<Form> {
        let options = self.view().rematch?;
        if !options["days"].is_null() || !options["fen"].is_null() {
            return None;
        }
        Some(Form {
            minutes: options["minutes"].as_f64()?,
            increment: options["increment"].as_f64().unwrap_or_default(),
            target: String::new(),
            color: "random".into(),
            rated: options["rated"] == json!(true),
            variant: Variant::from_lichess(options["variant"].as_str())?,
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
        self.run_busy(Op::Reconnect, cx).detach();
    }

    /// One of the account's games in progress on the board.
    pub fn open(&mut self, account: &str, id: &str, cx: &mut Context<Self>) {
        let op = Op::Open {
            account: account.to_string(),
            id: id.to_string(),
        };
        self.run_busy(op, cx).detach();
    }

    pub fn play(&mut self, uci: String, cx: &mut Context<Self>) {
        self.perform(Op::Move(uci), cx).detach();
    }

    /// `onlineAction`: resign, abort, draw and takeback offers, claiming the win, berserk.
    pub fn action(&mut self, action: &str, cx: &mut Context<Self>) {
        self.perform(Op::Action(action.to_string()), cx).detach();
    }

    pub fn send_chat(
        &mut self,
        text: String,
        room: ChatRoom,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let id = self.id().to_string();
        if id.is_empty() || text.trim().is_empty() {
            return Task::ready(false);
        }
        let call = cx.global::<AppCore>().request(calls::SendChat {
            id: GameId(id),
            room,
            text: kchess_contracts::check::ChatText(text.trim().to_string()),
        });
        cx.spawn(async move |_, cx| match call.await {
            Ok(_) => true,
            Err(cause) => {
                cx.update(|cx| notice(cx, cause.message));
                false
            }
        })
    }

    /// An accepted challenge's game is about to start.
    fn prepare_for_game(&mut self, account: &str, id: &str) {
        self.model.online_account = account.to_string();
        self.model.online_id = id.to_string();
        self.model.online_status = "Starting the game…".into();
    }

    fn start_ticking(&mut self, cx: &mut Context<Self>) {
        self._ticker = Some(crate::timer::every(crate::timer::TICK, cx, |this, cx| {
            this.tick(cx);
            true
        }));
        self._presence = Some(crate::timer::every(PRESENCE_EVERY, cx, |this, cx| {
            this.poll_presence(cx);
            true
        }));
    }

    /// Redraw the clocks; the low-time alert sounds once a side.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.phase != Phase::Playing {
            return;
        }
        if self.model.online_clock_config.is_some()
            && let Some(turn) = self.clock_running().map(str::to_string)
            && self.clock.low_time_alert(&turn, now())
        {
            crate::sound::play(crate::sound::Sound::LowTime, cx);
            let mine = self.model.online_color == turn;
            if mine {
                let left = self.clock_text(&turn);
                let opponent = &self.model.online_opponent;
                crate::notifications::desktop(
                    NotificationKind::LowTime,
                    "Low on time",
                    &format!("{left} left against {opponent}"),
                    cx,
                );
            }
        }
        // Redraw only when something the game shows by the clock changes: a clock face (each
        // second, each tenth in the last ten) or a countdown to abort or claim.
        let now = now();
        let seconds_until =
            |at: Option<f64>| at.map(|at| ((at - now) / 1000.).ceil().max(0.) as i64);
        let shown = (
            [self.clock_text("white"), self.clock_text("black")],
            [
                seconds_until(self.model.first_move_by),
                seconds_until(self.model.claim_at),
            ],
        );
        if self.ticked.changed(shown) {
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
        let opponent = self.model.opponent_id.clone();
        let ids = if opponent.is_empty() {
            PresenceUsers(vec![me])
        } else {
            PresenceUsers(vec![me, opponent])
        };
        let epoch = self.presence_epoch.current();
        self.presence_pending = true;
        let call = cx
            .global::<AppCore>()
            .request(calls::Presence { usernames: ids });
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            let _ = this.update(cx, |this, cx| {
                this.presence_pending = false;
                if !this.presence_epoch.is_current(epoch) || this.phase != Phase::Playing {
                    return;
                }
                // A missed poll leaves the last reading; a later one recovers.
                if let Ok(report) = answer {
                    this.latency.push(report.latency_ms as f64);
                    if this.latency.len() > LATENCY_KEPT {
                        this.latency.remove(0);
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
                        for game in &games {
                            let id = game.game_id.clone();
                            let mine = game.is_my_turn;
                            let before = this.turns.get(&id).copied();
                            if game.speed == "correspondence"
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
            let joined = joined.await.unwrap_or_default();
            let now = kchess_games::now_ms();
            let current: Vec<_> = joined.iter().filter(|entry| entry.until > now).collect();
            let participant = current
                .iter()
                .find(|entry| entry.account == active)
                .or(current.last())
                .map(|entry| entry.account.clone());
            let account = participant.unwrap_or(if wanted { active } else { String::new() });
            if let Err(cause) = core
                .request(calls::StayConnected {
                    account: OptionalAccount(account.clone()),
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
                    this.challenges = list;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn challenge_act(
        &mut self,
        id: String,
        call: impl Future<Output = Result<(), CoreError>> + 'static,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        self.busy_challenge = id;
        cx.notify();
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
    pub fn accept(&mut self, challenge: &ChallengeInfo, cx: &mut Context<Self>) -> Task<bool> {
        let id = challenge.id.clone();
        let live = matches!(challenge.time_control, ChallengeTimeControl::Clock { .. });
        if live {
            self.prepare_for_game(&challenge.account, &id);
        }
        let opponent = challenge.opponent.name.clone();
        let call = cx.global::<AppCore>().request(calls::AcceptChallenge {
            id: GameId(id.clone()),
        });
        let done = self.challenge_act(id, call, cx);
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

    pub fn decline(
        &mut self,
        challenge: &ChallengeInfo,
        reason: DeclineReason,
        cx: &mut Context<Self>,
    ) {
        let id = challenge.id.clone();
        let call = cx.global::<AppCore>().request(calls::DeclineChallenge {
            id: GameId(id.clone()),
            reason,
        });
        self.challenge_act(id, call, cx).detach();
    }

    pub fn withdraw(&mut self, challenge: &ChallengeInfo, cx: &mut Context<Self>) {
        let id = challenge.id.clone();
        let call = cx.global::<AppCore>().request(calls::CancelChallenge {
            id: GameId(id.clone()),
        });
        self.challenge_act(id, call, cx).detach();
    }

    /// The opponent's rematch offer for the game just finished.
    pub fn rematch_offer(&self) -> Option<ChallengeInfo> {
        let last = self.model.last_game.as_ref()?.id.as_str();
        self.challenges
            .iter()
            .find(|c| {
                c.direction == kchess_contracts::types::ChallengeDirection::In
                    && c.rematch_of.as_deref() == Some(last)
            })
            .cloned()
    }

    fn receive(&mut self, event: &CoreEvent, cx: &mut Context<Self>) {
        match event {
            CoreEvent::OnlineEvent(payload) => {
                let event = ServerEvent::read(payload);
                let started = match &event {
                    ServerEvent::GameStart { game } => Some(game.clone().unwrap_or_default()),
                    _ => None,
                };
                self.transition(|model, ctx| model.event(&event, ctx), None, cx);
                let Some(game) = started else {
                    return;
                };
                // The event stream announces a game whose board stream already failed for good
                // (on launch the two race): the failure stands, as the core opens no new stream.
                if let Some(failed) = self.failed_state.clone()
                    && ConnectionReport::read(&failed).game_id == game.game_id
                {
                    self.transition(|model, ctx| model.connection(&failed, ctx), None, cx);
                }
                // A pairing or an accepted challenge takes you to the board, as Lichess does; not
                // for a game the Board API cannot play, such as one started on another device.
                if !game.correspondence() && game.board_compatible() {
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
            CoreEvent::OnlineState(state) => {
                // The online-game model reads the report as JSON.
                let payload = &serde_json::to_value(state).unwrap_or_default();
                let report = ConnectionReport::read(payload);
                let kind = report.kind;
                if report.lane.as_deref() == Some("game") {
                    // Only a failure that ended the stream carries a kind.
                    self.failed_state = kind.is_some().then(|| payload.clone());
                    self.failure = kind;
                } else if kind.is_some() {
                    // The event stream's own states never clear the game's failure.
                    self.failure = kind;
                }
                self.transition(|model, ctx| model.connection(payload, ctx), None, cx);
            }
            CoreEvent::OnlineError(message) => notice(cx, message.clone()),
            CoreEvent::OnlineLobby(lobby) => {
                self.lobby = Some(lobby.clone());
                cx.notify();
            }
            CoreEvent::OngoingChanged(()) => self.refresh_ongoing(cx),
            CoreEvent::ChallengesUpdate(challenges) => {
                self.challenges = challenges.clone();
                cx.notify();
            }
            // A new challenge for one of the accounts: Accept and Decline from its notification.
            CoreEvent::ChallengeReceived(challenge) => {
                let challenge = ChallengeInfo::clone(challenge);
                if challenge.direction != kchess_contracts::types::ChallengeDirection::In {
                    return;
                }
                let (title, body) = challenge_alert(&challenge);
                // The notification's buttons find it here, before the inbox's next update.
                let id = challenge.id.clone();
                if !self.challenges.iter().any(|known| known.id == challenge.id) {
                    self.challenges.push(challenge);
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
        self._ongoing_poll = Some(crate::timer::every(every, cx, |this, cx| {
            this.refresh_ongoing(cx);
            true
        }));
    }
}

#[derive(Clone)]
pub struct OnlineHandle(pub Entity<OnlineState>);

impl Global for OnlineHandle {}

pub fn handle(cx: &App) -> Entity<OnlineState> {
    cx.global::<OnlineHandle>().0.clone()
}

/// The account Lichess actions use: the one chosen for online play when it
/// is connected, else the first connected one.
pub fn active_account(accounts: &[kchess_contracts::types::Account], cx: &App) -> Option<String> {
    let preferred = crate::prefs::get::<String>(cx, crate::accounts::ONLINE_ACCOUNT, String::new());
    let listed: Vec<ListedAccount> = accounts
        .iter()
        .map(|account| ListedAccount {
            username: Some(account.username.as_str()),
            connected: account.connected,
        })
        .collect();
    let index = connected_account_index(&listed, Some(&preferred))?;
    listed[index].username.map(str::to_string)
}

/// Which connected account plays.
pub fn set_active_account(account: &str, cx: &mut App) {
    crate::prefs::set(cx, crate::accounts::ONLINE_ACCOUNT, account.to_string());
}

pub fn init(cx: &mut App) {
    let state = cx.new(|cx| {
        let mut state = OnlineState {
            phase: Phase::Checking,
            model: Model::idle(),
            clock: ClockState::initial(),
            view_ply: None,
            flipped: false,
            form: Form::default(),
            presence: None,
            latency: Vec::new(),
            move_ack: None,
            presence_epoch: Epoch::default(),
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
            ticked: Faces::default(),
            _presence: None,
            _ongoing_poll: None,
            resumes: 0,
        };
        state.transition(Model::checking, None, cx);
        state
    });
    cx.set_global(OnlineHandle(state.clone()));
    let events = cx.global::<AppCore>().events.clone();
    cx.subscribe(&events, {
        let state = state.clone();
        move |_, event: &ClientEvent, cx| {
            if let Some(event) = event.core() {
                state.update(cx, |state, cx| state.receive(event, cx));
            }
        }
    })
    .detach();
    // A restarted core attached no game, joined no stream: ask Lichess again, as on startup,
    // never replaying a move that was pending on the old core.
    cx.subscribe(&events, {
        let state = state.clone();
        move |_, event: &ClientEvent, cx| {
            if *event != ClientEvent::Reconnected {
                return;
            }
            state.update(cx, |state, cx| {
                // The new core counts its sessions from the start: the old core's epoch would
                // drop every report it sends. The old connection's events all came before this.
                state.model.state_epoch = 0.0;
                let phase = state.model.online_phase.as_str();
                if matches!(phase, "playing" | "seeking" | "disconnected") {
                    state.phase = Phase::Checking;
                    state.transition(Model::checking, None, cx);
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
    let phase = state.read(cx).model.online_phase.as_str();
    if !matches!(phase, "playing" | "seeking") {
        // Checking from now, not from the core's first report: a seek begun before that report
        // cut the check short, and the check's end put the page back to its form while Lichess
        // still held the seek.
        state.update(cx, |state, cx| {
            state.transition(Model::checking, None, cx);
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
                .request(calls::StayConnected {
                    account: OptionalAccount(String::new()),
                })
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
                Ok(Some(game)) => {
                    // The game may belong to any connected account; it becomes the active one.
                    set_active_account(&game.account, cx);
                    state.model.online_account = game.account;
                    state.model.online_id = game.id;
                    if state.model.online_phase != "playing" {
                        state.model.online_phase = "disconnected".into();
                    }
                }
                Ok(_) => state.model.online_phase = "idle".into(),
                Err(cause) => {
                    log(&format!("resuming online game failed: {}", cause.message));
                    if let Some(connection) = state.model.online_connection.as_mut()
                        && ConnectionReport::read(connection).phase.as_deref() == Some("checking")
                    {
                        connection["phase"] = json!("disconnected");
                    }
                    if state.model.online_phase != "playing" {
                        state.model.online_phase = "idle".into();
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

/// A call the rules made, as its contract type: refused as the core would refuse it.
fn typed<T: Validate>(value: &Value) -> Result<T, CoreError> {
    T::validate(value.into()).map_err(CoreError::new)
}

/// A typed answer as the rules read it.
fn answer<T: serde::Serialize + 'static>(
    call: impl Future<Output = Result<T, CoreError>> + 'static,
) -> futures::future::LocalBoxFuture<'static, Result<Value, CoreError>> {
    call.map(|answer| answer.map(|answer| serde_json::to_value(answer).unwrap_or_default()))
        .boxed_local()
}

/// A challenge's notification: who sent it, then its clock, variant and whether it is rated
/// ("Alice (1800) challenges you", "5+3 · Standard · Rated").
pub fn challenge_alert(challenge: &ChallengeInfo) -> (String, String) {
    let opponent = &challenge.opponent;
    let name = &opponent.name;
    let title = match opponent
        .rating
        .as_ref()
        .and_then(serde_json::Number::as_f64)
    {
        Some(rating) => format!("{name} ({rating}) challenges you"),
        None => format!("{name} challenges you"),
    };
    let variant = &challenge.variant_name;
    let rated = if challenge.rated { "Rated" } else { "Casual" };
    let control = crate::pages::online::lists::describe_control(&challenge.time_control);
    (title, format!("{control} · {variant} · {rated}"))
}

#[cfg(test)]
mod tests {
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
}
