//! The live Lichess game, app-wide: every decision is the rules' (`kchess_domain::online_game`); this
//! keeps their state, makes the calls they ask for, and runs their effects (clock, sounds,
//! notices). Beside it: the challenges, the lobby stream and the games in progress.
//! Engine help stays off while a game is live or being looked for, and
//! until the core has said whether one was left running.

use crate::loading::{Epoch, Loading};
use kchess_contracts::calls;
use kchess_contracts::check::OptionalAccount;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use futures::FutureExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, AppContext as _, Context, Entity, Global, Task};
use kchess_app::log::ResultExt as _;
use kchess_contracts::check::{GameId, PresenceUsers, Uci, Username, Validate};
use kchess_contracts::types::{Assistance, Lane, OnlineConnection, OnlinePhase};
use kchess_contracts::types::{ChallengeColor, OnlineAction, OnlineOptions};
use kchess_contracts::types::{
    ChallengeInfo, ChallengeTimeControl, ChatRoom, DeclineReason, LobbyState, NotificationKind,
    OngoingGame, PresenceReport,
};
use kchess_contracts::{CoreError, ErrorKind, Level};
use kchess_domain::online_game::model::{
    Call, ChatEntry, Ctx, Effect, GamePhase, Model, Op, Outcome, Rematch, Reply, View,
};
use kchess_domain::online_game::server::ServerEvent;
use kchess_domain::training::clock::{ClockState, SetData, format_clock};
use kchess_domain::variant::{Setup, Variant};
use serde_json::{Value, json};

use crate::core::{AppCore, ClientEvent, CoreEvent};
use crate::game::surface::Faces;

mod assistance;
mod challenges;
pub use assistance::*;
pub use challenges::*;

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
    pub color: ChallengeColor,
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
            color: ChallengeColor::Random,
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
    /// The games-in-progress request: an answer to an earlier one is dropped.
    pub ongoing_loading: Loading,
    /// The kind of failure that ended the game's connection, until it recovers.
    pub failure: Option<ErrorKind>,
    /// The game connection's state that ended it for good, kept until the game connects again.
    failed_state: Option<OnlineConnection>,
    /// The game whose connection was lost for good and not yet back, to ask about once the
    /// game's page shows it.
    lost: Option<String>,
    /// The game whose lost connection was last put to the player in a dialog, so one loss asks
    /// once; cleared when its connection comes back.
    alerted: Option<String>,
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
    /// The help the core lends, as it last said (`online:assistance`): its lock is the one that
    /// counts. Locked until it says otherwise, as a core starts.
    pub assistance: Assistance,
    /// `online:assistance` reports seen, so an answer to `assistanceStatus` older than one is
    /// dropped.
    assistance_reports: u64,
    /// How long after a failed check for a game in progress the next one runs; doubles up to
    /// `RECHECK_MAX`, and starts over once a check answers.
    recheck_after: Duration,
    _recheck: Option<Task<()>>,
}

impl OnlineState {
    /// The core session whose reports the board follows.
    #[cfg(test)]
    pub fn state_epoch(&self) -> f64 {
        self.model.state_epoch
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

    /// A game connection gone for good (lost, taken by another device, its login refused) stops
    /// play until the player acts, so it is put to them in a dialog, once per loss. Reconnecting
    /// on its own is only told in the game's panel: a dialog would come and go with each try.
    fn alert_if_lost(&mut self, report: &OnlineConnection, cx: &mut Context<Self>) {
        let lost = matches!(
            report.phase,
            OnlinePhase::Disconnected | OnlinePhase::AuthRequired
        );
        if !lost {
            self.alerted = None;
            self.lost = None;
            return;
        }
        let game = report.game_id.as_str();
        if game.is_empty() || game != self.id() || self.alerted.as_deref() == Some(game) {
            return;
        }
        self.lost = Some(game.to_string());
        // Asked once the game's page shows it (`connection_lost`), now or when it is opened:
        // on launch the page opening and the loss arrive in either order.
        cx.defer(|cx| {
            if let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) {
                window
                    .update(cx, |_, window, cx| {
                        crate::pages::online::game::connection_lost(window, cx)
                    })
                    .ok();
            }
        });
    }

    pub fn id(&self) -> &str {
        &self.model.online_id
    }

    /// Note that the game's lost connection is being put to the player; false when nothing was
    /// lost or it already was asked about, so one loss asks once.
    /// A game played without a live connection (correspondence) reads as disconnected too, with
    /// nothing lost: only a loss the core reported is asked about.
    pub fn mark_alerted(&mut self) -> bool {
        if self.id().is_empty()
            || self.lost.as_deref() != Some(self.id())
            || self.alerted.as_deref() == Some(self.id())
        {
            return false;
        }
        self.alerted = Some(self.id().to_string());
        true
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
        self.model.online_color
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
    pub fn set_phase(&mut self, phase: GamePhase, cx: &mut Context<Self>) {
        self.model.online_phase = phase;
        self.sync_phase(cx);
        cx.notify();
    }

    fn sync_phase(&mut self, cx: &mut Context<Self>) {
        let before = self.phase;
        self.phase = match self.model.online_phase {
            GamePhase::Seeking => Phase::Seeking,
            GamePhase::Playing => Phase::Playing,
            GamePhase::Finished => Phase::Finished,
            GamePhase::Disconnected => Phase::Disconnected,
            GamePhase::Idle if self.phase == Phase::Checking => Phase::Checking,
            GamePhase::Idle => Phase::Idle,
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
        let done = |_| Reply::Done;
        let call = match call {
            Call::StartOnline(options) => {
                reply(core.request(calls::StartOnline { options }), |started| {
                    Reply::Started {
                        challenge: started.url.is_some_and(|url| !url.is_empty()),
                    }
                })
            }
            Call::CancelOnline => reply(core.request(calls::CancelOnline {}), done),
            Call::ResumeOnline => reply(core.request(calls::ResumeOnline {}), Reply::Resumed),
            Call::OpenGame { account, id } => reply(
                core.request(calls::OpenGame {
                    account: Username(account),
                    id: GameId(id),
                }),
                done,
            ),
            Call::PlayOnline { id, uci } => reply(
                core.request(calls::PlayOnline {
                    id: GameId(id),
                    r#move: Uci(uci),
                }),
                done,
            ),
            Call::OnlineAction { id, action } => reply(
                core.request(calls::OnlineAction {
                    id: GameId(id),
                    action,
                }),
                done,
            ),
            Call::OnlineChat { id } => reply(
                core.request(calls::OnlineChat { id: GameId(id) }),
                |lines| {
                    Reply::Chat(
                        lines
                            .into_iter()
                            .map(|line| ChatEntry {
                                user: Some(line.user),
                                text: Some(line.text),
                                room: line.room,
                            })
                            .collect(),
                    )
                },
            ),
        };
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            this.update(cx, |this, cx| {
                let (reply, cause) = match answer {
                    Ok(reply) => (reply, None),
                    Err(error) => (
                        Reply::Failed,
                        Some(kchess_app::loading::failure("online-game", &error)),
                    ),
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
        form.color = ChallengeColor::Random;
        form.target.clear();
        form.correspondence = false;
        self.start(None, cx);
    }

    pub fn start(&mut self, rematch: Option<Rematch>, cx: &mut Context<Self>) {
        let form = self.form.clone();
        let correspondence = match &rematch {
            Some(rematch) => rematch.days.is_some(),
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
        if let Some(Value::Object(options)) = rematch.as_ref().map(|rematch| json!(rematch)) {
            for (key, value) in options {
                wanted[key] = value;
            }
        }
        let options: OnlineOptions = match typed(&wanted) {
            Ok(options) => options,
            Err(refused) => {
                notice(cx, kchess_app::loading::failure("online-game", &refused));
                return;
            }
        };
        let target = options.target.clone();
        let started = self.run_busy(Op::Start(options), cx);
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
        if options.days.is_some() || options.fen.is_some() {
            return None;
        }
        Some(Form {
            minutes: options.minutes?,
            increment: options.increment.unwrap_or_default(),
            target: String::new(),
            color: ChallengeColor::Random,
            rated: options.rated,
            variant: options.variant.unwrap_or_default(),
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
    pub fn action(&mut self, action: OnlineAction, cx: &mut Context<Self>) {
        self.perform(Op::Action(action), cx).detach();
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
                cx.update(|cx| notice(cx, kchess_app::loading::failure("online-game", &cause)));
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
            let mine = kchess_domain::color::name(self.model.online_color) == turn;
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
                // gpui-lints: allow(dropped_failure) a missed poll keeps the last reading and a later one recovers; logging each would fill the log while offline
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
                    && game.game_id.as_ref() == Some(&failed.game_id)
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
                            window
                                .update(cx, |_, window, cx| {
                                    crate::shell::open(crate::pages::Page::Online, window, cx)
                                })
                                .ok();
                        }
                    });
                }
            }
            CoreEvent::OnlineState(report) => {
                let kind = report.kind;
                if report.lane == Lane::Game {
                    // Only a failure that ended the stream carries a kind.
                    self.failed_state = kind.is_some().then(|| report.clone());
                    self.failure = kind;
                } else if kind.is_some() {
                    // The event stream's own states never clear the game's failure.
                    self.failure = kind;
                }
                self.transition(|model, ctx| model.connection(report, ctx), None, cx);
                if report.lane == Lane::Game {
                    self.alert_if_lost(report, cx);
                }
            }
            CoreEvent::Assistance(assistance) => {
                self.assistance_reports += 1;
                self.assistance = *assistance;
                cx.notify();
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
}

#[derive(Clone)]
pub struct OnlineHandle(pub Entity<OnlineState>);

impl Global for OnlineHandle {}

pub fn handle(cx: &App) -> Entity<OnlineState> {
    cx.global::<OnlineHandle>().0.clone()
}

pub use crate::accounts::set_active_account;

/// Tells the board and the pages (`kchess_board::live_game`) whether engine help may show and
/// whether a game holds the connection.
fn tell_live_game(state: &Entity<OnlineState>, cx: &mut App) {
    let state = state.read(cx);
    let (allowed, busy) = (state.assistance_allowed(), state.busy_with_game());
    kchess_board::live_game::handle(cx).update(cx, |live, cx| live.set(allowed, busy, cx));
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
            ongoing_loading: Loading::default(),
            failure: None,
            failed_state: None,
            lost: None,
            alerted: None,
            busy_challenge: String::new(),
            turns: std::collections::HashMap::new(),
            _ticker: None,
            ticked: Faces::default(),
            _presence: None,
            _ongoing_poll: None,
            resumes: 0,
            assistance: Assistance::Unverified,
            assistance_reports: 0,
            recheck_after: RECHECK_FIRST,
            _recheck: None,
        };
        state.read_assistance(cx);
        state.transition(Model::checking, None, cx);
        state
    });
    cx.set_global(OnlineHandle(state.clone()));
    // What the board and the pages know of the game follows it.
    kchess_board::live_game::init(cx);
    tell_live_game(&state, cx);
    cx.observe(&state, |state, cx| tell_live_game(&state, cx))
        .detach();
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
                if matches!(
                    state.model.online_phase,
                    GamePhase::Playing | GamePhase::Seeking | GamePhase::Disconnected
                ) {
                    state.phase = Phase::Checking;
                    state.transition(Model::checking, None, cx);
                }
                state.read_assistance(cx);
            });
            resume(state.clone(), cx);
        }
    })
    .detach();
    // Back on a network: a check for a game that failed is tried again at once.
    if let Some(network) = cx.try_global::<crate::network::NetworkHandle>() {
        cx.observe(&network.0.clone(), |network, cx| {
            if network.read(cx).online {
                check_again(cx);
            }
        })
        .detach();
    }
    resume(state, cx);
}

/// An account was connected: its game and its event stream, as at startup. A game on the board
/// stays; the connection follows it.
pub fn account_connected(cx: &mut App) {
    let state = handle(cx);
    let phase = state.read(cx).model.online_phase;
    if !matches!(phase, GamePhase::Playing | GamePhase::Seeking) {
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
            core.request(calls::StayConnected {
                account: OptionalAccount(String::new()),
            })
            .await
            .warn_on_err();
        }
        core.request(calls::ResumeOnline {}).await
    };
    cx.spawn(async move |cx| {
        let resumed = resume.await;
        state.update(cx, |state, cx| {
            if state.resumes != attempt {
                return;
            }
            if resumed.is_ok() {
                state.recheck_after = RECHECK_FIRST;
                state._recheck = None;
            }
            match resumed {
                Ok(Some(game)) => {
                    // The game may belong to any connected account; it becomes the active one.
                    set_active_account(&game.account, cx);
                    state.model.online_account = game.account;
                    state.model.online_id = game.id;
                    if state.model.online_phase != GamePhase::Playing {
                        state.model.online_phase = GamePhase::Disconnected;
                    }
                }
                Ok(_) => state.model.online_phase = GamePhase::Idle,
                Err(cause) => {
                    log(&format!("resuming online game failed: {}", cause.message));
                    if let Some(connection) = state.model.online_connection.as_mut()
                        && connection.phase == OnlinePhase::Checking
                    {
                        connection.phase = OnlinePhase::Disconnected;
                    }
                    if state.model.online_phase != GamePhase::Playing {
                        state.model.online_phase = GamePhase::Idle;
                    }
                    // The core keeps help locked until a check answers: nothing else asks again.
                    state.schedule_recheck(cx);
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

/// Whether a correspondence game just became the player's move: it was the opponent's when last
/// seen, and it is not the game being played in the window now.
fn is_your_move_now(game: &OngoingGame, before: Option<bool>, playing: &str) -> bool {
    game.correspondence() && game.is_my_turn && before == Some(false) && game.game_id != playing
}

/// What the rules read of a call's answer.
fn reply<T: 'static>(
    call: impl Future<Output = Result<T, CoreError>> + 'static,
    read: impl FnOnce(T) -> Reply + 'static,
) -> futures::future::LocalBoxFuture<'static, Result<Reply, CoreError>> {
    call.map(|answer| answer.map(read)).boxed_local()
}

#[cfg(test)]
mod tests;
