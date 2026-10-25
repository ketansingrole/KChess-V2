//! Square colours (`SquareColorTrainer.vue`): is this square light or dark? Answer with the buttons
//! L and D, or by voice, for thirty seconds; the best score stays on this computer.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use super::coordinates::random_square;
use super::run::{self, RunClock};
use crate::assets::icon;
use crate::game::rules;
use crate::play_layout::{PAGE_BOTTOM, side_padding};
use crate::theme::{Palette, tint};
use crate::voice::control::{BarSpec, Control, DrillLog};
use crate::voice::{Attempt, VoiceEvent};
use crate::widgets;

const FLASH: Duration = Duration::from_millis(300);
const COLOR_GRAMMAR: [&str; 3] = ["light", "dark", "[unk]"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Running,
    Over,
}

/// `squareColor`: a1 is dark, h1 is light.
fn square_color(square: &str) -> &'static str {
    match rules("squareColor", &[json!(square)]) {
        Ok(color) if color == "light" => "light",
        _ => "dark",
    }
}

pub struct SquareColors {
    stage: Stage,
    square: String,
    score: u32,
    mistakes: u32,
    flash: Option<bool>,
    clock: RunClock,
    summary: Value,
    saved: Value,
    timers: Vec<Task<()>>,
    voice: Control,
    voice_log: DrillLog,
    /// A start waiting for voice to load.
    starting: bool,
    /// Bumped for each square asked, which names what each phrase answered.
    question: u32,
    _subscriptions: Vec<Subscription>,
}

fn grammar() -> Vec<String> {
    COLOR_GRAMMAR.map(String::from).to_vec()
}

impl SquareColors {
    pub fn new(cx: &mut Context<Self>) -> SquareColors {
        run::load_summary("squareColor", cx, |this, summary, cx| {
            this.summary = summary;
            cx.notify();
        });
        let voice = crate::voice::handle(cx);
        let subscriptions = vec![
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
        SquareColors {
            stage: Stage::Idle,
            square: "e4".into(),
            score: 0,
            mistakes: 0,
            flash: None,
            clock: RunClock::default(),
            summary: Value::Null,
            saved: Value::Null,
            timers: Vec::new(),
            voice: Control::new(false, cx),
            voice_log: DrillLog::default(),
            starting: false,
            question: 0,
            _subscriptions: subscriptions,
        }
    }

    /// The recognizer takes a fresh phrase for each square, during a run.
    fn sync_voice(&mut self, cx: &mut Context<Self>) {
        let running = self.stage == Stage::Running;
        self.voice.sync(grammar(), running, cx);
    }

    fn voice_heard(&mut self, event: &VoiceEvent, cx: &mut Context<Self>) {
        if !self.voice.owns(event) || self.stage != Stage::Running {
            return;
        }
        let (heard, missed) = match event {
            VoiceEvent::Heard { heard, .. } => (heard, false),
            VoiceEvent::Missed { heard, .. } => (heard, true),
        };
        let expected = square_color(&self.square);
        let text = heard.text.to_lowercase();
        let words: Vec<&str> = text.split_whitespace().collect();
        let color = ["light", "dark"]
            .into_iter()
            .find(|color| !missed && words.contains(color));
        let outcome = match color {
            _ if missed => "unclear",
            None => "invalid",
            Some(color) if color == expected => "correct",
            Some(_) => "wrong",
        };
        let attempt = Attempt {
            source: "squareColor",
            heard,
            outcome,
            parsed: color,
            expected: Some(expected),
            fen: None,
        };
        self.voice_log.log(&attempt, &self.question.to_string(), cx);
        if missed {
            return;
        }
        let Some(color) = color else {
            self.voice.feedback = Some("Say “light” or “dark”.".into());
            cx.notify();
            return;
        };
        self.answer(color, cx);
        self.voice.feedback = Some(format!("{color} · {outcome}").into());
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        // The microphone and model are ready before the clock starts, as in Coordinates.
        if self.stage == Stage::Running
            || (self.voice.enabled && !self.voice.gate(&mut self.starting, cx))
        {
            cx.notify();
            return;
        }
        self.voice.feedback = None;
        self.question += 1;
        self.score = 0;
        self.mistakes = 0;
        self.saved = Value::Null;
        self.square = random_square(None, None);
        self.stage = Stage::Running;
        self.clock.start(30);
        self.sync_voice(cx);
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
        cx.notify();
    }

    fn answer(&mut self, color: &str, cx: &mut Context<Self>) {
        if self.stage != Stage::Running {
            return;
        }
        let correct = square_color(&self.square) == color;
        self.flash = Some(correct);
        self.timers.push(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FLASH).await;
            let _ = this.update(cx, |this, cx| {
                this.flash = None;
                cx.notify();
            });
        }));
        if correct {
            self.score += 1;
        } else {
            self.mistakes += 1;
            crate::sound::play_at(crate::sound::Sound::LowTime, 0.5, cx);
        }
        self.question += 1;
        self.square = random_square(Some(&self.square), None);
        self.sync_voice(cx);
        cx.notify();
    }

    pub fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "l" => self.answer("light", cx),
            "d" => self.answer("dark", cx),
            _ => {}
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
        self.sync_voice(cx);
        cx.notify();
        let run = json!({
            "kind": "squareColor",
            "variant": "standard",
            "score": self.score,
            "detail": { "mistakes": self.mistakes, "accuracy": self.accuracy() },
        });
        run::save_run(run, cx, |this, saved, cx| {
            this.summary = saved["summary"].clone();
            this.saved = saved;
            cx.notify();
        });
    }
}

impl Render for SquareColors {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let best = run::best(&self.summary, "standard");
        let muted = |text: String| div().text_color(palette.text_muted).child(text);
        let stage: Vec<AnyElement> = match self.stage {
            Stage::Running => vec![
                div()
                    .text_size(px(18.))
                    .font_weight(FontWeight(650.))
                    .text_color(if self.clock.low() {
                        palette.loss
                    } else {
                        palette.text_muted
                    })
                    .child(self.clock.text())
                    .into_any_element(),
                div()
                    .text_size(px(72.))
                    .font_weight(FontWeight::BOLD)
                    .line_height(relative(1.))
                    .child(self.square.clone())
                    .into_any_element(),
                muted(format!("{} correct", self.score)).into_any_element(),
            ],
            Stage::Over => vec![
                div()
                    .text_size(px(36.))
                    .font_weight(FontWeight::BOLD)
                    .child(format!("{} correct", self.score))
                    .into_any_element(),
                muted(format!("{}% accuracy", self.accuracy())).into_any_element(),
                if self.saved["isBest"] == json!(true) {
                    div()
                        .text_color(palette.success)
                        .child("A new personal best!")
                        .into_any_element()
                } else {
                    muted(format!(
                        "Best so far: {}",
                        best.unwrap_or(f64::from(self.score))
                    ))
                    .into_any_element()
                },
            ],
            Stage::Idle => vec![
                div()
                    .text_size(px(36.))
                    .font_weight(FontWeight::BOLD)
                    .child("Ready?")
                    .into_any_element(),
                muted(format!(
                    "Best: {}",
                    best.map_or_else(|| "—".to_string(), |best| best.to_string())
                ))
                .into_any_element(),
            ],
        };
        let key = |key: &'static str| {
            div()
                .ml(px(6.))
                .px(px(5.))
                .rounded(px(4.))
                .border_1()
                .border_color(palette.border_accented)
                .text_size(px(11.))
                .child(key)
        };
        // The answers in the squares' own colors (`.answer-light`, and a solid neutral button):
        // GPUI Kit's custom buttons only tint, so these are drawn here.
        let answer =
            |id: &'static str, label: &'static str, shortcut: &'static str, light: bool| {
                let (bg, fg) = if light {
                    (rgb(0xf0d9b5).into(), rgb(0x3a2e1e).into())
                } else {
                    (palette.highlighted, palette.bg)
                };
                let color: &'static str = if light { "light" } else { "dark" };
                h_flex()
                    .id(id)
                    .flex_1()
                    .h(px(44.))
                    .justify_center()
                    .gap(px(6.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if light { palette.border_accented } else { bg })
                    .bg(bg)
                    .text_color(fg)
                    .text_size(px(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .hover(|button| button.opacity(0.9))
                    .on_click(cx.listener(move |this, _, _, cx| this.answer(color, cx)))
                    .child(label)
                    .child(key(shortcut))
            };
        let answers = if self.stage == Stage::Running {
            h_flex()
                .gap(px(10.))
                .child(answer("answer-light", "Light", "L", true))
                .child(answer("answer-dark", "Dark", "D", false))
        } else {
            h_flex().child(
                Button::new("square-colors-start")
                    .primary()
                    .large()
                    .flex_1()
                    .icon(icon("play"))
                    .label(if self.stage == Stage::Over {
                        "Play again"
                    } else {
                        "Start"
                    })
                    .loading(self.starting)
                    .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
            )
        };
        let card = widgets::card(&palette)
            .w_full()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child("Name each square’s color · 30 seconds"),
            )
            .child(
                v_flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .min_h(px(220.))
                    .my(px(16.))
                    .rounded(px(14.))
                    .when(self.flash == Some(true), |stage| {
                        stage.bg(tint(palette.win, 22.))
                    })
                    .when(self.flash == Some(false), |stage| {
                        stage.bg(tint(palette.loss, 22.))
                    })
                    .children(stage),
            )
            .child(answers)
            .child(div().mt(px(16.)).child(self.voice.bar(
                cx.entity().downgrade(),
                |this: &mut Self| &mut this.voice,
                BarSpec {
                    id: "square-colors-voice",
                    hint: "Say “light” or “dark”.",
                    active: self.stage == Stage::Running,
                    grammar: grammar(),
                },
                cx,
            )));
        div()
            .id("square-colors")
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .px(side_padding(window))
                    .pb(PAGE_BOTTOM)
                    .child(card),
            )
    }
}
