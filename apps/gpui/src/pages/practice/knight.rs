//! Knight paths: get the knight to the marked square in the fewest jumps. The
//! shortest path is worked out by the rules and told up front; a timed run keeps a best per
//! difficulty and length, untimed practice none.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{IndexPath, Sizable as _, h_flex, v_flex};
use gpui_kit::*;
use kchess_contracts::types::CoordinateMode;
use serde_json::{Value, json};
use shakmaty::{Color, Square};

use super::run::{self, RunClock};
use crate::assets::icon;
use crate::board::{BoardEvent, BoardState, ChessBoard, Circle};
use crate::game::{random_unit, rules};
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::settings::SettingsHandle;
use crate::theme::Palette;
use crate::voice::control::{BarSpec, Control, DrillLog};
use crate::voice::{Attempt, VoiceEvent};
use crate::widgets::{self, Tone};

const DEFAULT_SECONDS: u32 = 60;
const LENGTH: &str = "kchess:knight-length";
/// The pause on a reached goal before the next round.
const ADVANCE: Duration = Duration::from_millis(450);

/// `LEVELS`: the record name, its label and the jumps the shortest path takes.
const LEVELS: [(&str, &str, u32, u32); 3] = [
    ("easy", "Easy · 2–3 jumps", 2, 3),
    ("medium", "Medium · 3–4 jumps", 3, 4),
    ("hard", "Hard · 4–6 jumps", 4, 6),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Running,
    Over,
}

fn knight_moves(square: &str) -> Vec<String> {
    rules("knightMoves", &[json!(square)])
        .ok()
        .and_then(|moves| serde_json::from_value(moves).ok())
        .unwrap_or_default()
}

/// `knightChallenge`: a start, a goal and the shortest path between them, from random draws.
fn knight_challenge(min: u32, max: u32) -> (String, String, u32) {
    let mut draws: Vec<f64> = Vec::new();
    for _ in 0..64 {
        draws.push(random_unit());
        draws.push(random_unit());
        if let Ok(found) = rules("knightChallenge", &[json!(min), json!(max), json!(draws)])
            && !found.is_null()
        {
            return (
                found["from"].as_str().unwrap_or("e4").to_string(),
                found["to"].as_str().unwrap_or("e4").to_string(),
                crate::game::whole(&found["best"]) as u32,
            );
        }
    }
    ("b1".into(), "c3".into(), 1)
}

pub struct KnightPaths {
    level: usize,
    stage: Stage,
    knight: String,
    goal: String,
    best: u32,
    jumps: u32,
    optimal: u32,
    reached: u32,
    missed: u32,
    notice: Option<String>,
    clock: RunClock,
    summary: Value,
    saved: Value,
    level_select: Entity<SelectState<Vec<SharedString>>>,
    board: Entity<ChessBoard>,
    layout: PlayLayout,
    ticker: Option<Task<()>>,
    advance: Option<Task<()>>,
    /// The square to jump to, said aloud.
    voice: Control,
    voice_log: DrillLog,
    /// A start waiting for voice to load.
    starting: bool,
    /// Bumped each round, which with the jumps names what each phrase answered.
    round: u32,
    _subscriptions: Vec<Subscription>,
}

impl KnightPaths {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> KnightPaths {
        let board = cx.new(|_| {
            ChessBoard::new(BoardState::of(
                &kchess_domain::rules::setup_start("standard", crate::game::STANDARD_FEN)
                    .unwrap_or_default(),
                Color::White,
                None,
                None,
            ))
        });
        let level_select = cx.new(|cx| {
            SelectState::new(
                LEVELS
                    .iter()
                    .map(|(_, label, _, _)| SharedString::from(*label))
                    .collect(),
                Some(IndexPath::new(1)),
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event
                    && let Some(to) = uci.get(2..4)
                {
                    this.jump(to, cx);
                }
            }),
            cx.subscribe(
                &crate::voice::handle(cx),
                |this, _, event: &VoiceEvent, cx| this.voice_heard(event, cx),
            ),
            cx.observe(&crate::voice::handle(cx), |this, _, cx| {
                if this.starting && this.voice.gate(&mut this.starting, cx) {
                    this.start(cx);
                }
                cx.notify();
            }),
            cx.subscribe(
                &level_select,
                |this, state, _: &SelectEvent<Vec<SharedString>>, cx| {
                    this.level = state
                        .read(cx)
                        .selected_index(cx)
                        .map_or(1, |index| index.row);
                    cx.notify();
                },
            ),
        ];
        run::load_summary("knightPath", cx, |this, summary, cx| {
            this.summary = summary;
            cx.notify();
        });
        let mut drill = KnightPaths {
            level: 1,
            stage: Stage::Idle,
            // The idle board: a knight in the centre.
            knight: "e4".into(),
            goal: "e4".into(),
            best: 0,
            jumps: 0,
            optimal: 0,
            reached: 0,
            missed: 0,
            notice: None,
            clock: RunClock::default(),
            summary: Value::Null,
            saved: Value::Null,
            level_select,
            board,
            layout: PlayLayout::default(),
            ticker: None,
            advance: None,
            voice: Control::new(false, cx),
            voice_log: DrillLog::default(),
            starting: false,
            round: 0,
            _subscriptions: subscriptions,
        };
        drill.redraw(cx);
        drill
    }

    fn length(cx: &App) -> u32 {
        crate::prefs::get(cx, LENGTH, DEFAULT_SECONDS)
    }

    fn record_variant(&self, cx: &App) -> String {
        run::record_variant(LEVELS[self.level].0, Self::length(cx), DEFAULT_SECONDS)
    }

    fn next_round(&mut self, cx: &mut Context<Self>) {
        self.advance = None;
        self.round += 1;
        let (_, _, min, max) = LEVELS[self.level];
        let (from, to, best) = knight_challenge(min, max);
        self.knight = from;
        self.goal = to;
        self.best = best;
        self.jumps = 0;
        self.redraw(cx);
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
        self.optimal = 0;
        self.reached = 0;
        self.missed = 0;
        self.notice = None;
        self.saved = Value::Null;
        self.next_round(cx);
        self.stage = Stage::Running;
        self.clock.start(Self::length(cx));
        self.ticker = Some(run::ticker(cx, |this, cx| {
            if this.stage != Stage::Running {
                return false;
            }
            if this.clock.expired() {
                this.finish(cx);
                return false;
            }
            cx.notify();
            true
        }));
        self.redraw(cx);
    }

    /// Take the jump when it is one.
    fn jump(&mut self, to: &str, cx: &mut Context<Self>) {
        if self.stage != Stage::Running
            || self.knight == self.goal
            || !knight_moves(&self.knight).iter().any(|square| square == to)
        {
            self.redraw(cx);
            return;
        }
        self.knight = to.to_string();
        self.jumps += 1;
        crate::sound::play(crate::sound::Sound::Move, cx);
        if self.knight == self.goal {
            self.reached += 1;
            if self.jumps <= self.best {
                self.optimal += 1;
                self.notice = Some("Shortest path".into());
            } else {
                self.missed += 1;
                self.notice = Some(format!(
                    "Reached it in {}; the shortest was {}",
                    self.jumps, self.best
                ));
            }
            self.advance = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(ADVANCE).await;
                let _ = this.update(cx, |this, cx| this.next_round(cx));
            }));
        }
        self.redraw(cx);
    }

    /// A spoken square is where the knight jumps to.
    fn voice_heard(&mut self, event: &VoiceEvent, cx: &mut Context<Self>) {
        if !self.voice.owns(event) || self.stage != Stage::Running {
            return;
        }
        let (heard, missed) = match event {
            VoiceEvent::Heard { heard, .. } => (heard, false),
            VoiceEvent::Missed { heard, .. } => (heard, true),
        };
        let asked = format!("{}:{}", self.round, self.jumps);
        let goal = self.goal.clone();
        let log = |this: &mut Self, outcome: &str, parsed: Option<&str>, cx: &mut Context<Self>| {
            let attempt = Attempt {
                source: "knightPath",
                heard,
                outcome,
                parsed,
                expected: Some(&goal),
                fen: None,
            };
            this.voice_log.log(&attempt, &asked, cx);
        };
        if missed {
            return log(self, "unclear", None, cx);
        }
        if self.knight == self.goal {
            return;
        }
        let square = rules("spokenSquare", &[json!(heard.text)])
            .ok()
            .and_then(|square| square.as_str().map(str::to_string));
        let feedback = match square {
            None => {
                log(self, "invalid", None, cx);
                "Say a square, such as “F three” or “Foxtrot three”".to_string()
            }
            Some(square) if !knight_moves(&self.knight).contains(&square) => {
                log(self, "invalid", Some(&square), cx);
                format!("{square} · not a knight jump from {}", self.knight)
            }
            Some(square) => {
                log(self, "played", Some(&square), cx);
                self.jump(&square, cx);
                square
            }
        };
        self.voice.feedback = Some(feedback.into());
        cx.notify();
    }

    fn skip(&mut self, cx: &mut Context<Self>) {
        // The goal is already reached: only move on sooner.
        if self.knight != self.goal {
            self.missed += 1;
            self.notice = Some(format!(
                "Skipped — the shortest path was {} jumps",
                self.best
            ));
        }
        self.next_round(cx);
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        if self.stage != Stage::Running {
            return;
        }
        self.advance = None;
        self.clock.stop();
        self.stage = Stage::Over;
        self.redraw(cx);
        // An untimed score only grows with time, so it is never a record.
        if self.clock.untimed {
            return;
        }
        let run = json!({
            "kind": "knightPath",
            "variant": self.record_variant(cx),
            "score": self.optimal,
            "detail": { "reached": self.reached, "skipped": self.missed },
        });
        run::save_run(run, cx, |this, saved, cx| {
            this.summary = saved["summary"].clone();
            this.saved = saved;
            cx.notify();
        });
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        let running = self.stage == Stage::Running;
        self.voice
            .sync(crate::voice::grammar("COORDINATE_GRAMMAR"), running, cx);
        let fen = rules("knightFen", &[json!(self.knight)])
            .ok()
            .and_then(|fen| fen.as_str().map(str::to_string))
            .unwrap_or_default();
        let position = kchess_domain::rules::setup_start("standard", crate::game::STANDARD_FEN)
            .unwrap_or_default();
        let movable = running && self.knight != self.goal;
        let mut state = BoardState::of(&position, Color::White, None, None);
        // A lone knight is no legal position: the board draws the FEN's pieces.
        state.fen = fen;
        if movable && let Ok(from) = self.knight.parse::<Square>() {
            state.movable = Some(Color::White);
            state.dests = [(
                from,
                knight_moves(&self.knight)
                    .iter()
                    .filter_map(|square| square.parse().ok())
                    .collect(),
            )]
            .into_iter()
            .collect();
        }
        let frontend = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .clone();
        state.coordinates = Some(if frontend.coordinates == CoordinateMode::None {
            CoordinateMode::Inside
        } else {
            frontend.coordinates
        });
        state.blindfold = frontend.blindfold;
        if running && let Ok(goal) = self.goal.parse::<Square>() {
            state.circles.push(Circle::green(goal));
        }
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }
}

impl Render for KnightPaths {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let running = self.stage == Stage::Running;
        let target = format!(
            "Reach {} in {} {}",
            self.goal,
            self.best,
            if self.best == 1 { "jump" } else { "jumps" }
        );
        let board_size = self.layout.board_size(window);
        let frontend = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .clone();
        let length = Self::length(cx);
        let mut panel = widgets::side_panel(&palette)
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(14.))
            .child(
                h_flex()
                    .justify_between()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child("Reach the green square in the fewest moves"),
                    )
                    .child(widgets::view_toggles(frontend.blindfold, frontend.zen_mode)),
            )
            .child(run::field(
                "Difficulty",
                Select::new(&self.level_select)
                    .small()
                    .disabled(running)
                    .accessibility_label("Difficulty"),
                &palette,
            ))
            .child(run::length_picker(
                "knight-length",
                length,
                running,
                move |seconds, _, cx| crate::prefs::set(cx, LENGTH, seconds),
                &palette,
            ))
            .child(self.voice.bar(
                cx.entity().downgrade(),
                |this: &mut Self| &mut this.voice,
                BarSpec {
                    id: "knight-voice",
                    hint: "Say the square to jump to, such as “F three”",
                    active: running,
                    grammar: crate::voice::grammar("COORDINATE_GRAMMAR"),
                },
                cx,
            ))
            .child(div().h(px(1.)).bg(palette.border));
        if running {
            panel = panel
                .child(run::hud(
                    &self.clock,
                    f64::from(self.optimal),
                    "shortest paths",
                    &palette,
                ))
                .child(
                    h_flex()
                        .items_baseline()
                        .justify_between()
                        .gap(px(8.))
                        .text_size(px(18.))
                        .child(
                            div()
                                .id("knight-target")
                                .test_support()
                                .aria_label(target.clone())
                                .font_weight(FontWeight::MEDIUM)
                                .child(target),
                        )
                        .child(
                            div()
                                .text_color(palette.text_muted)
                                .child(format!("{} so far", self.jumps)),
                        ),
                )
                .children(self.notice.clone().map(|notice| {
                    div()
                        .id("knight-notice")
                        .test_support()
                        .aria_label(notice.clone())
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .child(notice)
                }))
                .child(
                    Button::new("knight-skip")
                        .outline()
                        .icon(icon("skip-forward"))
                        .label("Skip")
                        .on_click(cx.listener(|this, _, _, cx| this.skip(cx))),
                )
                .child(
                    Button::new("knight-end")
                        .ghost()
                        .icon(icon("flag"))
                        .label("End run")
                        .on_click(cx.listener(|this, _, _, cx| this.finish(cx))),
                );
        } else {
            let best = run::best(&self.summary, &self.record_variant(cx));
            if self.stage == Stage::Over {
                let is_best = !self.clock.untimed && self.saved["isBest"] == json!(true);
                panel = panel.child(
                    widgets::StatusBanner::new(
                        if is_best { Tone::Win } else { Tone::Plain },
                        if is_best {
                            icon("trophy")
                        } else {
                            icon("timer")
                        },
                        format!("{} shortest paths · {} reached", self.optimal, self.reached)
                            .into(),
                    )
                    .announce()
                    .detail(
                        if self.clock.untimed {
                            format!("in {}", self.clock.text())
                        } else if is_best {
                            "New personal best".to_string()
                        } else {
                            format!("Best so far: {}", best.unwrap_or(f64::from(self.optimal)))
                        }
                        .into(),
                    ),
                );
            } else if length != 0 {
                panel = panel.child(
                    div()
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .child(format!(
                            "Best ({}, {}): {}",
                            LEVELS[self.level].0,
                            run::length_label(length),
                            best.map_or_else(|| "—".to_string(), |best| best.to_string())
                        )),
                );
            }
            panel = panel.child(
                Button::new("knight-start")
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
                    .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
            );
        }
        let board_column = v_flex()
            .w(board_size)
            .flex_none()
            .child(crate::board::cached(&self.board));
        div().size_full().child(
            self.layout
                .render("knight-paths", board_column, panel, window, cx),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{knight_challenge, knight_moves};

    #[test]
    fn challenges_have_the_asked_length() {
        for _ in 0..10 {
            let (from, to, best) = knight_challenge(3, 4);
            assert_ne!(from, to);
            assert!((3..=4).contains(&best), "{from} to {to} in {best}");
        }
        assert_eq!(knight_moves("a1").len(), 2);
    }
}
