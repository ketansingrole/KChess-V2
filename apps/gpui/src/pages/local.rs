//! Over the board: two people at one
//! board, any variant, with or without a clock; or a chess clock for a real board. Both keep their
//! place in Game history, and the board game resumes in either app.

use kchess_domain::training::clock::format_clock;
use std::time::Instant;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::input::{Input, InputEvent, InputState, Undo};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    Disableable as _, IndexPath, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::Level;
use kchess_domain::games::otb_clock::{Config, OtbClock, Side};
use kchess_games::Outcome;
use kchess_games::history::session_kind;
use serde_json::{Value, json};
use shakmaty::Color;

use crate::assets::icon;
use crate::board::{BoardEvent, ChessBoard};
use crate::export::Exporter;
use crate::game::history::HistoryPlace;
use crate::game::played::PlayedGame;
use crate::game::session::{Resumes, SessionSaver, Sessions};
use crate::game::surface::{self, Faces, MoveBox, Steps};
use crate::game::{STANDARD_FEN, chess960, position_problem, view};
use crate::part::Part;
use crate::play_layout::{Measured, PANEL_MAX, PANEL_MIN, PANEL_ZEN, PlayLayout, panel_style};
use crate::played::PlayedHandle;
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::{Palette, radii};
use crate::widgets::{self, KeyboardFocus as _, Side as PieceSide, Tone};
use kchess_domain::games::local::{self, LocalGame};

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Board,
    Clock,
}

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum ClockLayout {
    Horizontal,
    Vertical,
}

use crate::game::variants;

/// What the panel's buttons do, for Quick Pick to offer the same.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Action {
    TakeBack,
    Pause,
    Resume,
    Draw,
    Resign,
    NewGame,
    Flip,
    CopyPgn,
    Analyse,
}

/// The page's own preferences: the clock's layout, chess clock or board, turning the board for
/// the side to move, and each clock face's turn.
const LAYOUT_KEY: &str = "kchess:clock-layout";
const MODE_KEY: &str = "kchess:local-mode";
const AUTOFLIP_KEY: &str = "kchess:local-autoflip";
const ROTATION_KEY: &str = "kchess:clock-rotation";

fn clock_layout(cx: &App) -> ClockLayout {
    crate::prefs::get(cx, LAYOUT_KEY, ClockLayout::Horizontal)
}

pub struct LocalPage {
    /// Either player's spoken moves.
    voice: crate::voice::moves::VoiceMoves,
    /// Turned clock faces as pictures, by side, with what they show. A face replaced or no
    /// longer turned releases its texture (`App::drop_image`).
    turned: std::collections::HashMap<usize, (String, std::sync::Arc<RenderImage>)>,
    game: LocalGame,
    board: Entity<ChessBoard>,
    exporter: Entity<Exporter>,
    moves: MoveBox,
    started: Instant,
    /// The move shown while looking back; `None` at the latest.
    shown: Option<usize>,
    fixed_orientation: Color,
    /// The board game's session: read to resume it, saved as it changes.
    saver: SessionSaver<LocalPage>,
    history: Option<HistoryPlace>,
    clock: OtbClock,
    clock_history: Option<HistoryPlace>,
    clock_inputs: [Entity<InputState>; 3],
    clock_layout: Entity<SelectState<Vec<SharedString>>>,
    ticker: Option<Task<()>>,
    /// Each player's low-time warning.
    low_time: crate::game::low_time::LowTime,
    clock_ticker: Option<Task<()>>,
    /// What the chess clock's faces showed at the last tick.
    clock_faces: Faces<[String; 2]>,
    layout: PlayLayout,
    /// The side panel, measured for where its action buttons wrap.
    panel_area: Measured,
    /// The side panel beside the board, a cached view of its own (`render_panel`).
    panel: Entity<Part<LocalPage>>,
    /// White's and Black's clock faces (`render_clock_face`): a tick redraws only these.
    faces: [Entity<Part<LocalPage>>; 2],
    /// Whether each clock is running, as the page last drew it.
    faces_active: [bool; 2],
    /// What the game's clock faces showed at the last tick.
    game_faces: Faces<[String; 2]>,
    focus: FocusHandle,
    /// The mouse wheel over the board, which steps through the moves.
    wheel: view::Wheel,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for LocalPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Steps for LocalPage {
    /// The chess clock has no moves to step through.
    fn steps(&self, cx: &App) -> Option<(usize, usize)> {
        (self.mode(cx) == Mode::Board).then(|| (self.shown_ply(), self.moves().len()))
    }

    fn step_to(&mut self, ply: usize, _: &mut Window, cx: &mut Context<Self>) {
        self.show_ply(ply, cx);
    }

    fn wheel(&mut self) -> &mut view::Wheel {
        &mut self.wheel
    }
}

impl Resumes for LocalPage {
    fn saver(&mut self) -> &mut SessionSaver<Self> {
        &mut self.saver
    }

    fn snapshot(&mut self, _: &mut Context<Self>) -> Value {
        json!(self.game.snapshot(self.now()))
    }

    /// The board game and its history places resume from their sessions alone: reading the
    /// whole library replays every played game.
    fn resume(&mut self, saved: Option<Sessions>, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(mut saved) = saved {
            let game = saved.take(local::SESSION);
            if !game.is_null() {
                match LocalGame::resume(Some(&game), self.now()) {
                    Ok(game) => self.game = game,
                    Err(cause) => crate::log::write(
                        Level::Warn,
                        "local",
                        &format!("The saved game could not be read: {cause}"),
                    ),
                }
            }
            let (source, has_play) = (json!(self.game.archive_snapshot()), self.has_play());
            self.history = Some(HistoryPlace::new(
                local::HISTORY,
                saved.take(&session_kind(local::HISTORY)),
                true,
                source,
                has_play,
                cx,
            ));
            let clock_source = self.clock_snapshot();
            self.clock_history = Some(HistoryPlace::new(
                local::CLOCK_HISTORY,
                saved.take(&session_kind(local::CLOCK_HISTORY)),
                false,
                clock_source,
                false,
                cx,
            ));
        }
        self.after_change(cx);
    }
}

impl LocalPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> LocalPage {
        let started = Instant::now();
        // The pages are made afresh after a sign-out: the faces shown last release their
        // textures with this one, deferred as the window being drawn is out of the app's list.
        cx.on_release(|this: &mut LocalPage, cx| {
            let faces: Vec<_> = this.turned.drain().map(|(_, (_, image))| image).collect();
            cx.defer(move |cx| {
                for image in faces {
                    cx.drop_image(image, None);
                }
            });
        })
        .detach();
        let game = match LocalGame::resume(None, 0.) {
            Ok(game) => game,
            Err(cause) => panic!("a new local game always starts: {cause}"),
        };
        let board = cx.new(|_| {
            ChessBoard::new(view::board_state(
                &json!(game.setup()),
                &[],
                0,
                Color::White,
                None,
            ))
        });
        let (moves, typed) = MoveBox::new(Self::typed_move, window, cx);
        let clock = OtbClock::default();
        let number_input = |value: f64, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut input = InputState::new(window, cx);
                input.set_value(value.to_string(), window, cx);
                input
            })
        };
        let clock_inputs = [
            number_input(clock.config.minutes, window, cx),
            number_input(clock.config.bottom_minutes, window, cx),
            number_input(clock.config.increment, window, cx),
        ];
        let layout_choice: ClockLayout = clock_layout(cx);
        let clock_layout = cx.new(|cx| {
            SelectState::new(
                vec!["Side by side".into(), "Stacked".into()],
                Some(IndexPath::new(usize::from(
                    layout_choice == ClockLayout::Vertical,
                ))),
                window,
                cx,
            )
        });
        let settings = cx.global::<SettingsHandle>().0.clone();
        let voice = crate::voice::handle(cx);
        let mut subscriptions = vec![
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
            typed,
            cx.subscribe_in(&settings, window, |this, _, _: &SettingsChanged, _, cx| {
                this.after_change(cx)
            }),
            cx.subscribe_in(
                &clock_layout,
                window,
                |_, state, _: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let row = state
                        .read(cx)
                        .selected_index(cx)
                        .map_or(0, |index| index.row);
                    let layout = if row == 1 {
                        ClockLayout::Vertical
                    } else {
                        ClockLayout::Horizontal
                    };
                    crate::prefs::set(cx, LAYOUT_KEY, layout);
                    cx.notify();
                },
            ),
        ];
        for input in &clock_inputs {
            subscriptions.push(cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.configure_clock(cx);
                }
            }));
        }
        let saver = SessionSaver::start(
            local::SESSION,
            &[
                session_kind(local::HISTORY),
                session_kind(local::CLOCK_HISTORY),
            ],
            window,
            cx,
        );
        // The chess clock's session ends with the app: it is saved to history as it stands.
        let page = cx.entity().downgrade();
        crate::quit::on_quit(cx, move |cx| {
            let page = page.upgrade()?;
            Some(page.update(cx, |page, cx| page.end_clock(cx)))
        });
        LocalPage {
            game,
            board,
            moves,
            started,
            shown: None,
            fixed_orientation: Color::White,
            saver,
            history: None,
            clock,
            clock_history: None,
            clock_inputs,
            turned: std::collections::HashMap::new(),
            voice: crate::voice::moves::VoiceMoves::new("local", cx),
            clock_layout,
            ticker: None,
            low_time: Default::default(),
            clock_ticker: None,
            clock_faces: Faces::default(),
            layout: PlayLayout::default(),
            panel_area: Measured::default(),
            panel: Part::panel(|this, window, cx| this.render_panel(window, cx), cx),
            faces: [
                Part::new(|this, _, cx| this.render_clock_face(Color::White, cx), cx),
                Part::new(|this, _, cx| this.render_clock_face(Color::Black, cx), cx),
            ],
            faces_active: [false; 2],
            game_faces: Faces::default(),
            exporter: cx.new(|_| Exporter::default()),
            focus: cx.focus_handle(),
            wheel: Default::default(),
            _subscriptions: subscriptions,
        }
    }

    /// The turned faces' pictures as drawn now, by player.
    #[cfg(test)]
    pub(crate) fn turned_faces(&self) -> Vec<std::sync::Arc<RenderImage>> {
        let mut faces: Vec<_> = self.turned.iter().collect();
        faces.sort_by_key(|(index, _)| **index);
        faces
            .into_iter()
            .map(|(_, (_, image))| image.clone())
            .collect()
    }

    /// The saved game has been read.
    #[cfg(test)]
    pub(crate) fn resumed(&self) -> bool {
        self.saver.loaded()
    }

    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.
    }

    fn moves(&self) -> Vec<String> {
        self.game.moves().to_vec()
    }

    fn setup(&self) -> Value {
        json!(self.game.setup())
    }

    fn shown_ply(&self) -> usize {
        let count = self.moves().len();
        self.shown.unwrap_or(count).min(count)
    }

    fn at_live(&self) -> bool {
        self.shown_ply() == self.moves().len()
    }

    fn has_play(&self) -> bool {
        !self.moves().is_empty() || self.game.result().is_some()
    }

    fn mode(&self, cx: &App) -> Mode {
        crate::prefs::get(cx, MODE_KEY, Mode::Board)
    }

    fn auto_flip(&self, cx: &App) -> bool {
        crate::prefs::get(cx, AUTOFLIP_KEY, false)
    }

    /// Who is to move at the latest position.
    fn turn(&self) -> Color {
        view::turn(view::position_at(&self.setup(), &self.moves(), self.moves().len()).as_ref())
    }

    /// The board faces whoever is to move while automatic turning is on.
    pub(crate) fn orientation(&self, cx: &App) -> Color {
        if self.auto_flip(cx) && !self.game.over() {
            self.turn()
        } else {
            self.fixed_orientation
        }
    }

    fn flagged(&mut self, flagged: bool, cx: &mut Context<Self>) {
        if flagged {
            crate::sound::play(crate::sound::Sound::LowTime, cx);
        }
    }

    /// Moves are taken at the latest position of a game on, and not before the saved game is
    /// restored, which would replace the move (and nothing is saved before then).
    fn interactive(&self) -> bool {
        self.saver.loaded() && !self.game.over() && self.at_live()
    }

    pub(crate) fn play(&mut self, uci: &str, cx: &mut Context<Self>) {
        if !self.interactive() {
            return self.after_change(cx);
        }
        let now = self.now();
        let (san, flagged) = self.game.play(uci, now);
        if let Some(san) = san {
            self.shown = None;
            crate::sound::move_sound(&san, cx);
        }
        self.flagged(flagged, cx);
        self.changed(cx);
    }

    /// Spoken moves are taken while the game is on, at its last move.
    fn voice_taking(&self) -> bool {
        self.interactive()
    }

    /// The live position as the rules read a spoken move in it.
    fn voice_setup(&self) -> Value {
        let moves = self.moves();
        view::position_at(&self.setup(), &moves, moves.len()).map_or(Value::Null, |position| {
            json!({
                "variant": self.setup()["variant"],
                "fen": kchess_domain::rules::make_fen(&position),
            })
        })
    }

    fn sync_voice(&mut self, cx: &mut Context<Self>) {
        let context = format!("{}:{}", self.moves().len(), self.shown_ply());
        let taking = self.voice_taking();
        let line = view::sans(&self.setup(), &self.moves());
        self.voice.sync(
            &context,
            Some(line),
            kchess_domain::voice::grammar(kchess_domain::voice::Grammar::Move).to_vec(),
            taking,
            cx,
        );
    }

    fn voice_heard(
        &mut self,
        event: &crate::voice::VoiceEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.voice.owns(event) {
            return;
        }
        let setup = self.voice_setup();
        let heard = match event {
            crate::voice::VoiceEvent::Missed { heard, .. } => {
                let fen = setup["fen"].as_str().unwrap_or_default().to_string();
                self.voice.missed(heard, &fen, self.voice_taking(), cx);
                return;
            }
            crate::voice::VoiceEvent::Heard { heard, .. } => heard.clone(),
        };
        let turn = crate::voice::moves::Turn {
            taking: self.voice_taking(),
            idle: None,
            immediate: false,
        };
        if let crate::voice::moves::Action::Play(uci) =
            self.voice.hear(&heard, &setup, turn, |_| None, cx)
        {
            self.play(&uci, cx);
        }
        cx.notify();
    }

    fn typed_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.interactive() {
            return;
        }
        let position = view::position_at(&self.setup(), &self.moves(), self.moves().len());
        let variant = self.setup()["variant"]
            .as_str()
            .unwrap_or("standard")
            .to_string();
        match self.moves.take(&variant, position.as_ref(), window, cx) {
            Some(uci) => self.play(&uci, cx),
            None => cx.notify(),
        }
    }

    /// The game changed: show it, keep the clock running, save it shortly, keep its history place.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.after_change(cx);
        SessionSaver::changed(self, cx);
        let (source, has_play) = (json!(self.game.archive_snapshot()), self.has_play());
        if let Some(history) = &mut self.history {
            history.save(false, source, has_play, cx);
        }
    }

    /// The app quits: the chess clock stops and its session is saved to history as it ended.
    fn end_clock(&mut self, cx: &mut Context<Self>) -> crate::quit::BoxedSave {
        let now = self.now();
        self.clock.pause(now);
        let (source, has_play) = (self.clock_snapshot(), self.clock.has_play());
        if let Some(history) = &mut self.clock_history {
            history.save(true, source, has_play, cx);
        }
        cx.global::<PlayedHandle>().0.read(cx).flushed()
    }

    /// The game's clocks tick: a flag falls, a clock runs low, and a face redraws only when it
    /// shows something else.
    fn tick(&mut self, cx: &mut Context<Self>) -> bool {
        let now = self.now();
        let flagged = self.game.expire(now);
        let fell = flagged;
        self.flagged(flagged, cx);
        if fell {
            self.ticker = None;
            self.changed(cx);
            return false;
        }
        if !self.game.over() {
            for color in [Color::White, Color::Black] {
                let initial = self
                    .game
                    .clock()
                    .map_or(0., |clock| clock.side(color).minutes)
                    * 60.;
                let remaining = self.game.remaining(color, now);
                self.low_time.warn(color, remaining, initial, cx);
            }
        }
        let shown =
            [Color::White, Color::Black].map(|color| clock_face(self.game.remaining(color, now)));
        let changed = self.game_faces.each_changed(shown);
        for (face, changed) in self.faces.iter().zip(changed) {
            if changed {
                face.update(cx, |_, cx| cx.notify());
            }
        }
        true
    }

    fn after_change(&mut self, cx: &mut Context<Self>) {
        self.sync_voice(cx);
        let over = self.game.over();
        let moves = self.moves();
        let shown = self.shown_ply();
        let turn = self.turn();
        let mut state = view::board_state(
            &self.setup(),
            &moves,
            shown,
            self.orientation(cx),
            self.interactive().then_some(turn),
        );
        surface::board_options(&mut state, !over, None, cx);
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        let running = self.game.timed() && !over;
        if running && self.ticker.is_none() {
            self.ticker = Some(crate::timer::every(crate::timer::TICK, cx, Self::tick));
        } else if !running {
            self.ticker = None;
        }
        cx.notify();
    }

    fn show_ply(&mut self, ply: usize, cx: &mut Context<Self>) {
        let count = self.moves().len();
        let clamped = ply.min(count);
        self.shown = (clamped != count).then_some(clamped);
        self.after_change(cx);
    }

    /// Flipping by hand turns the board, and stops the automatic turning.
    fn flip(&mut self, cx: &mut Context<Self>) {
        let shown = self.orientation(cx);
        crate::prefs::set(cx, AUTOFLIP_KEY, false);
        self.fixed_orientation = !shown;
        self.after_change(cx);
    }

    fn set_auto_flip(&mut self, on: bool, cx: &mut Context<Self>) {
        // Turning it off leaves the board facing the way it is now.
        if !on && self.auto_flip(cx) && !self.game.over() {
            self.fixed_orientation = self.turn();
        }
        crate::prefs::set(cx, AUTOFLIP_KEY, on);
        self.after_change(cx);
    }

    /// A new game starts: the one before keeps its place in history and this one gets its own.
    /// After a saved game that could not be read, this one is the players' to keep.
    fn start(&mut self, setup: Value, clock: Value, cx: &mut Context<Self>) {
        self.saver.open();
        let (source, has_play) = (json!(self.game.archive_snapshot()), self.has_play());
        match &mut self.history {
            Some(history) => history.reset(source, has_play, cx),
            None => {
                self.history = Some(HistoryPlace::new(
                    local::HISTORY,
                    Value::Null,
                    true,
                    source,
                    has_play,
                    cx,
                ))
            }
        }
        let started = serde_json::from_value(setup)
            .and_then(|setup| Ok((setup, serde_json::from_value(clock)?)))
            .map_err(|error| format!("the chosen game: {error}"))
            .and_then(|(setup, clock)| self.game.start(setup, clock, self.now()));
        if let Err(cause) = started {
            crate::log::write(Level::Warn, "local", &cause);
        }
        self.shown = None;
        self.fixed_orientation = Color::White;
        self.changed(cx);
    }

    fn takeback(&mut self, cx: &mut Context<Self>) {
        self.game.takeback(self.now());
        self.shown = None;
        self.changed(cx);
    }

    fn resign(&mut self, color: Color, cx: &mut Context<Self>) {
        self.game.resign(color);
        self.changed(cx);
    }

    fn agree_draw(&mut self, cx: &mut Context<Self>) {
        self.game.agree_draw();
        self.changed(cx);
    }

    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        let flagged = self.game.toggle_pause(self.now());
        self.flagged(flagged, cx);
        self.changed(cx);
    }

    fn ask_resign(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let color = self.turn();
        let name = if color == Color::White {
            "White"
        } else {
            "Black"
        };
        let page = cx.entity().downgrade();
        crate::widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
            let page = page.clone();
            dialog
                .title(format!("Resign for {name}?"))
                .description("The other side wins. The game stays on the board.")
                .ok_text("Resign")
                .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let _ = page.update(cx, |page, cx| page.resign(color, cx));
                    true
                })
        });
    }

    fn open_new_game(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let page = cx.entity().downgrade();
        let current = self.setup();
        let form = cx.new(|cx| {
            NewGame::new(
                &current,
                move |setup, clock, window, cx| {
                    let _ = page.update(cx, |page, cx| page.replace_game(setup, clock, window, cx));
                },
                window,
                cx,
            )
        });
        crate::widgets::open_dialog(window, cx, move |dialog, _, _| {
            dialog
                .title("New game over the board")
                .w(px(512.))
                .child(form.clone())
        });
    }

    /// A game over the board from a position (the board editor's, the analysis board's), untimed.
    pub fn start_from(&mut self, setup: Value, window: &mut Window, cx: &mut Context<Self>) {
        crate::prefs::set(cx, MODE_KEY, Mode::Board);
        self.replace_game(setup, Value::Null, window, cx);
    }

    /// Start a game, asking first when one on the board is still being played: once the saved
    /// game is read, so that it is the one asked about.
    pub(crate) fn replace_game(
        &mut self,
        setup: Value,
        clock: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        SessionSaver::when_read(
            self,
            move |this, window, cx| this.confirm_new_game(setup, clock, window, cx),
            window,
            cx,
        );
    }

    fn confirm_new_game(
        &mut self,
        setup: Value,
        clock: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.moves().is_empty() || self.game.over() {
            return self.start(setup, clock, cx);
        }
        let page = cx.entity().downgrade();
        let chosen = std::rc::Rc::new((setup, clock));
        crate::widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
            let (page, chosen) = (page.clone(), chosen.clone());
            dialog
                .title("Start a new game?")
                .description("The game on the board is still in progress and will be lost")
                .ok_text("New game")
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let (setup, clock) = (chosen.0.clone(), chosen.1.clone());
                    let _ = page.update(cx, |page, cx| page.start(setup, clock, cx));
                    true
                })
        });
    }

    /// The game as its Export saves it and the analysis board opens it, between "White" and
    /// "Black".
    fn played_game(&self, cx: &App) -> PlayedGame {
        PlayedGame {
            result: self.game.outcome(),
            ..PlayedGame::new(
                &self.setup(),
                self.moves(),
                self.orientation(cx),
                "White",
                "Black",
            )
        }
    }

    /// Analyse the game on the analysis board at the position shown (standard and Chess960).
    fn analyse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let game = PlayedGame {
            ply: Some(self.shown_ply()),
            ..self.played_game(cx)
        };
        crate::pages::analysis::replace(window, cx, move |analysis, _, cx| {
            analysis.open_played(&game, false, cx)
        });
    }

    fn copy_pgn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let date = chrono::Local::now().format("%Y.%m.%d").to_string();
        let headers = json!({
            "Event": local::EVENT,
            "Date": date,
            "White": "White",
            "Black": "Black",
            "Result": Outcome::tag(self.game.outcome()),
        });
        match crate::game::setup_pgn(&self.setup(), &self.moves(), &headers) {
            Some(pgn) => {
                cx.write_to_clipboard(ClipboardItem::new_string(pgn));
                window.push_notification("PGN copied", cx);
            }
            _ => window.push_notification("Could not copy the PGN", cx),
        }
    }

    /* ── Chess clock ─────────────────────────────────────────────────── */

    fn clock_snapshot(&self) -> Value {
        json!({
            "source": "clock",
            "setup": { "variant": "standard", "fen": STANDARD_FEN },
            "moves": [],
            "white": "Bottom player",
            "black": "Top player",
            "result": "*",
            "reason": self.clock.reason(),
            "timeControl": self.clock.time_control(),
            "clockSummary": self.clock.summary(),
        })
    }

    fn clock_changed(&mut self, cx: &mut Context<Self>) {
        if self.clock.running.is_some() && self.clock_ticker.is_none() {
            self.clock_ticker = Some(crate::timer::every(crate::timer::TICK, cx, |this, cx| {
                let now = this.now();
                if this.clock.expire(now) {
                    crate::sound::play(crate::sound::Sound::LowTime, cx);
                    this.clock_flagged(cx);
                }
                let shown =
                    [Side::Top, Side::Bottom].map(|side| clock_face(this.clock.left(side, now)));
                if this.clock_faces.changed(shown) {
                    cx.notify();
                }
                true
            }));
        } else if self.clock.running.is_none() {
            self.clock_ticker = None;
        }
        cx.notify();
    }

    /// A flag fell: the session is saved to history as it ended.
    fn clock_flagged(&mut self, cx: &mut Context<Self>) {
        let (source, has_play) = (self.clock_snapshot(), self.clock.has_play());
        if let Some(history) = &mut self.clock_history {
            history.save(true, source, has_play, cx);
        }
        self.clock_changed(cx);
    }

    fn press(&mut self, side: Side, cx: &mut Context<Self>) {
        if self.clock.press(side, self.now()) {
            crate::sound::play(crate::sound::Sound::LowTime, cx);
            self.clock_flagged(cx);
        }
        self.clock_changed(cx);
    }

    fn start_or_pause(&mut self, cx: &mut Context<Self>) {
        let now = self.now();
        let fell = if self.clock.running.is_some() {
            self.clock.pause(now)
        } else {
            if self.clock.flagged.is_some() {
                self.reset_clock_history(cx);
            }
            self.clock.start(now)
        };
        if fell {
            crate::sound::play(crate::sound::Sound::LowTime, cx);
            self.clock_flagged(cx);
        }
        self.clock_changed(cx);
    }

    fn reset_clock(&mut self, cx: &mut Context<Self>) {
        self.reset_clock_history(cx);
        self.clock.reset();
        self.clock_changed(cx);
    }

    fn reset_clock_history(&mut self, cx: &mut Context<Self>) {
        let (source, has_play) = (self.clock_snapshot(), self.clock.has_play());
        if let Some(history) = &mut self.clock_history {
            history.reset(source, has_play, cx);
        }
    }

    fn configure_clock(&mut self, cx: &mut Context<Self>) {
        let read = |input: &Entity<InputState>, cx: &App| {
            input.read(cx).value().trim().parse::<f64>().ok()
        };
        let [top, bottom, increment] = &self.clock_inputs;
        let (top, bottom, increment) = (read(top, cx), read(bottom, cx), read(increment, cx));
        let config = Config {
            minutes: top
                .filter(|m| *m >= 1.)
                .unwrap_or(self.clock.config.minutes),
            bottom_minutes: bottom
                .filter(|m| *m >= 1.)
                .unwrap_or(self.clock.config.bottom_minutes),
            increment: increment
                .filter(|s| *s >= 0.)
                .unwrap_or(self.clock.config.increment),
        };
        self.clock.configure(config);
        cx.notify();
    }

    /// The chess clock's keys when the page itself has focus (not a text field): Space presses
    /// the running clock and P pauses.
    fn clock_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !crate::board::page_has_keys(&self.focus, window, cx)
            || event.keystroke.modifiers.modified()
        {
            return;
        }
        match event.keystroke.key.as_str() {
            "space" => {
                let side = self.clock.running.unwrap_or(Side::Top);
                self.press(side, cx);
            }
            "p" => {
                if self.clock.pause(self.now()) {
                    self.clock_flagged(cx);
                }
                self.clock_changed(cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// How far a clock face is turned (`kchess:clock-rotation`), so a player across the table
    /// reads it the right way up.
    fn rotation(side: Side, cx: &App) -> u16 {
        let turned: serde_json::Value =
            crate::prefs::get(cx, ROTATION_KEY, json!({ "top": 0, "bottom": 0 }));
        let key = if side == Side::Top { "top" } else { "bottom" };
        (turned[key].as_f64().unwrap_or_default() as u16 / 90 * 90) % 360
    }

    /// A quarter turn clockwise.
    fn rotate(side: Side, cx: &mut App) {
        let mut turned: serde_json::Value =
            crate::prefs::get(cx, ROTATION_KEY, json!({ "top": 0, "bottom": 0 }));
        let key = if side == Side::Top { "top" } else { "bottom" };
        turned[key] = json!((Self::rotation(side, cx) + 90) % 360);
        crate::prefs::set(cx, ROTATION_KEY, turned);
    }

    fn render_clock(
        &mut self,
        palette: &Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = self.now();
        let vertical = clock_layout(cx) == ClockLayout::Vertical;
        // The clocks take the board's place, so switching modes moves nothing else: its side
        // across, and its column's height beside the panel (its side again when stacked).
        let board_size = self.board_size(window);
        let page_stacked = self.layout.is_stacked();
        let area = size(
            board_size,
            if page_stacked {
                board_size
            } else {
                self.layout.panel_height_px(board_size)
            },
        );
        let face_size = if vertical {
            size(area.width, (area.height - FACE_GAP) / 2.)
        } else {
            size((area.width - FACE_GAP) / 2., area.height)
        };
        // As large as the face holds "10:00" with its lines above and below.
        let digits = ((face_size.width - px(32.)) / 3.2)
            .min((face_size.height - px(120.)) / 1.2)
            .clamp(px(36.), px(180.));
        let mut face = |side: Side, index: usize, cx: &mut Context<Self>| {
            let running = self.clock.running == Some(side);
            let flagged = self.clock.flagged == Some(side);
            let time = format_clock(self.clock.left(side, now));
            let state = if flagged {
                "Time up"
            } else if running {
                "Your turn"
            } else if self.clock.running.is_some() {
                "Waiting"
            } else {
                "Ready"
            };
            // Red when time is up, and while the last ten seconds run.
            let (background, foreground) = if flagged || (running && widgets::low_time(&time)) {
                (palette.error, palette.bg)
            } else if running {
                (palette.primary, palette.bg)
            } else {
                (palette.elevated, palette.text)
            };
            let moves = self.clock.moves.get(side);
            let rotation = Self::rotation(side, cx);
            if rotation == 0
                && let Some((_, old)) = self.turned.remove(&index)
            {
                cx.drop_image(old, Some(window));
            }
            let turned = (rotation != 0).then(|| {
                let lines = [
                    (format!("Player {index}"), 14., true, 0.75),
                    (time.clone(), 120., true, 1.),
                    (
                        crate::widgets::format_counted(moves as f64, "move", "moves"),
                        14.,
                        false,
                        0.75,
                    ),
                    (state.to_string(), 12., true, 1.),
                ];
                let rgb = Rgba::from(foreground);
                let color = format!(
                    "#{:02x}{:02x}{:02x}",
                    (rgb.r * 255.).round() as u8,
                    (rgb.g * 255.).round() as u8,
                    (rgb.b * 255.).round() as u8
                );
                let key = format!("{lines:?}{color}{rotation}");
                let image = match self.turned.get(&index) {
                    Some((shown, image)) if *shown == key => Some(image.clone()),
                    _ => crate::game_image::rotated_text_image(
                        &lines.map(|(text, size, bold, opacity)| crate::game_image::TextLine {
                            text,
                            size,
                            bold,
                            opacity,
                        }),
                        &color,
                        rotation,
                        window.scale_factor(),
                    )
                    .ok()
                    .inspect(|image| {
                        if let Some((_, old)) = self.turned.insert(index, (key, image.clone())) {
                            cx.drop_image(old, Some(window));
                        }
                    }),
                };
                image.map(|image| {
                    img(image)
                        .size_full()
                        .p(px(16.))
                        .object_fit(ObjectFit::Contain)
                })
            });
            let content = v_flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .opacity(0.75)
                        .child(format!("Player {index}")),
                )
                .child(crate::widgets::timer(time.clone(), digits))
                .child(div().text_size(px(14.)).opacity(0.75).child(
                    crate::widgets::format_counted(moves as f64, "move", "moves"),
                ))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(state),
                );
            div()
                .id(("clock-face", index))
                .test_support()
                .role(Role::Button)
                .keyboard_focus(palette.primary)
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .rounded(radii::XL)
                .border_1()
                .border_color(palette.border)
                .bg(background)
                .text_color(foreground)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.press(side, cx)))
                .aria_label(format!(
                    "Player {index} clock, {time}. Press after your move."
                ))
                .map(|face| match turned.flatten() {
                    Some(image) => face.child(image),
                    None => face.child(content),
                })
        };
        let faces = if vertical { v_flex() } else { h_flex() }
            .id("local-clock-faces")
            .w(area.width)
            .h(area.height)
            .flex_none()
            .items_stretch()
            .gap(FACE_GAP)
            .child(face(Side::Top, 1, cx))
            .child(face(Side::Bottom, 2, cx));
        let [top_minutes, bottom_minutes, increment] = &self.clock_inputs;
        let number = |input: &Entity<InputState>, name: &'static str, unit: &'static str| {
            h_flex()
                .gap(px(6.))
                .text_size(px(13.))
                .text_color(palette.text_muted)
                .child(
                    div()
                        .w(px(64.))
                        .child(Input::new(input).small().aria_label(name)),
                )
                .child(unit)
        };
        let row = |title: &'static str, control: AnyElement| {
            widgets::setting_row(widgets::setting_info(title, None, palette), control)
        };
        let rotate = h_flex()
            .gap(px(8.))
            .children(
                [(Side::Top, 1usize), (Side::Bottom, 2)].map(|(side, index)| {
                    let degrees = Self::rotation(side, cx);
                    Button::new(("clock-rotate", index))
                        .small()
                        .outline()
                        .icon(icon("rotate-cw"))
                        .label(format!("{index} · {degrees}°"))
                        .accessibility_label(format!(
                            "Rotate Player {index} clock, currently {degrees} degrees"
                        ))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            Self::rotate(side, cx);
                            cx.notify();
                        }))
                }),
            );
        let running = self.clock.running.is_some();
        let zen = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .zen_mode;
        // The shared board's panel's place and size, its actions at the foot as there.
        let panel = widgets::side_panel(palette)
            .id("local-clock-panel")
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(if zen { PANEL_ZEN } else { PANEL_MAX })
            .child(widgets::setting_list(
                [
                    row(
                        "Player 1",
                        number(top_minutes, "Player 1 minutes", "min").into_any_element(),
                    ),
                    row(
                        "Player 2",
                        number(bottom_minutes, "Player 2 minutes", "min").into_any_element(),
                    ),
                    row(
                        "Increment",
                        number(increment, "Increment in seconds", "s").into_any_element(),
                    ),
                    row(
                        "Layout",
                        div()
                            .w(px(160.))
                            .child(
                                Select::new(&self.clock_layout)
                                    .small()
                                    .accessibility_label("Clock layout"),
                            )
                            .into_any_element(),
                    ),
                    row("Rotate", rotate.into_any_element()),
                ],
                palette,
            ))
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap(px(8.))
                    .pt(px(14.))
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        Button::new("clock-reset")
                            .outline()
                            .flex_1()
                            .icon(icon("rotate-ccw"))
                            .label("Reset")
                            .on_click(cx.listener(|this, _, _, cx| this.reset_clock(cx))),
                    )
                    .child(
                        Button::new("clock-start")
                            .primary()
                            .flex_1()
                            .icon(if running { icon("pause") } else { icon("play") })
                            .label(if running { "Pause" } else { "Start" })
                            .tooltip("Space switches turns · P pauses")
                            .on_click(cx.listener(|this, _, _, cx| this.start_or_pause(cx))),
                    ),
            );
        let tabs = mode_tabs(Mode::Clock, palette, cx).into_any_element();
        self.layout
            .render_under("local-clock", tabs, faces, panel, window, cx)
            .into_any_element()
    }
}

/// The row above the page that holds the mode tabs, and its gap to the board: the board is
/// sized to leave room for it.
const MODE_BAR: Pixels = px(48.);
/// The gap between the two clock faces.
const FACE_GAP: Pixels = px(16.);

/// "Shared board" and "Chess clock", above the page in both modes, so switching moves nothing but
/// what is in the board's place.
fn mode_tabs(mode: Mode, palette: &Palette, cx: &mut Context<LocalPage>) -> Div {
    let page = cx.entity().downgrade();
    div().h(MODE_BAR).child(widgets::pills(
        "local-mode",
        [("users-round", "Shared board"), ("timer", "Chess clock")]
            .into_iter()
            .map(|(glyph, label)| widgets::Pill {
                icon: Some(icon(glyph)),
                label: label.into(),
                badge: None,
                disabled: false,
            })
            .collect(),
        usize::from(mode == Mode::Clock),
        move |index, _, cx| {
            let mode = if index == 1 { Mode::Clock } else { Mode::Board };
            crate::prefs::set(cx, MODE_KEY, mode);
            let _ = page.update(cx, |page, cx| page.after_change(cx));
        },
        false,
        palette,
    ))
}

impl Render for LocalPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        if self.mode(cx) == Mode::Clock {
            let clock = self.render_clock(&palette, window, cx);
            return surface::keys(div().size_full(), &self.focus, cx)
                .on_key_down(cx.listener(Self::clock_key))
                .child(clock)
                .into_any_element();
        }
        let board_size = self.board_size(window);
        let moves = self.moves();
        let shown = self.shown_ply();
        let live = self.game.result().is_none();
        let orientation = self.orientation(cx);
        let display = view::position_at(&self.setup(), &moves, shown);
        let display_turn = view::turn(display.as_ref());
        let frontend = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .clone();
        let fen = display
            .as_ref()
            .map(kchess_domain::rules::make_fen)
            .unwrap_or_default();
        let (timed, paused) = (self.game.timed(), self.game.paused());
        self.faces_active = [Color::White, Color::Black]
            .map(|color| live && display_turn == color && timed && !paused);
        let mut pockets = [
            crate::board::pocket(&self.board, Color::White, cx),
            crate::board::pocket(&self.board, Color::Black, cx),
        ];
        let mut line =
            |color: Color, middle: Option<AnyElement>| {
                let name = if color == Color::White {
                    "White"
                } else {
                    "Black"
                };
                let active = live && display_turn == color;
                widgets::player_line(
                    h_flex()
                        .gap(px(8.))
                        .child(widgets::avatar(
                            icon("user"),
                            if color == Color::White {
                                PieceSide::White
                            } else {
                                PieceSide::Black
                            },
                            active,
                            &palette,
                        ))
                        .child(name)
                        .children(pockets[usize::from(color == Color::Black)].take().or_else(
                            || {
                                widgets::material(
                                    &fen,
                                    color == Color::White,
                                    &frontend.piece_set,
                                    &palette,
                                )
                            },
                        )),
                    middle,
                    timed.then(|| self.faces[face(color)].clone().into_any_element()),
                )
            };
        let interactive = self.interactive();
        let entry = widgets::move_entry(&self.moves.input, interactive, &palette);
        let (voice, voice_line) = crate::voice::moves::board_bar(
            &self.voice,
            cx.entity().downgrade(),
            crate::voice::moves::Access {
                write: |this: &mut Self| &mut this.voice,
                play: |this: &mut Self, uci, _, cx| this.play(uci, cx),
            },
            crate::voice::moves::BoardBar {
                id: "local-voice",
                hint: "Either player can say “E four” or “Knight F three”",
                active: self.voice_taking(),
                listening: self.voice_taking(),
                grammar: kchess_domain::voice::grammar(kchess_domain::voice::Grammar::Move)
                    .to_vec(),
            },
            cx,
        );
        let board_column = v_flex()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(line(!orientation, None))
            .child(crate::board::cached(&self.board))
            .child(line(orientation, Some(widgets::move_row(entry, voice))))
            .children(voice_line)
            .children(self.moves.message(&palette))
            .children(self.saver.problems(cx).into_iter().map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.error)
                    .child(error)
            }));
        let board_column = surface::wheel(board_column, cx);

        let panel = if self.layout.is_stacked() {
            self.render_panel(window, cx)
        } else {
            let max_width = if frontend.zen_mode {
                PANEL_ZEN
            } else {
                PANEL_MAX
            };
            self.panel
                .clone()
                .cached(panel_style(
                    self.layout.panel_height_px(board_size),
                    max_width,
                ))
                .into_any_element()
        };
        let tabs = mode_tabs(Mode::Board, &palette, cx).into_any_element();
        let page = self
            .layout
            .render_under("local-page", tabs, board_column, panel, window, cx)
            // ⌘Z outside the move box takes back, as the button does.
            .on_action(
                cx.listener(|this, _: &Undo, window, cx| this.act(Action::TakeBack, window, cx)),
            );
        surface::keys(page, &self.focus, cx).into_any_element()
    }
}

impl LocalPage {
    /// The board's side, under the mode tabs.
    fn board_size(&self, window: &Window) -> Pixels {
        self.layout.board_size_in(px(0.), MODE_BAR, window)
    }

    /// The panel beside (or under) the board: the game's setup and status, its moves and the
    /// actions. Beside the board it is the page's `Part`, kept while only a clock face redraws.
    fn render_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = *cx.global::<Palette>();
        let board_size = self.board_size(window);
        let narrow = widgets::is_narrow(self.panel_area.size().width);
        let moves = self.moves();
        let sans = view::sans(&self.setup(), &moves);
        let shown = self.shown_ply();
        let over = self.game.over();
        let result = self.game.result();
        let display = view::position_at(&self.setup(), &moves, shown);
        let frontend = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .clone();
        let (timed, paused) = (self.game.timed(), self.game.paused());
        let variant_label = crate::game::setup_label(&self.setup());
        let clock_label = self.game.clock().map(|clock| {
            let mut label = widgets::time_control(clock.white.minutes, clock.white.increment);
            if clock.black.minutes != clock.white.minutes {
                label.push_str(&format!(
                    " (Black {} min)",
                    widgets::clock_minutes(clock.black.minutes)
                ));
            }
            label
        });
        let banner = match &result {
            Some(result) => {
                let winner = result.winner;
                widgets::StatusBanner::new(
                    if winner.is_some() {
                        Tone::Win
                    } else {
                        Tone::Plain
                    },
                    if winner.is_some() {
                        icon("trophy")
                    } else {
                        icon("handshake")
                    },
                    match winner {
                        Some(Color::White) => "White wins".into(),
                        Some(Color::Black) => "Black wins".into(),
                        None => "Draw".into(),
                    },
                )
                .announce()
                .detail(SharedString::from(result.reason.clone()))
            }
            None if !self.at_live() => widgets::StatusBanner::new(
                Tone::Plain,
                icon("history"),
                "Reviewing an earlier position".into(),
            )
            .extra(widgets::back_to_game(cx.listener(|this, _, _, cx| {
                let last = this.moves().len();
                this.show_ply(last, cx)
            }))),
            None => {
                let check = display.as_ref().is_some_and(shakmaty::Position::is_check);
                let detail = if timed && paused && !moves.is_empty() {
                    Some("Clock paused".into())
                } else if check {
                    Some("Check".into())
                } else {
                    None
                };
                widgets::StatusBanner::turn(
                    format!(
                        "{} to move",
                        if self.turn() == Color::White {
                            "White"
                        } else {
                            "Black"
                        }
                    )
                    .into(),
                )
                .when_some(detail, |banner, value| banner.detail(value))
            }
        };
        let opening =
            (frontend.show_opening_name && self.setup()["variant"] == "standard").then(|| {
                let opening = crate::game::opening_at(&self.setup(), &moves, shown)?;
                Some((opening.eco, opening.name))
            });
        let page = cx.entity().downgrade();
        let select = move |ply: usize, _: &mut Window, cx: &mut App| {
            let _ = page.update(cx, |page, cx| page.show_ply(ply, cx));
        };
        let flip_page = cx.entity().downgrade();
        let flip: widgets::ClickHandler = Box::new(move |_, _, cx| {
            let _ = flip_page.update(cx, |page, cx| page.flip(cx));
        });
        let auto_flip = self.auto_flip(cx);
        let in_progress = !over && !moves.is_empty();
        widgets::side_panel(&palette)
            .relative()
            .child(self.panel_area.probe(cx))
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(if frontend.zen_mode {
                PANEL_ZEN
            } else {
                PANEL_MAX
            })
            .child(
                h_flex()
                    .gap(px(8.))
                    .flex_wrap()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .children(
                        variant_label
                            .map(|label| div().font_weight(FontWeight::MEDIUM).child(label)),
                    )
                    .children(clock_label)
                    .child(div().flex_1())
                    .child({
                        let page = cx.entity().downgrade();
                        widgets::labelled_switch(
                            "auto-flip",
                            "Flip after each move",
                            auto_flip,
                            false,
                            move |on, _, cx| {
                                let _ = page.update(cx, |this, cx| this.set_auto_flip(*on, cx));
                            },
                        )
                        .text_size(px(12.))
                    })
                    .child(widgets::view_toggles(cx)),
            )
            .child(banner)
            .child(widgets::section_title(
                "Moves",
                opening.flatten().map(|(eco, name)| format!("{eco} {name}")),
                &palette,
            ))
            .child(div().flex_1().min_h(px(120.)).child(widgets::moves_list(
                "local-moves",
                widgets::MoveSheet {
                    sans: &sans,
                    shown,
                    first: view::first_move(&self.setup()),
                },
                if !timed {
                    "No moves yet".into()
                } else {
                    "The first move starts the clock".into()
                },
                select.clone(),
                crate::notation::figurine(cx),
                &palette,
            )))
            .child(widgets::move_controls(
                shown,
                moves.len(),
                select,
                widgets::TurnButton::Flip(flip),
                cx,
            ))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .pt(px(14.))
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        Button::new("local-takeback")
                            .outline()
                            .flex_1()
                            .when(narrow, |button| button.min_w(relative(0.45)))
                            .icon(icon("undo-2"))
                            .label("Take back")
                            .disabled(moves.is_empty() || self.timed_out())
                            .on_click(cx.listener(|this, _, _, cx| this.takeback(cx))),
                    )
                    .when(timed && !over, |row| {
                        row.child(
                            Button::new("local-pause")
                                .outline()
                                .flex_1()
                                .when(narrow, |button| button.min_w(relative(0.45)))
                                .icon(if paused { icon("play") } else { icon("pause") })
                                .label(if paused { "Resume" } else { "Pause" })
                                .disabled(moves.is_empty())
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
                        )
                    })
                    .when(in_progress, |row| {
                        row.child(
                            Button::new("local-draw")
                                .outline()
                                .flex_1()
                                .when(narrow, |button| button.min_w(relative(0.45)))
                                .icon(icon("handshake"))
                                .label("Draw")
                                .on_click(cx.listener(|this, _, _, cx| this.agree_draw(cx))),
                        )
                        .child(
                            Button::new("local-resign")
                                .outline()
                                .danger()
                                .flex_1()
                                .when(narrow, |button| button.min_w(relative(0.45)))
                                .icon(icon("flag"))
                                .label("Resign")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.ask_resign(window, cx)),
                                ),
                        )
                    })
                    .child(
                        Button::new("local-new")
                            .primary()
                            .flex_1()
                            .when(narrow, |button| button.min_w(relative(0.45)))
                            .icon(icon("plus"))
                            .label("New game")
                            .tooltip_with_action("New game", &crate::menu::New, None)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_new_game(window, cx)),
                            ),
                    ),
            )
            .when(!frontend.zen_mode, |panel| {
                let analysable =
                    crate::game::analysable(self.setup()["variant"].as_str().unwrap_or("standard"));
                // Copy PGN, Export and Analyse share the row.
                panel.child(widgets::panel_actions(
                    narrow,
                    [
                        Button::new("copy-pgn")
                            .ghost()
                            .small()
                            .w_full()
                            .icon(icon("copy"))
                            .label("Copy PGN")
                            .disabled(moves.is_empty())
                            .on_click(cx.listener(|this, _, window, cx| this.copy_pgn(window, cx)))
                            .into_any_element()
                            .into(),
                        crate::export::export_menu(
                            "local-export",
                            &self.exporter,
                            self.played_game(cx),
                            Anchor::BottomCenter,
                            false,
                            cx,
                        )
                        .into_any_element()
                        .into(),
                    ]
                    .into_iter()
                    .chain(analysable.then(|| {
                        Button::new("local-analyse")
                            .ghost()
                            .small()
                            .w_full()
                            .icon(icon("microscope"))
                            .label("Analyse")
                            .disabled(moves.is_empty())
                            .on_click(cx.listener(|this, _, window, cx| this.analyse(window, cx)))
                            .into_any_element()
                            .into()
                    })),
                ))
            })
            .into_any_element()
    }

    /// A player's clock beside their name: its own `Part`, which a tick redraws.
    fn render_clock_face(&mut self, color: Color, cx: &mut Context<Self>) -> AnyElement {
        let palette = *cx.global::<Palette>();
        let text = clock_face(self.game.remaining(color, self.now()));
        let label = format!(
            "{} clock, {text}",
            if color == Color::White {
                "White"
            } else {
                "Black"
            }
        );
        widgets::clock(text.into(), self.faces_active[face(color)], &palette)
            .id(("local-clock", face(color)))
            .test_support()
            .aria_label(label)
            .into_any_element()
    }
}

impl LocalPage {
    /// The actions its buttons allow now. The clock alone has none of the board's.
    pub(crate) fn actions(&self, cx: &App) -> Vec<Action> {
        if self.mode(cx) == Mode::Clock {
            return Vec::new();
        }
        let over = self.game.over();
        let played = !self.moves().is_empty();
        let mut actions = Vec::new();
        if played && !self.timed_out() {
            actions.push(Action::TakeBack);
        }
        if played && !over && self.game.timed() {
            actions.push(if self.game.paused() {
                Action::Resume
            } else {
                Action::Pause
            });
        }
        if played && !over {
            actions.extend([Action::Draw, Action::Resign]);
        }
        actions.extend([Action::NewGame, Action::Flip]);
        if played {
            actions.push(Action::CopyPgn);
            if crate::game::analysable(self.setup()["variant"].as_str().unwrap_or("standard")) {
                actions.push(Action::Analyse);
            }
        }
        actions
    }

    /// Run an action as its button does, if it is still allowed.
    pub(crate) fn act(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.actions(cx).contains(&action) {
            return;
        }
        match action {
            Action::TakeBack => self.takeback(cx),
            Action::Pause | Action::Resume => self.toggle_pause(cx),
            Action::Draw => self.agree_draw(cx),
            Action::Resign => self.ask_resign(window, cx),
            Action::NewGame => self.open_new_game(window, cx),
            Action::Flip => self.flip(cx),
            Action::CopyPgn => self.copy_pgn(window, cx),
            Action::Analyse => self.analyse(window, cx),
        }
    }

    /// A clock ran out: taking back cannot refill it, so the game stays lost on time.
    fn timed_out(&self) -> bool {
        self.game
            .state()
            .times
            .is_some_and(|times| times.white == 0. || times.black == 0.)
    }
}

/// Starts the game the form chose, from its setup and clock.
type StartGame = Box<dyn Fn(Value, Value, &mut Window, &mut App)>;

/// The new-game form (`New game over the board`): the variant, its start, the clock and odds.
struct NewGame {
    variant: Entity<SelectState<Vec<SharedString>>>,
    number: Entity<InputState>,
    fen: Entity<InputState>,
    clock: Entity<SelectState<Vec<SharedString>>>,
    odds: Entity<InputState>,
    on_start: StartGame,
    _subscriptions: Vec<Subscription>,
}

impl NewGame {
    fn new(
        current: &Value,
        on_start: impl Fn(Value, Value, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> NewGame {
        let texts = variants();
        // The form opens on the game being played: its variant, its Chess960 number, or the
        // position it started from.
        let current_variant = current["variant"].as_str().unwrap_or("standard");
        let current_fen = current["fen"].as_str().unwrap_or(STANDARD_FEN).to_string();
        let row = texts
            .iter()
            .position(|text| text.key == current_variant)
            .unwrap_or(0);
        let variant = cx.new(|cx| {
            SelectState::new(
                texts
                    .iter()
                    .map(|text| SharedString::from(text.label.clone()))
                    .collect(),
                Some(IndexPath::new(row)),
                window,
                cx,
            )
        });
        let current_number = (current_variant == "chess960")
            .then(|| (0..960).find(|&n| chess960(n) == current_fen))
            .flatten();
        let number = cx.new(|cx| {
            let mut input = InputState::new(window, cx);
            let n = current_number
                .or_else(|| crate::game::drawn(kchess_games::random_chess960()))
                .unwrap_or(518);
            input.set_value(n.to_string(), window, cx);
            input
        });
        let usual = match current_variant {
            "standard" => Some(STANDARD_FEN.to_string()),
            "chess960" => None,
            variant => crate::game::default_fen(variant),
        };
        let fen = cx.new(|cx| {
            let label = format!(
                "{} start",
                crate::game::variant_label(texts.get(row).map_or("standard", |text| &text.key))
            );
            let mut input = InputState::new(window, cx).placeholder(label);
            if usual.as_ref().is_some_and(|usual| *usual != current_fen) {
                input.set_value(current_fen.clone(), window, cx);
            }
            input
        });
        let clock = cx.new(|cx| {
            SelectState::new(
                crate::game::CLOCKS
                    .iter()
                    .map(|(label, _)| SharedString::from(*label))
                    .collect(),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let odds = cx.new(|cx| InputState::new(window, cx));
        let refresh = |_: &mut NewGame,
                       _: Entity<InputState>,
                       _: &InputEvent,
                       cx: &mut Context<NewGame>| cx.notify();
        let subscriptions = vec![
            cx.subscribe(&number, refresh),
            cx.subscribe(&fen, refresh),
            cx.subscribe(&odds, refresh),
            // The FEN's placeholder names the start it replaces: "Crazyhouse start".
            cx.subscribe_in(
                &variant,
                window,
                |this, _, _: &SelectEvent<Vec<SharedString>>, window, cx| {
                    let label = format!(
                        "{} start",
                        crate::game::variant_label(&this.variant_key(cx))
                    );
                    this.fen
                        .update(cx, |input, cx| input.set_placeholder(label, window, cx));
                    cx.notify()
                },
            ),
            cx.subscribe(&clock, |_, _, _: &SelectEvent<Vec<SharedString>>, cx| {
                cx.notify()
            }),
        ];
        NewGame {
            variant,
            number,
            fen,
            clock,
            odds,
            on_start: Box::new(on_start),
            _subscriptions: subscriptions,
        }
    }

    fn variant_key(&self, cx: &App) -> String {
        let row = self
            .variant
            .read(cx)
            .selected_index(cx)
            .map_or(0, |index| index.row);
        variants()
            .get(row)
            .map_or_else(|| "standard".into(), |text| text.key.clone())
    }

    fn number960(&self, cx: &App) -> Option<i64> {
        self.number
            .read(cx)
            .value()
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|n| (0..=959).contains(n))
    }

    fn fen_problem(&self, cx: &App) -> Option<SharedString> {
        let text = self.fen.read(cx).value().trim().to_string();
        let variant = self.variant_key(cx);
        if variant == "chess960" || text.is_empty() {
            return None;
        }
        if variant != "standard" {
            return crate::game::setup_position(&variant, &text)
                .is_none()
                .then(|| "Not a legal position".into());
        }
        position_problem(&text).map(SharedString::from).or_else(|| {
            kchess_domain::rules::setup_start(kchess_domain::variant::Variant::Standard, &text)
                .is_none()
                .then(|| "Not a legal position".into())
        })
    }

    /// The chosen start, or `None` while the form has a problem.
    fn setup(&self, cx: &App) -> Option<Value> {
        let variant = self.variant_key(cx);
        if variant == "chess960" {
            return self
                .number960(cx)
                .map(|n| json!({ "variant": "chess960", "fen": chess960(n) }));
        }
        let text = self.fen.read(cx).value().trim().to_string();
        if variant == "standard" && !text.is_empty() {
            return self
                .fen_problem(cx)
                .is_none()
                .then(|| crate::game::setup_from_fen(&text));
        }
        if !text.is_empty() {
            return self
                .fen_problem(cx)
                .is_none()
                .then(|| json!({ "variant": variant, "fen": text }));
        }
        let fen = crate::game::default_fen(&variant)?;
        Some(json!({ "variant": variant, "fen": fen }))
    }

    /// The chosen clock: equal times unless Black's minutes give odds.
    fn clock(&self, cx: &App) -> Value {
        let row = self
            .clock
            .read(cx)
            .selected_index(cx)
            .map_or(0, |index| index.row);
        let Some((minutes, increment)) = crate::game::CLOCKS.get(row).and_then(|(_, clock)| *clock)
        else {
            return Value::Null;
        };
        let odds = self
            .odds
            .read(cx)
            .value()
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|m| *m >= 1.);
        json!({
            "white": { "minutes": minutes, "increment": increment },
            "black": { "minutes": odds.unwrap_or(minutes), "increment": increment },
        })
    }
}

impl Render for NewGame {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let variant = self.variant_key(cx);
        let hint = variants()
            .iter()
            .find(|text| text.key == variant)
            .map(|text| text.hint.clone());
        let setup = self.setup(cx);
        let no_clock = self
            .clock
            .read(cx)
            .selected_index(cx)
            .is_none_or(|index| index.row == 0);
        v_flex()
            .gap(px(16.))
            .child(
                widgets::field(
                    "Variant",
                    Select::new(&self.variant).accessibility_label("Choose a variant"),
                    None,
                    &palette,
                )
                .children(hint.map(|hint| {
                    div()
                        .text_size(px(13.))
                        .text_color(palette.text_muted)
                        .child(hint)
                })),
            )
            .when(variant == "chess960", |form| {
                form.child(
                    h_flex()
                        .items_end()
                        .gap(px(8.))
                        .child(
                            widgets::field(
                                "Position number (0–959)",
                                Input::new(&self.number).aria_label("Position number (0–959)"),
                                self.number960(cx)
                                    .is_none()
                                    .then(|| "Enter a number from 0 to 959".into()),
                                &palette,
                            )
                            .flex_1(),
                        )
                        .child(
                            Button::new("random-960")
                                .outline()
                                .icon(icon("shuffle"))
                                .label("Random")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let Some(n) =
                                        crate::game::drawn(kchess_games::random_chess960())
                                    else {
                                        return;
                                    };
                                    this.number.update(cx, |input, cx| {
                                        input.set_value(n.to_string(), window, cx)
                                    });
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when(variant != "chess960", |form| {
                form.child(widgets::field(
                    "Start from a FEN (optional)",
                    Input::new(&self.fen).aria_label("Start from a FEN (optional)"),
                    self.fen_problem(cx),
                    &palette,
                ))
            })
            .child(
                h_flex()
                    .gap(px(12.))
                    .items_start()
                    .child(
                        widgets::field(
                            "Clock",
                            Select::new(&self.clock).accessibility_label("Clock"),
                            None,
                            &palette,
                        )
                        .flex_1(),
                    )
                    .child(
                        widgets::field(
                            "Black's minutes (odds)",
                            Input::new(&self.odds)
                                .aria_label("Black's minutes (odds)")
                                .disabled(no_clock),
                            None,
                            &palette,
                        )
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(palette.text_muted)
                                .child("Leave empty for equal time"),
                        )
                        .flex_1(),
                    ),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("cancel-local")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("start-local")
                            .primary()
                            .label("Start")
                            .disabled(setup.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                let Some(setup) = this.setup(cx) else {
                                    return;
                                };
                                let clock = this.clock(cx);
                                window.close_dialog(cx);
                                (this.on_start)(setup, clock, window, cx);
                            })),
                    ),
            )
    }
}

/// A clock face's text, as the page shows it.
/// White's or Black's place in `LocalPage::faces`.
fn face(color: Color) -> usize {
    usize::from(color == Color::Black)
}

fn clock_face(millis: f64) -> String {
    format_clock(millis)
}

impl crate::pages::PageAction for Action {
    fn info(&self) -> crate::pages::ActionInfo {
        use crate::pages::{ActionInfo, ActionRole};
        match self {
            Action::TakeBack => ActionInfo::new("local:takeback", "Take back", "undo-2", "undo"),
            Action::Pause => ActionInfo::new("local:pause", "Pause", "pause", "clock stop"),
            Action::Resume => ActionInfo::new("local:resume", "Resume", "play", "clock continue"),
            Action::Draw => ActionInfo::new("local:draw", "Draw", "handshake", "agree"),
            Action::Resign => ActionInfo::new("local:resign", "Resign", "flag", "give up"),
            Action::NewGame => ActionInfo::new("local:new-game", "New game", "plus", "restart")
                .role(ActionRole::New),
            Action::Flip => ActionInfo::flip("local:flip"),
            Action::CopyPgn => {
                ActionInfo::new("local:copy-pgn", "Copy PGN", "copy", "game moves clipboard")
                    .role(ActionRole::Copy)
            }
            Action::Analyse => {
                ActionInfo::new("local:analyse", "Analyse", "microscope", "analysis review")
            }
        }
    }
}

impl crate::pages::PageView for LocalPage {
    const INFO: crate::pages::PageInfo = crate::pages::PageInfo {
        page: crate::pages::Page::Local,
        label: "Over the board",
        short: "Over the board",
        icon: "swords",
        group: Some(crate::pages::NavGroup::Play),
        keywords: "otb friend two players pass and play clock",
    };

    type Action = Action;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self {
        LocalPage::new(window, cx)
    }

    fn zen(&self, _: &App) -> bool {
        true
    }

    /// The chess clock alone keeps its keys for the clock.
    fn zen_key(&self, cx: &App) -> bool {
        self.mode(cx) != Mode::Clock
    }

    fn actions(&self, cx: &App) -> Vec<Action> {
        LocalPage::actions(self, cx)
    }

    fn act(this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        this.update(cx, |page, cx| page.act(action, window, cx));
    }
}

/// A game over the board from a setup's position. A position that cannot be played only opens
/// the page.
pub fn play_from(setup: Value, window: &mut Window, cx: &mut App) {
    if !crate::pages::computer::playable(&setup) {
        crate::shell::open(crate::pages::Page::Local, window, cx);
        return;
    }
    crate::shell::show::<LocalPage>(window, cx, move |page, window, cx| {
        page.start_from(setup, window, cx)
    });
}
