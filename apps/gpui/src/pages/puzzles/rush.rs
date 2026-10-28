//! Puzzles → Storm · Streak · Rush (`PuzzleRush.vue`, rules in `training/rush.rs` through
//! `rules()`): timed and survival runs over a ladder of puzzles from the database on this computer,
//! scored and kept here; Lichess's own Storm and Streak results beside the menu.

use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::stats::PuzzleStats;
use crate::assets::icon;
use crate::core::AppCore;
use crate::game::rules;
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::puzzle_board::{PuzzleBoard, PuzzleBoardEvent};
use crate::theme::{Palette, tint};
use crate::widgets::{self, KeyboardFocus as _, Tone};

const TICK: Duration = Duration::from_millis(100);
/// The opponent replies faster in a run.
const REPLY: Duration = Duration::from_millis(250);
/// The pause before the next puzzle: short after a solve, longer to see a miss.
const AFTER_SOLVE: Duration = Duration::from_millis(250);
const AFTER_MISS: Duration = Duration::from_millis(750);
const BONUS_SHOWN: Duration = Duration::from_millis(1200);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Training {
    rush_configs: Map<String, Value>,
    rush_choices: Vec<String>,
}

fn training() -> Training {
    serde_json::from_str(include_str!(
        "../../../../../crates/kchess-domain/data/training.json"
    ))
    .unwrap_or(Training {
        rush_configs: Map::new(),
        rush_choices: Vec::new(),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Menu,
    Loading,
    Running,
    Over,
}

pub struct PuzzleRush {
    configs: Map<String, Value>,
    choices: Vec<String>,
    choice: String,
    stage: Stage,
    error: Option<String>,
    ladder: Vec<Value>,
    index: usize,
    /// `RushState`, as the rules keep it.
    state: Value,
    /// `runSummary` by mode.
    summaries: Map<String, Value>,
    saved: Option<Value>,
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

/// A rule over the run's state that answers the next state (`update`).
fn step(method: &str, args: &[Value]) -> Option<Value> {
    rules(method, args).ok()
}

impl PuzzleRush {
    pub fn new(
        stats: Entity<PuzzleStats>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PuzzleRush {
        let training = training();
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
        let state = training
            .rush_configs
            .get("storm")
            .and_then(|config| step("newRush", std::slice::from_ref(config)))
            .unwrap_or(Value::Null);
        let mut rush = PuzzleRush {
            configs: training.rush_configs,
            choices: training.rush_choices,
            choice: "storm".into(),
            stage: Stage::Menu,
            error: None,
            ladder: Vec::new(),
            index: 0,
            state,
            summaries: Map::new(),
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

    fn config(&self) -> Value {
        self.configs
            .get(&self.choice)
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn title_of(&self, key: &str) -> String {
        self.configs
            .get(key)
            .and_then(|config| config["title"].as_str())
            .unwrap_or(key)
            .to_string()
    }

    fn load_summaries(&mut self, cx: &mut Context<Self>) {
        let core = cx.global::<AppCore>().clone();
        for mode in ["storm", "streak", "rush"] {
            let summary = core.call("runSummary", vec![json!(mode)]);
            cx.spawn(async move |this, cx| {
                if let Ok(summary) = summary.await {
                    let _ = this.update(cx, |this, cx| {
                        this.summaries.insert(mode.to_string(), summary);
                        cx.notify();
                    });
                }
            })
            .detach();
        }
    }

    /// The best score of a kind of run, kept here.
    fn best_of(&self, key: &str) -> Option<f64> {
        let config = self.configs.get(key)?;
        let mode = config["mode"].as_str()?;
        let variant = config["variant"].as_str()?;
        self.summaries.get(mode)?["best"][variant]["score"].as_f64()
    }

    fn stop_timers(&mut self) {
        self.generation += 1;
        self.timers.clear();
        self.locked = false;
        self.bonus = None;
    }

    /// `start`: a ladder from the database, then the clock.
    fn start(&mut self, cx: &mut Context<Self>) {
        self.stop_timers();
        let generation = self.generation;
        let config = self.config();
        self.error = None;
        self.stage = Stage::Loading;
        cx.notify();
        let ladder = cx
            .global::<AppCore>()
            .call("localLadder", vec![config["ladder"].clone()]);
        cx.spawn(async move |this, cx| {
            let ladder = ladder.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                let ladder = match ladder {
                    Ok(Value::Array(ladder)) if ladder.len() >= 10 => ladder,
                    Ok(_) => {
                        this.error =
                            Some("The puzzle database is too small; download it again.".into());
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
                this.state = step("newRush", std::slice::from_ref(&config)).unwrap_or(Value::Null);
                this.saved = None;
                this.warned = false;
                let now = Instant::now();
                this.started = Some(now);
                this.last_tick = Some(now);
                this.stage = Stage::Running;
                this.show_puzzle(cx);
                if config["durationMs"].is_number() {
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
        self.timers.push(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let going = this
                    .update(cx, |this, cx| {
                        if generation != this.generation || this.stage != Stage::Running {
                            return false;
                        }
                        this.tick(cx);
                        true
                    })
                    .unwrap_or(false);
                if !going {
                    return;
                }
            }
        }));
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let elapsed = self
            .last_tick
            .map_or(0., |last| (now - last).as_millis() as f64);
        self.last_tick = Some(now);
        if let Some(next) = step("tick", &[self.state.clone(), json!(elapsed)]) {
            self.state = next;
        }
        let left = self.state["timeLeftMs"].as_f64();
        if !self.warned && left.is_some_and(|left| left > 0. && left < 10_000.) {
            self.warned = true;
            crate::sound::play(crate::sound::Sound::LowTime, cx);
        }
        if self.over() {
            self.end_run(cx);
        }
        cx.notify();
    }

    fn over(&self) -> bool {
        self.state["over"].is_string()
    }

    fn on_move(&mut self, correct: bool, cx: &mut Context<Self>) {
        if self.stage != Stage::Running {
            return;
        }
        let config = self.config();
        if correct {
            if let Some(next) = step("correctMove", &[self.state.clone(), config]) {
                self.state = next["state"].clone();
                let bonus = next["result"].as_f64().unwrap_or_default();
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
            }
        } else if let Some(next) = step("mistake", &[self.state.clone(), config]) {
            self.state = next;
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
            && let Some(next) = step("puzzleSolved", &[self.state.clone(), json!(rating)])
        {
            self.state = next;
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
            if let Some(next) = step("finish", &[self.state.clone(), json!("complete")]) {
                self.state = next;
            }
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
        if let Some(next) = step("skip", std::slice::from_ref(&self.state)) {
            self.state = next["state"].clone();
            if next["result"] == json!(true) {
                self.advance(cx);
            }
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
        if !self.over()
            && let Some(next) = step("finish", &[self.state.clone(), json!("ended")])
        {
            self.state = next;
        }
        self.stage = Stage::Over;
        self.board
            .update(cx, |board, cx| board.set_frozen(true, cx));
        let config = self.config();
        let accuracy = rules("accuracy", std::slice::from_ref(&self.state)).unwrap_or(Value::Null);
        let state = &self.state;
        let run = json!({
            "kind": config["mode"],
            "variant": config["variant"],
            "score": state["score"],
            "detail": {
                "moves": state["moves"],
                "mistakes": state["mistakes"],
                "accuracy": accuracy,
                "maxCombo": state["maxCombo"],
                "highest": state["highest"],
                "seconds": self.elapsed.as_secs_f64().round(),
            },
        });
        let mode = config["mode"].as_str().unwrap_or("storm").to_string();
        let save = cx.global::<AppCore>().call("saveRun", vec![run]);
        cx.spawn(async move |this, cx| {
            let saved = save.await;
            let _ = this.update(cx, |this, cx| {
                match saved {
                    Ok(saved) => {
                        this.summaries.insert(mode, saved["summary"].clone());
                        if generation == this.generation {
                            this.saved = Some(saved);
                        }
                    }
                    Err(cause) => this.error = Some(cause.message),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn clock_text(ms: f64) -> String {
        rules("formatRunClock", &[json!(ms)])
            .ok()
            .and_then(|text| text.as_str().map(str::to_string))
            .unwrap_or_default()
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
                    .py(px(10.))
                    .rounded(px(10.))
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
                            .font_weight(FontWeight::BOLD)
                            .child(self.title_of(key)),
                    )
                    .children(best.map(|best| {
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(format!("Best: {best}"))
                    }))
            }));
        let mode = config["mode"].as_str().unwrap_or("storm");
        let recent: Vec<Value> = self
            .summaries
            .get(mode)
            .and_then(|summary| summary["recent"].as_array())
            .map(|recent| recent.iter().take(6).cloned().collect())
            .unwrap_or_default();
        let title = config["title"].as_str().unwrap_or_default().to_string();
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
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.text_muted)
                        .child(div().flex_1().child("WHEN"))
                        .child(cell("KIND".into(), 90.))
                        .child(cell("SCORE".into(), 60.))
                        .child(cell("ACCURACY".into(), 80.)),
                )
                .children(recent.iter().map(|run| {
                    use chrono::TimeZone as _;
                    let when = run["playedAt"]
                        .as_f64()
                        .and_then(|at| chrono::Local.timestamp_millis_opt(at as i64).single())
                        .map(|at| at.format("%b %-d, %Y %-I:%M %p").to_string())
                        .unwrap_or_default();
                    let variant = run["variant"].as_str().unwrap_or_default();
                    h_flex()
                        .py(px(3.))
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
                        .child(cell(
                            run["score"].as_f64().unwrap_or_default().to_string(),
                            60.,
                        ))
                        .child(cell(
                            run["detail"]["accuracy"]
                                .as_f64()
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
                    .child(config["rules"].as_str().unwrap_or_default().to_string()),
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
        let number = |key: &str| state[key].as_f64().unwrap_or_default();
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
                    .px(px(10.))
                    .py(px(4.))
                    .rounded_full()
                    .bg(palette.primary)
                    .text_color(palette.inverted)
                    .font_weight(FontWeight::BOLD)
                    .child(format!("+{}s", bonus / 1000.))
            }));
        let title = config["title"].as_str().unwrap_or_default().to_string();
        let mut panel = widgets::side_panel(palette)
            .h(board_size)
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(14.))
            .child(widgets::heading(title, palette));
        if self.stage == Stage::Running {
            let left = state["timeLeftMs"].as_f64();
            let low = left.is_some_and(|left| left < 10_000.);
            let hud = h_flex()
                .justify_between()
                .items_end()
                .children(left.map(|left| {
                    div()
                        .text_size(px(34.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(if low {
                            palette.error
                        } else {
                            palette.highlighted
                        })
                        .child(Self::clock_text(left))
                }))
                .child(
                    h_flex()
                        .gap(px(6.))
                        .items_baseline()
                        .child(
                            div()
                                .text_size(px(26.))
                                .font_weight(FontWeight::BOLD)
                                .child(number("score").to_string()),
                        )
                        .child(div().text_color(palette.text_muted).child("solved")),
                );
            let combo = (config["combo"] == json!(true)).then(|| {
                let progress = rules("comboProgress", &[json!(number("combo"))])
                    .ok()
                    .and_then(|progress| progress.as_f64())
                    .unwrap_or_default();
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
                                    .font_weight(FontWeight::BOLD)
                                    .child(number("combo").to_string()),
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
            let strikes = config["strikes"]
                .as_u64()
                .filter(|strikes| *strikes > 1)
                .map(|strikes| {
                    let used = number("mistakes") as u64;
                    h_flex()
                        .gap(px(6.))
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
            let skips = number("skipsLeft");
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
                        .when(
                            config["skips"].as_f64().is_some_and(|skips| skips > 0.),
                            |row| {
                                row.child(
                                    Button::new("rush-skip")
                                        .outline()
                                        .icon(icon("skip-forward"))
                                        .label(format!("Skip ({skips})"))
                                        .disabled(skips <= 0. || self.locked)
                                        .on_click(cx.listener(|this, _, _, cx| this.skip(cx))),
                                )
                            },
                        )
                        .child(
                            Button::new("rush-end")
                                .outline()
                                .icon(icon("flag"))
                                .label("End run")
                                .on_click(cx.listener(|this, _, _, cx| this.end_run(cx))),
                        ),
                );
        } else {
            let best = self
                .saved
                .as_ref()
                .is_some_and(|saved| saved["isBest"] == json!(true));
            let over = match state["over"].as_str() {
                Some("time") => "Time’s up".to_string(),
                Some("strikes") if config["strikes"].as_u64() == Some(1) => "Streak over".into(),
                Some("strikes") => "Out of strikes".into(),
                Some("complete") => "You cleared every puzzle!".into(),
                _ => "Run ended".into(),
            };
            let detail = if best {
                "A new personal best!".to_string()
            } else if self.saved.is_some() {
                "Saved on this computer.".into()
            } else if let Some(error) = &self.error {
                format!("Not saved: {error}")
            } else {
                "Saving…".into()
            };
            let accuracy = rules("accuracy", std::slice::from_ref(state))
                .ok()
                .and_then(|accuracy| accuracy.as_f64())
                .unwrap_or_default();
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
                            .font_weight(FontWeight::BOLD)
                            .child(value),
                    )
            };
            let highest = number("highest");
            panel = panel
                .child(
                    widgets::StatusBanner::new(
                        if best { Tone::Win } else { Tone::Plain },
                        if best { icon("trophy") } else { icon("flag") },
                        over.into(),
                    )
                    .detail(detail.into()),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap(px(14.))
                        .child(result("Score", number("score").to_string()))
                        .child(result("Moves", number("moves").to_string()))
                        .child(result("Accuracy", format!("{accuracy}%")))
                        .when(config["combo"] == json!(true), |row| {
                            row.child(result("Best combo", number("maxCombo").to_string()))
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
                            cx.listener(|this, _, _, cx| {
                                this.stage = Stage::Menu;
                                cx.notify();
                            }),
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
    use super::{step, training};
    use serde_json::json;

    #[test]
    fn a_storm_run_follows_the_rules() {
        let training = training();
        assert_eq!(training.rush_choices[0], "storm");
        let config = training.rush_configs["storm"].clone();
        let state = step("newRush", std::slice::from_ref(&config)).unwrap();
        assert_eq!(state["timeLeftMs"].as_f64(), Some(180_000.));
        let state = step("mistake", &[state, config]).unwrap();
        assert_eq!(state["timeLeftMs"].as_f64(), Some(170_000.));
        let state = step("tick", &[state, json!(170000)]).unwrap();
        assert_eq!(state["over"], json!("time"));
    }
}
