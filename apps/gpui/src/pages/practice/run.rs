//! What the practice drills share (`useRunLength.ts`, `RunLengthPicker.vue`, `utils/countdown.ts`):
//! the length of a run and its clock, a countdown that ends it or a stopwatch for untimed practice,
//! and the best scores the core keeps for each drill (`runSummary`, `saveRun`).

use std::time::{Duration, Instant};

use gpui_kit::component::h_flex;
use gpui_kit::component::v_flex;
use gpui_kit::*;
use kchess_core::Level;
use serde_json::{Value, json};

use crate::core::AppCore;
use crate::game::rules;
use crate::theme::{Palette, tint};
use crate::widgets::{self, Pill};

/// Run lengths in seconds; 0 is untimed practice.
pub const LENGTHS: [(u32, &str); 5] = [
    (30, "30s"),
    (60, "1 min"),
    (120, "2 min"),
    (300, "5 min"),
    (0, "Untimed"),
];

/// How often a running clock is redrawn (`useCountdown` ticks every 100 ms).
const TICK: Duration = Duration::from_millis(100);

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

    /// What the clock shows: time left, or time spent when untimed (`formatRunClock`).
    pub fn text(&self) -> String {
        let ms = if self.untimed {
            self.elapsed()
        } else {
            self.left()
        }
        .as_millis() as f64;
        rules("formatRunClock", &[json!(ms)])
            .ok()
            .and_then(|text| text.as_str().map(str::to_string))
            .unwrap_or_default()
    }

    /// Under ten seconds left.
    pub fn low(&self) -> bool {
        !self.untimed && self.left() < Duration::from_secs(10)
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

/// Redraw a drill while its clock runs; `tick` says whether to keep going.
pub fn ticker<V: 'static>(
    cx: &mut Context<V>,
    tick: fn(&mut V, &mut Context<V>) -> bool,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            match this.update(cx, tick) {
                Ok(true) => {}
                _ => break,
            }
        }
    })
}

/// A drill's saved runs (`RunSummary`), read when it opens.
pub fn load_summary<V: 'static>(
    kind: &'static str,
    cx: &mut Context<V>,
    keep: fn(&mut V, Value, &mut Context<V>),
) {
    let summary = cx.global::<AppCore>().call("runSummary", vec![json!(kind)]);
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
    run: Value,
    cx: &mut Context<V>,
    keep: fn(&mut V, Value, &mut Context<V>),
) {
    let saved = cx.global::<AppCore>().call("saveRun", vec![run]);
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
pub fn best(summary: &Value, variant: &str) -> Option<f64> {
    summary["best"][variant]["score"].as_f64()
}

/// `.field` with its label above the control.
pub fn field(
    label: &'static str,
    control: impl IntoElement,
    palette: &Palette,
) -> impl IntoElement {
    v_flex()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.highlighted)
                .child(label),
        )
        .child(control)
}

/// `RunLengthPicker.vue`.
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

/// `.run-hud`: the clock, and the score with what it counts.
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
            div()
                .text_size(px(44.))
                .font_weight(FontWeight(750.))
                .line_height(relative(1.))
                .text_color(if clock.low() {
                    palette.loss
                } else {
                    palette.text
                })
                .child(clock.text()),
        )
        .child(
            v_flex()
                .items_end()
                .child(
                    div()
                        .text_size(px(32.))
                        .font_weight(FontWeight::BOLD)
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

/// `ScoreSparkline.vue`: recent scores, oldest first, as a small area with headroom above the best.
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
    use super::{Duration, RunClock, length_label, record_variant};

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
        // At the start, and with a second gone; the clock rounds down, as Electron's does.
        clock.stopped_at = Some(Duration::ZERO);
        assert_eq!(clock.text(), "0:30");
        clock.stopped_at = Some(Duration::from_millis(1500));
        assert_eq!(clock.text(), "0:28");
        clock.start(0);
        assert!(clock.untimed);
        assert!(!clock.expired());
    }
}
