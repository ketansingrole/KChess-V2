//! What the practice drills share:
//! the length of a run and its clock, a countdown that ends it or a stopwatch for untimed practice,
//! and the best scores the core keeps for each drill (`runSummary`, `saveRun`).

use kchess_contracts::calls;
use kchess_contracts::types::{RunInput, RunKind, RunSaved, RunSummary};
use std::time::{Duration, Instant};

use gpui_kit::component::h_flex;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::Level;
use serde_json::Value;

use crate::assets::icon;
use crate::core::AppCore;
use crate::theme::{Palette, radii, tint};
use crate::widgets::Pressable as _;
use crate::widgets::{self, KeyboardFocus as _, Pill};

/// Run lengths in seconds; 0 is untimed practice.
pub const LENGTHS: [(u32, &str); 5] = [
    (30, "30s"),
    (60, "1 min"),
    (120, "2 min"),
    (300, "5 min"),
    (0, "Untimed"),
];

/// The clock of one run: a countdown from its length, or a stopwatch when untimed.
#[derive(Default)]
pub struct RunClock {
    started: Option<Instant>,
    total: Duration,
    /// Whether the run on screen (or just ended) was untimed, whatever is chosen since.
    pub untimed: bool,
    stopped_at: Option<Duration>,
}

impl RunClock {
    pub fn start(&mut self, seconds: u32) {
        self.untimed = seconds == 0;
        self.total = Duration::from_secs(u64::from(seconds));
        self.started = Some(Instant::now());
        self.stopped_at = None;
    }

    pub fn stop(&mut self) {
        self.stopped_at = Some(self.elapsed());
    }

    fn elapsed(&self) -> Duration {
        self.stopped_at
            .or_else(|| self.started.map(|started| started.elapsed()))
            .unwrap_or_default()
    }

    fn left(&self) -> Duration {
        self.total.saturating_sub(self.elapsed())
    }

    /// The countdown has reached zero.
    pub fn expired(&self) -> bool {
        !self.untimed && self.started.is_some() && self.left().is_zero()
    }

    /// What the clock shows: time left, or time spent when untimed.
    pub fn text(&self) -> String {
        let ms = if self.untimed {
            self.elapsed()
        } else {
            self.left()
        }
        .as_millis() as f64;
        kchess_domain::training::rush::format_run_clock(ms)
    }

    /// Under ten seconds left.
    pub fn low(&self) -> bool {
        !self.untimed && self.left() < Duration::from_secs(10)
    }
}

/// Where a drill is: choosing, in a run, or showing the run that ended.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Stage {
    #[default]
    Idle,
    Running,
    Over,
}

/// A timed drill: its run, and what it does when the run ends.
pub trait Drill: Sized + 'static {
    fn run(&mut self) -> &mut DrillRun;
    fn finish(&mut self, cx: &mut Context<Self>);
}

/// What every timed drill keeps of its run: the stage, the clock and its ticks, and the saved
/// runs with whether the last one is a best.
#[derive(Default)]
pub struct DrillRun {
    pub stage: Stage,
    pub clock: RunClock,
    pub summary: Option<RunSummary>,
    pub is_best: bool,
    ticker: Option<Task<()>>,
}

impl DrillRun {
    pub fn running(&self) -> bool {
        self.stage == Stage::Running
    }

    /// Start a run of `seconds` (0 is untimed): the view redraws each tick and finishes when the
    /// countdown ends.
    pub fn begin<V: Drill>(&mut self, seconds: u32, cx: &mut Context<V>) {
        self.stage = Stage::Running;
        self.is_best = false;
        self.clock.start(seconds);
        self.ticker = Some(crate::timer::every(
            crate::timer::TICK,
            cx,
            |view: &mut V, cx| {
                let run = view.run();
                if !run.running() {
                    return false;
                }
                if run.clock.expired() {
                    view.finish(cx);
                    return false;
                }
                cx.notify();
                true
            },
        ));
    }

    /// End the run; false when none was running.
    pub fn end(&mut self) -> bool {
        if !self.running() {
            return false;
        }
        self.clock.stop();
        self.stage = Stage::Over;
        true
    }

    /// Read the drill's saved runs into its `summary`.
    pub fn load<V: Drill>(kind: RunKind, cx: &mut Context<V>) {
        load_summary(kind, cx, |view: &mut V, summary, cx| {
            view.run().summary = Some(summary);
            cx.notify();
        });
    }

    /// Save a finished run, and keep whether it is a best.
    pub fn save<V: Drill>(run: RunInput, cx: &mut Context<V>) {
        save_run(run, cx, |view: &mut V, saved, cx| {
            let run = view.run();
            run.is_best = saved.is_best;
            run.summary = Some(saved.summary);
            cx.notify();
        });
    }

    /// The best score of a variant in the saved runs.
    pub fn best(&self, variant: &str) -> Option<f64> {
        best(self.summary.as_ref(), variant)
    }

    /// Whether the clock is ticking.
    #[cfg(test)]
    pub fn ticking(&self) -> bool {
        self.ticker.is_some()
    }
}

/// Right answers and mistakes in a run.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Tally {
    pub score: u32,
    pub mistakes: u32,
}

impl Tally {
    pub fn record(&mut self, correct: bool) {
        if correct {
            self.score += 1;
        } else {
            self.mistakes += 1;
        }
    }

    /// Right answers as a percentage of all answers, rounded.
    pub fn accuracy(&self) -> u32 {
        let total = self.score + self.mistakes;
        if total == 0 {
            0
        } else {
            (f64::from(self.score) / f64::from(total) * 100.).round() as u32
        }
    }
}

/// The label of a length (`30s`, `1 min`, `Untimed`).
pub fn length_label(seconds: u32) -> String {
    LENGTHS
        .iter()
        .find(|(value, _)| *value == seconds)
        .map_or_else(|| format!("{seconds}s"), |(_, label)| (*label).to_string())
}

/// The record name for a run of a length (`find`, `find-120s`): the drill's original length keeps
/// the original name, so earlier bests still count.
pub fn record_variant(base: &str, seconds: u32, default_seconds: u32) -> String {
    if seconds == default_seconds {
        base.to_string()
    } else {
        format!("{base}-{seconds}s")
    }
}

/// A drill's saved runs (`RunSummary`), read when it opens.
pub fn load_summary<V: 'static>(
    kind: RunKind,
    cx: &mut Context<V>,
    keep: fn(&mut V, RunSummary, &mut Context<V>),
) {
    let summary = cx.global::<AppCore>().request(calls::RunSummary { kind });
    cx.spawn(async move |this, cx| match summary.await {
        Ok(summary) => {
            let _ = this.update(cx, |view, cx| keep(view, summary, cx));
        }
        Err(cause) => crate::log::write(
            Level::Warn,
            "practice",
            &format!("Could not load saved runs: {}", cause.message),
        ),
    })
    .detach();
}

/// Save a finished run; `keep` gets the `RunSaved` (its summary, and whether it is a best).
pub fn save_run<V: 'static>(
    run: RunInput,
    cx: &mut Context<V>,
    keep: fn(&mut V, RunSaved, &mut Context<V>),
) {
    let saved = cx.global::<AppCore>().request(calls::SaveRun { run });
    cx.spawn(async move |this, cx| match saved.await {
        Ok(saved) => {
            let _ = this.update(cx, |view, cx| keep(view, saved, cx));
        }
        // The score is still on screen; only the record is lost.
        Err(cause) => crate::log::write(
            Level::Warn,
            "practice",
            &format!("Could not save run: {}", cause.message),
        ),
    })
    .detach();
}

/// The best score of a variant in a summary.
pub fn best(summary: Option<&RunSummary>, variant: &str) -> Option<f64> {
    Some(summary?.best.get(variant)?.score)
}

/// A finished run of `kind` to save; `detail` is an object of numbers, words and flags.
pub fn input(kind: RunKind, variant: impl Into<String>, score: f64, detail: Value) -> RunInput {
    RunInput {
        kind,
        variant: variant.into(),
        score,
        detail: match detail {
            Value::Object(detail) => detail,
            _ => serde_json::Map::new(),
        },
    }
}

/// A form field: its label above the control.
pub fn field(
    label: &'static str,
    control: impl IntoElement,
    palette: &Palette,
) -> impl IntoElement {
    v_flex()
        .gap(px(8.))
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.highlighted)
                .child(label),
        )
        .child(control)
}

pub fn length_picker(
    id: &'static str,
    seconds: u32,
    disabled: bool,
    choose: impl Fn(u32, &mut Window, &mut App) + Clone + 'static,
    palette: &Palette,
) -> impl IntoElement {
    field(
        "Length",
        widgets::pills(
            id,
            LENGTHS
                .iter()
                .map(|(_, label)| Pill {
                    icon: None,
                    label: (*label).into(),
                    badge: None,
                    disabled,
                })
                .collect(),
            LENGTHS
                .iter()
                .position(|(value, _)| *value == seconds)
                .unwrap_or(0),
            move |index, window, cx| choose(LENGTHS[index].0, window, cx),
            false,
            palette,
        ),
        palette,
    )
}

/// The clock, and the score with what it counts.
pub fn hud(
    clock: &RunClock,
    score: f64,
    what: &'static str,
    palette: &Palette,
) -> impl IntoElement {
    h_flex()
        .items_end()
        .justify_between()
        .gap(px(12.))
        .child(
            crate::widgets::timer(clock.text(), px(40.)).text_color(if clock.low() {
                palette.loss
            } else {
                palette.text
            }),
        )
        .child(
            v_flex()
                .items_end()
                .child(
                    div()
                        .id("run-score")
                        .test_support()
                        .aria_label(format!("{score}"))
                        .text_size(px(32.))
                        .font_weight(FontWeight::MEDIUM)
                        .line_height(relative(1.))
                        .child(format!("{score}")),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .child(what),
                ),
        )
}

/// A row of the training list: an endgame drill or a lesson chapter.
pub struct TrainingItem {
    pub id: String,
    pub title: String,
    pub done: bool,
    /// A short label at the end of the row (a level).
    pub aside: Option<String>,
}

/// Grouped practice items with a tick for each finished one.
pub fn training_list(
    groups: Vec<(Option<&'static str>, Vec<TrainingItem>)>,
    active: &str,
    select: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
    palette: &Palette,
) -> impl IntoElement {
    v_flex()
        .gap(px(8.))
        .children(groups.into_iter().map(|(label, items)| {
            v_flex()
                .gap(px(4.))
                .children(label.map(|label| {
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.highlighted)
                        .child(label)
                }))
                .children(items.into_iter().map(|item| {
                    let current = item.id == active;
                    let select = select.clone();
                    let id = item.id.clone();
                    h_flex()
                        .id(SharedString::from(format!("training-{}", item.id)))
                        .test_support()
                        .role(Role::Button)
                        .keyboard_focus(palette.primary)
                        .aria_label(item.title.clone())
                        .aria_selected(current)
                        .gap(px(8.))
                        .px(px(12.))
                        .py(px(8.))
                        .border_1()
                        .rounded(radii::MD)
                        .text_size(px(13.))
                        .cursor_pointer()
                        .border_color(if current {
                            palette.primary
                        } else {
                            gpui_kit::transparent_black()
                        })
                        .when(current, |row| row.bg(tint(palette.primary, 10.)))
                        .when(!current, |row| row.pressable(palette))
                        .on_click(move |_, window, cx| select(id.clone(), window, cx))
                        .child(
                            if item.done {
                                icon("circle-check")
                            } else {
                                icon("circle")
                            }
                            .size(px(16.))
                            .flex_none()
                            .text_color(if item.done {
                                palette.success
                            } else {
                                palette.text_muted
                            }),
                        )
                        .child(div().min_w_0().child(item.title))
                        .children(item.aside.map(|aside| {
                            div()
                                .ml_auto()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(aside)
                        }))
                }))
        }))
}

/// Recent scores, oldest first, as a small area with headroom above the best.
pub fn sparkline(scores: Vec<f64>, palette: &Palette) -> impl IntoElement {
    let color = palette.primary;
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            if scores.len() < 2 {
                return;
            }
            let top = scores.iter().copied().fold(1., f64::max) * 1.25;
            let width = bounds.size.width.as_f32();
            let height = bounds.size.height.as_f32() - 2.;
            let at = |index: usize, score: f64| {
                point(
                    bounds.origin.x + px(width * index as f32 / (scores.len() - 1) as f32),
                    bounds.origin.y + px(2. + height * (1. - (score / top) as f32)),
                )
            };
            let mut area = PathBuilder::fill();
            area.move_to(point(bounds.origin.x, bounds.bottom()));
            for (index, score) in scores.iter().enumerate() {
                area.line_to(at(index, *score));
            }
            area.line_to(point(bounds.right(), bounds.bottom()));
            area.close();
            if let Ok(path) = area.build() {
                window.paint_path(path, tint(color, 18.));
            }
            let mut line = PathBuilder::stroke(px(2.));
            line.move_to(at(0, scores[0]));
            for (index, score) in scores.iter().enumerate().skip(1) {
                line.line_to(at(index, *score));
            }
            if let Ok(path) = line.build() {
                window.paint_path(path, color);
            }
        },
    )
    .w_full()
    .h(px(44.))
}

#[cfg(test)]
mod tests {
    use super::{Duration, RunClock, Tally, length_label, record_variant};

    #[test]
    fn a_tally_counts_answers_and_rounds_its_accuracy() {
        let mut tally = Tally::default();
        assert_eq!(tally.accuracy(), 0);
        tally.record(true);
        tally.record(true);
        tally.record(false);
        assert_eq!(
            tally,
            Tally {
                score: 2,
                mistakes: 1
            }
        );
        assert_eq!(tally.accuracy(), 67);
        tally.record(true);
        assert_eq!(tally.accuracy(), 75);
    }

    #[test]
    fn records_keep_their_names_per_length() {
        assert_eq!(record_variant("find", 30, 30), "find");
        assert_eq!(record_variant("find", 120, 30), "find-120s");
        assert_eq!(length_label(60), "1 min");
        assert_eq!(length_label(0), "Untimed");
    }

    #[test]
    fn a_countdown_shows_what_is_left() {
        let mut clock = RunClock::default();
        clock.start(30);
        assert!(!clock.expired());
        assert!(!clock.low());
        // At the start, and with a second gone; the clock rounds down.
        clock.stopped_at = Some(Duration::ZERO);
        assert_eq!(clock.text(), "0:30");
        clock.stopped_at = Some(Duration::from_millis(1500));
        assert_eq!(clock.text(), "0:28");
        clock.start(0);
        assert!(clock.untimed);
        assert!(!clock.expired());
    }
}
