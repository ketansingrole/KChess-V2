//! One endgame course (`EndgameLesson.vue`): its chapters, each a few steps to read, follow or
//! solve, against scripted replies or Stockfish. A finished chapter is saved as a `lesson` run.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_core::Level;
use serde_json::{Value, json};
use shakmaty::Color;

use super::endgame_drills::{color_of, move_squares, san_from};
use super::endgames::{Course, LessonChapter, LessonStep, TrainerEvent, finished};
use super::engine_move::{EngineMove, HasEngine};
use super::run::{self, TrainingItem};
use crate::assets::icon;
use crate::board::{Arrow, BoardEvent, BoardState, ChessBoard, Circle};
use crate::core::AppCore;
use crate::game::{rules, view};
use crate::play_layout::{LAYOUT_GAP, PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::Palette;
use crate::widgets::{self, Tone};

/// The chapters' column beside the board, and the page width below which it goes under it.
const CHAPTERS_WIDTH: Pixels = px(250.);
const NARROW: Pixels = px(860.);
/// A scripted reply comes after a beat.
const REPLY: Duration = Duration::from_millis(450);

/// What a finished step's reason reads as (`REASONS`).
fn reason_title(reason: &str) -> &'static str {
    match reason {
        "lineDone" => "Well done",
        "checkmate" => "Checkmate!",
        "mated" => "You were checkmated",
        "stalemate" => "Stalemate",
        "insufficient" => "Insufficient material",
        "repetition" => "Threefold repetition",
        "fiftyMoves" => "Fifty-move rule",
        "promoted" => "Promoted — the rest is technique",
        "bareKing" => "Only a bare king is left",
        "reached" => "Key square reached",
        "confined" => "The king is boxed in",
        "held" => "Draw held",
        "outOfMoves" => "Out of moves",
        "materialLost" => "Material lost",
        "lost" => "The position slipped away",
        _ => "",
    }
}

/// How a finished step ended; a draw held by a drawing rule says so.
fn outcome(status: &Value) -> String {
    let reason = status["reason"].as_str().unwrap_or_default();
    if status["over"] != json!(true) || reason.is_empty() || reason == "read" {
        return String::new();
    }
    let title = reason_title(reason);
    if status["success"] == json!(true)
        && matches!(
            reason,
            "stalemate" | "insufficient" | "repetition" | "fiftyMoves"
        )
    {
        format!("Draw held — {}", title.to_lowercase())
    } else {
        title.to_string()
    }
}

/// What the step asks, while it is yours to play.
fn goal(task: &Value, side: &str) -> String {
    let within = crate::game::whole(&task["within"]);
    let moves = format!("{within} {}", if within == 1 { "move" } else { "moves" });
    match task["kind"].as_str().unwrap_or_default() {
        "line" => format!("Find the best move for {side}."),
        "mate" if within == 1 => "Checkmate in one move.".into(),
        "mate" => format!("Checkmate in {within} moves."),
        "play" => match task["goal"].as_str().unwrap_or_default() {
            "mate" => format!("Checkmate the engine within {moves}."),
            "win" => format!("Win against the engine within {moves}."),
            "draw" => format!("Hold the draw for {moves}."),
            "reach" => format!("Bring your king to a marked square within {moves}."),
            "confine" => format!("Box the king into the marked area within {moves}."),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

pub struct EndgameLesson {
    course: Course,
    summary: Value,
    chapter: usize,
    step: usize,
    moves: Vec<String>,
    hint: Option<String>,
    wrong: bool,
    flipped: bool,
    /// The chapter already saved as finished, so it is saved once.
    saved: Option<String>,
    reply: Option<Task<()>>,
    engine: EngineMove,
    board: Entity<ChessBoard>,
    move_input: Entity<InputState>,
    layout: PlayLayout,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TrainerEvent> for EndgameLesson {}

impl HasEngine for EndgameLesson {
    fn engine(&mut self) -> &mut EngineMove {
        &mut self.engine
    }
}

impl EndgameLesson {
    pub fn new(
        course: Course,
        summary: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EndgameLesson {
        let board = cx.new(|_| {
            ChessBoard::new(BoardState::of(
                &kchess_domain::rules::setup_start("standard", crate::game::STANDARD_FEN)
                    .unwrap_or_default(),
                Color::White,
                None,
                None,
            ))
        });
        let move_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type a move: Nf3 or g1f3"));
        let settings = cx.global::<SettingsHandle>().0.clone();
        let subscriptions = vec![
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event {
                    this.on_move(uci, cx);
                }
            }),
            cx.subscribe_in(
                &move_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.typed_move(window, cx);
                    }
                },
            ),
            cx.subscribe(&settings, |this, _, _: &SettingsChanged, cx| {
                this.redraw(cx)
            }),
        ];
        // The first chapter not yet finished.
        let chapter = course
            .chapters
            .iter()
            .position(|chapter| !finished(&summary, &chapter.id))
            .unwrap_or(0);
        let mut lesson = EndgameLesson {
            course,
            summary,
            chapter,
            step: 0,
            moves: Vec::new(),
            hint: None,
            wrong: false,
            flipped: false,
            saved: None,
            reply: None,
            engine: EngineMove::default(),
            board,
            move_input,
            layout: PlayLayout::default(),
            _subscriptions: subscriptions,
        };
        EngineMove::check(cx);
        lesson.moved(cx);
        lesson
    }

    pub fn course_id(&self) -> &str {
        &self.course.id
    }

    fn chapter(&self) -> &LessonChapter {
        &self.course.chapters[self.chapter]
    }

    fn step(&self) -> &LessonStep {
        &self.chapter().steps[self.step]
    }

    fn last_step(&self) -> bool {
        self.step + 1 == self.chapter().steps.len()
    }

    /// `lessonStatus`: where the step stands after `moves`.
    fn status_after(&self, moves: &[String]) -> Value {
        let step = self.step();
        rules(
            "lessonStatus",
            &[json!(step.fen), step.task.clone(), json!(moves)],
        )
        .unwrap_or(Value::Null)
    }

    fn status(&self) -> Value {
        self.status_after(&self.moves)
    }

    fn playable(&self) -> bool {
        self.step().task["kind"] != json!("read")
    }

    fn your_turn(&self) -> bool {
        let status = self.status();
        self.playable() && status["over"] != json!(true) && status["turn"] == status["player"]
    }

    fn player(&self) -> Color {
        color_of(self.status()["player"].as_str().unwrap_or("white"))
    }

    fn open_step(&mut self, index: usize, chapter: usize, cx: &mut Context<Self>) {
        self.engine.cancel(cx);
        self.reply = None;
        if chapter != self.chapter {
            self.saved = None;
        }
        self.chapter = chapter;
        self.step = index;
        self.moves.clear();
        self.hint = None;
        self.wrong = false;
        self.flipped = false;
        self.moved(cx);
    }

    fn next(&mut self, cx: &mut Context<Self>) {
        if !self.last_step() {
            return self.open_step(self.step + 1, self.chapter, cx);
        }
        if self.chapter + 1 < self.course.chapters.len() {
            self.open_step(0, self.chapter + 1, cx);
        }
    }

    fn play(&mut self, uci: String, cx: &mut Context<Self>) {
        self.moves.push(uci);
        let sans = san_from(&self.step().fen, &self.moves);
        crate::sound::move_sound(sans.last().map_or("", String::as_str), cx);
        self.moved(cx);
    }

    /// After every change of the moves: the reply, the chapter's record, the board.
    fn moved(&mut self, cx: &mut Context<Self>) {
        self.respond(cx);
        self.save(cx);
        self.redraw(cx);
    }

    fn on_move(&mut self, uci: &str, cx: &mut Context<Self>) {
        if !self.your_turn() {
            self.redraw(cx);
            return;
        }
        self.hint = None;
        let step = self.step();
        let verdict = rules(
            "lessonMove",
            &[
                json!(step.fen),
                step.task.clone(),
                json!(self.moves),
                json!(uci),
            ],
        )
        .unwrap_or(Value::Null);
        match verdict["uci"].as_str() {
            Some(played) if verdict["accepted"] == json!(true) => {
                self.wrong = false;
                self.play(played.to_string(), cx);
            }
            _ => {
                self.wrong = true;
                // The move goes back.
                self.redraw(cx);
            }
        }
    }

    fn typed_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fen = self.status()["fen"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let text = self.move_input.read(cx).value().trim().to_string();
        let setup = json!({ "variant": "standard", "fen": fen });
        let typed = kchess_domain::rules::setup_start("standard", &fen)
            .and_then(|position| view::typed_uci(&setup, &position, &text));
        if let Some(uci) = typed {
            self.move_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.on_move(&uci, cx);
        }
    }

    /// The opponent answers: a scripted or solved reply after a beat, else the engine.
    fn respond(&mut self, cx: &mut Context<Self>) {
        let status = self.status();
        if status["over"] == json!(true)
            || status["turn"] == status["player"]
            || !self.playable()
            || self.engine.thinking
            || self.reply.is_some()
        {
            return;
        }
        if let Some(reply) = status["reply"].as_str().map(str::to_string) {
            let at = self.moves.len();
            self.reply = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(REPLY).await;
                let _ = this.update(cx, |this, cx| {
                    this.reply = None;
                    if this.moves.len() == at {
                        this.play(reply, cx);
                    }
                });
            }));
            return;
        }
        if status["engine"] != json!(true) {
            return;
        }
        let (fen, moves) = (self.step().fen.clone(), self.moves.clone());
        EngineMove::ask(
            self,
            &fen,
            &moves,
            350,
            true,
            |this, reply, cx| this.play(reply, cx),
            cx,
        );
    }

    fn show_hint(&mut self, cx: &mut Context<Self>) {
        if !self.your_turn() || self.engine.thinking {
            return;
        }
        let step = self.step();
        let known = rules(
            "lessonHint",
            &[json!(step.fen), step.task.clone(), json!(self.moves)],
        )
        .ok()
        .and_then(|hint| hint.as_str().map(str::to_string));
        if let Some(known) = known {
            self.hint = Some(known);
            self.redraw(cx);
            return;
        }
        let (fen, moves) = (step.fen.clone(), self.moves.clone());
        EngineMove::ask(
            self,
            &fen,
            &moves,
            600,
            false,
            |this, best, cx| {
                if this.your_turn() {
                    this.hint = Some(best);
                    this.redraw(cx);
                }
            },
            cx,
        );
    }

    fn takeback(&mut self, cx: &mut Context<Self>) {
        if self.engine.thinking {
            return;
        }
        self.engine.cancel(cx);
        self.reply = None;
        self.hint = None;
        self.wrong = false;
        let player = self.status()["player"].clone();
        let mut kept = self.moves.clone();
        kept.pop();
        while !kept.is_empty() && self.status_after(&kept)["turn"] != player {
            kept.pop();
        }
        self.moves = kept;
        self.moved(cx);
    }

    /// The last step solved finishes the chapter: saved once as a `lesson` run.
    fn save(&mut self, cx: &mut Context<Self>) {
        let status = self.status();
        if status["over"] != json!(true) || status["success"] != json!(true) || !self.last_step() {
            return;
        }
        let id = self.chapter().id.clone();
        if self.saved.as_ref() == Some(&id) {
            return;
        }
        self.saved = Some(id.clone());
        let saved = cx.global::<AppCore>().call(
            "saveRun",
            vec![json!({
                "kind": "lesson",
                "variant": id,
                "score": 1,
                "detail": { "course": self.course.id, "steps": self.chapter().steps.len() },
            })],
        );
        cx.spawn(async move |this, cx| {
            let saved = saved.await;
            let _ = this.update(cx, |this, cx| match saved {
                Ok(saved) => {
                    this.summary = saved["summary"].clone();
                    cx.emit(TrainerEvent::Saved(this.summary.clone()));
                    cx.notify();
                }
                Err(cause) => {
                    crate::log::write(
                        Level::Warn,
                        "endgame-lesson",
                        &format!("Could not save progress: {}", cause.message),
                    );
                    this.saved = None;
                }
            });
        })
        .detach();
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        let status = self.status();
        let fen = status["fen"].as_str().unwrap_or_default().to_string();
        let Some(position) = kchess_domain::rules::setup_start("standard", &fen) else {
            return;
        };
        let last = self.moves.last().and_then(|last| {
            let before = self.status_after(&self.moves[..self.moves.len() - 1]);
            move_squares(before["fen"].as_str()?, last)
        });
        let player = self.player();
        let orientation = if self.flipped { !player } else { player };
        let movable = self.your_turn().then_some(player);
        let mut state = BoardState::of(&position, orientation, movable, last);
        for shape in &self.step().shapes {
            let Ok(orig) = shape.orig.parse() else {
                continue;
            };
            match shape.dest.as_deref().map(str::parse) {
                Some(Ok(dest)) => state.arrows.push(Arrow::brush(&shape.brush, orig, dest)),
                _ => state.circles.push(Circle::brush(&shape.brush, orig)),
            }
        }
        if let Some((from, to)) = self
            .hint
            .as_deref()
            .and_then(|hint| move_squares(&fen, hint))
        {
            state.arrows.push(Arrow::hint(from, to));
        }
        // The pieces show while there is nothing to play, and once the step is over.
        let reveal = status["over"] == json!(true) || !self.playable();
        state.blindfold = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .blindfold
            && !reveal;
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }

    fn render_chapters(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let page = cx.entity().downgrade();
        let items = self
            .course
            .chapters
            .iter()
            .map(|chapter| TrainingItem {
                id: chapter.id.clone(),
                title: chapter.title.clone(),
                done: finished(&self.summary, &chapter.id),
                aside: None,
            })
            .collect();
        v_flex()
            .gap(px(10.))
            .p(px(12.))
            .border_1()
            .border_color(palette.border)
            .rounded(px(14.))
            .bg(palette.elevated)
            .child(
                h_flex()
                    .gap(px(4.))
                    .min_w_0()
                    .child(
                        Button::new("lesson-back")
                            .ghost()
                            .small()
                            .icon(icon("chevron-left"))
                            .accessibility_label("All courses")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.engine.cancel(cx);
                                this.reply = None;
                                cx.emit(TrainerEvent::Back);
                            })),
                    )
                    .child(widgets::heading(self.course.title.clone(), palette).truncate()),
            )
            .child(run::training_list(
                vec![(None, items)],
                &self.chapter().id,
                move |id, _, cx| {
                    let _ = page.update(cx, |page, cx| {
                        if let Some(index) = page
                            .course
                            .chapters
                            .iter()
                            .position(|chapter| chapter.id == id)
                        {
                            page.open_step(0, index, cx);
                        }
                    });
                },
                palette,
            ))
    }

    fn render_panel(
        &self,
        height: Length,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let status = self.status();
        let over = status["over"] == json!(true);
        let success = status["success"] == json!(true);
        let thinking = self.engine.thinking;
        let playable = self.playable();
        let your_turn = self.your_turn();
        let engine_step = status["engine"] == json!(true);
        let missing = self.engine.ready == Some(false);
        let side = if self.player() == Color::White {
            "White"
        } else {
            "Black"
        };
        let other = if side == "White" { "Black" } else { "White" };
        let step = self.step();
        let chapters = self.course.chapters.len();
        let frontend = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .clone();
        let banner = playable.then(|| {
            let (tone, glyph, title, detail) = if over {
                (
                    if success { Tone::Win } else { Tone::Loss },
                    if success {
                        icon("trophy")
                    } else {
                        icon("flag")
                    },
                    outcome(&status),
                    None,
                )
            } else if engine_step && missing {
                (
                    Tone::Alert,
                    icon("triangle-alert"),
                    "Stockfish not found".to_string(),
                    Some("Install or choose an engine in Settings to play this out.".to_string()),
                )
            } else if your_turn {
                (
                    if self.wrong { Tone::Alert } else { Tone::Plain },
                    if self.wrong {
                        icon("x")
                    } else {
                        icon("mouse-pointer-click")
                    },
                    if self.wrong {
                        "Not the move — try again".to_string()
                    } else {
                        "Your turn".to_string()
                    },
                    Some(goal(&step.task, side)),
                )
            } else {
                (
                    Tone::Plain,
                    if thinking {
                        icon("loader-circle")
                    } else {
                        icon("mouse-pointer-click")
                    },
                    if thinking {
                        "Stockfish is thinking…".to_string()
                    } else {
                        format!("{other} to move")
                    },
                    None,
                )
            };
            widgets::status_banner(
                tone,
                glyph,
                title.into(),
                detail.map(SharedString::from),
                None,
                palette,
            )
        });
        let actions = h_flex()
            .flex_wrap()
            .gap(px(8.))
            .when(playable, |actions| {
                actions
                    .child(
                        Button::new("lesson-hint")
                            .outline()
                            .icon(icon("lightbulb"))
                            .label("Hint")
                            .disabled(!your_turn || thinking)
                            .on_click(cx.listener(|this, _, _, cx| this.show_hint(cx))),
                    )
                    .when(step.task["kind"] == json!("play"), |actions| {
                        actions.child(
                            Button::new("lesson-takeback")
                                .outline()
                                .icon(icon("undo-2"))
                                .label("Take back")
                                .disabled(self.moves.is_empty() || thinking)
                                .on_click(cx.listener(|this, _, _, cx| this.takeback(cx))),
                        )
                    })
                    .child(
                        Button::new("lesson-flip")
                            .outline()
                            .icon(icon("arrow-up-down"))
                            .tooltip("Flip board")
                            .accessibility_label("Flip board")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.flipped = !this.flipped;
                                this.redraw(cx);
                            })),
                    )
                    .child(
                        Button::new("lesson-retry")
                            .when(over && !success, |button| button.primary())
                            .when(!(over && !success), |button| button.outline())
                            .icon(icon("rotate-ccw"))
                            .label("Retry")
                            .disabled(self.moves.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_step(this.step, this.chapter, cx)
                            })),
                    )
            })
            .when(self.step > 0, |actions| {
                actions.child(
                    Button::new("lesson-previous")
                        .outline()
                        .icon(icon("chevron-left"))
                        .accessibility_label("Previous step")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_step(this.step - 1, this.chapter, cx)
                        })),
                )
            })
            .when(
                !self.last_step() || self.chapter + 1 < chapters,
                |actions| {
                    actions.child(
                        Button::new("lesson-next")
                            .when(over && success, |button| button.primary())
                            .when(!(over && success), |button| button.outline())
                            .child(
                                h_flex()
                                    .gap(px(6.))
                                    .child(if self.last_step() {
                                        "Next chapter"
                                    } else {
                                        "Next"
                                    })
                                    .child(icon("chevron-right").size(px(14.))),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.next(cx))),
                    )
                },
            );
        widgets::side_panel(palette)
            .h(height)
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .id("lesson-panel")
            .overflow_y_scroll()
            .child(
                h_flex()
                    .items_start()
                    .justify_between()
                    .gap(px(8.))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(widgets::heading(self.chapter().title.clone(), palette))
                            .when(self.chapter().steps.len() > 1, |title| {
                                title.child(widgets::hint(
                                    format!("{} / {}", self.step + 1, self.chapter().steps.len()),
                                    palette,
                                ))
                            }),
                    )
                    .child(widgets::view_toggles(frontend.blindfold, frontend.zen_mode)),
            )
            .child(
                v_flex()
                    .gap(px(10.))
                    .text_size(px(14.))
                    .line_height(relative(1.5))
                    .children(
                        step.text
                            .split("\n\n")
                            .map(|paragraph| div().child(paragraph.trim().to_string())),
                    ),
            )
            .children(banner)
            .children(self.engine.error.clone().map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .child(actions)
            .when(!self.moves.is_empty(), |panel| {
                panel.child(widgets::hint(
                    san_from(&step.fen, &self.moves).join(" "),
                    palette,
                ))
            })
            .into_any_element()
    }
}

impl Render for EndgameLesson {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let narrow = self.layout.width(window) <= NARROW;
        let board_size = if narrow {
            self.layout.board_size(window)
        } else {
            self.layout
                .board_size_beside(CHAPTERS_WIDTH + LAYOUT_GAP, window)
        };
        let your_turn = self.your_turn();
        let board = v_flex()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(self.board.clone())
            .child(widgets::move_entry(&self.move_input, your_turn, &palette).h(px(36.)));
        let chapters = self.render_chapters(cx, &palette);
        let board_column = if narrow {
            v_flex()
                .flex_none()
                .gap(LAYOUT_GAP)
                .child(board)
                .child(chapters.w(board_size))
                .into_any_element()
        } else {
            h_flex()
                .items_start()
                .flex_none()
                .gap(LAYOUT_GAP)
                .child(
                    chapters
                        .w(CHAPTERS_WIDTH)
                        .max_h(board_size)
                        .id("lesson-chapters")
                        .overflow_y_scroll(),
                )
                .child(board)
                .into_any_element()
        };
        let panel = self.render_panel(self.layout.panel_height(board_size), cx, &palette);
        div().size_full().child(self.layout.render(
            "endgame-lesson",
            board_column,
            panel,
            window,
            cx,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{goal, outcome};
    use serde_json::json;

    #[test]
    fn steps_say_what_they_ask() {
        assert_eq!(
            goal(&json!({ "kind": "mate", "within": 1 }), "White"),
            "Checkmate in one move."
        );
        assert_eq!(
            goal(
                &json!({ "kind": "play", "goal": "draw", "within": 1.0 }),
                "Black"
            ),
            "Hold the draw for 1 move."
        );
        assert_eq!(
            goal(&json!({ "kind": "line", "moves": [] }), "Black"),
            "Find the best move for Black."
        );
    }

    #[test]
    fn a_drawing_rule_reads_as_a_held_draw() {
        let held = json!({ "over": true, "success": true, "reason": "stalemate" });
        assert_eq!(outcome(&held), "Draw held — stalemate");
        let lost = json!({ "over": true, "success": false, "reason": "stalemate" });
        assert_eq!(outcome(&lost), "Stalemate");
        assert_eq!(outcome(&json!({ "over": false })), "");
    }
}
