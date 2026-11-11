//! Endgame drills: endgames you must convert (or hold) against Stockfish at
//! full strength, from positions checked to be won or drawn. Results are runs the core keeps.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::Level;
use kchess_contracts::types::{RunKind, RunSummary};
use kchess_domain::training::endgames::{self, Status};
use serde_json::json;
use shakmaty::Color;

use super::endgames::{Drill, TrainerEvent, drills, finished};
use super::engine_move::{EngineMove, HasEngine};
use super::run::{self, TrainingItem};
use crate::assets::icon;
use crate::board::{Arrow, BoardEvent, BoardState, ChessBoard};
use crate::game::view;
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::Palette;
use crate::widgets::{self, Tone};

const GROUPS: [&str; 3] = ["Basic mates", "Pawn endings", "Rook endings"];

pub fn color_of(name: &str) -> Color {
    if name == "black" {
        Color::Black
    } else {
        Color::White
    }
}

pub use kchess_domain::training::endgames::san_from;

/// A move's squares as the board marks them (castling king to its square).
pub fn move_squares(fen: &str, uci: &str) -> Option<(shakmaty::Square, shakmaty::Square)> {
    let squares = kchess_domain::training::puzzle::move_squares(fen, uci)?;
    Some((
        squares.first()?.parse().ok()?,
        squares.get(1)?.parse().ok()?,
    ))
}

/// A drill's actions, as its buttons offer them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DrillAction {
    Hint,
    TakeBack,
    Flip,
    Restart,
}

pub struct EndgameDrills {
    drill: Drill,
    moves: Vec<String>,
    hint: Option<String>,
    flipped: bool,
    summary: Option<RunSummary>,
    recorded: bool,
    engine: EngineMove,
    board: Entity<ChessBoard>,
    move_input: Entity<InputState>,
    layout: PlayLayout,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TrainerEvent> for EndgameDrills {}

impl HasEngine for EndgameDrills {
    fn engine(&mut self) -> &mut EngineMove {
        &mut self.engine
    }
}

impl EndgameDrills {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> EndgameDrills {
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
        let mut page = EndgameDrills {
            drill: drills()[0].clone(),
            moves: Vec::new(),
            hint: None,
            flipped: false,
            summary: None,
            recorded: false,
            engine: EngineMove::default(),
            board,
            move_input,
            layout: PlayLayout::default(),
            _subscriptions: subscriptions,
        };
        run::load_summary(RunKind::Endgame, cx, |this, summary, cx| {
            this.summary = Some(summary);
            cx.notify();
        });
        EngineMove::check(cx);
        page.redraw(cx);
        page.engine_turn(cx);
        page
    }

    /// Where the drill stands, its title and detail, whose turn it is. A drill whose position
    /// cannot be read stands nowhere: nothing to play, nothing over.
    fn status_after(&self, moves: &[String]) -> Status {
        let drill = &self.drill;
        endgames::evaluate(&drill.fen, moves, &drill.player, &drill.goal).unwrap_or_else(|cause| {
            crate::log::write(
                Level::Warn,
                "endgame-drill",
                &format!("{}: {cause}", drill.id),
            );
            Status {
                over: false,
                success: None,
                title: String::new(),
                detail: None,
                turn: "",
                fen: String::new(),
            }
        })
    }

    fn status(&self) -> Status {
        self.status_after(&self.moves)
    }

    fn player(&self) -> Color {
        color_of(&self.drill.player)
    }

    fn your_turn(&self) -> bool {
        let status = self.status();
        !status.over && status.turn == self.drill.player
    }

    fn restart(&mut self, next: Option<Drill>, cx: &mut Context<Self>) {
        self.engine.cancel(cx);
        if let Some(next) = next {
            self.drill = next;
        }
        self.moves.clear();
        self.hint = None;
        self.recorded = false;
        self.flipped = false;
        self.moved(cx);
    }

    /// After every change of the moves (`watch(moves)`): the engine's turn, and the record.
    fn moved(&mut self, cx: &mut Context<Self>) {
        self.engine_turn(cx);
        self.record(cx);
        self.redraw(cx);
    }

    /// Stockfish answers whenever it is its turn.
    fn engine_turn(&mut self, cx: &mut Context<Self>) {
        let status = self.status();
        if status.over || status.turn == self.drill.player || self.engine.thinking {
            return;
        }
        let (fen, moves) = (self.drill.fen.clone(), self.moves.clone());
        EngineMove::ask(
            self,
            &fen,
            &moves,
            350,
            true,
            |this, reply, cx| {
                this.moves.push(reply);
                let san = san_from(&this.drill.fen, &this.moves);
                crate::sound::move_sound(san.last().map_or("", String::as_str), cx);
                this.moved(cx);
            },
            cx,
        );
    }

    fn on_move(&mut self, uci: &str, cx: &mut Context<Self>) {
        if !self.your_turn() {
            self.redraw(cx);
            return;
        }
        self.hint = None;
        self.moves.push(uci.to_string());
        let san = san_from(&self.drill.fen, &self.moves);
        crate::sound::move_sound(san.last().map_or("", String::as_str), cx);
        self.moved(cx);
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

    /// A finished drill is a run: solved or not, and in how many moves.
    fn record(&mut self, cx: &mut Context<Self>) {
        let status = self.status();
        if !status.over || self.recorded {
            return;
        }
        self.recorded = true;
        let success = status.success == Some(true);
        let run = run::input(
            RunKind::Endgame,
            self.drill.id.clone(),
            if success { 1. } else { 0. },
            json!({ "moves": self.moves.len().div_ceil(2), "success": success }),
        );
        run::save_run(run, cx, |this, saved, cx| {
            this.summary = Some(saved.summary);
            cx.notify();
        });
    }

    /// What the drill's buttons can do now.
    pub(crate) fn actions(&self) -> Vec<DrillAction> {
        let thinking = self.engine.thinking;
        let mut actions = Vec::new();
        if self.your_turn() && !thinking {
            actions.push(DrillAction::Hint);
        }
        if !self.moves.is_empty() && !thinking {
            actions.push(DrillAction::TakeBack);
        }
        actions.extend([DrillAction::Flip, DrillAction::Restart]);
        actions
    }

    /// Run an action as its button does, if it is still allowed.
    pub(crate) fn act(&mut self, action: DrillAction, cx: &mut Context<Self>) {
        if !self.actions().contains(&action) {
            return;
        }
        match action {
            DrillAction::Hint => self.show_hint(cx),
            DrillAction::TakeBack => self.takeback(cx),
            DrillAction::Flip => self.flip(cx),
            DrillAction::Restart => self.restart(None, cx),
        }
    }

    fn flip(&mut self, cx: &mut Context<Self>) {
        self.flipped = !self.flipped;
        self.redraw(cx);
    }

    fn takeback(&mut self, cx: &mut Context<Self>) {
        if self.engine.thinking {
            return;
        }
        self.engine.cancel(cx);
        self.hint = None;
        let mut kept = self.moves.clone();
        kept.pop();
        while !kept.is_empty() && self.status_after(&kept).turn != self.drill.player {
            kept.pop();
        }
        self.moves = kept;
        self.recorded = false;
        self.moved(cx);
    }

    fn show_hint(&mut self, cx: &mut Context<Self>) {
        if !self.your_turn() || self.engine.thinking {
            return;
        }
        let (fen, moves) = (self.drill.fen.clone(), self.moves.clone());
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
        let orientation = if self.flipped {
            !self.player()
        } else {
            self.player()
        };
        let over = status.over;
        let movable = self.your_turn().then_some(self.player());
        let mut state = BoardState::of(&position, orientation, movable, last);
        if let Some((from, to)) = self
            .hint
            .as_deref()
            .and_then(|hint| move_squares(&fen, hint))
        {
            state.arrows = vec![Arrow::solution(from, to)];
        }
        // The pieces show again once the drill is over.
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
        cx.notify();
    }
}

impl Render for EndgameDrills {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let status = self.status();
        let over = status.over;
        let success = status.success == Some(true);
        let thinking = self.engine.thinking;
        let missing = self.engine.ready == Some(false);
        let your_turn = self.your_turn();
        let board_size = self.layout.board_size(window);
        let sans = san_from(&self.drill.fen, &self.moves);
        let (tone, glyph, title, detail) = if missing {
            (
                Tone::Alert,
                icon("triangle-alert"),
                "Stockfish not found".to_string(),
                Some("Install or choose an engine in Settings to use the drills".to_string()),
            )
        } else if over {
            (
                if success { Tone::Win } else { Tone::Loss },
                if success {
                    icon("trophy")
                } else {
                    icon("flag")
                },
                status.title.clone(),
                status.detail.map(str::to_string),
            )
        } else if thinking {
            (
                Tone::Plain,
                icon("cpu"),
                "Stockfish is thinking…".to_string(),
                None,
            )
        } else {
            (
                Tone::Plain,
                icon("mouse-pointer-click"),
                status.title.clone(),
                status.detail.map(str::to_string),
            )
        };
        let groups = GROUPS
            .iter()
            .map(|label| {
                (
                    Some(*label),
                    drills()
                        .iter()
                        .filter(|drill| drill.group == *label)
                        .map(|drill| TrainingItem {
                            id: drill.id.clone(),
                            title: drill.title.clone(),
                            done: finished(self.summary.as_ref(), &drill.id),
                            aside: Some(drill.level.clone()),
                        })
                        .collect(),
                )
            })
            .collect();
        let page = cx.entity().downgrade();
        let panel = widgets::side_panel(&palette)
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .id("endgame-drill-panel")
            .overflow_y_scroll()
            .child(
                h_flex()
                    .items_start()
                    .justify_between()
                    .gap(px(8.))
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .min_w_0()
                            .child(
                                Button::new("drills-back")
                                    .ghost()
                                    .small()
                                    .icon(icon("chevron-left"))
                                    .accessibility_label("All courses")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.engine.cancel(cx);
                                        cx.emit(TrainerEvent::Back);
                                    })),
                            )
                            .child(widgets::heading(self.drill.title.clone(), &palette)),
                    )
                    .child(widgets::view_toggles(cx)),
            )
            .child(widgets::hint(
                format!(
                    "{} {}",
                    if self.drill.goal == "win" {
                        "Checkmate within 50 moves"
                    } else {
                        "Hold the draw"
                    },
                    self.drill.idea
                ),
                &palette,
            ))
            .child(
                widgets::StatusBanner::new(tone, glyph, title.into())
                    .busy(thinking && !missing && !over)
                    .announce()
                    .when_some(detail.map(SharedString::from), |banner, value| {
                        banner.detail(value)
                    }),
            )
            .children(self.engine.error.clone().map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        Button::new("drill-hint")
                            .outline()
                            .icon(icon("lightbulb"))
                            .label("Hint")
                            .disabled(!your_turn || thinking)
                            .on_click(cx.listener(|this, _, _, cx| this.show_hint(cx))),
                    )
                    .child(
                        Button::new("drill-takeback")
                            .outline()
                            .icon(icon("undo-2"))
                            .label("Take back")
                            .disabled(self.moves.is_empty() || thinking)
                            .on_click(cx.listener(|this, _, _, cx| this.takeback(cx))),
                    )
                    .child(
                        Button::new("drill-flip")
                            .outline()
                            .icon(icon("arrow-up-down"))
                            .tooltip(crate::menu::flip_label(cx))
                            .accessibility_label("Flip board")
                            .on_click(cx.listener(|this, _, _, cx| this.flip(cx))),
                    )
                    .child(
                        Button::new("drill-restart")
                            .when(over, |button| button.primary())
                            .when(!over, |button| button.outline())
                            .icon(icon("rotate-ccw"))
                            .label("Restart")
                            .on_click(cx.listener(|this, _, _, cx| this.restart(None, cx))),
                    ),
            )
            .when(!self.moves.is_empty(), |panel| {
                panel.child(widgets::hint(
                    format!(
                        "{} moves · {}",
                        self.moves.len().div_ceil(2),
                        sans.join(" ")
                    ),
                    &palette,
                ))
            })
            .child(div().h(px(1.)).flex_none().bg(palette.border))
            .child(run::training_list(
                groups,
                &self.drill.id,
                move |id, _, cx| {
                    let next = drills().iter().find(|drill| drill.id == id).cloned();
                    let _ = page.update(cx, |page, cx| page.restart(next, cx));
                },
                &palette,
            ));
        let board_column = v_flex()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(crate::board::cached(&self.board))
            .child(widgets::lone_entry(widgets::move_entry(
                &self.move_input,
                your_turn,
                &palette,
            )));
        div().size_full().child(self.layout.render(
            "endgame-drills",
            board_column,
            panel,
            window,
            cx,
        ))
    }
}
