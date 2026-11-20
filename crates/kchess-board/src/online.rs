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
use kchess_contracts::types::{
    ChallengeInfo, ChallengeTimeControl, ChatRoom, DeclineReason, LobbyState, NotificationKind,
    OngoingGame, PresenceReport,
};
use kchess_contracts::types::{OnlineAction, OnlineOptions};
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

/// What the live game asks of the app around it: the system's notifications and the Online
/// page. The app sets them as it starts (`cx.set_global(Hooks { .. })`); until then, and in a
/// crate's own tests, nothing is shown.
#[derive(Clone, Copy)]
pub struct Hooks {
    /// A notice of `kind`, as a system notification when the window is not in front.
    pub notify: fn(NotificationKind, &str, &str, &mut App),
    /// It is your move in a game in progress elsewhere.
    pub your_move: fn(&OngoingGame, &mut App),
    /// A challenge came in, by its id, title and line.
    pub challenge: fn(&str, &str, &str, &mut App),
    /// The connection to a live game was lost: the Online page asks what to do.
    pub connection_lost: fn(&mut gpui_kit::Window, &mut App),
    /// A game started: show it on the Online page.
    pub show_game: fn(&mut gpui_kit::Window, &mut App),
}

impl Global for Hooks {}

fn hooks(cx: &App) -> Hooks {
    cx.try_global::<Hooks>().copied().unwrap_or(Hooks {
        notify: |_, _, _, _| {},
        your_move: |_, _| {},
        challenge: |_, _, _, _| {},
        connection_lost: |_, _| {},
        show_game: |_, _| {},
    })
}

/// A challenge's clock as its card and notice name it: "5+3", "2 days per move", "No clock".
pub fn describe_control(control: &ChallengeTimeControl) -> String {
    match control {
        ChallengeTimeControl::Clock { limit, increment } => crate::widgets::time_control(
            limit.as_f64().unwrap_or_default() / 60.,
            increment.as_f64().unwrap_or_default(),
        ),
        ChallengeTimeControl::Correspondence { days } => {
            let days = days.as_f64().unwrap_or_default();
            format!(
                "{days} {} per move",
                if days == 1. { "day" } else { "days" }
            )
        }
        ChallengeTimeControl::Unlimited => "No clock".into(),
    }
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

/// Why engine help is held back now (`OnlineState::pause`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pause {
    /// Lichess is being asked for a game in progress.
    Checking,
    /// Lichess could not be asked, or did not answer: offered again (`check_again`).
    Unverified,
    /// A live game is being played, looked for, or not yet seen to end.
    LiveGame,
}

impl Pause {
    pub fn text(self) -> &'static str {
        match self {
            Pause::Checking => "Checking Lichess for a game in progress…",
            Pause::Unverified => "Couldn't check Lichess for a game in progress",
            Pause::LiveGame => "Paused during your Lichess game",
        }
    }
}

/// The first wait before a failed check for a game in progress is tried again, and the longest.
const RECHECK_FIRST: Duration = Duration::from_secs(15);
const RECHECK_MAX: Duration = Duration::from_secs(5 * 60);

impl OnlineState {
    /// The core session whose reports the board follows.
    #[cfg(any(test, feature = "test-support"))]
    pub fn state_epoch(&self) -> f64 {
        self.model.state_epoch
    }

    /// The engine may help: no live game, none still being looked for, and the core lends it.
    /// (The app's phase alone took a failed check for a game as all clear, while the core kept
    /// refusing every engine call.)
    pub fn assistance_allowed(&self) -> bool {
        matches!(self.phase, Phase::Idle | Phase::Finished)
            && self.assistance == Assistance::Allowed
    }

    /// Why engine help is held back, if it is.
    pub fn pause(&self) -> Option<Pause> {
        if self.assistance_allowed() {
            return None;
        }
        let checking = self.phase == Phase::Checking
            || self
                .model
                .online_connection
                .as_ref()
                .is_some_and(OnlineConnection::checking);
        Some(if checking {
            Pause::Checking
        } else if matches!(self.phase, Phase::Idle | Phase::Finished)
            && self.assistance == Assistance::Unverified
        {
            Pause::Unverified
        } else {
            Pause::LiveGame
        })
    }

    /// Ask the core what help it lends now, unless it reports a change first.
    fn read_assistance(&mut self, cx: &mut Context<Self>) {
        let seen = self.assistance_reports;
        let status = cx.global::<AppCore>().request(calls::AssistanceStatus {});
        cx.spawn(async move |this, cx| match status.await {
            Ok(assistance) => {
                this.update(cx, |state, cx| {
                    if state.assistance_reports == seen {
                        state.assistance = assistance;
                        cx.notify();
                    }
                })
                .ok();
            }
            Err(cause) => log(&format!("reading engine help failed: {}", cause.message)),
        })
        .detach();
    }

    /// A check for a game in progress failed: try again later, longer each time. A refused login
    /// waits for the account to be connected again, which checks again itself.
    fn schedule_recheck(&mut self, cx: &mut Context<Self>) {
        let refused = self
            .model
            .online_connection
            .as_ref()
            .is_some_and(|connection| connection.phase == OnlinePhase::AuthRequired);
        if refused {
            self._recheck = None;
            return;
        }
        let wait = self.recheck_after;
        self.recheck_after = (wait * 2).min(RECHECK_MAX);
        self._recheck = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(wait).await;
            cx.update(check_again);
        }));
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
                let _ = window.update(cx, |_, window, cx| (hooks(cx).connection_lost)(window, cx));
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
    #[cfg(any(test, feature = "test-support"))]
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
                (hooks(cx).notify)(*kind, title, body, cx);
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
        form.color = "random".into();
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
            color: "random".into(),
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
                (hooks(cx).notify)(
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
        if crate::accounts::handle(cx).read(cx).connected().is_empty() {
            self.ongoing_loading.stop();
            self.ongoing.clear();
            cx.notify();
            return;
        }
        let turn = self.ongoing_loading.start();
        let call = cx.global::<AppCore>().request(calls::OngoingGames {});
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            let _ = this.update(cx, |this, cx| {
                if !this.ongoing_loading.settle(turn, "online-game", &answer) {
                    return;
                }
                if let Ok(games) = answer {
                    for game in &games {
                        let id = game.game_id.clone();
                        let mine = game.is_my_turn;
                        let before = this.turns.get(&id).copied();
                        if is_your_move_now(game, before, this.id()) {
                            (hooks(cx).your_move)(game, cx);
                        }
                        this.turns.insert(id, mine);
                    }
                    this.ongoing = games;
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
                        notice(cx, kchess_app::loading::failure("online-game", &cause));
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
                            let _ = window
                                .update(cx, |_, window, cx| (hooks(cx).show_game)(window, cx));
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
                (hooks(cx).challenge)(&id, &title, &body, cx);
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

pub use crate::accounts::set_active_account;

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

/// Ask Lichess again for a game in progress after a check that failed (its Check again button,
/// the retry timer, the network's return): the core lends engine help only once one answers.
pub fn check_again(cx: &mut App) {
    let Some(state) = cx
        .try_global::<OnlineHandle>()
        .map(|handle| handle.0.clone())
    else {
        return;
    };
    let failed = state.update(cx, |state, cx| {
        if state.pause() != Some(Pause::Unverified) {
            return false;
        }
        state._recheck = None;
        state.transition(Model::checking, None, cx);
        true
    });
    if failed {
        resume(state, cx);
    }
}

/// The Check again link shown where a failed check holds engine help back.
pub fn check_again_button(id: &'static str) -> gpui_kit::component::button::Button {
    use gpui_kit::component::Sizable as _;
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    Button::new(id)
        .link()
        .xsmall()
        .label("Check again")
        .on_click(|_, _, cx| check_again(cx))
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
    let control = describe_control(&challenge.time_control);
    (title, format!("{control} · {variant} · {rated}"))
}

#[cfg(test)]
mod tests {
    use super::describe_control;
    use crate::testing::Started;
    use kchess_contracts::types::ChallengeTimeControl;
    use serde_json::Number;

    fn clock(limit: u64, increment: u64) -> ChallengeTimeControl {
        ChallengeTimeControl::Clock {
            limit: Number::from(limit),
            increment: Number::from(increment),
        }
    }

    #[test]
    fn controls_read_like_lichess() {
        assert_eq!(describe_control(&clock(300, 3)), "5+3");
        assert_eq!(describe_control(&clock(90, 0)), "1.5+0");
        let days = ChallengeTimeControl::Correspondence {
            days: Number::from(1),
        };
        assert_eq!(describe_control(&days), "1 day per move");
        assert_eq!(
            describe_control(&ChallengeTimeControl::Unlimited),
            "No clock"
        );
    }

    thread_local! {
        static ASKED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn asked(what: String) {
        ASKED.with(|asked| asked.borrow_mut().push(what));
    }

    /// Hooks that keep what the live game asked of the app.
    fn keeping() -> super::Hooks {
        super::Hooks {
            notify: |kind, title, body, _| asked(format!("notify {kind:?}: {title} / {body}")),
            your_move: |game, _| asked(format!("your move {}", game.game_id)),
            challenge: |id, title, body, _| asked(format!("challenge {id}: {title} / {body}")),
            connection_lost: |_, _| asked("connection lost".into()),
            show_game: |_, _| asked("show game".into()),
        }
    }

    /// What the live game asks of the app reaches it through the hooks the app set: a notice, a
    /// challenge that came in, a game that started (told and shown) and its connection lost for
    /// good.
    /// A resume is counted as it starts, so an answer to an earlier one is known for stale.
    #[gpui_kit::test]
    fn each_resume_is_counted_as_it_starts(cx: &mut gpui_kit::TestAppContext) {
        let mut app = Started::new(cx);
        let online = app.with_window(|_, cx| super::handle(cx));
        let before = app.with_window(|_, cx| online.read(cx).resumes);
        app.with_window(|_, cx| super::resume_quietly(online.clone(), true, cx));
        app.with_window(|_, cx| super::resume(online.clone(), cx));
        assert_eq!(app.with_window(|_, cx| online.read(cx).resumes), before + 2);
    }

    #[gpui_kit::test]
    fn the_live_game_asks_the_app_through_its_hooks(cx: &mut gpui_kit::TestAppContext) {
        use kchess_contracts::events::CoreEvent;
        use serde_json::json;
        let mut app = Started::new(cx);
        app.with_window(|_, cx| cx.set_global(keeping()));
        let online = app.with_window(|_, cx| super::handle(cx));
        let challenge = serde_json::from_value(serde_json::json!({
            "id": "c1", "account": "me", "direction": "in", "opponent": { "name": "Bob" },
            "variant": "standard", "variantName": "Standard", "rated": false, "speed": "blitz",
            "timeControl": { "type": "unlimited" }, "color": "random", "playable": true,
            "receivedAt": 0,
        }))
        .expect("a challenge");
        app.with_window(|_, cx| {
            online.update(cx, |online, cx| {
                let notice = super::Effect::Notify {
                    kind: kchess_contracts::types::NotificationKind::GameEvents,
                    title: "Game over".into(),
                    body: "You won".into(),
                };
                online.run(&notice, None, cx);
                online.receive(&CoreEvent::ChallengeReceived(Box::new(challenge)), cx);
                let start = json!({ "type": "gameStart", "game": { "gameId": "g1" } });
                online.receive(&CoreEvent::OnlineEvent(start), cx);
            })
        });
        app.cx().run_until_parked();
        let session = app.with_window(|_, cx| online.read(cx).state_epoch());
        let lost = serde_json::from_value(serde_json::json!({
            "session": session, "account": "me", "gameId": "g1", "lane": "game",
            "phase": "disconnected",
        }))
        .expect("a connection report");
        app.with_window(|_, cx| {
            online.update(cx, |online, cx| {
                online.receive(&CoreEvent::OnlineState(lost), cx)
            })
        });
        app.cx().run_until_parked();
        let challenges = app.with_window(|_, cx| online.read(cx).challenges.len());
        assert_eq!(challenges, 1);
        assert_eq!(
            ASKED.with(|asked| asked.take()),
            [
                "notify GameEvents: Game over / You won",
                "challenge c1: Bob challenges you / No clock · Standard · Casual",
                "notify GameEvents: Game started / You are playing Opponent as white.",
                "show game",
                "connection lost",
            ]
        );
    }

    /// A game being played redraws its clocks as they tick, and nothing else does.
    #[gpui_kit::test]
    fn a_game_being_played_redraws_as_its_clocks_tick(cx: &mut gpui_kit::TestAppContext) {
        use kchess_domain::online_game::model::GamePhase;
        let mut app = Started::new(cx);
        let online = app.with_window(|_, cx| super::handle(cx));
        let redraws = std::rc::Rc::new(std::cell::Cell::new(0));
        let _watch = app.with_window(|_, cx| {
            let redraws = redraws.clone();
            cx.observe(&online, move |_, _| redraws.set(redraws.get() + 1))
        });
        app.with_window(|_, cx| online.update(cx, |online, cx| online.tick(cx)));
        app.cx().run_until_parked();
        let idle = redraws.get();
        app.with_window(|_, cx| {
            online.update(cx, |online, cx| online.set_phase(GamePhase::Playing, cx))
        });
        app.cx().run_until_parked();
        let before = redraws.get();
        app.with_window(|_, cx| online.update(cx, |online, cx| online.tick(cx)));
        app.cx().run_until_parked();
        assert_eq!((idle, redraws.get()), (0, before + 1));
    }

    /// Before the app sets its hooks the live game shows nothing, and does not fail.
    #[gpui_kit::test]
    fn without_the_apps_hooks_nothing_is_shown(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            let hooks = super::hooks(cx);
            (hooks.notify)(
                kchess_contracts::types::NotificationKind::LowTime,
                "t",
                "b",
                cx,
            );
            (hooks.challenge)("c", "t", "b", cx);
        });
    }

    /// How long the core may take to answer a status read, which it answers at once: a read
    /// that never lands fails the test inside the 20 s `cargo mutants` gives a test, which would
    /// otherwise report a hang instead of a caught mutant.
    const ANSWER: std::time::Duration = std::time::Duration::from_secs(10);

    /// The board's online state once its startup check has ended and the core lends help.
    fn settled(app: &mut Started) -> gpui_kit::Entity<super::OnlineState> {
        app.wait_for_engine_help();
        app.with_window(|_, cx| super::handle(cx))
    }

    /// A connection report in `phase`, for the state to hold.
    fn connection(
        lane: &str,
        phase: &str,
    ) -> kchess_domain::online_game::connection::OnlineConnection {
        serde_json::from_value(serde_json::json!({
            "session": 1, "account": "me", "gameId": "", "lane": lane, "phase": phase,
        }))
        .expect("a connection report")
    }

    #[test]
    fn each_pause_says_why() {
        use super::Pause;
        assert_eq!(
            Pause::Checking.text(),
            "Checking Lichess for a game in progress…"
        );
        assert_eq!(
            Pause::Unverified.text(),
            "Couldn't check Lichess for a game in progress"
        );
        assert_eq!(Pause::LiveGame.text(), "Paused during your Lichess game");
    }

    /// Engine help needs both: no game of the account's on the board, and the core lending it. A
    /// check that failed is told apart from a check under way and from a live game, which is
    /// what decides whether Check again is offered.
    #[gpui_kit::test]
    fn engine_help_follows_both_the_board_and_the_core(cx: &mut gpui_kit::TestAppContext) {
        use super::{Pause, Phase};
        use kchess_contracts::types::Assistance;
        let mut app = Started::new(cx);
        let online = settled(&mut app);
        let all = [
            Phase::Checking,
            Phase::Idle,
            Phase::Seeking,
            Phase::Playing,
            Phase::Finished,
            Phase::Disconnected,
        ];
        let lent = [
            Assistance::Allowed,
            Assistance::Unverified,
            Assistance::LiveGame,
        ];
        let seen = app.with_window(|_, cx| {
            online.update(cx, |state, _| {
                let mut seen = Vec::new();
                for phase in all {
                    for assistance in lent {
                        state.phase = phase;
                        state.assistance = assistance;
                        seen.push((phase, assistance, state.assistance_allowed(), state.pause()));
                    }
                }
                // The core reporting its check of the game lane counts as checking, whatever the
                // board's phase.
                state.phase = Phase::Idle;
                state.assistance = Assistance::Unverified;
                state.model.online_connection = Some(connection("game", "checking"));
                let reported = state.pause();
                state.model.online_connection = Some(connection("events", "checking"));
                let other_lane = state.pause();
                state.model.online_connection = None;
                (seen, reported, other_lane)
            })
        });
        let (seen, reported, other_lane) = seen;
        let expected = |phase: Phase, assistance: Assistance| match (phase, assistance) {
            (Phase::Idle | Phase::Finished, Assistance::Allowed) => (true, None),
            (Phase::Checking, _) => (false, Some(Pause::Checking)),
            (Phase::Idle | Phase::Finished, Assistance::Unverified) => {
                (false, Some(Pause::Unverified))
            }
            _ => (false, Some(Pause::LiveGame)),
        };
        for (phase, assistance, allowed, pause) in seen {
            assert_eq!(
                (allowed, pause),
                expected(phase, assistance),
                "{phase:?} with the core's {assistance:?}"
            );
        }
        assert_eq!(reported, Some(Pause::Checking));
        assert_eq!(other_lane, Some(Pause::Unverified));
    }

    /// Each change the core reports is taken and counted, so a status read asked before it is
    /// known for stale; a read with nothing reported since sets the state to the core's.
    #[gpui_kit::test]
    fn the_cores_lock_is_followed(cx: &mut gpui_kit::TestAppContext) {
        use kchess_contracts::events::CoreEvent;
        use kchess_contracts::types::Assistance;
        let mut app = Started::new(cx);
        let online = settled(&mut app);
        let (before, after, taken) = app.with_window(|_, cx| {
            online.update(cx, |state, cx| {
                let before = state.assistance_reports;
                state.receive(&CoreEvent::Assistance(Assistance::LiveGame), cx);
                (before, state.assistance_reports, state.assistance)
            })
        });
        assert_eq!((after, taken), (before + 1, Assistance::LiveGame));
        // The core still lends help (nothing changed there): reading it corrects the state.
        app.with_window(|_, cx| online.update(cx, |state, cx| state.read_assistance(cx)));
        app.wait_within(ANSWER, "the core's answer", |_, cx| {
            online.read(cx).assistance == Assistance::Allowed
        });
    }

    /// A failed check is tried again later, twice as late each time up to five minutes; not
    /// while Lichess refuses the login, which connecting the account again checks itself.
    #[gpui_kit::test]
    fn a_failed_check_is_retried_later_each_time(cx: &mut gpui_kit::TestAppContext) {
        use std::time::Duration;
        let mut app = Started::new(cx);
        let online = settled(&mut app);
        let waits = app.with_window(|_, cx| {
            online.update(cx, |state, cx| {
                state.recheck_after = super::RECHECK_FIRST;
                state._recheck = None;
                state.schedule_recheck(cx);
                let first = (state.recheck_after, state._recheck.is_some());
                state.recheck_after = Duration::from_secs(4 * 60);
                state.schedule_recheck(cx);
                let capped = state.recheck_after;
                state.recheck_after = super::RECHECK_FIRST;
                state.model.online_connection = Some(connection("events", "auth-required"));
                state.schedule_recheck(cx);
                let refused = (state.recheck_after, state._recheck.is_some());
                state.model.online_connection = None;
                (first, capped, refused)
            })
        });
        assert_eq!(
            waits,
            (
                (Duration::from_secs(30), true),
                Duration::from_secs(5 * 60),
                (Duration::from_secs(15), false),
            )
        );
    }

    /// Check again asks Lichess once more after a failed check, and only then.
    #[gpui_kit::test]
    fn check_again_follows_only_a_failed_check(cx: &mut gpui_kit::TestAppContext) {
        use super::Phase;
        use kchess_contracts::types::Assistance;
        let mut app = Started::new(cx);
        let online = settled(&mut app);
        let asked = |app: &mut Started, assistance: Assistance| {
            app.with_window(|_, cx| {
                let before = online.update(cx, |state, _| {
                    state.model.online_connection = None;
                    state.phase = Phase::Idle;
                    state.assistance = assistance;
                    state.resumes
                });
                super::check_again(cx);
                let state = online.read(cx);
                (state.resumes - before, state.pause())
            })
        };
        assert_eq!(asked(&mut app, Assistance::Allowed), (0, None));
        assert_eq!(
            asked(&mut app, Assistance::Unverified),
            (1, Some(super::Pause::Checking))
        );
    }

    /// Signing out while the games in progress loaded kept that request current: its answer
    /// brought the account's games back, and an earlier failure stayed on the empty list.
    #[gpui_kit::test]
    fn games_in_progress_from_before_signing_out_do_not_come_back(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let mut app = Started::new(cx);
        let (stale, error) = app.with_window(|_, cx| {
            super::handle(cx).update(cx, |online, cx| {
                online.ongoing_loading.show_error("Lichess did not answer");
                let asked = online.ongoing_loading.current();
                // No account is connected in a fresh profile.
                online.refresh_ongoing(cx);
                (
                    online.ongoing_loading.is_current(asked),
                    online.ongoing_loading.error().cloned(),
                )
            })
        });
        assert!(!stale);
        assert_eq!(error, None);
    }

    fn ongoing(id: &str, speed: &str, my_turn: bool) -> kchess_contracts::types::OngoingGame {
        serde_json::from_value(serde_json::json!({
            "gameId": id,
            "account": "alice",
            "opponent": { "name": "bob" },
            "fen": "8/8/8/8/8/8/8/8 w - - 0 1",
            "isMyTurn": my_turn,
            "variant": "standard",
            "speed": speed,
            "rated": false,
        }))
        .expect("an ongoing game")
    }

    /// A correspondence game announces the player's move only when it just turned to them, in a
    /// game not being played in the window: each other case stays quiet.
    #[test]
    fn only_a_correspondence_game_that_just_turned_to_you_is_your_move() {
        let game = ongoing("abc", "correspondence", true);
        assert!(super::is_your_move_now(&game, Some(false), "other"));
        // Not correspondence.
        assert!(!super::is_your_move_now(
            &ongoing("abc", "blitz", true),
            Some(false),
            "other"
        ));
        // Still the opponent's turn.
        assert!(!super::is_your_move_now(
            &ongoing("abc", "correspondence", false),
            Some(false),
            "other"
        ));
        // Already the player's move when last seen, or never seen.
        assert!(!super::is_your_move_now(&game, Some(true), "other"));
        assert!(!super::is_your_move_now(&game, None, "other"));
        // The game open in the window needs no announcement.
        assert!(!super::is_your_move_now(&game, Some(false), "abc"));
    }

    /// Asking for the games in progress with an account connected marks the list as read once the
    /// current request's answer lands, failed or not.
    #[gpui_kit::test]
    fn the_current_answer_for_games_in_progress_lands(cx: &mut gpui_kit::TestAppContext) {
        let server = kchess_testkit::Server::start(|_, out| {
            kchess_testkit::respond(out, 500, "text/plain", b"")
        })
        .expect("a stand-in Lichess");
        let mut app = Started::with_lichess(cx, Some(server.base().to_string()));
        app.wait_for("the accounts read", |_, cx| {
            crate::accounts::handle(cx).read(cx).data.is_some()
        });
        app.with_window(|_, cx| {
            let settings = kchess_contracts::defaults::default_settings()
                .expect("the default settings")
                .core;
            crate::accounts::handle(cx).update(cx, |accounts, cx| {
                accounts.data = Some(kchess_contracts::types::AppData {
                    settings,
                    accounts: vec![kchess_contracts::types::Account {
                        username: "alice".into(),
                        connected: true,
                        last_synced_at: None,
                        synced_play_time: None,
                    }],
                    game_count: 0,
                });
                cx.notify();
            });
            super::handle(cx).update(cx, |online, cx| {
                assert!(!online.ongoing_loaded);
                online.refresh_ongoing(cx);
            });
        });
        app.wait_for("the answer to land", |_, cx| {
            super::handle(cx).read(cx).ongoing_loaded
        });
    }

    /// Starting a game asks the core at once: the state is busy until its answer comes.
    #[gpui_kit::test]
    fn starting_a_game_is_busy_until_the_core_answers(cx: &mut gpui_kit::TestAppContext) {
        let server = kchess_testkit::Server::start(|_, out| {
            kchess_testkit::respond(out, 500, "text/plain", b"")
        })
        .expect("a stand-in Lichess");
        let mut app = Started::with_lichess(cx, Some(server.base().to_string()));
        let busy = app.with_window(|_, cx| {
            super::handle(cx).update(cx, |online, cx| {
                online.start(None, cx);
                online.busy
            })
        });
        assert!(busy);
        app.wait_for("the start to end", |_, cx| !super::handle(cx).read(cx).busy);
    }
}
