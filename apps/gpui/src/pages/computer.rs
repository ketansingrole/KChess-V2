//! Play with computer: a game against
//! Stockfish at a chosen level. The rules run the game (`game::computer`); this page answers their
//! reads, runs the engine searches the game asks for, keeps the clocks moving and saves the
//! unfinished game in the profile, to resume it.

use kchess_contracts::calls;
use kchess_contracts::check::Moves;
use kchess_contracts::types::EngineLevel;
use kchess_domain::training::clock::format_clock;
use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::input::Undo;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    Disableable as _, IndexPath, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::Level;
use serde_json::{Value, json};
use shakmaty::Color;

use crate::assets::icon;
use crate::board::{BoardEvent, ChessBoard};
use crate::core::AppCore;
use crate::export::Exporter;
use crate::game::computer::{self, ComputerGame, Effect, Host, SearchRequest};
use crate::game::history::HistoryPlace;
use crate::game::played::PlayedGame;
use crate::game::session::{Resumes, SessionSaver, Sessions};
use crate::game::surface::{self, Faces, MoveBox, Steps};
use crate::game::view;
use crate::pages::computer_options::{Chosen, Current, GameOptions};
use crate::play_layout::{Measured, PANEL_MAX, PANEL_MIN, PANEL_ZEN, PlayLayout};
use crate::played::PlayedHandle;
use crate::settings::SettingsHandle;
use crate::theme::{Palette, radii};
use crate::widgets::{self, Side, Tone};

/// The engine as last checked.
#[derive(Clone, Copy, PartialEq)]
enum Engine {
    Checking,
    Ready,
    Missing,
}

/// Work a spoken command leaves for after the phrase is read.
type Deferred = Box<dyn FnOnce(&mut ComputerPage, &mut Window, &mut Context<ComputerPage>)>;

/// The game's actions Quick Pick offers on this page.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Action {
    TakeBack,
    Resign,
    NewGame,
    SwitchSides,
    Flip,
    CopyPgn,
    Review,
}

/// A question the page asks before it throws a game away.
#[derive(Clone, Copy, PartialEq)]
enum Question {
    NewGame,
    Resign,
    SwitchSides,
    /// A new game with the options the player chose.
    Options,
}

pub struct ComputerPage {
    game: ComputerGame,
    board: Entity<ChessBoard>,
    exporter: Entity<Exporter>,
    level: Entity<SelectState<Vec<SharedString>>>,
    /// The level ids in the order `level` lists them.
    levels: Vec<String>,
    moves: MoveBox,
    started: Instant,
    engine: Engine,
    /// No live Lichess game blocks the engine.
    allowed: bool,
    /// The unfinished game the profile keeps, read to resume it and saved as it changes.
    saver: SessionSaver<ComputerPage>,
    search: Option<Task<()>>,
    ticker: Option<Task<()>>,
    /// The player's low-time warning; the computer's clock runs low unannounced.
    low_time: crate::game::low_time::LowTime,
    faces: Faces<[Option<SharedString>; 2]>,
    error: Option<SharedString>,
    /// Options waiting for the player to confirm throwing away the game in progress.
    pending: Option<Chosen>,
    /// The game's place in Game history, once the saved game has been read.
    history: Option<HistoryPlace>,
    /// Spoken moves and commands, and the question voice can answer.
    voice: crate::voice::moves::VoiceMoves,
    asked: Option<Question>,
    /// The board turned (F) to face the computer's side.
    flipped: bool,
    layout: PlayLayout,
    /// The side panel, measured for its narrow layout.
    panel_area: Measured,
    focus: FocusHandle,
    wheel: view::Wheel,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for ComputerPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Steps for ComputerPage {
    fn steps(&self, _: &App) -> Option<(usize, usize)> {
        Some((self.ply(), self.game.moves().len()))
    }

    fn step_to(&mut self, ply: usize, _: &mut Window, cx: &mut Context<Self>) {
        self.show_ply(ply, cx);
    }

    fn wheel(&mut self) -> &mut view::Wheel {
        &mut self.wheel
    }
}

impl Resumes for ComputerPage {
    fn saver(&mut self) -> &mut SessionSaver<Self> {
        &mut self.saver
    }

    fn snapshot(&mut self, _: &mut Context<Self>) -> Value {
        json!(self.game.snapshot(self.host().now))
    }

    fn resume(&mut self, saved: Option<Sessions>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mut saved) = saved {
            let game = saved.take(computer::SESSION);
            if !game.is_null() {
                match ComputerGame::resume(Some(&game), &self.host()) {
                    Ok(game) => self.game = game,
                    Err(cause) => {
                        self.log_warn(&format!("The saved game could not be read: {cause}"))
                    }
                }
            }
            let identity = saved.take(&kchess_games::history::session_kind(computer::HISTORY));
            self.keep_history(identity, cx);
        }
        self.select_level(window, cx);
        self.after_change(cx);
        self.computer_turn(cx);
    }
}

impl ComputerPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> ComputerPage {
        let started = Instant::now();
        let host = Host {
            now: 0.,
            allowed: true,
            ready: false,
        };
        let game = match ComputerGame::resume(None, &host) {
            Ok(game) => game,
            Err(cause) => panic!("a new computer game always starts: {cause}"),
        };
        let board = cx.new(|_| {
            ChessBoard::new(view::board_state(
                &json!(game.setup()),
                &[],
                0,
                game.color(),
                None,
            ))
        });
        let (levels, labels) = level_choices(cx);
        let selected = levels.iter().position(|id| id == game.level());
        let level = cx.new(|cx| SelectState::new(labels, selected.map(IndexPath::new), window, cx));
        let (moves, typed) = MoveBox::new(Self::typed_move, window, cx);
        let voice = crate::voice::handle(cx);
        let subscriptions = vec![
            cx.subscribe_in(&board, window, |this, _, event: &BoardEvent, _, cx| {
                if let BoardEvent::Move(uci) = event {
                    this.play(uci, cx)
                }
            }),
            cx.subscribe_in(
                &voice,
                window,
                |this, _, event: &crate::voice::VoiceEvent, window, cx| {
                    this.voice_heard(event, window, cx)
                },
            ),
            cx.observe(&voice, |_, _, cx| cx.notify()),
            // A save that failed is said on the page until one succeeds.
            cx.observe(&cx.global::<PlayedHandle>().0.clone(), |_, _, cx| {
                cx.notify()
            }),
            cx.subscribe_in(
                &level,
                window,
                |this, state, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(Some(_)) = event else {
                        return;
                    };
                    if let Some(row) = state.read(cx).selected_index(cx).map(|index| index.row)
                        && let Some(id) = this.levels.get(row)
                    {
                        let id = id.clone();
                        this.game.set_level(&id);
                        this.changed(cx);
                    }
                },
            ),
            cx.observe(
                &cx.global::<crate::online::OnlineHandle>().0.clone(),
                |this, online, cx| {
                    let allowed = online.read(cx).assistance_allowed();
                    if allowed != this.allowed {
                        this.allowed = allowed;
                        this.availability(cx);
                    }
                },
            ),
            cx.subscribe_in(
                &cx.global::<SettingsHandle>().0.clone(),
                window,
                |this, _, _: &crate::settings::SettingsChanged, _, cx| this.after_change(cx),
            ),
            typed,
        ];
        let mut page = ComputerPage {
            game,
            board,
            level,
            levels,
            moves,
            started,
            engine: Engine::Checking,
            allowed: cx
                .global::<crate::online::OnlineHandle>()
                .0
                .read(cx)
                .assistance_allowed(),
            saver: SessionSaver::start(
                computer::SESSION,
                &[kchess_games::history::session_kind(computer::HISTORY)],
                window,
                cx,
            ),
            search: None,
            ticker: None,
            low_time: Default::default(),
            faces: Faces::default(),
            error: None,
            pending: None,
            history: None,
            voice: crate::voice::moves::VoiceMoves::new("computer", cx),
            asked: None,
            flipped: false,
            layout: PlayLayout::default(),
            panel_area: Measured::default(),
            exporter: cx.new(|_| Exporter::default()),
            focus: cx.focus_handle(),
            wheel: Default::default(),
            _subscriptions: subscriptions,
        };
        page.check_engine(cx);
        page
    }

    fn host(&self) -> Host {
        Host {
            now: self.started.elapsed().as_secs_f64() * 1000.,
            allowed: self.allowed,
            ready: self.engine == Engine::Ready,
        }
    }

    /// The game keeps its place in Game history from `identity` (null: a place of its own).
    fn keep_history(&mut self, identity: Value, cx: &mut Context<Self>) {
        let Some(source) = self.archive_source() else {
            return;
        };
        let has_play = !self.game.moves().is_empty();
        self.history = Some(HistoryPlace::new(
            computer::HISTORY,
            identity,
            true,
            source,
            has_play,
            cx,
        ));
    }

    /// The game as Game history keeps it; none, and a warning, when it cannot be described.
    fn archive_source(&self) -> Option<Value> {
        match self.game.archive_snapshot() {
            Ok(archived) => Some(json!(archived)),
            Err(cause) => {
                self.log_warn(&format!(
                    "The game could not be kept in Game history: {cause}"
                ));
                None
            }
        }
    }

    /// Engine help became allowed or forbidden (a live Lichess game started or ended).
    fn availability(&mut self, cx: &mut Context<Self>) {
        let host = self.host();
        let effects = self.game.availability_changed(&host);
        self.perform(Ok(effects), cx);
        self.computer_turn(cx);
        self.after_change(cx);
    }

    /// Look for the engine that plays this game's variant again: Stockfish, or Fairy-Stockfish
    /// for the variants it cannot play. The game resumes once it is there.
    fn check_engine(&mut self, cx: &mut Context<Self>) {
        self.engine = Engine::Checking;
        cx.notify();
        let status = cx.global::<AppCore>().request(calls::EngineStatus {
            variant: Some(self.game.setup().variant),
        });
        cx.spawn(async move |this, cx| {
            let ready = matches!(status.await, Ok(status) if status.ready);
            let _ = this.update(cx, |this, cx| {
                this.engine = if ready {
                    Engine::Ready
                } else {
                    Engine::Missing
                };
                this.availability(cx);
            });
        })
        .detach();
    }

    /// The player may move now: the saved game is restored and the engine ready.
    #[cfg(test)]
    pub(crate) fn takes_moves(&self) -> bool {
        self.interactive()
    }

    fn log_warn(&self, message: &str) {
        crate::log::write(Level::Warn, "computer-game", message);
    }

    /// Carry out what a transition asks, in order.
    fn perform(&mut self, effects: Result<Vec<Effect>, String>, cx: &mut Context<Self>) {
        let effects = match effects {
            Ok(effects) => effects,
            Err(cause) => {
                self.log_warn(&cause);
                return;
            }
        };
        for effect in effects {
            match effect {
                Effect::StopEngine => {
                    let stop = cx.global::<AppCore>().request(calls::StopEngine {});
                    cx.background_executor()
                        .spawn(async move {
                            if let Err(cause) = stop.await {
                                crate::log::write(
                                    Level::Warn,
                                    "computer-game",
                                    &format!("Could not stop engine: {}", cause.message),
                                );
                            }
                        })
                        .detach();
                }
                Effect::ComputerTurn => self.computer_turn(cx),
                Effect::Moved { san, computer } => {
                    crate::sound::move_sound(&san, cx);
                    // Settings decide whether this reaches the player, and how.
                    if computer {
                        crate::notifications::desktop(
                            "computerMove",
                            "Stockfish moved",
                            &format!("{san}. Your move."),
                            cx,
                        );
                    }
                }
                Effect::Flagged => crate::sound::play(crate::sound::Sound::LowTime, cx),
                Effect::Failed => {
                    self.check_engine(cx);
                }
                Effect::Cancelled => {}
            }
        }
    }

    /// Start the engine's search when it is the engine's turn.
    fn computer_turn(&mut self, cx: &mut Context<Self>) {
        // The saved game comes first; loading it takes the computer's turn.
        if !self.saver.loaded() {
            return;
        }
        let host = self.host();
        let request = match self.game.search_start(&host) {
            Ok(Some(request)) => request,
            Ok(None) => return,
            Err(cause) => return self.log_warn(&cause),
        };
        let Some(level) = EngineLevel::from_key(&request.level) else {
            return self.log_warn("Invalid engine level.");
        };
        let search = cx.global::<AppCore>().request(calls::BestMove {
            moves: Moves(request.moves.clone()),
            level,
            options: Some((&request.options).into()),
        });
        cx.notify();
        self.search = Some(cx.spawn(async move |this, cx| {
            let reply = search.await;
            let _ = this.update(cx, |this, cx| this.search_finished(&request, reply, cx));
        }));
    }

    fn search_finished(
        &mut self,
        request: &SearchRequest,
        reply: Result<String, kchess_contracts::CoreError>,
        cx: &mut Context<Self>,
    ) {
        let host = self.host();
        let effects = match reply {
            Ok(reply) => {
                self.error = None;
                self.game.search_reply(request, &reply, &host)
            }
            Err(cause) => {
                if !cause.aborted {
                    self.log_warn(&format!("engine operation failed: {}", cause.message));
                    self.error = Some(cause.message.into());
                }
                self.game.search_failed(request)
            }
        };
        self.perform(Ok(effects), cx);
        self.changed(cx);
    }

    fn play(&mut self, uci: &str, cx: &mut Context<Self>) {
        // Not before the saved game is restored, which would replace the move.
        if !self.saver.loaded() {
            return self.after_change(cx);
        }
        let host = self.host();
        let (played, effects) = self.game.play(uci, &host);
        self.perform(Ok(effects), cx);
        if !played {
            // Put the piece back where the position says it is.
            self.after_change(cx);
        }
        self.changed(cx);
    }

    /// A move typed as SAN or UCI.
    fn typed_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.interactive() {
            return;
        }
        let position = self.position_at(self.ply());
        let variant = self.setup_variant();
        match self.moves.take(&variant, position.as_ref(), window, cx) {
            Some(uci) => self.play(&uci, cx),
            None => cx.notify(),
        }
    }

    /// The game as its Export saves it and the analysis board opens it: "You" against
    /// `engine`.
    fn played_game(&self, engine: &str) -> PlayedGame {
        let user = self.user_color();
        let (white, black) = computer::players(user, engine);
        let mut game = PlayedGame::new(
            &json!(self.game.setup()),
            self.game.moves().to_vec(),
            self.orientation(),
            white,
            black,
        );
        game.ply = Some(self.ply());
        game.result = self.game.outcome();
        game.player = Some(user);
        game
    }

    /// The game an Export saves, named "You" and "Stockfish".
    fn game_export(&self) -> PlayedGame {
        self.played_game("Stockfish")
    }

    /// Review the game on the analysis board, opened at the move being looked at, with the
    /// level Stockfish played at.
    fn review_game(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let engine = format!(
            "Stockfish ({})",
            crate::game::engine_level_label(self.game.level())
        );
        let game = self.played_game(&engine);
        crate::pages::analysis::replace(window, cx, move |analysis, _, cx| {
            analysis.open_played(&game, true, cx)
        });
    }

    fn moves(&self) -> Vec<String> {
        self.game.moves().to_vec()
    }

    fn ply(&self) -> usize {
        self.game.ply()
    }

    /// Sound the low-time warning as the player's clock runs low.
    fn warn_low_time(&mut self, cx: &mut Context<Self>) {
        let host = self.host();
        if self.game.over() {
            return;
        }
        let color = self.user_color();
        let initial = self.game.clock().map_or(0., |clock| clock.minutes) * 60.;
        let remaining = self.game.remaining(color, host.now);
        self.low_time.warn(color, remaining, initial, cx);
    }

    fn user_color(&self) -> Color {
        self.game.color()
    }

    /// The side at the bottom of the board: the player's, unless turned.
    fn orientation(&self) -> Color {
        if self.flipped {
            !self.user_color()
        } else {
            self.user_color()
        }
    }

    pub(crate) fn setup_variant(&self) -> String {
        self.game.setup().variant.key().to_string()
    }

    /// The FEN the game started from.
    #[cfg(test)]
    pub(crate) fn setup_fen(&self) -> String {
        self.game.setup().fen.clone()
    }

    fn thinking(&self) -> bool {
        self.game.thinking()
    }

    /// The position after the first `ply` moves.
    fn position_at(&self, ply: usize) -> Option<shakmaty::variant::VariantPosition> {
        view::position_at(&json!(self.game.setup()), &self.moves(), ply)
    }

    fn sans(&self) -> Vec<String> {
        view::sans(&json!(self.game.setup()), &self.moves())
    }

    /// The player may move now: it is their turn at the latest position, and not before the
    /// saved game is restored, which would replace the move (and nothing is saved before then).
    fn interactive(&self) -> bool {
        self.saver.loaded()
            && self.engine == Engine::Ready
            && self.allowed
            && !self.game.over()
            && self.ply() == self.moves().len()
            && self
                .position_at(self.ply())
                .is_some_and(|position| shakmaty::Position::turn(&position) == self.user_color())
    }

    /// A move of the player's is on the board.
    fn can_takeback(&self) -> bool {
        let moves = self.moves().len();
        let user_to_move = self
            .position_at(moves)
            .is_some_and(|position| shakmaty::Position::turn(&position) == self.user_color());
        !self.game.flagged() && (moves >= 2 || (moves == 1 && !user_to_move))
    }

    fn in_progress(&self) -> bool {
        !self.moves().is_empty() && self.game.result().is_none()
    }

    /// The game changed: show it, keep the clocks running, save it shortly and keep its place in
    /// Game history up to date.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.after_change(cx);
        SessionSaver::changed(self, cx);
        let has_play = !self.moves().is_empty();
        if let Some(source) = self.archive_source()
            && let Some(history) = &mut self.history
        {
            history.save(false, source, has_play, cx);
        }
    }

    /// A new game starts: the one before keeps its place in history and this one gets its own.
    /// After a saved game that could not be read, this one is the player's to keep.
    fn reset_archive(&mut self, cx: &mut Context<Self>) {
        self.saver.open();
        if self.history.is_none() {
            return self.keep_history(Value::Null, cx);
        }
        let has_play = !self.moves().is_empty();
        if let Some(source) = self.archive_source()
            && let Some(history) = &mut self.history
        {
            history.reset(source, has_play, cx);
        }
    }

    fn after_change(&mut self, cx: &mut Context<Self>) {
        let interactive = self.interactive();
        let live = self.game.result().is_none();
        let mut state = view::board_state(
            &json!(self.game.setup()),
            &self.moves(),
            self.ply(),
            self.orientation(),
            interactive.then_some(self.user_color()),
        );
        // While Stockfish thinks, a move can be queued for the next turn.
        let premove =
            (!interactive && live && self.ply() == self.moves().len()).then_some(self.user_color());
        surface::board_options(&mut state, live, premove, cx);
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        self.sync_voice(cx);
        let running = self.game.timed() && !self.game.over();
        if running && self.ticker.is_none() {
            self.ticker = Some(crate::timer::every(crate::timer::TICK, cx, |this, cx| {
                let host = this.host();
                let effects = this.game.expire(host.now);
                let acted = !effects.is_empty();
                this.perform(Ok(effects), cx);
                this.warn_low_time(cx);
                let shown = [Color::White, Color::Black].map(|color| this.clock_text(color));
                if this.faces.changed(shown) || acted {
                    cx.notify();
                }
                if this.game.over() {
                    this.ticker = None;
                    this.changed(cx);
                    return false;
                }
                true
            }));
        } else if !running {
            self.ticker = None;
        }
        cx.notify();
    }

    /// Show the game's level in the picker (a resumed game, or one started from the options).
    fn select_level(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = self.levels.iter().position(|id| id == self.game.level());
        self.level.update(cx, |state, cx| {
            state.set_selected_index(row.map(IndexPath::new), window, cx)
        });
    }

    /// New game against Stockfish, from the options the player chose.
    fn open_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = Current {
            setup: json!(self.game.setup()),
            clock: json!(self.game.clock()),
            color: kchess_domain::color::name(self.user_color()).to_string(),
            level: self.game.level().to_string(),
        };
        let page = cx.entity().downgrade();
        let levels = level_choices(cx);
        let options = cx.new(|cx| {
            GameOptions::new(
                current,
                levels,
                move |chosen, window, cx| {
                    let _ = page.update(cx, |page, cx| page.start_chosen(chosen, window, cx));
                },
                window,
                cx,
            )
        });
        crate::pages::computer_options::open(options, window, cx);
    }

    /// Play Stockfish from a position (the board editor's, the analysis board's), as the side to
    /// move, at the level and clock chosen last.
    pub fn start_from(
        &mut self,
        setup: Value,
        color: Color,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        SessionSaver::when_read(
            self,
            move |this, window, cx| {
                let chosen = Chosen {
                    setup,
                    clock: json!(this.game.clock()),
                    color: kchess_domain::color::name(color),
                    level: this.game.level().to_string(),
                };
                this.start_chosen(chosen, window, cx);
            },
            window,
            cx,
        );
    }

    /// Play Stockfish at `level` as `color` (`white`, `black` or `random`) from the start, with the
    /// clock chosen last.
    pub fn play_level(
        &mut self,
        level: &str,
        color: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let color = match crate::game::draw_color(color) {
            Ok(color) => color,
            Err(cause) => return self.log_warn(&cause),
        };
        let level = level.to_string();
        SessionSaver::when_read(
            self,
            move |this, window, cx| {
                let chosen = Chosen {
                    setup: json!({ "variant": "standard", "fen": crate::game::STANDARD_FEN }),
                    clock: json!(this.game.clock()),
                    color: kchess_domain::color::name(color),
                    level,
                };
                this.start_chosen(chosen, window, cx);
            },
            window,
            cx,
        );
    }

    /// Like the page's own New game, a game in progress is only thrown away when confirmed.
    fn start_chosen(&mut self, chosen: Chosen, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_progress() {
            self.pending = Some(chosen);
            self.ask_now(Question::Options, window, cx);
        } else {
            self.begin(chosen, window, cx);
        }
    }

    fn begin(&mut self, chosen: Chosen, window: &mut Window, cx: &mut Context<Self>) {
        self.game.set_level(&chosen.level);
        self.game.set_color(if chosen.color == "black" {
            Color::Black
        } else {
            Color::White
        });
        self.flipped = false;
        self.reset_archive(cx);
        let host = self.host();
        let effects = serde_json::from_value(chosen.setup)
            .and_then(|setup| Ok((setup, serde_json::from_value(chosen.clock)?)))
            .map_err(|error| format!("the chosen game: {error}"))
            .and_then(|(setup, clock)| self.game.start(Some(setup), Some(clock), &host));
        self.perform(effects, cx);
        self.select_level(window, cx);
        self.changed(cx);
        // A variant may need the other engine.
        self.check_engine(cx);
    }
    /// The actions its buttons allow now.
    pub(crate) fn actions(&self) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.can_takeback() {
            actions.push(Action::TakeBack);
        }
        if self.in_progress() {
            actions.push(Action::Resign);
        }
        actions.extend([Action::NewGame, Action::SwitchSides, Action::Flip]);
        if !self.moves().is_empty() {
            actions.push(Action::CopyPgn);
        }
        if self.can_review() {
            actions.push(Action::Review);
        }
        actions
    }

    /// The finished game can be reviewed on the analysis board.
    fn can_review(&self) -> bool {
        self.game.result().is_some() && !self.moves().is_empty()
    }

    /// Run an action as its button does, asking first where the button asks.
    pub(crate) fn act(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            Action::TakeBack if self.can_takeback() => self.takeback(cx),
            Action::TakeBack => {}
            Action::Resign if self.in_progress() => self.ask(Question::Resign, window, cx),
            Action::Resign => {}
            Action::NewGame => self.ask(Question::NewGame, window, cx),
            Action::SwitchSides => self.ask(Question::SwitchSides, window, cx),
            Action::Flip => self.flip(cx),
            Action::CopyPgn => crate::export::copy_pgn(&self.game_export(), window, cx),
            Action::Review if self.can_review() => self.review_game(window, cx),
            Action::Review => {}
        }
    }

    fn new_game(&mut self, cx: &mut Context<Self>) {
        self.reset_archive(cx);
        let host = self.host();
        let effects = self.game.start(None, None, &host);
        self.perform(effects, cx);
        self.changed(cx);
        self.computer_turn(cx);
    }

    fn takeback(&mut self, cx: &mut Context<Self>) {
        let host = self.host();
        let effects = self.game.takeback(&host);
        self.perform(Ok(effects), cx);
        self.changed(cx);
    }

    fn resign(&mut self, cx: &mut Context<Self>) {
        let effects = self.game.resign();
        self.perform(Ok(effects), cx);
        self.changed(cx);
    }

    fn switch_sides(&mut self, cx: &mut Context<Self>) {
        let other = !self.user_color();
        self.game.set_color(other);
        self.flipped = false;
        self.new_game(cx);
    }

    /// Turn the board to look at the game from the other side; the sides stay as they are.
    fn flip(&mut self, cx: &mut Context<Self>) {
        self.flipped = !self.flipped;
        self.after_change(cx);
    }

    fn show_ply(&mut self, ply: usize, cx: &mut Context<Self>) {
        self.game.show_ply(ply);
        self.changed(cx);
    }

    /// Ask before a game in progress is thrown away; otherwise go ahead.
    fn ask(&mut self, question: Question, window: &mut Window, cx: &mut Context<Self>) {
        let needed = match question {
            Question::NewGame | Question::Options => self.in_progress(),
            Question::Resign => true,
            Question::SwitchSides => !self.moves().is_empty(),
        };
        if !needed {
            return self.answer(question, window, cx);
        }
        self.ask_now(question, window, cx);
    }

    fn ask_now(&mut self, question: Question, window: &mut Window, cx: &mut Context<Self>) {
        let other = if self.user_color() == Color::White {
            "White"
        } else {
            "Black"
        };
        let (title, description, label): (SharedString, SharedString, SharedString) = match question
        {
            Question::NewGame => (
                "Start a new game?".into(),
                "Your current game is still in progress and will be lost".into(),
                "New game".into(),
            ),
            Question::Options => (
                "Start a new game?".into(),
                "Your computer game is still in progress and will be lost".into(),
                "New game".into(),
            ),
            Question::Resign => (
                "Resign this game?".into(),
                "Stockfish wins. The game stays on the board so you can go through it.".into(),
                "Resign".into(),
            ),
            Question::SwitchSides => (
                format!(
                    "Play as {}?",
                    if other == "White" { "Black" } else { "White" }
                )
                .into(),
                if self.in_progress() {
                    "Your current game is still in progress. Switching sides starts a new game."
                        .into()
                } else {
                    "Switching sides starts a new game".into()
                },
                "New game".into(),
            ),
        };
        let page = cx.entity().downgrade();
        crate::widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
            let page = page.clone();
            dialog
                .title(title.clone())
                .description(description.clone())
                .ok_text(label.clone())
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok({
                    let page = page.clone();
                    move |_, window, cx| {
                        let _ = page.update(cx, |page, cx| {
                            page.asked = None;
                            page.answer(question, window, cx)
                        });
                        true
                    }
                })
                .on_cancel(move |_, _, cx| {
                    let _ = page.update(cx, |page, cx| {
                        page.asked = None;
                        cx.notify();
                    });
                    true
                })
        });
        self.asked = Some(question);
        self.sync_voice(cx);
    }

    /// Spoken moves are taken on your turn; commands whenever Stockfish is there to play.
    fn voice_taking(&self) -> bool {
        self.interactive() && !self.thinking() && self.asked.is_none()
    }

    fn voice_listening(&self) -> bool {
        self.voice_taking() || self.engine == Engine::Ready
    }

    fn sync_voice(&mut self, cx: &mut Context<Self>) {
        let context = format!(
            "{}:{}:{}",
            self.user_color() == Color::White,
            self.moves().join(" "),
            self.asked.is_some()
        );
        let listening = self.voice_listening();
        self.voice.sync(
            &context,
            Some(self.sans()),
            crate::voice::grammar(crate::voice::Grammar::Game),
            listening,
            cx,
        );
    }

    /// What voice input says while no move is taken.
    fn voice_idle(&self) -> &'static str {
        if self.game.result().is_some() {
            "The game is over. Say “new game” to play again."
        } else if self.ply() != self.moves().len() {
            "Go to the last move to keep playing"
        } else {
            "Wait for Stockfish, then say your move"
        }
    }

    fn voice_heard(
        &mut self,
        event: &crate::voice::VoiceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.voice.owns(event) {
            return;
        }
        let setup = self
            .position_at(self.moves().len())
            .map(|position| {
                json!({
                    "variant": self.game.setup().variant.key(),
                    "fen": kchess_domain::rules::make_fen(&position),
                })
            })
            .unwrap_or(Value::Null);
        let fen = setup["fen"].as_str().unwrap_or_default().to_string();
        let heard = match event {
            crate::voice::VoiceEvent::Missed { heard, .. } => {
                self.voice.missed(heard, &fen, self.voice_taking(), cx);
                return;
            }
            crate::voice::VoiceEvent::Heard { heard, .. } => heard.clone(),
        };
        let taking = self.voice_taking();
        let idle = self.voice_idle();
        let asked = self.asked;
        let mut run: Option<Deferred> = None;
        let action = {
            let command = |text: &str| -> Option<crate::voice::moves::Reply> {
                let reply = |said: &str, feedback: &str| crate::voice::moves::Reply {
                    said: said.into(),
                    feedback: feedback.into(),
                    understood: true,
                };
                if let Some(question) = asked {
                    let answer = crate::voice::moves::spoken_move(text, &setup);
                    return Some(match answer {
                        kchess_domain::voice::SpokenMove::Confirm => {
                            run = Some(Box::new(move |this, window, cx| {
                                this.asked = None;
                                window.close_dialog(cx);
                                this.answer(question, window, cx);
                            }));
                            reply("confirm", "Done")
                        }
                        kchess_domain::voice::SpokenMove::Cancel => {
                            run = Some(Box::new(|this, window, cx| {
                                this.asked = None;
                                window.close_dialog(cx);
                                cx.notify();
                            }));
                            reply("cancel", "Cancelled")
                        }
                        _ => crate::voice::moves::Reply {
                            said: "expected confirm or cancel".into(),
                            feedback: "Say “confirm” or “cancel”".into(),
                            understood: false,
                        },
                    });
                }
                let command = kchess_domain::voice::game_command(text)?;
                let (said, question, feedback) = match command {
                    "takeback" => {
                        run = Some(Box::new(|this, _, cx| {
                            if this.can_takeback() {
                                this.takeback(cx);
                            }
                        }));
                        ("take back", None, "Took back your last move")
                    }
                    "newGame" => (
                        "new game",
                        Some(Question::NewGame),
                        "Start a new game? Say “confirm” or “cancel”.",
                    ),
                    "resign" => (
                        "resign",
                        Some(Question::Resign),
                        "Resign this game? Say “confirm” or “cancel”.",
                    ),
                    "flip" => (
                        "switch sides",
                        Some(Question::SwitchSides),
                        "Switch sides and restart? Say “confirm” or “cancel”.",
                    ),
                    _ => return None,
                };
                if let Some(question) = question {
                    run = Some(Box::new(move |this, window, cx| {
                        this.ask(question, window, cx)
                    }));
                }
                Some(reply(said, feedback))
            };
            self.voice.hear(
                &heard,
                &setup,
                crate::voice::moves::Turn {
                    taking,
                    idle: Some(idle),
                    immediate: false,
                },
                command,
                cx,
            )
        };
        if let Some(run) = run {
            run(self, window, cx);
        }
        if let crate::voice::moves::Action::Play(uci) = action {
            self.play(&uci, cx);
        }
        cx.notify();
    }

    fn answer(&mut self, question: Question, window: &mut Window, cx: &mut Context<Self>) {
        match question {
            Question::NewGame => self.new_game(cx),
            Question::Resign => self.resign(cx),
            Question::SwitchSides => self.switch_sides(cx),
            Question::Options => {
                if let Some(chosen) = self.pending.take() {
                    self.begin(chosen, window, cx);
                }
            }
        }
    }

    /// The opening the moves on the board reached, as `(eco, name)`.
    fn opening(&self) -> Option<(SharedString, SharedString)> {
        let opening =
            crate::game::opening_at(&json!(self.game.setup()), &self.moves(), self.ply())?;
        Some((opening.eco.into(), opening.name.into()))
    }

    fn clock_text(&self, color: Color) -> Option<SharedString> {
        if !self.game.timed() {
            return None;
        }
        let remaining = self.game.remaining(color, self.host().now);
        Some(format_clock(remaining).into())
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.engine == Engine::Missing {
            let checking = self.engine == Engine::Checking;
            let standard = crate::game::standard_rules(&self.setup_variant());
            let banner = widgets::StatusBanner::new(
                Tone::Alert,
                icon("triangle-alert"),
                if standard {
                    "Stockfish not found"
                } else {
                    "Fairy-Stockfish not found"
                }
                .into(),
            );
            // Fairy-Stockfish comes with KChess; Settings cannot install it.
            let banner = if standard {
                banner.detail("Install or choose an engine in Settings to play".into())
            } else {
                banner
            };
            return banner
                .extra(
                    h_flex()
                        .gap(px(12.))
                        .child(
                            Button::new("check-engine")
                                .link()
                                .xsmall()
                                .label("Check again")
                                .loading(checking)
                                .on_click(cx.listener(|this, _, _, cx| this.check_engine(cx))),
                        )
                        .into_any_element(),
                )
                .into_any_element();
        }
        if let Some(result) = self.game.result() {
            let (tone, glyph) = match result.kind {
                computer::Kind::Win => (Tone::Win, icon("trophy")),
                computer::Kind::Loss => (Tone::Loss, icon("flag")),
                computer::Kind::Draw => (Tone::Plain, icon("handshake")),
            };
            return widgets::StatusBanner::new(tone, glyph, result.title.into())
                .announce()
                .detail(SharedString::from(result.detail))
                .when_some(
                    self.can_review().then(|| {
                        Button::new("review-game")
                            .outline()
                            .small()
                            .label("Review game")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.review_game(window, cx)),
                            )
                            .into_any_element()
                    }),
                    |banner, value| banner.extra(value),
                )
                .into_any_element();
        }
        if self.ply() != self.moves().len() {
            return widgets::StatusBanner::new(
                Tone::Plain,
                icon("history"),
                "Reviewing an earlier position".into(),
            )
            .extra(widgets::back_to_game(cx.listener(|this, _, _, cx| {
                let last = this.moves().len();
                this.show_ply(last, cx)
            })))
            .into_any_element();
        }
        if self.thinking() {
            return widgets::StatusBanner::turn("Stockfish is thinking…".into()).into_any_element();
        }
        let position = self.position_at(self.ply());
        let turn = position
            .as_ref()
            .map_or(Color::White, shakmaty::Position::turn);
        let check = position.as_ref().is_some_and(shakmaty::Position::is_check);
        widgets::StatusBanner::turn(if turn == self.user_color() {
            "Your move".into()
        } else {
            "Stockfish's move".into()
        })
        .when(check, |banner| banner.detail("Check".into()))
        .into_any_element()
    }
}

/// The levels the player keeps in Settings → Gameplay, with their labels.
fn level_choices(cx: &App) -> (Vec<String>, Vec<SharedString>) {
    let ids = cx
        .global::<SettingsHandle>()
        .0
        .read(cx)
        .settings
        .core
        .engine_levels
        .clone();
    let ids: Vec<String> = ids
        .into_iter()
        .map(|level| {
            serde_json::to_value(level)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        })
        .collect();
    let labels = ids
        .iter()
        .map(|id| match crate::game::engine_level_label(id) {
            label if label.is_empty() => id.clone().into(),
            label => SharedString::from(label),
        })
        .collect();
    (ids, labels)
}

impl Render for ComputerPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let board_size = self.layout.board_size(window);
        let narrow = widgets::is_narrow(self.panel_area.size().width);
        let user = self.user_color();
        let position = self.position_at(self.ply());
        let turn = position
            .as_ref()
            .map_or(Color::White, shakmaty::Position::turn);
        let live = self.game.result().is_none();
        let sans = self.sans();
        let moves = self.moves();
        let ply = self.ply();
        let interactive = self.interactive();
        let first = view::first_move(&json!(self.game.setup()));
        let in_progress = self.in_progress();
        let result = self.game.result();
        let side_of = |color: Color| {
            if color == Color::White {
                Side::White
            } else {
                Side::Black
            }
        };
        let fen = position
            .as_ref()
            .map(kchess_domain::rules::make_fen)
            .unwrap_or_default();
        let piece_set = crate::settings::frontend(cx).piece_set.clone();
        // A running clock says whose turn it is; only an untimed game needs the outline.
        let untimed = !self.game.timed();
        let computer_line = widgets::player_line(
            h_flex()
                .min_w_0()
                .gap(px(8.))
                .child(widgets::avatar(
                    icon("cpu"),
                    side_of(!user),
                    live && turn != user && untimed,
                    &palette,
                ))
                .child("Stockfish")
                .child(
                    h_flex()
                        .w(px(196.))
                        .min_w_0()
                        .pl(px(8.))
                        .gap(px(4.))
                        .rounded(radii::SM)
                        .bg(palette.elevated)
                        .text_color(palette.highlighted)
                        .font_weight(FontWeight::NORMAL)
                        .child(icon("gauge").size(px(14.)).text_color(palette.text_muted))
                        .child(
                            div().flex_1().min_w_0().child(
                                Select::new(&self.level)
                                    .small()
                                    .appearance(false)
                                    // Wider than its trigger: every level's name and rating
                                    // in full ("International Master · ~2400").
                                    .menu_width(px(260.))
                                    .accessibility_label("Difficulty"),
                            ),
                        ),
                )
                .children(widgets::material(
                    &fen,
                    user != Color::White,
                    &piece_set,
                    &palette,
                )),
            None,
            self.clock_text(!user).map(|text| {
                widgets::clock(text, live && turn != user && !untimed, &palette).into_any_element()
            }),
        );
        let entry = widgets::move_entry(&self.moves.input, interactive, &palette);
        let (voice, voice_line) = crate::voice::moves::board_bar(
            &self.voice,
            cx.entity().downgrade(),
            crate::voice::moves::Access {
                write: |this: &mut Self| &mut this.voice,
                play: |this: &mut Self, uci, _, cx| this.play(uci, cx),
            },
            crate::voice::moves::BoardBar {
                id: "computer-voice",
                hint: "Say “E four”, “Knight F three”, or “take back”",
                active: self.voice_taking(),
                listening: self.voice_listening(),
                grammar: crate::voice::grammar(crate::voice::Grammar::Game),
            },
            cx,
        );
        let you_line = widgets::player_line(
            h_flex()
                .gap(px(8.))
                .child(widgets::avatar(
                    icon("user"),
                    side_of(user),
                    live && turn == user && untimed,
                    &palette,
                ))
                .child("You")
                .children(widgets::material(
                    &fen,
                    user == Color::White,
                    &piece_set,
                    &palette,
                )),
            Some(widgets::move_row(entry, voice)),
            self.clock_text(user).map(|text| {
                widgets::clock(text, live && turn == user && !untimed, &palette).into_any_element()
            }),
        );
        // The lines stay with their sides when the board turns.
        let (top, bottom) = if self.flipped {
            (you_line, computer_line)
        } else {
            (computer_line, you_line)
        };
        let board_column = v_flex()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(top)
            .child(surface::wheel(
                div().child(crate::board::cached(&self.board)),
                cx,
            ))
            .child(bottom)
            .children(voice_line)
            .children(self.moves.message(&palette));

        let page = cx.entity().downgrade();
        let select = move |ply: usize, _: &mut Window, cx: &mut App| {
            let _ = page.update(cx, |page, cx| page.show_ply(ply, cx));
        };
        let other = if user == Color::White {
            "black"
        } else {
            "white"
        };
        let flip_page = cx.entity().downgrade();
        let flip: widgets::ClickHandler = Box::new(move |_, window, cx| {
            let _ = flip_page.update(cx, |page, cx| page.ask(Question::SwitchSides, window, cx));
        });
        let clock_label = self
            .game
            .clock()
            .map(|clock| widgets::time_control(clock.minutes, clock.increment));
        let frontend = crate::settings::frontend(cx);
        let zen = frontend.zen_mode;
        let show_opening = frontend.show_opening_name && self.setup_variant() == "standard";
        let setup_label = crate::game::setup_label(&json!(self.game.setup()));
        let opening = show_opening.then(|| self.opening()).flatten();
        let panel = widgets::side_panel(&palette)
            .relative()
            .child(self.panel_area.probe(cx))
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(if zen { PANEL_ZEN } else { PANEL_MAX })
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .children(
                        setup_label.map(|label| div().font_weight(FontWeight::MEDIUM).child(label)),
                    )
                    .children(clock_label)
                    .child(div().flex_1())
                    .child(
                        Button::new("game-options")
                            .ghost()
                            .small()
                            .icon(icon("settings-2"))
                            .label("Game options")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_options(window, cx)),
                            ),
                    )
                    .when(!zen, |row| {
                        row.child(crate::export::export_menu(
                            "computer-export",
                            &self.exporter,
                            self.game_export(),
                            Anchor::TopRight,
                            true,
                            cx,
                        ))
                    })
                    .child(widgets::view_toggles(cx)),
            )
            .child(self.render_banner(cx))
            .children(self.errors(cx).into_iter().map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .child(widgets::section_title(
                "Moves",
                opening.map(|(eco, name)| format!("{eco} {name}")),
                &palette,
            ))
            .child(div().flex_1().min_h(px(120.)).child(widgets::moves_list(
                "computer-moves",
                widgets::MoveSheet {
                    sans: &sans,
                    shown: ply,
                    first,
                },
                "No moves yet".into(),
                select.clone(),
                crate::notation::figurine(cx),
                &palette,
            )))
            .child(widgets::move_controls(
                ply,
                moves.len(),
                select,
                widgets::TurnButton::Named(format!("Switch sides (play {other})").into(), flip),
                cx,
            ))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .pt(px(12.))
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        Button::new("takeback")
                            .outline()
                            .flex_1()
                            .when(narrow, |button| button.min_w(relative(0.45)))
                            .icon(icon("undo-2"))
                            .label("Take back")
                            .disabled(!self.can_takeback())
                            .on_click(cx.listener(|this, _, _, cx| this.takeback(cx))),
                    )
                    .when(in_progress, |row| {
                        row.child(
                            Button::new("resign")
                                .outline()
                                .flex_1()
                                .when(narrow, |button| button.min_w(relative(0.45)))
                                .icon(icon("flag"))
                                .label("Resign")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.ask(Question::Resign, window, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("new-game")
                            .when(result.is_some(), |button| button.primary())
                            .when(result.is_none(), |button| button.outline())
                            .flex_1()
                            .when(narrow, |button| button.min_w(relative(0.45)))
                            .icon(icon("rotate-ccw"))
                            .label("New game")
                            .tooltip_with_action("New game", &crate::menu::New, None)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.ask(Question::NewGame, window, cx)
                            })),
                    ),
            );

        let page = self
            .layout
            .render("computer-page", board_column, panel, window, cx)
            // ⌘Z outside the move box takes back, as the button does.
            .on_action(
                cx.listener(|this, _: &Undo, window, cx| this.act(Action::TakeBack, window, cx)),
            );
        surface::keys(page, &self.focus, cx)
    }
}

impl ComputerPage {
    /// What went wrong that the player should know: the engine's failure, the saved game that
    /// could not be read, a save that failed.
    fn errors(&self, cx: &App) -> Vec<SharedString> {
        self.error
            .clone()
            .into_iter()
            .chain(self.saver.problems(cx))
            .collect()
    }
}

impl crate::pages::PageAction for Action {
    fn info(&self) -> crate::pages::ActionInfo {
        use crate::pages::{ActionInfo, ActionRole};
        match self {
            Action::TakeBack => ActionInfo::new("computer:takeback", "Take back", "undo-2", "undo"),
            Action::Resign => ActionInfo::new("computer:resign", "Resign", "flag", "give up"),
            Action::NewGame => {
                ActionInfo::new("computer:new-game", "New game", "rotate-ccw", "restart")
                    .role(ActionRole::New)
            }
            Action::SwitchSides => ActionInfo::new(
                "computer:switch-sides",
                "Switch sides",
                "repeat",
                "colour color",
            ),
            Action::Flip => ActionInfo::flip("computer:flip"),
            Action::CopyPgn => ActionInfo::new(
                "computer:copy-pgn",
                "Copy PGN",
                "copy",
                "game moves clipboard",
            )
            .role(ActionRole::Copy),
            Action::Review => ActionInfo::new(
                "computer:review",
                "Review game",
                "microscope",
                "analyse analysis",
            ),
        }
    }
}

impl crate::pages::PageView for ComputerPage {
    const INFO: crate::pages::PageInfo = crate::pages::PageInfo {
        page: crate::pages::Page::Computer,
        label: "Play with computer",
        short: "Computer",
        icon: "bot",
        group: Some(crate::pages::NavGroup::Play),
        keywords: "stockfish bot ai engine vs",
    };

    type Action = Action;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self {
        ComputerPage::new(window, cx)
    }

    fn zen(&self, _: &App) -> bool {
        true
    }

    fn actions(&self, _: &App) -> Vec<Action> {
        ComputerPage::actions(self)
    }

    fn act(this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        this.update(cx, |page, cx| page.act(action, window, cx));
    }
}

/// Whether a game can start from `setup` (`{ variant, fen }`): a standard position the engine
/// takes (not one with too many pieces), or a legal one in its variant.
pub(crate) fn playable(setup: &Value) -> bool {
    let variant = setup["variant"].as_str().unwrap_or("standard");
    let fen = setup["fen"].as_str().unwrap_or_default();
    if crate::game::standard_rules(variant) {
        crate::game::position_problem(fen).is_none()
    } else {
        crate::game::setup_position(variant, fen).is_some()
    }
}

/// Play Stockfish at a level and side, from Quick Pick.
pub fn play_level(level: &str, color: &'static str, window: &mut Window, cx: &mut App) {
    let level = level.to_string();
    crate::shell::show::<ComputerPage>(window, cx, move |page, window, cx| {
        page.play_level(&level, color, window, cx)
    });
}

/// Play Stockfish (Fairy-Stockfish in another variant) from a setup's position, as the side to
/// move. A position that cannot be played only opens the page.
pub fn play_from(setup: Value, window: &mut Window, cx: &mut App) {
    let color = crate::links::side_to_move(setup["fen"].as_str().unwrap_or_default());
    if !playable(&setup) {
        crate::shell::open(crate::pages::Page::Computer, window, cx);
        return;
    }
    crate::shell::show::<ComputerPage>(window, cx, move |page, window, cx| {
        page.start_from(setup, color, window, cx)
    });
}
