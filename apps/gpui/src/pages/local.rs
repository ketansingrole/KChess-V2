//! Over the board: two people at one
//! board, any variant, with or without a clock; or a chess clock for a real board. Both keep their
//! place in Game history, and the board game resumes in either app.

use std::time::{Duration, Instant};

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    Disableable as _, IndexPath, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_core::Level;
use kchess_domain::games::otb_clock::{Config, OtbClock, Side};
use serde_json::{Value, json};
use shakmaty::Color;

use crate::assets::icon;
use crate::board::{BoardEvent, ChessBoard};
use crate::core::AppCore;
use crate::export::{Exporter, GameExport};
use crate::game::history::HistoryPlace;
use crate::game::local::{Flagged, LocalGame};
use crate::game::{STANDARD_FEN, chess960, position_problem, rules, view};
use crate::part::Part;
use crate::play_layout::{Measured, PANEL_MAX, PANEL_MIN, PANEL_ZEN, PlayLayout, panel_style};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::{Palette, radii};
use crate::widgets::{self, KeyboardFocus as _, Side as PieceSide, Tone};

/// Clocks are checked this often while one runs (`useIntervalFn(…, 100)`).
const TICK: Duration = Duration::from_millis(100);
const SAVE_DELAY: Duration = Duration::from_millis(150);

/// The clocks the new-game form offers (`CLOCKS`).
const CLOCKS: &[(&str, Option<(f64, f64)>)] = &[
    ("No clock", None),
    ("3+2", Some((3., 2.))),
    ("5+0", Some((5., 0.))),
    ("5+3", Some((5., 3.))),
    ("10+0", Some((10., 0.))),
    ("15+10", Some((15., 10.))),
    ("30+0", Some((30., 0.))),
    ("90+30", Some((90., 30.))),
];

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

/// The variants, with their names and hints (`data/variants.json`).
#[derive(serde::Deserialize, Clone)]
struct VariantText {
    key: String,
    label: String,
    hint: String,
}

fn variants() -> Vec<VariantText> {
    serde_json::from_str(include_str!(
        "../../../../crates/kchess-domain/data/variants.json"
    ))
    .unwrap_or_default()
}

pub struct LocalPage {
    /// Either player's spoken moves.
    voice: crate::voice::moves::VoiceMoves,
    /// Turned clock faces as pictures, by side, with what they show.
    turned: std::collections::HashMap<usize, (String, std::sync::Arc<Image>)>,
    game: LocalGame,
    board: Entity<ChessBoard>,
    exporter: Entity<Exporter>,
    move_input: Entity<InputState>,
    move_message: Option<SharedString>,
    started: Instant,
    /// The move shown while looking back; `None` at the latest.
    shown: Option<usize>,
    fixed_orientation: Color,
    history: Option<HistoryPlace>,
    clock: OtbClock,
    clock_history: Option<HistoryPlace>,
    clock_inputs: [Entity<InputState>; 3],
    clock_layout: Entity<SelectState<Vec<SharedString>>>,
    save: Option<Task<()>>,
    ticker: Option<Task<()>>,
    clock_ticker: Option<Task<()>>,
    /// What the clocks showed at the last tick: a tick redraws only when a clock face changes
    /// (each second, each tenth in the last ten).
    ticked: Option<[String; 2]>,
    layout: PlayLayout,
    /// The side panel, measured for `.panel-actions`' narrow rule.
    panel_area: Measured,
    /// The side panel beside the board, a cached view of its own (`render_panel`).
    panel: Entity<Part<LocalPage>>,
    /// White's and Black's clock faces (`render_clock_face`): a tick redraws only these.
    faces: [Entity<Part<LocalPage>>; 2],
    /// Whether each clock is running, as the page last drew it.
    faces_active: [bool; 2],
    /// What the game's clock faces showed at its last tick (`ticked` is the chess clock's).
    faces_ticked: Option<[String; 2]>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for LocalPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl LocalPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> LocalPage {
        let started = Instant::now();
        // The library was read before the window opened; the board game resumes from it.
        let library = cx
            .global::<AppCore>()
            .bridge
            .call_now("library", Vec::new())
            .unwrap_or(Value::Null);
        let saved = &library["sessions"]["local"];
        let game = LocalGame::new((!saved.is_null()).then_some(saved), 0.)
            .or_else(|_| LocalGame::new(None, 0.));
        let game = match game {
            Ok(game) => game,
            Err(cause) => panic!("a new local game always starts: {cause}"),
        };
        let board = cx.new(|_| {
            ChessBoard::new(view::board_state(
                &game.state["setup"],
                &[],
                0,
                Color::White,
                None,
            ))
        });
        let move_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type a move: Nf3 or g1f3"));
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
        let layout_choice: ClockLayout =
            crate::prefs::get(cx, "kchess:clock-layout", ClockLayout::Horizontal);
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
                &move_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.typed_move(window, cx);
                    }
                },
            ),
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
                    crate::prefs::set(cx, "kchess:clock-layout", layout);
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
        let mut page = LocalPage {
            game,
            board,
            move_input,
            move_message: None,
            started,
            shown: None,
            fixed_orientation: Color::White,
            history: None,
            clock,
            clock_history: None,
            clock_inputs,
            turned: std::collections::HashMap::new(),
            voice: crate::voice::moves::VoiceMoves::new("local", cx),
            clock_layout,
            save: None,
            ticker: None,
            clock_ticker: None,
            ticked: None,
            layout: PlayLayout::default(),
            panel_area: Measured::default(),
            panel: Part::panel(|this, window, cx| this.render_panel(window, cx), cx),
            faces: [
                Part::new(|this, _, cx| this.render_clock_face(Color::White, cx), cx),
                Part::new(|this, _, cx| this.render_clock_face(Color::Black, cx), cx),
            ],
            faces_active: [false; 2],
            faces_ticked: None,
            exporter: cx.new(|_| Exporter::default()),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        let (source, has_play) = (page.game.archive_snapshot(0.), page.has_play());
        page.history = Some(HistoryPlace::new(
            "board", &library, true, source, has_play, cx,
        ));
        let clock_source = page.clock_snapshot();
        page.clock_history = Some(HistoryPlace::new(
            "clock",
            &library,
            false,
            clock_source,
            false,
            cx,
        ));
        page.after_change(cx);
        let weak = cx.entity().downgrade();
        crate::core::on_quit(cx, move |cx| {
            let page = weak.upgrade()?;
            let pending = page.update(cx, |page, cx| page.flush(cx));
            Some(crate::core::all_saved(pending))
        });
        page
    }

    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.
    }

    fn moves(&self) -> Vec<String> {
        serde_json::from_value(self.game.state["moves"].clone()).unwrap_or_default()
    }

    fn setup(&self) -> &Value {
        &self.game.state["setup"]
    }

    fn shown_ply(&self) -> usize {
        let count = self.moves().len();
        self.shown.unwrap_or(count).min(count)
    }

    fn at_live(&self) -> bool {
        self.shown_ply() == self.moves().len()
    }

    fn has_play(&self) -> bool {
        !self.moves().is_empty() || self.game.result(self.now()).is_some()
    }

    fn mode(&self, cx: &App) -> Mode {
        crate::prefs::get(cx, "kchess:local-mode", Mode::Board)
    }

    fn auto_flip(&self, cx: &App) -> bool {
        crate::prefs::get(cx, "kchess:local-autoflip", false)
    }

    /// Who is to move at the latest position.
    fn turn(&self) -> Color {
        view::turn(view::position_at(self.setup(), &self.moves(), self.moves().len()).as_ref())
    }

    /// The board faces whoever is to move while automatic turning is on.
    fn orientation(&self, cx: &App) -> Color {
        if self.auto_flip(cx) && !self.game.over(self.now()) {
            self.turn()
        } else {
            self.fixed_orientation
        }
    }

    fn flagged(&mut self, flagged: Result<Flagged, String>, cx: &mut Context<Self>) {
        match flagged {
            Ok(Flagged(true)) => crate::sound::play(crate::sound::Sound::LowTime, cx),
            Ok(Flagged(false)) => {}
            Err(cause) => crate::log::write(Level::Warn, "local", &cause),
        }
    }

    pub(crate) fn play(&mut self, uci: &str, _: &mut Window, cx: &mut Context<Self>) {
        if !self.at_live() {
            return self.after_change(cx);
        }
        let now = self.now();
        match self.game.play(uci, now) {
            Ok((Some(san), flagged)) => {
                self.shown = None;
                crate::sound::move_sound(&san, cx);
                self.flagged(Ok(flagged), cx);
            }
            Ok((None, flagged)) => self.flagged(Ok(flagged), cx),
            Err(cause) => crate::log::write(Level::Warn, "local", &cause),
        }
        self.changed(cx);
    }

    /// Spoken moves are taken while the game is on, at its last move.
    fn voice_taking(&self) -> bool {
        !self.game.over(self.now()) && self.at_live()
    }

    /// The live position as the rules read a spoken move in it.
    fn voice_setup(&self) -> Value {
        let moves = self.moves();
        view::position_at(self.setup(), &moves, moves.len()).map_or(Value::Null, |position| {
            json!({
                "variant": self.setup()["variant"],
                "fen": kchess_domain::rules::make_fen(&position),
            })
        })
    }

    fn sync_voice(&mut self, cx: &mut Context<Self>) {
        let context = format!("{}:{}", self.moves().len(), self.shown_ply());
        let taking = self.voice_taking();
        let line = view::sans(self.setup(), &self.moves());
        self.voice.sync(
            &context,
            Some(line),
            crate::voice::grammar("MOVE_GRAMMAR"),
            taking,
            cx,
        );
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
            self.play(&uci, window, cx);
        }
        cx.notify();
    }

    fn typed_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.move_input.read(cx).value().trim().to_string();
        if text.is_empty() || self.game.over(self.now()) || !self.at_live() {
            return;
        }
        let position = view::position_at(self.setup(), &self.moves(), self.moves().len());
        match position.and_then(|position| view::typed_uci(self.setup(), &position, &text)) {
            Some(uci) => {
                self.move_message = None;
                self.move_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.play(&uci, window, cx);
            }
            None => {
                self.move_message = Some("Enter a legal move, such as Nf3 or g1f3.".into());
                cx.notify();
            }
        }
    }

    /// The game changed: show it, keep the clock running, save it shortly, keep its history place.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.after_change(cx);
        self.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let save = this.update(cx, |this, cx| this.save_now(cx)).ok();
            if let Some(save) = save {
                let _ = save.await;
            }
        }));
        let (source, has_play) = (self.game.archive_snapshot(self.now()), self.has_play());
        if let Some(history) = &mut self.history {
            history.save(false, source, has_play, cx);
        }
    }

    fn save_now(&mut self, cx: &mut Context<Self>) -> crate::core::BoxedSave {
        self.save = None;
        let snapshot = self.game.snapshot(self.now());
        let save = cx
            .global::<AppCore>()
            .call("saveSession", vec![json!("local"), snapshot]);
        Box::pin(async move {
            save.await.map(|_| ()).map_err(|cause| {
                crate::log::write(
                    Level::Warn,
                    "library",
                    &format!("saving session failed: local {}", cause.message),
                );
                format!("The game could not be saved: {}", cause.message)
            })
        })
    }

    /// Everything to finish before the app quits: the board game, the clock session and history.
    fn flush(&mut self, cx: &mut Context<Self>) -> Vec<crate::core::BoxedSave> {
        let mut pending = vec![self.save_now(cx)];
        let now = self.now();
        self.clock.pause(now);
        let (source, has_play) = (self.clock_snapshot(), self.clock.has_play());
        if let Some(history) = &mut self.clock_history {
            history.save(true, source, has_play, cx);
            pending.push(history.flushed(cx));
        }
        if let Some(history) = &self.history {
            pending.push(history.flushed(cx));
        }
        pending
    }

    fn after_change(&mut self, cx: &mut Context<Self>) {
        self.sync_voice(cx);
        let now = self.now();
        let over = self.game.over(now);
        let moves = self.moves();
        let shown = self.shown_ply();
        let live = !over && self.at_live();
        let turn = self.turn();
        let mut state = view::board_state(
            self.setup(),
            &moves,
            shown,
            self.orientation(cx),
            live.then_some(turn),
        );
        state.blindfold = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .blindfold
            && !over;
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        let running = !self.game.state["times"].is_null() && !over;
        if running && self.ticker.is_none() {
            self.ticker = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(TICK).await;
                    let alive = this.update(cx, |this, cx| {
                        let now = this.now();
                        let flagged = this.game.expire(now);
                        let fell = matches!(flagged, Ok(Flagged(true)));
                        this.flagged(flagged, cx);
                        if fell {
                            this.ticker = None;
                            this.changed(cx);
                        }
                        let shown = ["white", "black"]
                            .map(|side| clock_face(this.game.remaining(side, now)));
                        this.redraw_faces_if_ticked(shown, cx);
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

    fn show_ply(&mut self, ply: usize, cx: &mut Context<Self>) {
        let count = self.moves().len();
        let clamped = ply.min(count);
        self.shown = (clamped != count).then_some(clamped);
        self.after_change(cx);
    }

    /// Flipping by hand turns the board, and stops the automatic turning.
    fn flip(&mut self, cx: &mut Context<Self>) {
        let shown = self.orientation(cx);
        crate::prefs::set(cx, "kchess:local-autoflip", false);
        self.fixed_orientation = !shown;
        self.after_change(cx);
    }

    fn set_auto_flip(&mut self, on: bool, cx: &mut Context<Self>) {
        // Turning it off leaves the board facing the way it is now.
        if !on && self.auto_flip(cx) && !self.game.over(self.now()) {
            self.fixed_orientation = self.turn();
        }
        crate::prefs::set(cx, "kchess:local-autoflip", on);
        self.after_change(cx);
    }

    fn start(&mut self, setup: Value, clock: Value, cx: &mut Context<Self>) {
        let (source, has_play) = (self.game.archive_snapshot(self.now()), self.has_play());
        if let Some(history) = &mut self.history {
            history.reset(source, has_play, cx);
        }
        let flagged = self.game.start(setup, clock, self.now());
        self.flagged(flagged, cx);
        self.shown = None;
        self.fixed_orientation = Color::White;
        self.changed(cx);
    }

    fn takeback(&mut self, cx: &mut Context<Self>) {
        let flagged = self.game.takeback(self.now());
        self.flagged(flagged, cx);
        self.shown = None;
        self.changed(cx);
    }

    fn resign(&mut self, color: Color, cx: &mut Context<Self>) {
        let flagged = self.game.resign(
            if color == Color::White {
                "white"
            } else {
                "black"
            },
            self.now(),
        );
        self.flagged(flagged, cx);
        self.changed(cx);
    }

    fn agree_draw(&mut self, cx: &mut Context<Self>) {
        let flagged = self.game.agree_draw(self.now());
        self.flagged(flagged, cx);
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
        window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
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
        let form = cx.new(|cx| {
            NewGame::new(
                move |setup, clock, window, cx| {
                    let _ = page.update(cx, |page, cx| page.replace_game(setup, clock, window, cx));
                },
                window,
                cx,
            )
        });
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("New game over the board")
                .w(px(512.))
                .child(form.clone())
        });
    }

    /// A game over the board from a position (the board editor's, the analysis board's), untimed.
    pub fn start_from(&mut self, fen: &str, window: &mut Window, cx: &mut Context<Self>) {
        crate::prefs::set(cx, "kchess:local-mode", Mode::Board);
        self.replace_game(
            json!({ "variant": "standard", "fen": fen }),
            Value::Null,
            window,
            cx,
        );
    }

    /// Start a game, asking first when one on the board is still being played.
    pub(crate) fn replace_game(
        &mut self,
        setup: Value,
        clock: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.moves().is_empty() || self.game.over(self.now()) {
            return self.start(setup, clock, cx);
        }
        let page = cx.entity().downgrade();
        let chosen = std::rc::Rc::new((setup, clock));
        window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
            let (page, chosen) = (page.clone(), chosen.clone());
            dialog
                .title("Start a new game?")
                .description("The game on the board is still in progress and will be lost.")
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

    fn pgn_result(&self) -> &'static str {
        match self.game.result(self.now()) {
            None => "*",
            Some(result) => match result["winner"].as_str() {
                Some("white") => "1-0",
                Some("black") => "0-1",
                _ => "1/2-1/2",
            },
        }
    }

    /// The game an Export saves, between "White" and "Black".
    fn game_export(&self, cx: &App) -> GameExport {
        let setup = self.setup();
        GameExport {
            variant: setup["variant"].as_str().unwrap_or("standard").to_string(),
            fen: setup["fen"].as_str().unwrap_or_default().to_string(),
            moves: self.moves(),
            orientation: self.orientation(cx),
            white: "White".into(),
            black: "Black".into(),
            ply: None,
            result: Some(self.pgn_result().into()),
            lichess_id: None,
        }
    }

    /// Analyse the game on the analysis board at the position shown (standard rules only).
    fn analyse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pgn = rules(
            "setupPgn",
            &[self.setup().clone(), json!(self.moves()), json!({})],
        );
        let Ok(Value::String(pgn)) = pgn else { return };
        let ply = self.shown_ply();
        let origin = json!({ "white": "White", "black": "Black" });
        crate::shell::replace_analysis(window, cx, move |analysis, _, cx| {
            analysis.open_game(&pgn, ply, None, Some(origin.clone()), false, cx)
        });
    }

    fn copy_pgn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let today = rules("calendarToday", &[]).ok();
        let date = today
            .as_ref()
            .and_then(Value::as_str)
            .map(|day| day.replace('-', "."))
            .unwrap_or_else(|| "????.??.??".into());
        let headers = json!({
            "Event": "Over the board (KChess)",
            "Date": date,
            "White": "White",
            "Black": "Black",
            "Result": self.pgn_result(),
        });
        match rules(
            "setupPgn",
            &[self.setup().clone(), json!(self.moves()), headers],
        ) {
            Ok(Value::String(pgn)) => {
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
            self.clock_ticker = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(TICK).await;
                    let alive = this.update(cx, |this, cx| {
                        let now = this.now();
                        if this.clock.expire(now) {
                            crate::sound::play(crate::sound::Sound::LowTime, cx);
                            this.clock_flagged(cx);
                        }
                        let shown = [Side::Top, Side::Bottom]
                            .map(|side| clock_face(this.clock.left(side, now)));
                        this.redraw_if_ticked(shown, cx);
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            }));
        } else if self.clock.running.is_none() {
            self.clock_ticker = None;
        }
        cx.notify();
    }

    /// A tick redraws the page only when a clock face shows something else.
    fn redraw_if_ticked(&mut self, shown: [String; 2], cx: &mut Context<Self>) {
        if self.ticked.as_ref() != Some(&shown) {
            self.ticked = Some(shown);
            cx.notify();
        }
    }

    /// A game's tick redraws a clock face beside the board only when it shows something else,
    /// and only that face.
    fn redraw_faces_if_ticked(&mut self, shown: [String; 2], cx: &mut Context<Self>) {
        let before = self.faces_ticked.replace(shown.clone());
        for (ix, face) in self.faces.iter().enumerate() {
            if before.as_ref().map(|before| &before[ix]) != Some(&shown[ix]) {
                face.update(cx, |_, cx| cx.notify());
            }
        }
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

    /// Keys when the page itself has focus (not a text field): in the clock, Space presses the
    /// running clock and P pauses; on the board, the arrows step through the moves.
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) || event.keystroke.modifiers.modified() {
            return;
        }
        let key = event.keystroke.key.as_str();
        if self.mode(cx) != Mode::Clock && crate::shell::zen_key(key, cx) {
            cx.stop_propagation();
            return;
        }
        if self.mode(cx) == Mode::Clock {
            match key {
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
        } else if let Some(ply) = view::navigate(key, self.shown_ply(), self.moves().len()) {
            self.show_ply(ply, cx);
        } else {
            return;
        }
        cx.stop_propagation();
    }

    /// How far a clock face is turned (`kchess:clock-rotation`), so a player across the table
    /// reads it the right way up.
    fn rotation(side: Side, cx: &App) -> u16 {
        let turned: serde_json::Value = crate::prefs::get(
            cx,
            "kchess:clock-rotation",
            json!({ "top": 0, "bottom": 0 }),
        );
        let key = if side == Side::Top { "top" } else { "bottom" };
        (turned[key].as_f64().unwrap_or_default() as u16 / 90 * 90) % 360
    }

    /// A quarter turn clockwise (`rotateClock`).
    fn rotate(side: Side, cx: &mut App) {
        let mut turned: serde_json::Value = crate::prefs::get(
            cx,
            "kchess:clock-rotation",
            json!({ "top": 0, "bottom": 0 }),
        );
        let key = if side == Side::Top { "top" } else { "bottom" };
        turned[key] = json!((Self::rotation(side, cx) + 90) % 360);
        crate::prefs::set(cx, "kchess:clock-rotation", turned);
    }

    fn render_clock(
        &mut self,
        palette: &Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = self.now();
        let stacked = crate::prefs::get(cx, "kchess:clock-layout", ClockLayout::Horizontal)
            == ClockLayout::Vertical;
        let mut face = |side: Side, index: usize, cx: &mut Context<Self>| {
            let running = self.clock.running == Some(side);
            let flagged = self.clock.flagged == Some(side);
            let time = rules("formatClock", &[json!(self.clock.left(side, now))])
                .ok()
                .and_then(|text| text.as_str().map(str::to_string))
                .unwrap_or_default();
            let state = if flagged {
                "Time up"
            } else if running {
                "Your turn"
            } else if self.clock.running.is_some() {
                "Waiting"
            } else {
                "Ready"
            };
            let (background, foreground) = if flagged {
                (palette.error, palette.bg)
            } else if running {
                (palette.primary, palette.bg)
            } else {
                (palette.elevated, palette.text)
            };
            let moves = self.clock.moves.get(side);
            let rotation = Self::rotation(side, cx);
            let turned = (rotation != 0).then(|| {
                let lines = [
                    (format!("Player {index}"), 14., true, 0.75),
                    (time.clone(), 120., true, 1.),
                    (format!("{moves} moves"), 14., false, 0.75),
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
                    _ => crate::game_image::rotated_text_png(
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
                    .map(|png| {
                        let image = std::sync::Arc::new(Image::from_bytes(ImageFormat::Png, png));
                        self.turned.insert(index, (key, image.clone()));
                        image
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
                        .font_weight(FontWeight::SEMIBOLD)
                        .opacity(0.75)
                        .child(format!("Player {index}")),
                )
                .child(
                    div()
                        .text_size(px(120.))
                        .font_weight(FontWeight::BOLD)
                        .line_height(px(120.))
                        .child(time.clone()),
                )
                .child(
                    div()
                        .text_size(px(14.))
                        .opacity(0.75)
                        .child(format!("{moves} moves")),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
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
        let faces = if stacked {
            v_flex().flex_1().gap(px(16.)).min_h(px(480.))
        } else {
            h_flex()
                .flex_1()
                .items_stretch()
                .gap(px(16.))
                .min_h(px(300.))
        }
        .child(face(Side::Top, 1, cx))
        .child(face(Side::Bottom, 2, cx));
        let [top_minutes, bottom_minutes, increment] = &self.clock_inputs;
        let small_input =
            |input: &Entity<InputState>| div().w(px(64.)).child(Input::new(input).small());
        v_flex()
            .id("local-clock")
            .size_full()
            .gap(px(16.))
            .pt(crate::play_layout::PAGE_TOP)
            .pb(crate::play_layout::PAGE_BOTTOM)
            .px(crate::play_layout::side_padding(window))
            // The page header holds the tabs at the right; the clock's own controls sit below.
            .child(
                h_flex()
                    .justify_end()
                    .child(div().w(px(420.)).child(mode_tabs(Mode::Clock, palette, cx))),
            )
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(palette.text_muted)
                            .child("Layout"),
                    )
                    .child(
                        div().w(px(144.)).child(
                            Select::new(&self.clock_layout)
                                .small()
                                .accessibility_label("Clock layout"),
                        ),
                    )
                    .child(div().flex_1())
                    .children(
                        [(Side::Top, 1usize), (Side::Bottom, 2)].map(|(side, index)| {
                            let degrees = Self::rotation(side, cx);
                            Button::new(("clock-rotate", index))
                                .small()
                                .outline()
                                .icon(icon("rotate-cw"))
                                .label(format!("Player {index} · {degrees}°"))
                                .accessibility_label(format!(
                                    "Rotate Player {index} clock, currently {degrees} degrees"
                                ))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    Self::rotate(side, cx);
                                    cx.notify();
                                }))
                        }),
                    ),
            )
            .child(faces)
            .child(
                h_flex()
                    .justify_center()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        Button::new("clock-start")
                            .outline()
                            .small()
                            .icon(if self.clock.running.is_some() {
                                icon("pause")
                            } else {
                                icon("play")
                            })
                            .label(if self.clock.running.is_some() {
                                "Pause"
                            } else {
                                "Start"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.start_or_pause(cx))),
                    )
                    .child(
                        Button::new("clock-reset")
                            .outline()
                            .small()
                            .icon(icon("rotate-ccw"))
                            .label("Reset")
                            .on_click(cx.listener(|this, _, _, cx| this.reset_clock(cx))),
                    )
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .text_size(px(14.))
                            .child(small_input(top_minutes))
                            .child(div().text_color(palette.text_muted).child("/"))
                            .child(small_input(bottom_minutes))
                            .child("min +")
                            .child(small_input(increment))
                            .child("s"),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .text_center()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child("Space switches turns · P pauses"),
            )
            .into_any_element()
    }
}

/// "Shared board" and "Chess clock".
fn mode_tabs(mode: Mode, palette: &Palette, cx: &mut Context<LocalPage>) -> impl IntoElement {
    let page = cx.entity().downgrade();
    widgets::pill_tabs(
        "local-mode",
        vec![
            (Some(icon("users-round")), "Shared board".into()),
            (Some(icon("timer")), "Chess clock".into()),
        ],
        usize::from(mode == Mode::Clock),
        move |index, _, cx| {
            let mode = if index == 1 { Mode::Clock } else { Mode::Board };
            crate::prefs::set(cx, "kchess:local-mode", mode);
            let _ = page.update(cx, |page, cx| page.after_change(cx));
        },
        palette,
    )
}

impl Render for LocalPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        if self.mode(cx) == Mode::Clock {
            let clock = self.render_clock(&palette, window, cx);
            return div()
                .size_full()
                .track_focus(&self.focus)
                .on_key_down(cx.listener(Self::key))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)),
                )
                .child(clock)
                .into_any_element();
        }
        let now = self.now();
        let board_size = self.layout.board_size(window);
        let moves = self.moves();
        let shown = self.shown_ply();
        let over = self.game.over(now);
        let live = self.game.result(now).is_none();
        let orientation = self.orientation(cx);
        let display = view::position_at(self.setup(), &moves, shown);
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
        let timed = !self.game.state["times"].is_null();
        let paused = self.game.state["paused"].as_bool() == Some(true);
        self.faces_active = [Color::White, Color::Black]
            .map(|color| live && display_turn == color && timed && !paused);
        let line = |color: Color, middle: Option<AnyElement>| {
            let name = if color == Color::White {
                "White"
            } else {
                "Black"
            };
            let active = live && display_turn == color;
            widgets::player_line(
                h_flex()
                    .gap(px(10.))
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
                    .children(widgets::material(
                        &fen,
                        color == Color::White,
                        &frontend.piece_set,
                        &palette,
                    )),
                middle,
                timed.then(|| self.faces[face(color)].clone().into_any_element()),
            )
        };
        let interactive = !over && self.at_live();
        let entry = widgets::move_entry(&self.move_input, interactive, &palette);
        let voice = crate::voice::moves::board_bar(
            &self.voice,
            cx.entity().downgrade(),
            crate::voice::moves::Access {
                write: |this: &mut Self| &mut this.voice,
                play: |this: &mut Self, uci, window, cx| this.play(uci, window, cx),
            },
            crate::voice::moves::BoardBar {
                id: "local-voice",
                hint: "Either player can say “E four” or “Knight F three”.",
                active: self.voice_taking(),
                listening: self.voice_taking(),
                grammar: crate::voice::grammar("MOVE_GRAMMAR"),
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
            .children(self.move_message.clone().map(|message| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(message)
            }));

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
        self.layout
            .render("local-page", board_column, panel, window, cx)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .into_any_element()
    }
}

impl LocalPage {
    /// The panel beside (or under) the board: the game's setup and status, its moves and the
    /// actions. Beside the board it is the page's `Part`, kept while only a clock face redraws.
    fn render_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = *cx.global::<Palette>();
        let now = self.now();
        let board_size = self.layout.board_size(window);
        let narrow = widgets::is_narrow(self.panel_area.size().width);
        let moves = self.moves();
        let sans = view::sans(self.setup(), &moves);
        let shown = self.shown_ply();
        let over = self.game.over(now);
        let result = self.game.result(now);
        let display = view::position_at(self.setup(), &moves, shown);
        let frontend = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .clone();
        let timed = !self.game.state["times"].is_null();
        let paused = self.game.state["paused"].as_bool() == Some(true);
        let variant_label = {
            let variant = self.setup()["variant"].as_str().unwrap_or("standard");
            if variant == "standard" {
                (self.setup()["fen"].as_str() != Some(STANDARD_FEN))
                    .then(|| "From position".to_string())
            } else {
                variants()
                    .into_iter()
                    .find(|text| text.key == variant)
                    .map(|text| text.label)
            }
        };
        let clock_label = self.game.state["clock"].as_object().map(|clock| {
            let white = &clock["white"];
            let black = &clock["black"];
            let mut label = format!(
                "{}+{}",
                white["minutes"].as_f64().unwrap_or_default(),
                white["increment"].as_f64().unwrap_or_default()
            );
            if black["minutes"] != white["minutes"] {
                label.push_str(&format!(
                    " (Black {} min)",
                    black["minutes"].as_f64().unwrap_or_default()
                ));
            }
            label
        });
        let banner = match &result {
            Some(result) => {
                let winner = result["winner"].as_str();
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
                        Some("white") => "White wins".into(),
                        Some(_) => "Black wins".into(),
                        None => "Draw".into(),
                    },
                )
                .announce()
                .when_some(
                    result["reason"]
                        .as_str()
                        .map(|reason| SharedString::from(reason.to_string())),
                    |banner, value| banner.detail(value),
                )
            }
            None if !self.at_live() => widgets::StatusBanner::new(
                Tone::Plain,
                icon("history"),
                "Reviewing an earlier position".into(),
            )
            .detail("Go to the last move to keep playing.".into()),
            None => {
                let check = display.as_ref().is_some_and(shakmaty::Position::is_check);
                let detail = if timed && paused && !moves.is_empty() {
                    Some("Clock paused".into())
                } else if check {
                    Some("Check!".into())
                } else {
                    None
                };
                widgets::StatusBanner::new(
                    Tone::Plain,
                    icon("mouse-pointer-click"),
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
                let opening = rules(
                    "openingAt",
                    &[self.setup().clone(), json!(moves), json!(shown)],
                )
                .ok()?;
                Some((
                    opening["eco"].as_str()?.to_string(),
                    opening["name"].as_str()?.to_string(),
                ))
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
            .child(mode_tabs(Mode::Board, &palette, cx))
            .child(
                h_flex()
                    .gap(px(8.))
                    .flex_wrap()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .children(
                        variant_label
                            .map(|label| div().font_weight(FontWeight::SEMIBOLD).child(label)),
                    )
                    .children(clock_label)
                    .child(div().flex_1())
                    .child(
                        Switch::new("auto-flip")
                            .xsmall()
                            .checked(auto_flip)
                            .label("Flip after each move")
                            .on_click(
                                cx.listener(|this, on: &bool, _, cx| this.set_auto_flip(*on, cx)),
                            ),
                    )
                    .child(widgets::view_toggles(frontend.blindfold, frontend.zen_mode)),
            )
            .children(opening.map(|opening| {
                div()
                    .h(px(20.))
                    .text_size(px(14.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .children(opening.map(|(eco, name)| {
                        h_flex()
                            .gap(px(4.))
                            .child(div().text_color(palette.text_muted).child(eco))
                            .child(name)
                    }))
            }))
            .child(banner)
            .child(widgets::section_title(
                "Moves",
                (!moves.is_empty()).then(|| {
                    let count = moves.len().div_ceil(2);
                    format!("{count} {}", if count == 1 { "move" } else { "moves" })
                }),
                &palette,
            ))
            .child(div().flex_1().min_h(px(120.)).child(widgets::moves_list(
                "local-moves",
                &sans,
                shown,
                view::black_first(self.setup()),
                "White moves first. The first move starts the clock.".into(),
                select.clone(),
                &palette,
            )))
            .child(widgets::move_controls(
                shown,
                moves.len(),
                select,
                ("Flip board".into(), flip),
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
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_new_game(window, cx)),
                            ),
                    ),
            )
            .when(!frontend.zen_mode, |panel| {
                let standard = self.setup()["variant"].as_str().unwrap_or("standard") == "standard";
                // `.panel-actions`: Copy PGN, Export and Analyse share the row.
                panel.child(widgets::panel_actions(
                    narrow,
                    [
                        Button::new("copy-pgn")
                            .ghost()
                            .xsmall()
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
                            self.game_export(cx),
                            Anchor::BottomCenter,
                            cx,
                        )
                        .into_any_element()
                        .into(),
                    ]
                    .into_iter()
                    .chain(standard.then(|| {
                        Button::new("local-analyse")
                            .ghost()
                            .xsmall()
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
        let name = if color == Color::White {
            "white"
        } else {
            "black"
        };
        let text = clock_face(self.game.remaining(name, self.now()));
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
    /// A clock ran out: taking back cannot refill it, so the game stays lost on time.
    fn timed_out(&self) -> bool {
        let times = &self.game.state["times"];
        !times.is_null()
            && (times["white"].as_f64() == Some(0.) || times["black"].as_f64() == Some(0.))
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
        on_start: impl Fn(Value, Value, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> NewGame {
        let texts = variants();
        let variant = cx.new(|cx| {
            SelectState::new(
                texts
                    .iter()
                    .map(|text| SharedString::from(text.label.clone()))
                    .collect(),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let number = cx.new(|cx| {
            let mut input = InputState::new(window, cx);
            input.set_value(random_960().to_string(), window, cx);
            input
        });
        let fen = cx.new(|cx| InputState::new(window, cx).placeholder("Standard start"));
        let clock = cx.new(|cx| {
            SelectState::new(
                CLOCKS
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
            cx.subscribe(&variant, |_, _, _: &SelectEvent<Vec<SharedString>>, cx| {
                cx.notify()
            }),
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
        if self.variant_key(cx) != "standard" || text.is_empty() {
            return None;
        }
        position_problem(&text).map(SharedString::from).or_else(|| {
            kchess_domain::rules::setup_start("standard", &text)
                .is_none()
                .then(|| "Not a legal position.".into())
        })
    }

    /// The chosen start (`chosenSetup`), or `None` while the form has a problem.
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
                .then(|| json!({ "variant": "standard", "fen": text }));
        }
        let fen = rules("defaultFen", &[json!(variant)]).ok()?;
        Some(json!({ "variant": variant, "fen": fen }))
    }

    /// The chosen clock (`chosenClock`): equal times unless Black's minutes give odds.
    fn clock(&self, cx: &App) -> Value {
        let row = self
            .clock
            .read(cx)
            .selected_index(cx)
            .map_or(0, |index| index.row);
        let Some((minutes, increment)) = CLOCKS.get(row).and_then(|(_, clock)| *clock) else {
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

fn random_960() -> u16 {
    let mut bytes = [0u8; 2];
    if getrandom::fill(&mut bytes).is_err() {
        return 518;
    }
    u16::from_le_bytes(bytes) % 960
}

impl Render for NewGame {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let variant = self.variant_key(cx);
        let hint = variants()
            .into_iter()
            .find(|text| text.key == variant)
            .map(|text| text.hint);
        let setup = self.setup(cx);
        let no_clock = self
            .clock
            .read(cx)
            .selected_index(cx)
            .is_none_or(|index| index.row == 0);
        v_flex()
            .gap(px(16.))
            .child(
                widgets::field("Variant", Select::new(&self.variant), None, &palette).children(
                    hint.map(|hint| {
                        div()
                            .text_size(px(13.))
                            .text_color(palette.text_muted)
                            .child(hint)
                    }),
                ),
            )
            .when(variant == "chess960", |form| {
                form.child(
                    h_flex()
                        .items_end()
                        .gap(px(8.))
                        .child(
                            widgets::field(
                                "Position number (0–959)",
                                Input::new(&self.number),
                                self.number960(cx)
                                    .is_none()
                                    .then(|| "Enter a number from 0 to 959.".into()),
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
                                    this.number.update(cx, |input, cx| {
                                        input.set_value(random_960().to_string(), window, cx)
                                    });
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when(variant == "standard", |form| {
                form.child(widgets::field(
                    "Start from a FEN (optional)",
                    Input::new(&self.fen),
                    self.fen_problem(cx),
                    &palette,
                ))
            })
            .child(
                h_flex()
                    .gap(px(12.))
                    .items_start()
                    .child(
                        widgets::field("Clock", Select::new(&self.clock), None, &palette).flex_1(),
                    )
                    .child(
                        widgets::field(
                            "Black's minutes (odds)",
                            Input::new(&self.odds).disabled(no_clock),
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

/// A clock face's text (`formatClock`), as the page shows it.
/// White's or Black's place in `LocalPage::faces`.
fn face(color: Color) -> usize {
    usize::from(color == Color::Black)
}

fn clock_face(millis: f64) -> String {
    rules("formatClock", &[json!(millis)])
        .ok()
        .and_then(|text| text.as_str().map(str::to_string))
        .unwrap_or_default()
}
