//! Play with computer: a game against
//! Stockfish at a chosen level. The rules run the game (`game::computer`); this page answers their
//! reads, runs the engine searches the game asks for, keeps the clocks moving and saves the
//! unfinished game in the profile, to resume it.

use crate::pages::PageView;
use kchess_contracts::calls;
use kchess_contracts::check::Moves;
use kchess_contracts::types::EngineLevel;
use kchess_domain::games::archive::Snapshot;
use kchess_domain::training::clock::format_clock;
use kchess_domain::variant::{Setup, Variant};
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
use kchess_contracts::types::{HistoryKind, SessionDocument, SessionKind, Sessions};
use kchess_domain::games::archive::Identity;
use serde_json::json;
use shakmaty::Color;

use crate::assets::icon;
use crate::board::{BoardEvent, ChessBoard};
use crate::core::AppCore;
use crate::export::Exporter;
use crate::game::history::HistoryPlace;
use crate::game::played::PlayedGame;
use crate::game::session::{Resumes, SessionSaver};
use crate::game::surface::{self, Faces, MoveBox, Steps};
use crate::game::view;
use crate::play_layout::{Measured, PANEL_MAX, PANEL_MIN, PANEL_ZEN, PlayLayout};
use crate::played::PlayedHandle;
use crate::settings::SettingsHandle;
use crate::theme::{Palette, radii};
use crate::widgets::{self, Side, Tone};
use kchess_domain::games::Host;
use kchess_domain::games::computer::{self, ComputerGame, Effect, SearchRequest};
use options::{Chosen, Current, GameOptions};

mod draw;
pub mod options;
mod voice;

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
pub enum Action {
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

    fn snapshot(&mut self, _: &mut Context<Self>) -> SessionDocument {
        SessionDocument::Computer(self.game.session(self.host().now))
    }

    fn resume(&mut self, saved: Option<Sessions>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mut saved) = saved {
            if let Some(game) = saved.computer.take() {
                match ComputerGame::resume(Some(game.into()), &self.host()) {
                    Ok(game) => self.game = game,
                    Err(cause) => {
                        self.log_warn(&format!("The saved game could not be read: {cause}"))
                    }
                }
            }
            let identity = saved.take_identity(HistoryKind::Computer);
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
        let board = cx
            .new(|_| ChessBoard::new(view::board_state(game.setup(), &[], 0, game.color(), None)));
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
                SessionKind::Computer,
                &[HistoryKind::Computer.session()],
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
            voice: crate::voice::moves::VoiceMoves::new(crate::voice::VoiceSource::Computer, cx),
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

    /// The game keeps its place in Game history from `identity` (none: a place of its own).
    fn keep_history(&mut self, identity: Option<Identity>, cx: &mut Context<Self>) {
        let Some(source) = self.archive_source() else {
            return;
        };
        let has_play = !self.game.moves().is_empty();
        self.history = Some(HistoryPlace::new(
            HistoryKind::Computer,
            identity,
            true,
            source,
            has_play,
            cx,
        ));
    }

    /// The game as Game history keeps it; none, and a warning, when it cannot be described.
    fn archive_source(&self) -> Option<Snapshot> {
        match self.game.archive_snapshot() {
            Ok(archived) => Some(archived),
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
                            kchess_contracts::types::NotificationKind::ComputerMove,
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
        if let Some(uci) = self.moves.take(position.as_ref(), window, cx) {
            self.play(&uci, cx);
        }
    }

    /// The game as its Export saves it and the analysis board opens it: "You" against
    /// `engine`.
    fn played_game(&self, engine: &str) -> PlayedGame {
        let user = self.user_color();
        let (white, black) = computer::players(user, engine);
        let mut game = PlayedGame::new(
            self.game.setup().clone(),
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
        kchess_board::analysis::open(
            kchess_board::analysis::Open::Played { game, review: true },
            window,
            cx,
        );
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

    pub(crate) fn setup_variant(&self) -> Variant {
        self.game.setup().variant
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
        view::position_at(self.game.setup(), &self.moves(), ply)
    }

    fn sans(&self) -> Vec<String> {
        view::sans(self.game.setup(), &self.moves())
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
            return self.keep_history(None, cx);
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
            self.game.setup(),
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
            setup: self.game.setup().clone(),
            clock: json!(self.game.clock()),
            color: self.user_color(),
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
        options::open(options, window, cx);
    }

    /// Play Stockfish from a position (the board editor's, the analysis board's), as the side to
    /// move, at the level and clock chosen last.
    pub fn start_from(
        &mut self,
        setup: Setup,
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
                    color,
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
        let color = match kchess_games::draw_color(color) {
            Ok(color) => color,
            Err(cause) => return self.log_warn(&cause),
        };
        let level = level.to_string();
        SessionSaver::when_read(
            self,
            move |this, window, cx| {
                let chosen = Chosen {
                    setup: Setup::standard(),
                    clock: json!(this.game.clock()),
                    color,
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
        self.game.set_color(chosen.color);
        self.flipped = false;
        self.reset_archive(cx);
        let host = self.host();
        let setup = chosen.setup;
        let effects = serde_json::from_value(chosen.clock)
            .map_err(|error| format!("the chosen game: {error}"))
            .and_then(|clock| self.game.start(Some(setup), Some(clock), &host));
        self.perform(effects, cx);
        self.select_level(window, cx);
        self.changed(cx);
        // A variant may need the other engine.
        self.check_engine(cx);
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

impl PageView for ComputerPage {
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

    fn act(this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        this.update(cx, |page, cx| page.act(action, window, cx));
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
pub fn play_from(setup: Setup, window: &mut Window, cx: &mut App) {
    let color = kchess_domain::rules::fen_turn(&setup.fen);
    if !crate::game::playable(&setup) {
        crate::shell::open(crate::pages::Page::Computer, window, cx);
        return;
    }
    crate::shell::show::<ComputerPage>(window, cx, move |page, window, cx| {
        page.start_from(setup, color, window, cx)
    });
}
