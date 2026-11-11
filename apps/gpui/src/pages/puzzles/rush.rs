//! Puzzles → Storm · Streak · Rush (rules in `kchess_domain::training::rush`): timed and survival
//! runs over a ladder of puzzles from the database on this computer,
//! scored and kept here; Lichess's own Storm and Streak results beside the menu.

use kchess_contracts::calls;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::{LocalLadderQuery, RunKind, RunRecord, RunSaved, RunSummary};
use kchess_domain::training::data::data;
use kchess_domain::training::rush::{RushConfig, RushState, combo_progress, format_run_clock};
use serde_json::{Value, json};

use super::stats::PuzzleStats;
use crate::assets::icon;
use crate::core::AppCore;
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::puzzle_board::{PuzzleBoard, PuzzleBoardEvent};
use crate::theme::{Palette, radii, tint};
use crate::widgets::{self, KeyboardFocus as _, Tone};

/// The opponent replies faster in a run.
const REPLY: Duration = Duration::from_millis(250);
/// The pause before the next puzzle: short after a solve, longer to see a miss.
const AFTER_SOLVE: Duration = Duration::from_millis(250);
const AFTER_MISS: Duration = Duration::from_millis(750);
const BONUS_SHOWN: Duration = Duration::from_millis(1200);

/// The kinds of run, and the order the menu offers them in.
fn runs() -> (&'static BTreeMap<String, RushConfig>, &'static [String]) {
    (&data().rush_configs, &data().rush_choices)
}

/// The state before any run: nothing scored, no clock.
fn no_run() -> RushState {
    RushState {
        score: 0.,
        moves: 0.,
        mistakes: 0.,
        combo: 0.,
        max_combo: 0.,
        time_left_ms: None,
        skips_left: 0.,
        highest: 0.,
        bonus_ms: 0.,
        over: None,
    }
}

/// What a run's buttons do, as Quick Pick offers them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RushAction {
    Start,
    Skip,
    End,
    Again,
    Back,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Menu,
    Loading,
    Running,
    Over,
}

pub struct PuzzleRush {
    configs: &'static BTreeMap<String, RushConfig>,
    choices: &'static [String],
    choice: String,
    stage: Stage,
    error: Option<String>,
    ladder: Vec<Value>,
    index: usize,
    state: RushState,
    /// `runSummary` by mode.
    summaries: BTreeMap<String, RunSummary>,
    saved: Option<RunSaved>,
    bonus: Option<f64>,
    started: Option<Instant>,
    last_tick: Option<Instant>,
    elapsed: Duration,
    locked: bool,
    warned: bool,
    /// Bumped when a run starts or ends, so timers of the last one do nothing.
    generation: u64,
    board: Entity<PuzzleBoard>,
    layout: PlayLayout,
    timers: Vec<Task<()>>,
    /// Lichess's Storm results, shown beside the menu.
    stats: Entity<PuzzleStats>,
    _subscriptions: Vec<Subscription>,
}

impl PuzzleRush {
    pub fn new(
        stats: Entity<PuzzleStats>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PuzzleRush {
        let (configs, choices) = runs();
        let board = cx.new(|cx| {
            let mut board = PuzzleBoard::new(false, window, cx);
            board.for_runs(REPLY);
            board
        });
        let subscriptions = vec![
            cx.subscribe(
                &board,
                |this, _, event: &PuzzleBoardEvent, cx| match event {
                    PuzzleBoardEvent::Move { correct, .. } => this.on_move(*correct, cx),
                    PuzzleBoardEvent::Done => this.on_done(cx),
                    PuzzleBoardEvent::Outcome { .. } => {}
                },
            ),
            cx.observe(&board, |_, _, cx| cx.notify()),
            cx.observe(&stats, |_, _, cx| cx.notify()),
        ];
        let state = configs.get("storm").map_or_else(no_run, RushState::new);
        let mut rush = PuzzleRush {
            configs,
            choices,
            choice: "storm".into(),
            stage: Stage::Menu,
            error: None,
            ladder: Vec::new(),
            index: 0,
            state,
            summaries: BTreeMap::new(),
            saved: None,
            bonus: None,
            started: None,
            last_tick: None,
            elapsed: Duration::ZERO,
            locked: false,
            warned: false,
            generation: 0,
            board,
            layout: PlayLayout::default(),
            timers: Vec::new(),
            stats,
            _subscriptions: subscriptions,
        };
        rush.load_summaries(cx);
        rush
    }

    fn config(&self) -> Option<&'static RushConfig> {
        self.configs.get(&self.choice)
    }

    fn title_of(&self, key: &str) -> String {
        self.configs
            .get(key)
            .map_or(key, |config| config.title.as_str())
            .to_string()
    }

    fn load_summaries(&mut self, cx: &mut Context<Self>) {
        let core = cx.global::<AppCore>().clone();
        for kind in [RunKind::Storm, RunKind::Streak, RunKind::Rush] {
            let summary = core.request(calls::RunSummary { kind });
            crate::loading::fetch(cx, "puzzles", summary, move |this, summary, _| {
                this.summaries.insert(kind.as_str().to_string(), summary);
            });
        }
    }

    /// The best score of a kind of run, kept here.
    fn best_of(&self, key: &str) -> Option<f64> {
        let config = self.configs.get(key)?;
        crate::pages::practice::run::best(self.summaries.get(&config.mode), &config.variant)
    }

    fn stop_timers(&mut self) {
        self.generation += 1;
        self.timers.clear();
        self.locked = false;
        self.bonus = None;
    }

    /// `start`: a ladder from the database, then the clock.
    fn start(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config() else {
            return;
        };
        self.stop_timers();
        let generation = self.generation;
        self.error = None;
        self.stage = Stage::Loading;
        cx.notify();
        let ladder = cx.global::<AppCore>().request(calls::LocalLadder {
            query: LocalLadderQuery {
                from: config.ladder.from as u32,
                to: config.ladder.to as u32,
                count: config.ladder.count as u32,
            },
        });
        cx.spawn(async move |this, cx| {
            let ladder = ladder.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                let ladder = match ladder {
                    Ok(ladder) if ladder.len() >= 10 => {
                        ladder.into_iter().map(|puzzle| json!(puzzle)).collect()
                    }
                    Ok(_) => {
                        this.error =
                            Some("The puzzle database is too small; download it again".into());
                        this.stage = Stage::Menu;
                        cx.notify();
                        return;
                    }
                    Err(cause) => {
                        this.error = Some(cause.message);
                        this.stage = Stage::Menu;
                        cx.notify();
                        return;
                    }
                };
                this.ladder = ladder;
                this.index = 0;
                this.state = RushState::new(config);
                this.saved = None;
                this.warned = false;
                let now = Instant::now();
                this.started = Some(now);
                this.last_tick = Some(now);
                this.stage = Stage::Running;
                this.show_puzzle(cx);
                if config.duration_ms.is_some() {
                    this.run_clock(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn show_puzzle(&mut self, cx: &mut Context<Self>) {
        let Some(puzzle) = self.ladder.get(self.index).cloned() else {
            return;
        };
        self.board.update(cx, |board, cx| {
            board.start(&puzzle, None, cx);
            board.set_frozen(false, cx);
        });
    }

    fn run_clock(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        self.timers.push(crate::timer::every(
            crate::timer::TICK,
            cx,
            move |this, cx| {
                if generation != this.generation || this.stage != Stage::Running {
                    return false;
                }
                this.tick(cx);
                true
            },
        ));
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let elapsed = self
            .last_tick
            .map_or(0., |last| (now - last).as_millis() as f64);
        self.last_tick = Some(now);
        self.state.tick(elapsed);
        let left = self.state.time_left_ms;
        if !self.warned && left.is_some_and(|left| left > 0. && left < 10_000.) {
            self.warned = true;
            crate::sound::play(crate::sound::Sound::LowTime, cx);
        }
        if self.over() {
            self.end_run(cx);
        }
        cx.notify();
    }

    /// A timed or survival run is going: a restart ends it.
    pub(crate) fn running(&self) -> bool {
        self.stage == Stage::Running
    }

    fn over(&self) -> bool {
        self.state.over.is_some()
    }

    fn on_move(&mut self, correct: bool, cx: &mut Context<Self>) {
        if self.stage != Stage::Running {
            return;
        }
        let Some(config) = self.config() else {
            return;
        };
        if correct {
            let bonus = self.state.correct_move(config);
            if bonus > 0. {
                self.bonus = Some(bonus);
                let generation = self.generation;
                self.timers.push(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(BONUS_SHOWN).await;
                    let _ = this.update(cx, |this, cx| {
                        if this.generation == generation {
                            this.bonus = None;
                            cx.notify();
                        }
                    });
                }));
            }
        } else {
            self.state.mistake(config);
        }
        if self.over() {
            self.end_run(cx);
        }
        cx.notify();
    }

    /// `onDone`: score a solve, then the next puzzle after a pause.
    fn on_done(&mut self, cx: &mut Context<Self>) {
        if self.stage != Stage::Running || self.over() {
            return;
        }
        let solved = self.board.read(cx).status() == "solved";
        if solved
            && let Some(rating) = self
                .ladder
                .get(self.index)
                .and_then(|p| p["rating"].as_f64())
        {
            self.state.puzzle_solved(rating);
        }
        self.locked = true;
        self.board
            .update(cx, |board, cx| board.set_frozen(true, cx));
        let generation = self.generation;
        let pause = if solved { AFTER_SOLVE } else { AFTER_MISS };
        self.timers.push(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(pause).await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation || this.stage != Stage::Running {
                    return;
                }
                this.locked = false;
                this.advance(cx);
            });
        }));
        cx.notify();
    }

    fn advance(&mut self, cx: &mut Context<Self>) {
        if self.over() {
            return;
        }
        self.index += 1;
        if self.index >= self.ladder.len() {
            self.state.finish("complete");
            self.end_run(cx);
            return;
        }
        self.show_puzzle(cx);
        cx.notify();
    }

    fn skip(&mut self, cx: &mut Context<Self>) {
        if self.locked {
            return;
        }
        if self.state.skip() {
            self.advance(cx);
        }
        cx.notify();
    }

    /// `endRun`: stop, then save the score here.
    fn end_run(&mut self, cx: &mut Context<Self>) {
        if self.stage != Stage::Running {
            return;
        }
        self.stop_timers();
        let generation = self.generation;
        self.elapsed = self
            .started
            .map_or(Duration::ZERO, |started| started.elapsed());
        self.state.finish("ended");
        self.stage = Stage::Over;
        self.board
            .update(cx, |board, cx| board.set_frozen(true, cx));
        let Some(config) = self.config() else {
            cx.notify();
            return;
        };
        let Some(kind) = RunKind::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_str() == config.mode)
        else {
            cx.notify();
            return;
        };
        let state = &self.state;
        let run = crate::pages::practice::run::input(
            kind,
            config.variant.clone(),
            state.score,
            json!({
                "moves": state.moves,
                "mistakes": state.mistakes,
                "accuracy": state.accuracy(),
                "maxCombo": state.max_combo,
                "highest": state.highest,
                "seconds": self.elapsed.as_secs_f64().round(),
            }),
        );
        let mode = config.mode.clone();
        let save = cx.global::<AppCore>().request(calls::SaveRun { run });
        cx.spawn(async move |this, cx| {
            let saved = save.await.map_err(|cause| cause.message);
            let _ = this.update(cx, |this, cx| this.run_saved(generation, mode, saved, cx));
        })
        .detach();
        cx.notify();
    }

    /// The answer to saving the run of `generation`: the mode's best scores either way, and the
    /// result or failure shown only while that run is still the one on screen. A late failure
    /// showed in the next run.
    fn run_saved(
        &mut self,
        generation: u64,
        mode: String,
        saved: Result<RunSaved, String>,
        cx: &mut Context<Self>,
    ) {
        let current = generation == self.generation;
        match saved {
            Ok(saved) => {
                self.summaries.insert(mode, saved.summary.clone());
                if current {
                    self.saved = Some(saved);
                }
            }
            Err(message) if current => self.error = Some(message),
            Err(message) => crate::log::write(
                kchess_contracts::Level::Warn,
                "rush",
                &format!("Saving an earlier run failed: {message}"),
            ),
        }
        cx.notify();
    }

    fn clock_text(ms: f64) -> String {
        format_run_clock(ms)
    }

    fn render_menu(
        &self,
        storm: AnyElement,
        window: &Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let config = self.config();
        let db_ready = cx
            .global::<crate::puzzle_db::PuzzleDbHandle>()
            .0
            .read(cx)
            .installed();
        let choices = h_flex()
            .flex_wrap()
            .gap(px(8.))
            .children(self.choices.iter().map(|key| {
                let chosen = *key == self.choice;
                let best = self.best_of(key);
                let key_clicked = key.clone();
                v_flex()
                    .id(SharedString::from(format!("rush-{key}")))
                    .role(Role::Button)
                    .keyboard_focus(palette.primary)
                    .aria_label(self.title_of(key))
                    .aria_selected(chosen)
                    .min_w(px(150.))
                    .flex_1()
                    .gap(px(2.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(radii::LG)
                    .border_1()
                    .text_size(px(13.))
                    .cursor_pointer()
                    .when(chosen, |choice| {
                        choice
                            .border_color(palette.primary)
                            .bg(tint(palette.primary, 12.))
                    })
                    .when(!chosen, |choice| {
                        choice.border_color(palette.border).bg(palette.bg)
                    })
                    .when(self.stage == Stage::Menu, |choice| {
                        choice.on_click(cx.listener(move |this, _, _, cx| {
                            this.choice = key_clicked.clone();
                            cx.notify();
                        }))
                    })
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.title_of(key)),
                    )
                    .children(best.map(|best| {
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(format!("Best: {best}"))
                    }))
            }));
        let mode = config.map_or("storm", |config| config.mode.as_str());
        let recent: Vec<RunRecord> = self
            .summaries
            .get(mode)
            .map(|summary| summary.recent.iter().take(6).cloned().collect())
            .unwrap_or_default();
        let title = config
            .map(|config| config.title.clone())
            .unwrap_or_default();
        let recent_table = (!recent.is_empty()).then(|| {
            let cell = |text: String, width: f32| div().w(px(width)).child(text);
            v_flex()
                .gap(px(4.))
                .mt(px(14.))
                .child(widgets::heading(
                    format!("Your recent {mode} runs"),
                    palette,
                ))
                .child(
                    h_flex()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text_muted)
                        .child(div().flex_1().child("WHEN"))
                        .child(cell("KIND".into(), 90.))
                        .child(cell("SCORE".into(), 60.))
                        .child(cell("ACCURACY".into(), 80.)),
                )
                .children(recent.iter().map(|run| {
                    use chrono::TimeZone as _;
                    let when = chrono::Local
                        .timestamp_millis_opt(run.played_at)
                        .single()
                        .map(|at| at.format("%b %-d, %Y %-I:%M %p").to_string())
                        .unwrap_or_default();
                    let variant = run.variant.as_str();
                    h_flex()
                        .py(px(4.))
                        .text_size(px(13.))
                        .border_t_1()
                        .border_color(palette.border)
                        .child(div().flex_1().child(when))
                        .child(cell(
                            if variant == "standard" {
                                title.clone()
                            } else {
                                variant.to_string()
                            },
                            90.,
                        ))
                        .child(cell(run.score.to_string(), 60.))
                        .child(cell(
                            run.detail
                                .get("accuracy")
                                .and_then(Value::as_f64)
                                .map_or_else(|| "—".into(), |accuracy| format!("{accuracy}%")),
                            80.,
                        ))
                }))
        });
        let play = widgets::card(palette)
            .flex_1()
            .min_w(px(320.))
            .gap(px(12.))
            .child(widgets::heading("Play a run", palette))
            .child(choices)
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(relative(1.55))
                    .text_color(palette.toned)
                    .child(
                        config
                            .map(|config| config.rules.clone())
                            .unwrap_or_default(),
                    ),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .child(
                h_flex().child(
                    Button::new("rush-start")
                        .primary()
                        .large()
                        .icon(icon("play"))
                        .label(format!("Start {title}"))
                        .loading(self.stage == Stage::Loading)
                        .disabled(!db_ready || self.stage == Stage::Loading)
                        .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                ),
            )
            .when(!db_ready, |card| {
                card.child(crate::puzzle_db::card(false, cx, palette))
            })
            .children(recent_table);
        let stacked = window.viewport_size().width < px(900.);
        div()
            .id("puzzle-rush")
            .size_full()
            .overflow_y_scroll()
            .child(
                if stacked {
                    v_flex()
                } else {
                    h_flex().items_start()
                }
                .gap(px(20.))
                .px(crate::play_layout::side_padding(window))
                .pt(px(20.))
                .pb(crate::play_layout::PAGE_BOTTOM)
                .child(play)
                .child(div().flex_1().min_w(px(300.)).child(storm)),
            )
            .into_any_element()
    }

    fn render_run(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let config = self.config();
        let board_size = self.layout.board_size(window);
        let state = &self.state;
        let combo_on = config.is_some_and(|config| config.combo);
        let board_column = div()
            .relative()
            .w(board_size)
            .flex_none()
            .child(self.board.clone())
            .children(self.bonus.map(|bonus| {
                div()
                    .absolute()
                    .top(px(12.))
                    .right(px(12.))
                    .px(px(12.))
                    .py(px(4.))
                    .rounded_full()
                    .bg(palette.primary)
                    .text_color(palette.inverted)
                    .font_weight(FontWeight::MEDIUM)
                    .child(format!("+{}s", bonus / 1000.))
            }));
        let title = config
            .map(|config| config.title.clone())
            .unwrap_or_default();
        let mut panel = widgets::side_panel(palette)
            .h(board_size)
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(14.))
            .child(widgets::heading(title, palette));
        if self.stage == Stage::Running {
            let left = state.time_left_ms;
            let low = left.is_some_and(|left| left < 10_000.);
            let hud = h_flex()
                .justify_between()
                .items_end()
                .children(left.map(|left| {
                    crate::widgets::timer(Self::clock_text(left), px(32.)).text_color(if low {
                        palette.error
                    } else {
                        palette.highlighted
                    })
                }))
                .child(
                    h_flex()
                        .gap(px(8.))
                        .items_baseline()
                        .child(
                            div()
                                .text_size(px(28.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(state.score.to_string()),
                        )
                        .child(div().text_color(palette.text_muted).child("solved")),
                );
            let combo = combo_on.then(|| {
                let progress = combo_progress(state.combo);
                v_flex()
                    .gap(px(4.))
                    .child(
                        h_flex()
                            .justify_between()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child("Combo")
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(state.combo.to_string()),
                            ),
                    )
                    .child(
                        div()
                            .h(px(8.))
                            .w_full()
                            .rounded_full()
                            .bg(palette.accented)
                            .child(
                                div()
                                    .h_full()
                                    .rounded_full()
                                    .bg(palette.primary)
                                    .w(relative(progress as f32)),
                            ),
                    )
            });
            let strikes = config
                .and_then(|config| config.strikes)
                .filter(|strikes| *strikes > 1.)
                .map(|strikes| {
                    let (strikes, used) = (strikes as u64, state.mistakes as u64);
                    h_flex()
                        .gap(px(8.))
                        .children((0..strikes).map(|slot| {
                            if slot < used {
                                icon("circle-x").size(px(20.)).text_color(palette.error)
                            } else {
                                icon("circle").size(px(20.)).text_color(palette.text_muted)
                            }
                        }))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child("strikes"),
                        )
                });
            let rating = self
                .ladder
                .get(self.index)
                .and_then(|p| p["rating"].as_f64())
                .unwrap_or_default();
            let skips = state.skips_left;
            panel = panel
                .child(hud)
                .children(combo)
                .children(strikes)
                .child(widgets::hint(
                    format!("Puzzle {} · rating {rating}", self.index + 1),
                    palette,
                ))
                .child(
                    h_flex()
                        .gap(px(8.))
                        .when(config.is_some_and(|config| config.skips > 0.), |row| {
                            row.child(
                                Button::new("rush-skip")
                                    .outline()
                                    .icon(icon("skip-forward"))
                                    .label(format!("Skip ({skips})"))
                                    .disabled(skips <= 0. || self.locked)
                                    .on_click(cx.listener(|this, _, _, cx| this.skip(cx))),
                            )
                        })
                        .child(
                            Button::new("rush-end")
                                .outline()
                                .icon(icon("flag"))
                                .label("End run")
                                .on_click(cx.listener(|this, _, _, cx| this.end_run(cx))),
                        ),
                );
        } else {
            let best = self.saved.as_ref().is_some_and(|saved| saved.is_best);
            let over = match state.over.as_deref() {
                Some("time") => "Time’s up".to_string(),
                Some("strikes") if config.and_then(|config| config.strikes) == Some(1.) => {
                    "Streak over".into()
                }
                Some("strikes") => "Out of strikes".into(),
                Some("complete") => "Every puzzle cleared".into(),
                _ => "Run ended".into(),
            };
            let detail = if best {
                "New personal best".to_string()
            } else if self.saved.is_some() {
                "Saved on this computer".into()
            } else if let Some(error) = &self.error {
                format!("Not saved: {error}")
            } else {
                "Saving…".into()
            };
            let accuracy = state.accuracy();
            let result = |label: &str, value: String| {
                v_flex()
                    .min_w(px(100.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(label.to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(value),
                    )
            };
            let highest = state.highest;
            panel = panel
                .child(
                    widgets::StatusBanner::new(
                        if best { Tone::Win } else { Tone::Plain },
                        if best { icon("trophy") } else { icon("flag") },
                        over.into(),
                    )
                    .announce()
                    .detail(detail.into()),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(px(14.))
                        .child(result("Score", state.score.to_string()))
                        .child(result("Moves", state.moves.to_string()))
                        .child(result("Accuracy", format!("{accuracy}%")))
                        .when(combo_on, |row| {
                            row.child(result("Best combo", state.max_combo.to_string()))
                        })
                        .child(result(
                            "Hardest solved",
                            if highest > 0. {
                                highest.to_string()
                            } else {
                                "—".into()
                            },
                        ))
                        .child(result(
                            "Time",
                            Self::clock_text(self.elapsed.as_millis() as f64),
                        )),
                )
                .child(
                    h_flex()
                        .gap(px(8.))
                        .child(
                            Button::new("rush-again")
                                .primary()
                                .icon(icon("rotate-ccw"))
                                .label("Play again")
                                .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                        )
                        .child(Button::new("rush-back").outline().label("Back").on_click(
                            cx.listener(|this, _, _, cx| this.act(RushAction::Back, cx)),
                        )),
                );
        }
        self.layout
            .render("puzzle-rush-run", board_column, panel, window, cx)
            .into_any_element()
    }

    /// The menu (with Lichess's Storm results beside it), or the run.
    fn render_stage(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let palette = *cx.global::<Palette>();
        match self.stage {
            Stage::Menu | Stage::Loading => {
                let storm = self
                    .stats
                    .update(cx, |stats, cx| stats.storm_panel(cx, &palette));
                self.render_menu(storm, window, cx, &palette)
            }
            Stage::Running | Stage::Over => self.render_run(window, cx, &palette),
        }
    }

    /// The buttons shown now that would do something.
    pub(crate) fn actions(&self, cx: &App) -> Vec<RushAction> {
        match self.stage {
            Stage::Menu => {
                let ready = cx
                    .global::<crate::puzzle_db::PuzzleDbHandle>()
                    .0
                    .read(cx)
                    .installed();
                if ready {
                    vec![RushAction::Start]
                } else {
                    Vec::new()
                }
            }
            Stage::Loading => Vec::new(),
            Stage::Running => {
                let skips = self.config().map_or(0., |config| config.skips);
                let left = self.state.skips_left;
                let mut actions = Vec::new();
                if skips > 0. && left > 0. && !self.locked {
                    actions.push(RushAction::Skip);
                }
                actions.push(RushAction::End);
                actions
            }
            Stage::Over => vec![RushAction::Again, RushAction::Back],
        }
    }

    /// Run an action as its button does, if it is still allowed.
    pub(crate) fn act(&mut self, action: RushAction, cx: &mut Context<Self>) {
        if !self.actions(cx).contains(&action) {
            return;
        }
        match action {
            RushAction::Start | RushAction::Again => self.start(cx),
            RushAction::Skip => self.skip(cx),
            RushAction::End => self.end_run(cx),
            RushAction::Back => {
                self.stage = Stage::Menu;
                cx.notify();
            }
        }
    }

    /// Leaving the tab ends a run, as unmounting it does.
    pub fn leave(&mut self, cx: &mut Context<Self>) {
        if self.stage == Stage::Running {
            self.end_run(cx);
        }
    }
}

impl Render for PuzzleRush {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_stage(window, cx)
    }
}

#[cfg(test)]
mod tests {
    use kchess_domain::training::rush::RushState;

    /// Saving a finished run failed after the next run had begun: the failure showed in it.
    #[gpui_kit::test]
    fn an_earlier_runs_failed_save_stays_out_of_the_next(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let mut app = crate::e2e::Harness::start(cx);
        let (late, current) = app.with_window(|window, cx| {
            let stats = cx.new(|cx| super::super::stats::PuzzleStats::new(Vec::new(), window, cx));
            let rush = cx.new(|cx| super::PuzzleRush::new(stats, window, cx));
            rush.update(cx, |rush, cx| {
                rush.generation = 2;
                rush.run_saved(1, "storm".into(), Err("The disk is full".into()), cx);
                let late = rush.error.clone();
                rush.run_saved(2, "storm".into(), Err("The disk is full".into()), cx);
                (late, rush.error.clone())
            })
        });
        assert_eq!(late, None);
        assert_eq!(current.as_deref(), Some("The disk is full"));
    }

    #[test]
    fn a_storm_run_follows_the_rules() {
        let (configs, choices) = super::runs();
        assert_eq!(choices[0], "storm");
        let config = &configs["storm"];
        let mut state = RushState::new(config);
        assert_eq!(state.time_left_ms, Some(180_000.));
        state.mistake(config);
        assert_eq!(state.time_left_ms, Some(170_000.));
        state.tick(170_000.);
        assert_eq!(state.over.as_deref(), Some("time"));
    }
}
