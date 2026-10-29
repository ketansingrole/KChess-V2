//! Board coordinates, the way Lichess trains them: find a square that is
//! named, or name one shown on the board. A timed run keeps a best score per mode and length; untimed
//! practice, or practising only some files and ranks, keeps none.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::CoordinateMode;
use serde_json::{Value, json};
use shakmaty::{Color, Square};

use super::run::{self, RunClock};
use crate::assets::icon;
use crate::board::{BoardEvent, BoardState, ChessBoard, Circle};
use crate::game::{STANDARD_FEN, random_unit, rules};
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::settings::SettingsHandle;
use crate::theme::Palette;
use crate::voice::control::{BarSpec, Control, DrillLog};
use crate::voice::{Attempt, VoiceEvent};
use crate::widgets::{self, Pill, Tone};

const FILES: [&str; 8] = ["a", "b", "c", "d", "e", "f", "g", "h"];
const RANKS: [&str; 8] = ["1", "2", "3", "4", "5", "6", "7", "8"];
const EMPTY_FEN: &str = "8/8/8/8/8/8/8/8 w - - 0 1";
const DEFAULT_SECONDS: u32 = 30;
const FLASH: Duration = Duration::from_millis(350);

const LENGTH: &str = "kchess:coordinates-length";
const SHOW: &str = "kchess:coordinates-show";
const ON_SQUARES: &str = "kchess:coordinates-on-squares";
const PIECES: &str = "kchess:coordinates-pieces";
const LIMITED: &str = "kchess:coordinates-limited";
const STORED_FILES: &str = "kchess:coordinates-files";
const STORED_RANKS: &str = "kchess:coordinates-ranks";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Find,
    Name,
    Voice,
}

const MODES: [Mode; 3] = [Mode::Find, Mode::Name, Mode::Voice];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Running,
    Over,
}

/// `randomSquare`: a square that is not `previous`, from `allowed` when given; the rules pick from
/// the draws in order, and want one more draw each time the first ones give `previous`.
pub fn random_square(previous: Option<&str>, allowed: Option<&[String]>) -> String {
    let mut draws: Vec<f64> = Vec::new();
    loop {
        draws.push(random_unit());
        if let Ok(square) = rules(
            "randomSquare",
            &[json!(previous), json!(draws), json!(allowed)],
        ) && let Some(square) = square.as_str()
        {
            return square.to_string();
        }
        if draws.len() > 64 {
            return "e4".into();
        }
    }
}

/// White text over the board, readable on light and dark squares alike: GPUI text has no shadow,
/// so dark copies set just around it stand in for `.coord-prompt`'s text shadow.
fn shadowed(text: String, size: Pixels, weight: FontWeight, opacity: f32) -> impl IntoElement {
    let copy = |dx: f32, dy: f32, alpha: f32| {
        div()
            .absolute()
            .top(px(dy))
            .left(px(dx))
            .text_color(gpui_kit::black().opacity(alpha))
            .child(text.clone())
    };
    div()
        .relative()
        .text_size(size)
        .font_weight(weight)
        .child(copy(0., 2., 0.45))
        .child(copy(-1., 0., 0.35))
        .child(copy(1., 0., 0.35))
        .child(copy(0., -1., 0.35))
        .child(
            div()
                .relative()
                .text_color(gpui_kit::white().opacity(opacity))
                .child(text.clone()),
        )
}

/// The stored choice, in board order; all of them when nothing valid was stored.
fn chosen(all: &[&str], stored: &[String]) -> Vec<String> {
    let list: Vec<String> = all
        .iter()
        .filter(|item| stored.iter().any(|stored| stored == *item))
        .map(|item| (*item).to_string())
        .collect();
    if list.is_empty() {
        all.iter().map(|item| (*item).to_string()).collect()
    } else {
        list
    }
}

/// Turn one file or rank on or off, always keeping at least one.
fn toggled(list: &[String], item: &str) -> Vec<String> {
    if !list.iter().any(|entry| entry == item) {
        let mut next = list.to_vec();
        next.push(item.to_string());
        next
    } else if list.len() > 1 {
        list.iter()
            .filter(|entry| *entry != item)
            .cloned()
            .collect()
    } else {
        list.to_vec()
    }
}

pub struct Coordinates {
    mode: Mode,
    stage: Stage,
    side: &'static str,
    orientation: Color,
    target: String,
    upcoming: String,
    score: u32,
    mistakes: u32,
    pending_file: Option<String>,
    flash: Option<(String, bool)>,
    run_limited: bool,
    clock: RunClock,
    summary: Value,
    saved: Value,
    board: Entity<ChessBoard>,
    layout: PlayLayout,
    timers: Vec<Task<()>>,
    /// Say the square: the voice switch, its log, and a start waiting for voice to load.
    voice: Control,
    voice_log: DrillLog,
    starting: bool,
    /// The run and the question within it, which name what each phrase answered.
    run_epoch: u32,
    question: u32,
    _subscriptions: Vec<Subscription>,
}

impl Coordinates {
    pub fn new(cx: &mut Context<Self>) -> Coordinates {
        let board = cx.new(|_| {
            ChessBoard::new(BoardState::of(
                &kchess_domain::rules::setup_start("standard", STANDARD_FEN).unwrap_or_default(),
                Color::White,
                None,
                None,
            ))
        });
        let voice = crate::voice::handle(cx);
        let subscriptions = vec![
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Select(square) = event
                    && this.mode == Mode::Find
                {
                    this.answer(&square.to_string(), cx);
                }
            }),
            cx.subscribe(&voice, |this, _, event: &VoiceEvent, cx| {
                this.voice_heard(event, cx)
            }),
            cx.observe(&voice, |this, _, cx| {
                if this.starting && this.voice.gate(&mut this.starting, cx) {
                    this.start(cx);
                }
                cx.notify();
            }),
        ];
        let mut drill = Coordinates {
            mode: Mode::Find,
            stage: Stage::Idle,
            side: "white",
            orientation: Color::White,
            target: "e4".into(),
            upcoming: "d5".into(),
            score: 0,
            mistakes: 0,
            pending_file: None,
            flash: None,
            run_limited: false,
            clock: RunClock::default(),
            summary: Value::Null,
            saved: Value::Null,
            board,
            layout: PlayLayout::default(),
            timers: Vec::new(),
            voice: Control::new(false, cx),
            voice_log: DrillLog::default(),
            starting: false,
            run_epoch: 0,
            question: 0,
            _subscriptions: subscriptions,
        };
        run::load_summary("coordinates", cx, |this, summary, cx| {
            this.summary = summary;
            cx.notify();
        });
        drill.redraw(cx);
        drill
    }

    fn length(cx: &App) -> u32 {
        crate::prefs::get(cx, LENGTH, DEFAULT_SECONDS)
    }

    fn files(cx: &App) -> Vec<String> {
        chosen(
            &FILES,
            &crate::prefs::get::<Vec<String>>(cx, STORED_FILES, Vec::new()),
        )
    }

    fn ranks(cx: &App) -> Vec<String> {
        chosen(
            &RANKS,
            &crate::prefs::get::<Vec<String>>(cx, STORED_RANKS, Vec::new()),
        )
    }

    /// The squares a run asks for: all of them, or those on the chosen files and ranks.
    fn allowed(cx: &App) -> Option<Vec<String>> {
        crate::prefs::get(cx, LIMITED, false).then(|| {
            let (files, ranks) = (Self::files(cx), Self::ranks(cx));
            files
                .iter()
                .flat_map(|file| ranks.iter().map(move |rank| format!("{file}{rank}")))
                .collect()
        })
    }

    fn record_variant(&self, cx: &App) -> String {
        let base = match self.mode {
            Mode::Find => "find",
            Mode::Name => "name",
            Mode::Voice => "name-voice",
        };
        run::record_variant(base, Self::length(cx), DEFAULT_SECONDS)
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.stage == Stage::Running
            || (self.mode == Mode::Voice && !self.voice.gate(&mut self.starting, cx))
        {
            cx.notify();
            return;
        }
        self.run_epoch += 1;
        self.question = 0;
        self.voice.feedback = None;
        self.orientation = match self.side {
            "black" => Color::Black,
            "random" if random_unit() < 0.5 => Color::White,
            "random" => Color::Black,
            _ => Color::White,
        };
        self.score = 0;
        self.mistakes = 0;
        self.pending_file = None;
        self.saved = Value::Null;
        self.run_limited = crate::prefs::get(cx, LIMITED, false);
        let allowed = Self::allowed(cx);
        self.target = random_square(None, allowed.as_deref());
        self.upcoming = random_square(Some(&self.target), allowed.as_deref());
        self.stage = Stage::Running;
        self.clock.start(Self::length(cx));
        self.timers = vec![run::ticker(cx, |this, cx| {
            if this.stage != Stage::Running {
                return false;
            }
            if this.clock.expired() {
                this.finish(cx);
                return false;
            }
            cx.notify();
            true
        })];
        self.redraw(cx);
    }

    fn answer(&mut self, square: &str, cx: &mut Context<Self>) {
        if self.stage != Stage::Running {
            return;
        }
        let correct = square == self.target;
        self.flash = Some((square.to_string(), correct));
        self.timers.push(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FLASH).await;
            let _ = this.update(cx, |this, cx| {
                this.flash = None;
                this.redraw(cx);
            });
        }));
        if correct {
            self.score += 1;
            self.question += 1;
            let allowed = Self::allowed(cx);
            self.target = std::mem::take(&mut self.upcoming);
            self.upcoming = random_square(Some(&self.target), allowed.as_deref());
            self.pending_file = None;
        } else {
            self.mistakes += 1;
            self.pending_file = None;
            crate::sound::play_at(crate::sound::Sound::LowTime, 0.5, cx);
        }
        self.redraw(cx);
    }

    fn voice_heard(&mut self, event: &VoiceEvent, cx: &mut Context<Self>) {
        if !self.voice.owns(event) || self.stage != Stage::Running || self.mode != Mode::Voice {
            return;
        }
        let asked = format!("{}:{}", self.run_epoch, self.question);
        let target = self.target.clone();
        let (heard, missed) = match event {
            VoiceEvent::Heard { heard, .. } => (heard, false),
            VoiceEvent::Missed { heard, .. } => (heard, true),
        };
        let square = (!missed)
            .then(|| rules("spokenSquare", &[json!(heard.text)]).ok())
            .flatten()
            .and_then(|square| square.as_str().map(str::to_string));
        let outcome = match &square {
            _ if missed => "unclear",
            None => "invalid",
            Some(square) if *square == target => "correct",
            Some(_) => "wrong",
        };
        let attempt = Attempt {
            source: "coordinates",
            heard,
            outcome,
            parsed: square.as_deref(),
            expected: Some(&target),
            fen: None,
        };
        self.voice_log.log(&attempt, &asked, cx);
        if missed {
            return;
        }
        let Some(square) = square else {
            self.voice.feedback = Some("Say one square, such as “E four” or “Echo four”.".into());
            cx.notify();
            return;
        };
        let verdict = if square == target {
            "correct"
        } else {
            "try again"
        };
        self.voice.feedback = Some(format!("{square} · {verdict}").into());
        self.answer(&square, cx);
    }

    /// Name mode: a file then a rank, from the buttons or the keyboard.
    fn press(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.mode != Mode::Name || self.stage != Stage::Running {
            return;
        }
        if FILES.contains(&key) {
            self.pending_file = Some(key.to_string());
            cx.notify();
        } else if RANKS.contains(&key)
            && let Some(file) = self.pending_file.clone()
        {
            self.answer(&format!("{file}{key}"), cx);
        }
    }

    pub fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        let key = event.keystroke.key.to_lowercase();
        if key.len() == 1 {
            self.press(&key, cx);
        }
    }

    fn accuracy(&self) -> u32 {
        let total = self.score + self.mistakes;
        if total == 0 {
            0
        } else {
            (f64::from(self.score) / f64::from(total) * 100.).round() as u32
        }
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        if self.stage != Stage::Running {
            return;
        }
        self.clock.stop();
        self.stage = Stage::Over;
        self.redraw(cx);
        // An untimed score only grows with time, and some files and ranks are easier: never records.
        if self.clock.untimed || self.run_limited {
            return;
        }
        let run = json!({
            "kind": "coordinates",
            "variant": self.record_variant(cx),
            "score": self.score,
            "detail": {
                "mistakes": self.mistakes,
                "accuracy": self.accuracy(),
                "side": if self.orientation == Color::White { "white" } else { "black" },
            },
        });
        run::save_run(run, cx, |this, saved, cx| {
            this.summary = saved["summary"].clone();
            this.saved = saved;
            cx.notify();
        });
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        let running = self.stage == Stage::Running;
        self.voice.sync(
            crate::voice::grammar("COORDINATE_GRAMMAR"),
            running && self.mode == Mode::Voice,
            cx,
        );
        let show_pieces = crate::prefs::get(cx, PIECES, false);
        let fen = if running && !show_pieces {
            EMPTY_FEN
        } else {
            STANDARD_FEN
        };
        let orientation = if running {
            self.orientation
        } else {
            match self.side {
                "black" => Color::Black,
                "white" => Color::White,
                _ => self.orientation,
            }
        };
        let position =
            kchess_domain::rules::setup_start("standard", STANDARD_FEN).unwrap_or_default();
        let mut state = BoardState::of(&position, orientation, None, None);
        // An empty board is no legal position: the board draws the FEN's pieces, none here.
        state.fen = fen.to_string();
        let show = crate::prefs::get(cx, SHOW, false);
        let on_squares = crate::prefs::get(cx, ON_SQUARES, false);
        let settings = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .clone();
        state.coordinates = Some(if show || on_squares {
            if settings.coordinates == CoordinateMode::None {
                CoordinateMode::Inside
            } else {
                settings.coordinates
            }
        } else {
            CoordinateMode::None
        });
        state.square_names = on_squares;
        state.blindfold = settings.blindfold;
        if running
            && self.mode != Mode::Find
            && let Ok(square) = self.target.parse::<Square>()
        {
            state.circles.push(Circle::yellow(square));
        }
        if let Some((square, correct)) = &self.flash
            && let Ok(square) = square.parse::<Square>()
        {
            state.circles.push(if *correct {
                Circle::green(square)
            } else {
                Circle::red(square)
            });
        }
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }

    /// Each side's recent timed runs of this mode and length, oldest first, and their average.
    fn side_scores(&self, cx: &App) -> Vec<(&'static str, Vec<f64>, f64)> {
        let variant = self.record_variant(cx);
        let runs: Vec<&Value> = self.summary["recent"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|entry| entry["variant"] == variant.as_str())
            .collect();
        ["white", "black"]
            .into_iter()
            .filter_map(|color| {
                let mut scores: Vec<f64> = runs
                    .iter()
                    .filter(|entry| entry["detail"]["side"] == color)
                    .filter_map(|entry| entry["score"].as_f64())
                    .collect();
                scores.reverse();
                if scores.is_empty() {
                    return None;
                }
                let average = scores.iter().sum::<f64>() / scores.len() as f64;
                Some((color, scores, (average * 100.).round() / 100.))
            })
            .collect()
    }

    fn switch(
        id: &'static str,
        label: &'static str,
        key: &'static str,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Switch::new(id)
            .checked(crate::prefs::get(cx, key, false))
            .disabled(disabled)
            .label(label)
            .on_click(cx.listener(move |this, on: &bool, _, cx| {
                crate::prefs::set(cx, key, *on);
                this.redraw(cx);
            }))
    }

    fn file_rank_row(
        &self,
        items: &'static [&'static str; 8],
        chosen: Vec<String>,
        key: &'static str,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex().gap(px(4.)).children(items.iter().map(|item| {
            let on = chosen.iter().any(|entry| entry == item);
            let chosen = chosen.clone();
            Button::new(SharedString::from(format!("{key}-{item}")))
                .small()
                .flex_1()
                .when(on, |button| button.primary())
                .when(!on, |button| button.outline())
                .label(*item)
                .disabled(disabled)
                .on_click(cx.listener(move |this, _, _, cx| {
                    crate::prefs::set(cx, key, toggled(&chosen, item));
                    this.redraw(cx);
                }))
        }))
    }
}

impl Render for Coordinates {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let running = self.stage == Stage::Running;
        // The board alone in its column: no player lines take height.
        let board_size = self
            .layout
            .board_size_around(px(0.), px(0.), px(0.), window);
        // As on Lichess: the square to find over the board, so no row above it takes height.
        let prompt = (running && self.mode == Mode::Find).then(|| {
            v_flex()
                .id("coordinates-target")
                .test_support()
                .aria_label(self.target.clone())
                .absolute()
                .inset_0()
                .items_center()
                .justify_center()
                .line_height(relative(1.))
                .child(shadowed(
                    self.target.clone(),
                    board_size * 0.2,
                    FontWeight::BOLD,
                    0.9,
                ))
                .child(shadowed(
                    self.upcoming.clone(),
                    board_size * 0.08,
                    FontWeight::NORMAL,
                    0.6,
                ))
        });
        let board_column = div()
            .w(board_size)
            .flex_none()
            .relative()
            .child(crate::board::cached(&self.board))
            .children(prompt);
        let frontend = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .clone();
        let page = cx.entity().downgrade();
        let modes = widgets::wrapping_pills(
            "coordinates-mode",
            vec![
                Pill {
                    icon: Some(icon("mouse-pointer-click")),
                    label: "Find the square".into(),
                    badge: None,
                    disabled: running,
                },
                Pill {
                    icon: Some(icon("keyboard")),
                    label: "Name the square".into(),
                    badge: None,
                    disabled: running,
                },
                Pill {
                    icon: Some(icon("mic")),
                    label: "Say the square".into(),
                    badge: None,
                    disabled: running,
                },
            ],
            MODES
                .iter()
                .position(|mode| *mode == self.mode)
                .unwrap_or(0),
            {
                let page = page.clone();
                move |index, _, cx| {
                    let _ = page.update(cx, |this, cx| {
                        this.mode = MODES[index];
                        // Leaving Say the square turns voice off, as its control goes away.
                        if this.mode != Mode::Voice && this.voice.enabled {
                            this.voice.toggle(false, Vec::new(), false, cx);
                        }
                        this.starting = false;
                        this.redraw(cx);
                    });
                }
            },
            &palette,
        );
        let sides = ["white", "black", "random"];
        let side_pills = widgets::pills(
            "coordinates-side",
            ["White", "Black", "Random"]
                .into_iter()
                .map(|label| Pill {
                    icon: None,
                    label: label.into(),
                    badge: None,
                    disabled: running,
                })
                .collect(),
            sides
                .iter()
                .position(|side| *side == self.side)
                .unwrap_or(0),
            {
                let page = page.clone();
                move |index, _, cx| {
                    let _ = page.update(cx, |this, cx| {
                        this.side = sides[index];
                        this.redraw(cx);
                    });
                }
            },
            false,
            &palette,
        );
        let limited = crate::prefs::get(cx, LIMITED, false);
        let length = Self::length(cx);
        let mut panel = widgets::side_panel(&palette)
            .id("coordinates-panel")
            .overflow_y_scroll()
            .h(self.layout.fixed(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(14.))
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.highlighted)
                                    .child("Mode"),
                            )
                            .child(widgets::view_toggles(frontend.blindfold, frontend.zen_mode)),
                    )
                    .child(modes),
            )
            .child(run::length_picker(
                "coordinates-length",
                length,
                running,
                move |seconds, _, cx| crate::prefs::set(cx, LENGTH, seconds),
                &palette,
            ))
            .child(run::field("Board from the side of", side_pills, &palette))
            .child(Self::switch(
                "coordinates-limited",
                "Practice only some files & ranks",
                LIMITED,
                running,
                cx,
            ))
            .when(limited, |panel| {
                panel.child(
                    v_flex()
                        .gap(px(6.))
                        .child(self.file_rank_row(
                            &FILES,
                            Self::files(cx),
                            STORED_FILES,
                            running,
                            cx,
                        ))
                        .child(self.file_rank_row(
                            &RANKS,
                            Self::ranks(cx),
                            STORED_RANKS,
                            running,
                            cx,
                        )),
                )
            })
            .child(Self::switch(
                "coordinates-show",
                "Show coordinates",
                SHOW,
                running,
                cx,
            ))
            .child(Self::switch(
                "coordinates-on-squares",
                "Coordinates on every square",
                ON_SQUARES,
                running,
                cx,
            ))
            .child(Self::switch(
                "coordinates-pieces",
                "Show pieces",
                PIECES,
                running,
                cx,
            ))
            .when(self.mode == Mode::Voice, |panel| {
                panel.child(self.voice.bar(
                    page.clone(),
                    |this: &mut Self| &mut this.voice,
                    BarSpec {
                        id: "coordinates-voice",
                        hint: "Say “E four” or “Echo four”.",
                        active: running,
                        grammar: crate::voice::grammar("COORDINATE_GRAMMAR"),
                    },
                    cx,
                ))
            })
            .child(div().h(px(1.)).bg(palette.border));
        if running {
            panel = panel.child(run::hud(
                &self.clock,
                f64::from(self.score),
                "correct",
                &palette,
            ));
            if self.mode == Mode::Name {
                let pending = self.pending_file.clone();
                panel = panel.child(
                    v_flex()
                        .gap(px(6.))
                        .child(h_flex().gap(px(4.)).children(FILES.iter().map(|file| {
                            let chosen = pending.as_deref() == Some(*file);
                            Button::new(SharedString::from(format!("answer-{file}")))
                                .small()
                                .flex_1()
                                .when(chosen, |button| button.primary())
                                .when(!chosen, |button| button.outline())
                                .label(*file)
                                .on_click(cx.listener(move |this, _, _, cx| this.press(file, cx)))
                        })))
                        .child(h_flex().gap(px(4.)).children(RANKS.iter().map(|rank| {
                            Button::new(SharedString::from(format!("answer-{rank}")))
                                .small()
                                .flex_1()
                                .outline()
                                .label(*rank)
                                .disabled(pending.is_none())
                                .on_click(cx.listener(move |this, _, _, cx| this.press(rank, cx)))
                        }))),
                );
            }
            panel = panel.child(
                Button::new("coordinates-end")
                    .outline()
                    .icon(icon("flag"))
                    .label("End run")
                    .on_click(cx.listener(|this, _, _, cx| this.finish(cx))),
            );
        } else {
            let variant = self.record_variant(cx);
            let best = run::best(&self.summary, &variant);
            if self.stage == Stage::Over {
                let is_best = !self.clock.untimed && self.saved["isBest"] == json!(true);
                let detail = if self.run_limited {
                    "Not saved: some files & ranks only".to_string()
                } else if self.clock.untimed {
                    format!("in {}", self.clock.text())
                } else if is_best {
                    "A new personal best!".to_string()
                } else {
                    format!("Best so far: {}", best.unwrap_or(f64::from(self.score)))
                };
                panel = panel.child(
                    widgets::StatusBanner::new(
                        if is_best { Tone::Win } else { Tone::Plain },
                        if is_best {
                            icon("trophy")
                        } else {
                            icon("timer")
                        },
                        format!("{} correct · {}% accuracy", self.score, self.accuracy()).into(),
                    )
                    .announce()
                    .detail(detail.into()),
                );
            }
            if length != 0 {
                for (color, scores, average) in self.side_scores(cx) {
                    let name = if color == "white" { "White" } else { "Black" };
                    panel = panel.child(
                        v_flex()
                            .gap(px(4.))
                            .child(
                                h_flex()
                                    .gap(px(4.))
                                    .text_size(px(14.))
                                    .child(format!("Average as {name}:"))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::BOLD)
                                            .child(format!("{average}")),
                                    ),
                            )
                            .when(scores.len() > 1, |row| {
                                row.child(run::sparkline(scores, &palette))
                            }),
                    );
                }
            }
            if self.stage != Stage::Over && length != 0 {
                panel = panel.child(
                    div()
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .child(format!(
                            "Best ({}, {}): {}",
                            match self.mode {
                                Mode::Find => "find",
                                Mode::Name => "name",
                                Mode::Voice => "voice",
                            },
                            run::length_label(length),
                            best.map_or_else(|| "—".to_string(), |best| best.to_string())
                        )),
                );
            }
            panel = panel.child(
                Button::new("coordinates-start")
                    .primary()
                    .large()
                    .w_full()
                    .icon(icon("play"))
                    .label(if self.stage == Stage::Over {
                        "Play again"
                    } else {
                        "Start"
                    })
                    .loading(self.starting)
                    .disabled(self.mode == Mode::Voice && !self.voice.enabled)
                    .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
            );
        }
        div().size_full().child(
            self.layout
                .render("coordinates", board_column, panel, window, cx),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{FILES, chosen, random_square, toggled};

    #[test]
    fn random_squares_never_repeat_and_stay_allowed() {
        let allowed = vec!["a1".to_string(), "b2".to_string()];
        for _ in 0..20 {
            let square = random_square(Some("a1"), Some(&allowed));
            assert_eq!(square, "b2");
        }
        assert_eq!(random_square(None, None).len(), 2);
    }

    #[test]
    fn at_least_one_file_stays_chosen() {
        let all = chosen(&FILES, &[]);
        assert_eq!(all.len(), 8);
        let one = vec!["c".to_string()];
        assert_eq!(toggled(&one, "c"), one);
        assert_eq!(toggled(&one, "d"), vec!["c", "d"]);
    }
}
