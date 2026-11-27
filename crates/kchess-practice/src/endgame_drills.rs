//! Endgame drills: endgames you must convert (or hold) against Stockfish at
//! full strength, from positions checked to be won or drawn: the shared drills, or the endgames
//! your own games let slip (`EndgameDrills::yours`). Results are runs the core keeps.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
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
use crate::board::{Arrow, BoardEvent, BoardState, ChessBoard};
use crate::settings::{SettingsChanged, SettingsHandle};
use crate::theme::Palette;
use crate::voice::moves::{
    Access, BoardBar, Offer, Said, Turn, VoiceMoves, board_bar, offered_grammar,
};
use crate::widgets::{self, Tone};
use kchess_domain::voice::PuzzleCommand;
use kchess_nav::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout, tab_bar_height};
use kchess_ui::icon;

pub(crate) const GROUPS: [&str; 3] = ["Basic mates", "Pawn endings", "Rook endings"];

use kchess_domain::training::endgames::san_from;

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
pub enum DrillAction {
    Hint,
    TakeBack,
    Flip,
    Restart,
}

pub struct EndgameDrills {
    /// The drills listed, each in one of `groups`.
    set: Vec<Drill>,
    groups: Vec<&'static str>,
    drill: Drill,
    moves: Vec<String>,
    hint: Option<String>,
    flipped: bool,
    summary: Option<RunSummary>,
    recorded: bool,
    engine: EngineMove,
    board: Entity<ChessBoard>,
    move_box: crate::game::surface::MoveBox,
    /// Spoken moves, and the drill's Hint and Restart said aloud.
    voice: VoiceMoves,
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
    /// The shared drills.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> EndgameDrills {
        Self::of(drills().to_vec(), GROUPS.to_vec(), window, cx)
    }

    /// `set`, listed under `groups` in that order: the endgames of your games, say. Not empty.
    pub fn of(
        set: Vec<Drill>,
        groups: Vec<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> EndgameDrills {
        let board = cx.new(|_| ChessBoard::new(BoardState::start()));
        let (move_box, typed) = crate::game::surface::MoveBox::new(Self::typed_move, window, cx);
        let settings = cx.global::<SettingsHandle>().0.clone();
        let voice = crate::voice::handle(cx);
        let subscriptions = vec![
            cx.subscribe(&voice, |this, _, event: &crate::voice::VoiceEvent, cx| {
                this.voice_heard(event, cx)
            }),
            cx.observe(&voice, |_, _, cx| cx.notify()),
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event {
                    this.on_move(uci, cx);
                }
            }),
            typed,
            cx.subscribe(&settings, |this, _, _: &SettingsChanged, cx| {
                this.redraw(cx)
            }),
        ];
        let first = set.first().cloned().unwrap_or_else(|| drills()[0].clone());
        let mut page = EndgameDrills {
            set,
            groups,
            drill: first,
            moves: Vec::new(),
            hint: None,
            flipped: false,
            summary: None,
            recorded: false,
            engine: EngineMove::default(),
            board,
            move_box,
            voice: VoiceMoves::new(crate::voice::VoiceSource::Endgames, cx),
            layout: PlayLayout::under(tab_bar_height::<crate::Tab>),
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

    /// The drill on the board.
    pub fn drill(&self) -> &Drill {
        &self.drill
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
        kchess_domain::color::from_name(&self.drill.player).unwrap_or(Color::White)
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
        let variant = kchess_domain::variant::Variant::Standard;
        let position = kchess_domain::rules::setup_start(variant, &self.status().fen);
        if let Some(uci) = self.move_box.take(position.as_ref(), window, cx) {
            self.on_move(&uci, cx);
        }
    }

    /// Voice listens while the drill waits on you: your move, or the drill over.
    fn voice_listening(&self) -> bool {
        self.your_turn() || self.status().over
    }

    /// The drill's buttons voice can press: Hint, and Restart (“retry”).
    fn voice_offers(&self) -> Vec<Offer> {
        let actions = self.actions();
        vec![
            Offer::new(
                PuzzleCommand::Hint,
                actions.contains(&DrillAction::Hint),
                "Hint shown",
                "No hint now",
            ),
            Offer::new(
                PuzzleCommand::Retry,
                actions.contains(&DrillAction::Restart),
                "Drill restarted",
                "Nothing to restart",
            ),
        ]
    }

    /// A spoken move belongs to this drill after these moves; another drops a choice waiting.
    fn sync_voice(&mut self, cx: &mut Context<Self>) {
        let context = format!("{}:{}", self.drill.id, self.moves.len());
        let line = san_from(&self.drill.fen, &self.moves);
        let (grammar, listening) = (
            offered_grammar(&self.voice_offers()),
            self.voice_listening(),
        );
        self.voice
            .sync(&context, Some(line), grammar, listening, cx);
    }

    fn voice_heard(&mut self, event: &crate::voice::VoiceEvent, cx: &mut Context<Self>) {
        if !self.voice.owns(event) {
            return;
        }
        let setup = kchess_domain::variant::Setup::standard_at(&self.status().fen);
        let heard = match event {
            crate::voice::VoiceEvent::Missed { heard, .. } => {
                self.voice.missed(heard, &setup.fen, self.your_turn(), cx);
                return;
            }
            crate::voice::VoiceEvent::Heard { heard, .. } => heard.clone(),
        };
        let turn = Turn {
            taking: self.your_turn(),
            idle: self.status().over.then_some("Say “retry” to play it again"),
            immediate: false,
        };
        let offers = self.voice_offers();
        match self.voice.hear_offered(&heard, &setup, turn, &offers, cx) {
            Said::Play(uci) => self.on_move(&uci, cx),
            Said::Command(PuzzleCommand::Hint) => self.act(DrillAction::Hint, cx),
            Said::Command(PuzzleCommand::Retry) => self.act(DrillAction::Restart, cx),
            Said::Command(
                PuzzleCommand::Next
                | PuzzleCommand::Back
                | PuzzleCommand::ShowMove
                | PuzzleCommand::RestartLine,
            )
            | Said::Nothing => {}
        }
        cx.notify();
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
    pub fn actions(&self) -> Vec<DrillAction> {
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
        self.sync_voice(cx);
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
            state.arrows = vec![Arrow::hint(from, to)];
        }
        // The pieces show again once the drill is over.
        state.blindfold = crate::settings::frontend(cx).blindfold && !over;
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
        let (board_size, panel_height) = self.layout.board_and_panel(widgets::MOVE_BOX_ROW, window);
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
        let groups = self
            .groups
            .iter()
            .map(|label| {
                (
                    Some(*label),
                    self.set
                        .iter()
                        .filter(|drill| drill.group == *label)
                        .map(|drill| TrainingItem {
                            id: drill.id.clone(),
                            title: drill.title.clone(),
                            done: finished(self.summary.as_ref(), &drill.id),
                            aside: Some(drill.level.clone()),
                        })
                        .collect::<Vec<_>>(),
                )
            })
            // A group with nothing in it (your games let no draw slip, say) is left out.
            .filter(|(_, items)| !items.is_empty())
            .collect();
        let page = cx.entity().downgrade();
        let set = self.set.clone();
        let panel = widgets::side_panel(&palette)
            .h(panel_height)
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .id("endgame-drill-panel")
            .test_support()
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
                        "Checkmate within 50 moves."
                    } else {
                        "Hold the draw."
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
                            .tooltip(kchess_board::flip_label(cx))
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
                    let next = set.iter().find(|drill| drill.id == id).cloned();
                    let _ = page.update(cx, |page, cx| page.restart(next, cx));
                },
                &palette,
            ));
        let entry = widgets::move_entry(&self.move_box.input, your_turn, &palette);
        let (voice, voice_line) = board_bar(
            &self.voice,
            cx.entity().downgrade(),
            Access {
                write: |this: &mut Self| &mut this.voice,
                play: |this: &mut Self, uci, _, cx| this.on_move(uci, cx),
            },
            BoardBar {
                id: "drill-voice",
                hint: "Say “E four”, “hint” or “retry”",
                active: self.voice_listening(),
                listening: self.voice_listening(),
                grammar: offered_grammar(&self.voice_offers()),
            },
            cx,
        );
        let board_column = v_flex()
            .id("drill-column")
            .test_support()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(crate::board::cached(&self.board))
            .child(widgets::input_row(entry, voice))
            .children(voice_line)
            .children(self.move_box.message(&palette));
        div().size_full().child(self.layout.render(
            "endgame-drills",
            board_column,
            panel,
            window,
            cx,
        ))
    }
}
