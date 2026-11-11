//! One endgame course: its chapters, each a few steps to read, follow or
//! solve, against scripted replies or Stockfish. A finished chapter is saved as a `lesson` run.

use kchess_contracts::calls;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::Level;
use kchess_contracts::types::{RunKind, RunSummary};
use kchess_domain::training::lessons::{self, Goal, Reason, Status};
use serde_json::json;
use shakmaty::Color;

use super::endgame_drills::move_squares;
use super::endgames::{Chapter, Course, Step, TrainerEvent, finished};
use super::engine_move::{EngineMove, HasEngine};
use super::run::{self, TrainingItem};
use crate::assets::icon;
use crate::board::{Arrow, BoardEvent, BoardState, ChessBoard, Circle};
use crate::core::AppCore;
use crate::game::view;
use crate::play_layout::{LAYOUT_GAP, PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::{Palette, radii};
use crate::widgets::{self, Tone};
use kchess_domain::training::endgames::san_from;

/// The chapters' column beside the board, and the page width below which it goes under it.
const CHAPTERS_WIDTH: Pixels = px(250.);
const NARROW: Pixels = px(860.);
/// A scripted reply comes after a beat.
const REPLY: Duration = Duration::from_millis(450);

/// What a finished step's reason reads as.
fn reason_title(reason: Reason) -> &'static str {
    match reason {
        Reason::Read => "",
        Reason::LineDone => "Well done",
        Reason::Checkmate => "Checkmate",
        Reason::Mated => "You were checkmated",
        Reason::Stalemate => "Stalemate",
        Reason::Insufficient => "Insufficient material",
        Reason::Repetition => "Threefold repetition",
        Reason::FiftyMoves => "Fifty-move rule",
        Reason::Promoted => "Promoted — the rest is technique",
        Reason::BareKing => "Only a bare king is left",
        Reason::Reached => "Key square reached",
        Reason::Confined => "The king is boxed in",
        Reason::Held => "Draw held",
        Reason::OutOfMoves => "Out of moves",
        Reason::MaterialLost => "Material lost",
        Reason::Lost => "The position slipped away",
    }
}

/// How a finished step ended; a draw held by a drawing rule says so.
fn outcome(status: &Status) -> String {
    let Some(reason) = status.reason.filter(|_| status.over) else {
        return String::new();
    };
    let title = reason_title(reason);
    if status.success == Some(true)
        && matches!(
            reason,
            Reason::Stalemate | Reason::Insufficient | Reason::Repetition | Reason::FiftyMoves
        )
    {
        format!("Draw held — {}", title.to_lowercase())
    } else {
        title.to_string()
    }
}

/// What the step asks, while it is yours to play.
fn goal(task: &lessons::Task, side: &str) -> String {
    let moves = |within: u32| format!("{within} {}", if within == 1 { "move" } else { "moves" });
    match task {
        lessons::Task::Read => String::new(),
        lessons::Task::Line { .. } => format!("Find the best move for {side}"),
        lessons::Task::Mate { within: 1 } => "Checkmate in one move".into(),
        lessons::Task::Mate { within } => format!("Checkmate in {within} moves"),
        lessons::Task::Play { goal, within, .. } => {
            let moves = moves(*within);
            match goal {
                Goal::Mate => format!("Checkmate the engine within {moves}"),
                Goal::Win => format!("Win against the engine within {moves}"),
                Goal::Draw => format!("Hold the draw for {moves}"),
                Goal::Reach => format!("Bring your king to a marked square within {moves}"),
                Goal::Confine => format!("Box the king into the marked area within {moves}"),
            }
        }
    }
}

/// Where a step stands when its position cannot be read: nothing to play, nothing over.
fn unreadable() -> Status {
    Status {
        over: false,
        success: None,
        reason: None,
        player: "",
        turn: "",
        fen: String::new(),
        check: false,
        played: 0,
        limit: None,
        reply: None,
        engine: false,
    }
}

/// A logged lesson rule that failed.
fn warn(cause: &str) {
    crate::log::write(Level::Warn, "endgame-lesson", cause);
}

/// A lesson's actions, as its buttons offer them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LessonAction {
    Back,
    Hint,
    TakeBack,
    Flip,
    Retry,
    Previous,
    Next,
    NextChapter,
}

pub struct EndgameLesson {
    course: Course,
    summary: Option<RunSummary>,
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
        summary: Option<RunSummary>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EndgameLesson {
        let board = cx.new(|_| {
            ChessBoard::new(BoardState::of(
                &kchess_domain::rules::setup_start(
                    kchess_domain::variant::Variant::Standard,
                    crate::game::STANDARD_FEN,
                )
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
            .position(|chapter| !finished(summary.as_ref(), &chapter.id))
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

    fn chapter(&self) -> &Chapter {
        &self.course.chapters[self.chapter]
    }

    fn step(&self) -> &Step {
        &self.chapter().steps[self.step]
    }

    fn last_step(&self) -> bool {
        self.step + 1 == self.chapter().steps.len()
    }

    /// Where the step stands after `moves`.
    fn status_after(&self, moves: &[String]) -> Status {
        let step = self.step();
        lessons::status(&step.fen, &step.task, moves).unwrap_or_else(|cause| {
            warn(&cause);
            unreadable()
        })
    }

    fn status(&self) -> Status {
        self.status_after(&self.moves)
    }

    fn playable(&self) -> bool {
        self.step().task != lessons::Task::Read
    }

    fn your_turn(&self) -> bool {
        let status = self.status();
        self.playable() && !status.over && status.turn == status.player
    }

    fn player(&self) -> Color {
        kchess_domain::color::from_name(self.status().player).unwrap_or(Color::White)
    }

    /// What Return goes on to: the next step or chapter, once this step is read or passed.
    pub(crate) fn next_on_return(&self) -> Option<LessonAction> {
        let status = self.status();
        let passed = !self.playable() || (status.over && status.success == Some(true));
        let actions = self.actions();
        [LessonAction::Next, LessonAction::NextChapter]
            .into_iter()
            .find(|next| passed && actions.contains(next))
    }

    /// What the lesson's buttons can do now.
    pub(crate) fn actions(&self) -> Vec<LessonAction> {
        let thinking = self.engine.thinking;
        let mut actions = vec![LessonAction::Back];
        if self.playable() {
            if self.your_turn() && !thinking {
                actions.push(LessonAction::Hint);
            }
            if matches!(self.step().task, lessons::Task::Play { .. })
                && !self.moves.is_empty()
                && !thinking
            {
                actions.push(LessonAction::TakeBack);
            }
            actions.push(LessonAction::Flip);
            if !self.moves.is_empty() {
                actions.push(LessonAction::Retry);
            }
        }
        if self.step > 0 {
            actions.push(LessonAction::Previous);
        }
        if !self.last_step() {
            actions.push(LessonAction::Next);
        } else if self.chapter + 1 < self.course.chapters.len() {
            actions.push(LessonAction::NextChapter);
        }
        actions
    }

    /// Run an action as its button does, if it is still allowed.
    pub(crate) fn act(&mut self, action: LessonAction, cx: &mut Context<Self>) {
        if !self.actions().contains(&action) {
            return;
        }
        match action {
            LessonAction::Back => self.back(cx),
            LessonAction::Hint => self.show_hint(cx),
            LessonAction::TakeBack => self.takeback(cx),
            LessonAction::Flip => self.flip(cx),
            LessonAction::Retry => self.open_step(self.step, self.chapter, cx),
            LessonAction::Previous => self.open_step(self.step - 1, self.chapter, cx),
            LessonAction::Next | LessonAction::NextChapter => self.next(cx),
        }
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        self.engine.cancel(cx);
        self.reply = None;
        cx.emit(TrainerEvent::Back);
    }

    fn flip(&mut self, cx: &mut Context<Self>) {
        self.flipped = !self.flipped;
        self.redraw(cx);
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
        let played = match lessons::try_move(&step.fen, &step.task, &self.moves, uci) {
            Ok(verdict) => verdict.uci.filter(|_| verdict.accepted),
            Err(cause) => {
                warn(&cause);
                None
            }
        };
        match played {
            Some(played) => {
                self.wrong = false;
                self.play(played, cx);
            }
            _ => {
                self.wrong = true;
                // The move goes back.
                self.redraw(cx);
                self.board.update(cx, |board, cx| board.undo_move(cx));
            }
        }
    }

    fn typed_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fen = self.status().fen;
        let text = self.move_input.read(cx).value().trim().to_string();
        let setup = json!({ "variant": "standard", "fen": fen });
        let typed =
            kchess_domain::rules::setup_start(kchess_domain::variant::Variant::Standard, &fen)
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
        if status.over
            || status.turn == status.player
            || !self.playable()
            || self.engine.thinking
            || self.reply.is_some()
        {
            return;
        }
        if let Some(reply) = status.reply {
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
        if !status.engine {
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
        let known = lessons::hint(&step.fen, &step.task, &self.moves).unwrap_or_else(|cause| {
            warn(&cause);
            None
        });
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
        let player = self.status().player;
        let mut kept = self.moves.clone();
        kept.pop();
        while !kept.is_empty() && self.status_after(&kept).turn != player {
            kept.pop();
        }
        self.moves = kept;
        self.moved(cx);
    }

    /// The last step solved finishes the chapter: saved once as a `lesson` run.
    fn save(&mut self, cx: &mut Context<Self>) {
        let status = self.status();
        if !status.over || status.success != Some(true) || !self.last_step() {
            return;
        }
        let id = self.chapter().id.clone();
        if self.saved.as_ref() == Some(&id) {
            return;
        }
        self.saved = Some(id.clone());
        let saved = cx.global::<AppCore>().request(calls::SaveRun {
            run: run::input(
                RunKind::Lesson,
                id,
                1.,
                json!({ "course": self.course.id, "steps": self.chapter().steps.len() }),
            ),
        });
        cx.spawn(async move |this, cx| {
            let saved = saved.await;
            let _ = this.update(cx, |this, cx| match saved {
                Ok(saved) => {
                    cx.emit(TrainerEvent::Saved(saved.summary.clone()));
                    this.summary = Some(saved.summary);
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
        let fen = status.fen.clone();
        let Some(position) =
            kchess_domain::rules::setup_start(kchess_domain::variant::Variant::Standard, &fen)
        else {
            return;
        };
        let last = self.moves.last().and_then(|last| {
            let before = self.status_after(&self.moves[..self.moves.len() - 1]);
            move_squares(&before.fen, last)
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
        let reveal = status.over || !self.playable();
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
                done: finished(self.summary.as_ref(), &chapter.id),
                aside: None,
            })
            .collect();
        v_flex()
            .gap(px(8.))
            .p(px(12.))
            .border_1()
            .border_color(palette.border)
            .rounded(radii::XL)
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
                            .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
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
        let over = status.over;
        let success = status.success == Some(true);
        let thinking = self.engine.thinking;
        let playable = self.playable();
        let your_turn = self.your_turn();
        let engine_step = status.engine;
        let missing = self.engine.ready == Some(false);
        let side = if self.player() == Color::White {
            "White"
        } else {
            "Black"
        };
        let other = if side == "White" { "Black" } else { "White" };
        let step = self.step();
        let chapters = self.course.chapters.len();
        let banner = playable.then(|| {
            let mut busy = false;
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
                    Some("Install or choose an engine in Settings to play this out".to_string()),
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
                busy = thinking;
                (
                    Tone::Plain,
                    icon("mouse-pointer-click"),
                    if thinking {
                        "Stockfish is thinking…".to_string()
                    } else {
                        format!("{other} to move")
                    },
                    None,
                )
            };
            widgets::StatusBanner::new(tone, glyph, title.into())
                .busy(busy)
                .announce()
                .when_some(detail.map(SharedString::from), |banner, value| {
                    banner.detail(value)
                })
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
                    .when(matches!(step.task, lessons::Task::Play { .. }), |actions| {
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
                            .tooltip(crate::menu::flip_label(cx))
                            .accessibility_label("Flip board")
                            .on_click(cx.listener(|this, _, _, cx| this.flip(cx))),
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
                                    .gap(px(8.))
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
                    .child(widgets::view_toggles(cx)),
            )
            .child(
                v_flex()
                    .gap(px(8.))
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
            .child(crate::board::cached(&self.board))
            .child(widgets::lone_entry(widgets::move_entry(
                &self.move_input,
                your_turn,
                &palette,
            )));
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
    use super::{goal, outcome, unreadable};
    use kchess_domain::training::lessons::{Goal, Reason, Status, Task};

    #[test]
    fn steps_say_what_they_ask() {
        assert_eq!(
            goal(&Task::Mate { within: 1 }, "White"),
            "Checkmate in one move"
        );
        let hold = Task::Play {
            goal: Goal::Draw,
            within: 1,
            squares: Vec::new(),
        };
        assert_eq!(goal(&hold, "Black"), "Hold the draw for 1 move");
        assert_eq!(
            goal(&Task::Line { moves: Vec::new() }, "Black"),
            "Find the best move for Black"
        );
    }

    #[test]
    fn a_drawing_rule_reads_as_a_held_draw() {
        let ended = |success| Status {
            over: true,
            success: Some(success),
            reason: Some(Reason::Stalemate),
            ..unreadable()
        };
        assert_eq!(outcome(&ended(true)), "Draw held — stalemate");
        assert_eq!(outcome(&ended(false)), "Stalemate");
        assert_eq!(outcome(&unreadable()), "");
    }
}
