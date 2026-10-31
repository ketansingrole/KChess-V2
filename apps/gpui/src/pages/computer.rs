//! Play with computer: a game against
//! Stockfish at a chosen level. The rules run the game (`game::computer`); this page answers their
//! reads, runs the engine searches the game asks for, keeps the clocks moving and saves the
//! unfinished game in the profile, where the Electron app resumes it too.

use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    Disableable as _, IndexPath, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_core::Level;
use serde_json::{Value, json};
use shakmaty::{Color, Square};

use crate::assets::icon;
use crate::board::{BoardEvent, BoardState, ChessBoard};
use crate::core::AppCore;
use crate::export::{Exporter, GameExport};
use crate::game::computer::{ComputerGame, Effect, Host, SearchRequest};
use crate::game::history::HistoryPlace;
use crate::game::rules;
use crate::pages::computer_options::{Chosen, Current, GameOptions};
use crate::play_layout::{Measured, PANEL_MAX, PANEL_MIN, PANEL_ZEN, PlayLayout};
use crate::settings::SettingsHandle;
use crate::theme::{Palette, radii};
use crate::widgets::{self, Side, Tone};

/// Clocks are checked this often while one runs (`useIntervalFn(…, 200)`).
const TICK: Duration = Duration::from_millis(200);
/// Saves wait this long for more changes (`persistSession`).
const SAVE_DELAY: Duration = Duration::from_millis(150);

/// The engine as last checked (`engineStatus`).
#[derive(Clone, Copy, PartialEq)]
enum Engine {
    Checking,
    Ready,
    Missing,
}

/// Work a spoken command leaves for after the phrase is read.
type Deferred = Box<dyn FnOnce(&mut ComputerPage, &mut Window, &mut Context<ComputerPage>)>;

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
    move_input: Entity<InputState>,
    move_message: Option<SharedString>,
    started: Instant,
    engine: Engine,
    /// No live Lichess game blocks the engine (`assistanceAllowed`).
    allowed: bool,
    /// The unfinished game has been read from the profile; saving before it would replace it.
    loaded: bool,
    search: Option<Task<()>>,
    save: Option<Task<()>>,
    ticker: Option<Task<()>>,
    /// What the clocks showed at the last tick: a tick redraws only when a clock face changes.
    ticked: Option<[Option<SharedString>; 2]>,
    error: Option<SharedString>,
    /// Options waiting for the player to confirm throwing away the game in progress.
    pending: Option<Chosen>,
    /// The game's place in Game history (`useGameArchive`), once the library has loaded.
    history: Option<HistoryPlace>,
    /// Spoken moves and commands, and the question voice can answer (`dialogOpen`).
    voice: crate::voice::moves::VoiceMoves,
    asked: Option<Question>,
    layout: PlayLayout,
    /// The side panel, measured for `.panel-actions`' narrow rule.
    panel_area: Measured,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for ComputerPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
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
        let game = match ComputerGame::new(None, &host) {
            Ok(game) => game,
            Err(cause) => panic!("a new computer game always starts: {cause}"),
        };
        let board = cx.new(|_| ChessBoard::new(board_state(&game, false)));
        let (levels, labels) = level_choices(cx);
        let selected = levels
            .iter()
            .position(|id| Some(id.as_str()) == game.state["level"].as_str());
        let level = cx.new(|cx| SelectState::new(labels, selected.map(IndexPath::new), window, cx));
        let move_input = cx.new(|cx| InputState::new(window, cx).placeholder("Type a move"));
        let voice = crate::voice::handle(cx);
        let subscriptions = vec![
            cx.subscribe_in(&board, window, |this, _, event: &BoardEvent, window, cx| {
                if let BoardEvent::Move(uci) = event {
                    this.play(uci, window, cx)
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
                        this.game.state["level"] = json!(id);
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
            cx.subscribe_in(
                &move_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.typed_move(window, cx);
                    }
                },
            ),
        ];
        let mut page = ComputerPage {
            game,
            board,
            level,
            levels,
            move_input,
            move_message: None,
            started,
            engine: Engine::Checking,
            allowed: cx
                .global::<crate::online::OnlineHandle>()
                .0
                .read(cx)
                .assistance_allowed(),
            loaded: false,
            search: None,
            save: None,
            ticker: None,
            ticked: None,
            error: None,
            pending: None,
            history: None,
            voice: crate::voice::moves::VoiceMoves::new("computer", cx),
            asked: None,
            layout: PlayLayout::default(),
            panel_area: Measured::default(),
            exporter: cx.new(|_| Exporter::default()),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        page.load(window, cx);
        page.check_engine(cx);
        let weak = cx.entity().downgrade();
        crate::core::on_quit(cx, move |cx| {
            let page = weak.upgrade()?;
            let (session, history) = page.update(cx, |page, cx| {
                (
                    page.save_now(cx),
                    page.history.as_ref().map(|history| history.flushed(cx)),
                )
            });
            let pending: Vec<crate::core::BoxedSave> =
                [session, history].into_iter().flatten().collect();
            Some(crate::core::all_saved(pending))
        });
        page
    }

    fn host(&self) -> Host {
        Host {
            now: self.started.elapsed().as_secs_f64() * 1000.,
            allowed: self.allowed,
            ready: self.engine == Engine::Ready,
        }
    }

    /// Engine help became allowed or forbidden (a live Lichess game started or ended).
    fn availability(&mut self, cx: &mut Context<Self>) {
        let host = self.host();
        let effects = self.game.availability_changed(&host);
        self.perform(effects, cx);
        self.computer_turn(cx);
        self.after_change(cx);
    }

    fn core(cx: &App) -> AppCore {
        cx.global::<AppCore>().clone()
    }

    /// Resume the unfinished game the profile keeps (`sessions.computer`).
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let library = Self::core(cx).call("library", Vec::new());
        cx.spawn_in(window, async move |this, cx| {
            let library = library.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match library {
                    Ok(library) => {
                        let saved = &library["sessions"]["computer"];
                        if !saved.is_null() {
                            match ComputerGame::new(Some(saved), &this.host()) {
                                Ok(game) => this.game = game,
                                Err(cause) => this.log_warn(&format!(
                                    "The saved game could not be read: {cause}"
                                )),
                            }
                        }
                        let source = this.game.archive_snapshot(&this.host());
                        let has_play = !this.moves().is_empty();
                        this.history = Some(HistoryPlace::new(
                            "computer", &library, true, source, has_play, cx,
                        ));
                    }
                    Err(cause) => {
                        this.log_warn(&format!("Loading the library failed: {}", cause.message))
                    }
                }
                this.loaded = true;
                this.select_level(window, cx);
                this.after_change(cx);
                this.computer_turn(cx);
            });
        })
        .detach();
    }

    /// Look for the engine again (`recheckEngine`); the game resumes once it is there.
    fn check_engine(&mut self, cx: &mut Context<Self>) {
        self.engine = Engine::Checking;
        cx.notify();
        let status = Self::core(cx).call("engineStatus", Vec::new());
        cx.spawn(async move |this, cx| {
            let ready =
                matches!(status.await, Ok(status) if status["ready"].as_bool() == Some(true));
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
                    let stop = Self::core(cx).call("stopEngine", Vec::new());
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
            }
        }
    }

    /// Start the engine's search when it is the engine's turn (`computerTurn`).
    fn computer_turn(&mut self, cx: &mut Context<Self>) {
        let host = self.host();
        let request = match self.game.search_start(&host) {
            Ok(Some(request)) => request,
            Ok(None) => return,
            Err(cause) => return self.log_warn(&cause),
        };
        let search = Self::core(cx).call(
            "bestMove",
            vec![
                json!(request.moves),
                json!(request.level),
                request.options.clone(),
            ],
        );
        cx.notify();
        self.search = Some(cx.spawn(async move |this, cx| {
            let reply = search.await;
            let _ = this.update(cx, |this, cx| this.search_finished(&request, reply, cx));
        }));
    }

    fn search_finished(
        &mut self,
        request: &SearchRequest,
        reply: Result<Value, kchess_core::CoreError>,
        cx: &mut Context<Self>,
    ) {
        let host = self.host();
        let effects = match reply {
            Ok(reply) => {
                self.error = None;
                self.game
                    .search_reply(request, reply.as_str().unwrap_or_default(), &host)
            }
            Err(cause) => {
                if !cause.aborted {
                    self.log_warn(&format!("engine operation failed: {}", cause.message));
                    self.error = Some(cause.message.into());
                }
                self.game.search_failed(request, &host)
            }
        };
        self.perform(effects, cx);
        self.changed(cx);
    }

    fn play(&mut self, uci: &str, window: &mut Window, cx: &mut Context<Self>) {
        let host = self.host();
        match self.game.play(uci, &host) {
            Ok((played, effects)) => {
                self.perform(Ok(effects), cx);
                if !played {
                    // Put the piece back where the position says it is.
                    self.after_change(cx);
                }
            }
            Err(cause) => self.log_warn(&cause),
        }
        self.changed(cx);
        let _ = window;
    }

    /// A move typed as SAN or UCI (`keyboardMove`).
    fn typed_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.move_input.read(cx).value().trim().to_string();
        if text.is_empty() || !self.interactive() {
            return;
        }
        let position = self.position_at(self.ply());
        let uci = position.and_then(|position| {
            let pos = kchess_domain::rules::setup_start(
                &self.setup_variant(),
                &kchess_domain::rules::make_fen(&position),
            )?;
            let is_uci = text.len() >= 4
                && text.as_bytes()[0].is_ascii_lowercase()
                && text.as_bytes()[1].is_ascii_digit()
                && text.as_bytes()[2].is_ascii_lowercase()
                && text.as_bytes()[3].is_ascii_digit();
            if is_uci {
                kchess_domain::position::play_uci(&pos, &text).map(|_| text.to_lowercase())
            } else {
                kchess_domain::position::play_san(&pos, &text).map(|played| played.uci)
            }
        });
        match uci {
            Some(uci) => {
                self.move_message = None;
                self.move_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.play(&uci, window, cx);
            }
            None => {
                self.move_message = Some("Enter a legal move, such as Nf3 or g1f3".into());
                cx.notify();
            }
        }
    }

    /// Review the game on the analysis board, opened at the move being looked at.
    /// The game an Export saves, named "You" and "Stockfish" (`pgnResult` for its result).
    fn game_export(&self) -> GameExport {
        let user = self.user_color();
        let setup = &self.game.state["setup"];
        let result = self.game.result(&self.host()).map(|result| {
            match result["kind"].as_str() {
                Some("draw") => "1/2-1/2",
                kind => {
                    let winner = if kind == Some("win") { user } else { !user };
                    if winner == Color::White { "1-0" } else { "0-1" }
                }
            }
            .to_string()
        });
        let (white, black) = if user == Color::White {
            ("You", "Stockfish")
        } else {
            ("Stockfish", "You")
        };
        GameExport {
            variant: setup["variant"].as_str().unwrap_or("standard").to_string(),
            fen: setup["fen"].as_str().unwrap_or_default().to_string(),
            moves: self.moves(),
            orientation: user,
            white: white.into(),
            black: black.into(),
            ply: Some(self.ply()),
            result: Some(result.unwrap_or_else(|| "*".into())),
            lichess_id: None,
        }
    }

    fn review_game(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pgn = rules(
            "setupPgn",
            &[
                self.game.state["setup"].clone(),
                json!(self.moves()),
                json!({}),
            ],
        )
        .ok()
        .and_then(|pgn| pgn.as_str().map(str::to_string));
        let Some(pgn) = pgn else { return };
        let (ply, color) = (self.ply(), self.user_color());
        let computer = format!(
            "Stockfish ({})",
            rules("engineLevelLabel", &[self.game.state["level"].clone()])
                .ok()
                .and_then(|label| label.as_str().map(str::to_string))
                .unwrap_or_default()
        );
        let (white, black) = if color == Color::White {
            ("You".to_string(), computer)
        } else {
            (computer, "You".to_string())
        };
        let origin = json!({
            "white": white,
            "black": black,
            "player": if color == Color::White { "white" } else { "black" },
        });
        crate::shell::replace_analysis(window, cx, move |analysis, _, cx| {
            analysis.open_game(&pgn, ply, Some(color), Some(origin.clone()), true, cx)
        });
    }

    fn moves(&self) -> Vec<String> {
        serde_json::from_value(self.game.state["moves"].clone()).unwrap_or_default()
    }

    fn ply(&self) -> usize {
        crate::game::whole(&self.game.state["ply"]) as usize
    }

    fn user_color(&self) -> Color {
        if self.game.state["color"] == "black" {
            Color::Black
        } else {
            Color::White
        }
    }

    fn setup_variant(&self) -> String {
        self.game.state["setup"]["variant"]
            .as_str()
            .unwrap_or("standard")
            .to_string()
    }

    fn thinking(&self) -> bool {
        self.game.state["thinking"].as_bool() == Some(true)
    }

    /// The position after the first `ply` moves.
    fn position_at(&self, ply: usize) -> Option<shakmaty::variant::VariantPosition> {
        let setup = &self.game.state["setup"];
        let variant = setup["variant"].as_str().unwrap_or("standard");
        let fen = setup["fen"].as_str().unwrap_or_default();
        let moves = self.moves();
        let replay =
            kchess_domain::replay::replay_setup(variant, fen, &moves[..ply.min(moves.len())])?;
        kchess_domain::rules::setup_start(variant, &replay.fen)
    }

    fn sans(&self) -> Vec<String> {
        let setup = &self.game.state["setup"];
        kchess_domain::replay::replay_setup(
            setup["variant"].as_str().unwrap_or("standard"),
            setup["fen"].as_str().unwrap_or_default(),
            &self.moves(),
        )
        .map(|replay| replay.played.into_iter().map(|played| played.san).collect())
        .unwrap_or_default()
    }

    /// The player may move now: it is their turn at the latest position (`localInteractive`).
    fn interactive(&self) -> bool {
        let host = self.host();
        self.engine == Engine::Ready
            && self.allowed
            && !self.game.over(&host)
            && self.ply() == self.moves().len()
            && self
                .position_at(self.ply())
                .is_some_and(|position| shakmaty::Position::turn(&position) == self.user_color())
    }

    /// A move of the player's is on the board (`localCanTakeback`).
    fn can_takeback(&self) -> bool {
        let moves = self.moves().len();
        let user_to_move = self
            .position_at(moves)
            .is_some_and(|position| shakmaty::Position::turn(&position) == self.user_color());
        self.game.state["flagged"].is_null() && (moves >= 2 || (moves == 1 && !user_to_move))
    }

    fn in_progress(&self) -> bool {
        !self.moves().is_empty() && self.game.result(&self.host()).is_none()
    }

    /// The game changed: show it, keep the clocks running, save it shortly and keep its place in
    /// Game history up to date (`watch(source, () => save())`).
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.after_change(cx);
        self.schedule_save(cx);
        let (source, has_play) = (
            self.game.archive_snapshot(&self.host()),
            !self.moves().is_empty(),
        );
        if let Some(history) = &mut self.history {
            history.save(false, source, has_play, cx);
        }
    }

    /// A new game starts: the one before keeps its place in history and this one gets its own.
    fn reset_archive(&mut self, cx: &mut Context<Self>) {
        let (source, has_play) = (
            self.game.archive_snapshot(&self.host()),
            !self.moves().is_empty(),
        );
        if let Some(history) = &mut self.history {
            history.reset(source, has_play, cx);
        }
    }

    fn after_change(&mut self, cx: &mut Context<Self>) {
        let mut state = board_state_at(self, self.interactive());
        // Blindfold hides the pieces while the game is on (`settings.blindfold && !localResult`).
        state.blindfold = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .blindfold
            && self.game.result(&self.host()).is_none();
        // While Stockfish thinks, a move can be queued for the next turn (`premovable`).
        let premove = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .premove;
        if premove
            && !self.interactive()
            && self.game.result(&self.host()).is_none()
            && self.ply() == self.moves().len()
        {
            state.premove = Some(self.user_color());
        }
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        self.sync_voice(cx);
        let host = self.host();
        let running = !self.game.state["times"].is_null() && !self.game.over(&host);
        if running && self.ticker.is_none() {
            self.ticker = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(TICK).await;
                    let alive = this.update(cx, |this, cx| {
                        let host = this.host();
                        let effects = this.game.expire(&host);
                        let acted = effects.as_ref().is_ok_and(|effects| !effects.is_empty());
                        this.perform(effects, cx);
                        let shown = ["white", "black"].map(|color| this.clock_text(color));
                        if acted || this.ticked.as_ref() != Some(&shown) {
                            this.ticked = Some(shown);
                            cx.notify();
                        }
                        if this.game.over(&this.host()) {
                            this.ticker = None;
                            this.changed(cx);
                        }
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            }));
        } else if !running {
            self.ticker = None;
        }
        cx.notify();
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        if !self.loaded {
            return;
        }
        self.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let save = this.update(cx, |this, cx| this.save_now(cx)).ok().flatten();
            if let Some(save) = save {
                let _ = save.await;
            }
        }));
    }

    /// Save the game now (`saveSession('computer', …)`); `None` before it has loaded.
    fn save_now(&mut self, cx: &mut Context<Self>) -> Option<crate::core::BoxedSave> {
        if !self.loaded {
            return None;
        }
        self.save = None;
        let snapshot = self.game.snapshot(&self.host());
        let save = Self::core(cx).call("saveSession", vec![json!("computer"), snapshot]);
        let page = cx.entity().downgrade();
        let mut cx = cx.to_async();
        Some(Box::pin(async move {
            let result = save
                .await
                .map(|_| ())
                .map_err(|cause| format!("Automatic saving: {}", cause.message));
            let _ = page.update(&mut cx, |page, cx| {
                page.error = result.clone().err().map(Into::into);
                cx.notify();
            });
            result
        }))
    }

    /// Show the game's level in the picker (a resumed game, or one started from the options).
    fn select_level(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = self
            .levels
            .iter()
            .position(|id| Some(id.as_str()) == self.game.state["level"].as_str());
        self.level.update(cx, |state, cx| {
            state.set_selected_index(row.map(IndexPath::new), window, cx)
        });
    }

    /// New game against Stockfish, from the options the player chose.
    fn open_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = Current {
            setup: self.game.state["setup"].clone(),
            clock: self.game.state["clock"].clone(),
            color: self.game.state["color"]
                .as_str()
                .unwrap_or("white")
                .to_string(),
            level: self.game.state["level"]
                .as_str()
                .unwrap_or("club")
                .to_string(),
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
    /// move, at the level and clock chosen last (`startComputerFrom`).
    pub fn start_from(
        &mut self,
        setup: Value,
        color: Color,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chosen = Chosen {
            setup,
            clock: self.game.state["clock"].clone(),
            color: if color == Color::White {
                "white"
            } else {
                "black"
            },
            level: self.game.state["level"]
                .as_str()
                .unwrap_or("club")
                .to_string(),
        };
        self.start_chosen(chosen, window, cx);
    }

    /// Play Stockfish at `level` as `color` (`white`, `black` or `random`) from the start, with the
    /// clock chosen last (the palette's `startComputerGame`).
    pub fn play_level(
        &mut self,
        level: &str,
        color: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let color = match color {
            "random" if crate::game::random_unit() < 0.5 => "white",
            "random" => "black",
            "black" => "black",
            _ => "white",
        };
        let chosen = Chosen {
            setup: json!({
                "variant": "standard",
                "fen": "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            }),
            clock: self.game.state["clock"].clone(),
            color,
            level: level.to_string(),
        };
        self.start_chosen(chosen, window, cx);
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
        self.game.state["level"] = json!(chosen.level);
        self.game.state["color"] = json!(chosen.color);
        self.reset_archive(cx);
        let host = self.host();
        let effects = self
            .game
            .start(Some(chosen.setup), Some(chosen.clock), &host);
        self.perform(effects, cx);
        self.select_level(window, cx);
        self.changed(cx);
        self.computer_turn(cx);
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
        self.perform(effects, cx);
        self.changed(cx);
    }

    fn resign(&mut self, cx: &mut Context<Self>) {
        let host = self.host();
        let effects = self.game.resign(&host);
        self.perform(effects, cx);
        self.changed(cx);
    }

    fn switch_sides(&mut self, cx: &mut Context<Self>) {
        let other = if self.user_color() == Color::White {
            "black"
        } else {
            "white"
        };
        self.game.state["color"] = json!(other);
        self.new_game(cx);
    }

    fn show_ply(&mut self, ply: usize, cx: &mut Context<Self>) {
        self.game.state["ply"] = json!(ply.min(self.moves().len()));
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
        window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
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
            crate::voice::grammar("GAME_GRAMMAR"),
            listening,
            cx,
        );
    }

    /// What voice input says while no move is taken (`voiceIdle`).
    fn voice_idle(&self) -> &'static str {
        if self.game.result(&self.host()).is_some() {
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
                    "variant": self.game.state["setup"]["variant"],
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
                    return Some(match answer["kind"].as_str() {
                        Some("confirm") => {
                            run = Some(Box::new(move |this, window, cx| {
                                this.asked = None;
                                window.close_dialog(cx);
                                this.answer(question, window, cx);
                            }));
                            reply("confirm", "Done")
                        }
                        Some("cancel") => {
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
                let command = crate::game::rules("spokenCommand", &[json!(text)]).ok()?;
                let (said, question, feedback) = match command.as_str()? {
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
            self.play(&uci, window, cx);
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

    /// The opening the moves on the board reached, as `(eco, name)` (`openingAt`).
    fn opening(&self) -> Option<(SharedString, SharedString)> {
        let opening = rules(
            "openingAt",
            &[
                self.game.state["setup"].clone(),
                json!(self.moves()),
                json!(self.ply()),
            ],
        )
        .ok()?;
        Some((
            opening["eco"].as_str()?.to_string().into(),
            opening["name"].as_str()?.to_string().into(),
        ))
    }

    fn clock_text(&self, color: &str) -> Option<SharedString> {
        if self.game.state["times"].is_null() {
            return None;
        }
        let remaining = self.game.remaining(color, &self.host());
        rules("formatClock", &[json!(remaining)])
            .ok()
            .and_then(|text| {
                text.as_str()
                    .map(|text| SharedString::from(text.to_string()))
            })
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let host = self.host();
        if self.engine == Engine::Missing {
            let checking = self.engine == Engine::Checking;
            return widgets::StatusBanner::new(
                Tone::Alert,
                icon("triangle-alert"),
                "Stockfish not found".into(),
            )
            .detail("Install or choose an engine in Settings to play".into())
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
        if let Some(result) = self.game.result(&host) {
            let kind = result["kind"].as_str().unwrap_or_default();
            let (tone, glyph) = match kind {
                "win" => (Tone::Win, icon("trophy")),
                "loss" => (Tone::Loss, icon("flag")),
                _ => (Tone::Plain, icon("handshake")),
            };
            return widgets::StatusBanner::new(
                tone,
                glyph,
                result["title"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
                    .into(),
            )
            .announce()
            .when_some(
                result["detail"]
                    .as_str()
                    .map(|detail| SharedString::from(detail.to_string())),
                |banner, value| banner.detail(value),
            )
            .when_some(
                // The analysis board and reviews play standard rules (from any position).
                (!self.moves().is_empty() && self.setup_variant() == "standard").then(|| {
                    Button::new("review-game")
                        .outline()
                        .small()
                        .label("Review game")
                        .on_click(cx.listener(|this, _, window, cx| this.review_game(window, cx)))
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
            .extra(
                Button::new("back-to-game")
                    .link()
                    .xsmall()
                    .label("Back to game")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let last = this.moves().len();
                        this.show_ply(last, cx)
                    }))
                    .into_any_element(),
            )
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
        .map(|id| {
            rules("engineLevelLabel", &[json!(id)])
                .ok()
                .and_then(|label| {
                    label
                        .as_str()
                        .map(|label| SharedString::from(label.to_string()))
                })
                .unwrap_or_else(|| id.clone().into())
        })
        .collect();
    (ids, labels)
}

fn board_state(game: &ComputerGame, interactive: bool) -> BoardState {
    let setup = &game.state["setup"];
    let variant = setup["variant"].as_str().unwrap_or("standard");
    let fen = setup["fen"].as_str().unwrap_or_default();
    let moves: Vec<String> =
        serde_json::from_value(game.state["moves"].clone()).unwrap_or_default();
    let ply = (crate::game::whole(&game.state["ply"]) as usize).min(moves.len());
    let position = kchess_domain::replay::replay_setup(variant, fen, &moves[..ply])
        .and_then(|replay| kchess_domain::rules::setup_start(variant, &replay.fen));
    let orientation = if game.state["color"] == "black" {
        Color::Black
    } else {
        Color::White
    };
    let last_move = ply
        .checked_sub(1)
        .and_then(|last| moves.get(last))
        .and_then(|uci| {
            let from = uci.get(0..2)?.parse::<Square>().ok()?;
            let to = uci.get(2..4)?.parse::<Square>().ok()?;
            Some((from, to))
        });
    match position {
        Some(position) => BoardState::of(
            &position,
            orientation,
            interactive.then_some(orientation),
            last_move,
        ),
        None => BoardState::of(
            &shakmaty::variant::VariantPosition::Chess(shakmaty::Chess::default()),
            orientation,
            None,
            None,
        ),
    }
}

fn board_state_at(page: &ComputerPage, interactive: bool) -> BoardState {
    board_state(&page.game, interactive)
}

impl Render for ComputerPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let host = self.host();
        let board_size = self.layout.board_size(window);
        let narrow = widgets::is_narrow(self.panel_area.size().width);
        let user = self.user_color();
        let (user_name, computer_name) = if user == Color::White {
            ("white", "black")
        } else {
            ("black", "white")
        };
        let position = self.position_at(self.ply());
        let turn = position
            .as_ref()
            .map_or(Color::White, shakmaty::Position::turn);
        let live = self.game.result(&host).is_none();
        let sans = self.sans();
        let moves = self.moves();
        let ply = self.ply();
        let interactive = self.interactive();
        let black_first = position.is_some()
            && self.game.state["setup"]["fen"]
                .as_str()
                .is_some_and(|fen| fen.split(' ').nth(1) == Some("b"));
        let in_progress = self.in_progress();
        let result = self.game.result(&host);
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
        let piece_set = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .piece_set
            .clone();
        // A running clock says whose turn it is; only an untimed game needs the outline.
        let untimed = self.game.state["times"].is_null();
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
            self.clock_text(computer_name).map(|text| {
                widgets::clock(text, live && turn != user && !untimed, &palette).into_any_element()
            }),
        );
        let entry = widgets::move_entry(&self.move_input, interactive, &palette);
        let voice = crate::voice::moves::board_bar(
            &self.voice,
            cx.entity().downgrade(),
            crate::voice::moves::Access {
                write: |this: &mut Self| &mut this.voice,
                play: |this: &mut Self, uci, window, cx| this.play(uci, window, cx),
            },
            crate::voice::moves::BoardBar {
                id: "computer-voice",
                hint: "Say “E four”, “Knight F three”, or “take back”",
                active: self.voice_taking(),
                listening: self.voice_listening(),
                grammar: crate::voice::grammar("GAME_GRAMMAR"),
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
            self.clock_text(user_name).map(|text| {
                widgets::clock(text, live && turn == user && !untimed, &palette).into_any_element()
            }),
        );
        let board_column = v_flex()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(computer_line)
            .child(crate::board::cached(&self.board))
            .child(you_line)
            .children(self.move_message.clone().map(|message| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(message)
            }));

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
        let clock_label = self.game.state["clock"].as_object().map(|clock| {
            format!(
                "{}+{}",
                clock
                    .get("minutes")
                    .and_then(Value::as_f64)
                    .unwrap_or_default(),
                clock
                    .get("increment")
                    .and_then(Value::as_f64)
                    .unwrap_or_default()
            )
        });
        let frontend = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .clone();
        let (blindfold, zen) = (frontend.blindfold, frontend.zen_mode);
        let show_opening = frontend.show_opening_name && self.setup_variant() == "standard";
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
                            cx,
                        ))
                    })
                    .child(widgets::view_toggles(blindfold, zen)),
            )
            .child(self.render_banner(cx))
            .children(self.error.clone().map(|error| {
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
                &sans,
                ply,
                black_first,
                "No moves yet".into(),
                select.clone(),
                &palette,
            )))
            .child(widgets::move_controls(
                ply,
                moves.len(),
                select,
                (format!("Switch sides (play {other})").into(), flip),
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
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.ask(Question::NewGame, window, cx)
                            })),
                    ),
            );

        self.layout
            .render("computer-page", board_column, panel, window, cx)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                // The arrows step through the moves when no text field has the keys.
                if !this.focus.is_focused(window) || event.keystroke.modifiers.modified() {
                    return;
                }
                if crate::shell::zen_key(&event.keystroke.key, cx) {
                    cx.stop_propagation();
                    return;
                }
                let moves = this.moves().len();
                if let Some(ply) =
                    crate::game::view::navigate(event.keystroke.key.as_str(), this.ply(), moves)
                {
                    this.show_ply(ply, cx);
                    cx.stop_propagation();
                }
            }))
    }
}
