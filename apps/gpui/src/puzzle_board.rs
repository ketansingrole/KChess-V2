//! One puzzle on a board: the player finds the moves, the board plays the
//! opponent's replies. It reports what happened and leaves everything else (scores, Lichess, the
//! next puzzle) to its page: Puzzles' Train tab and Practice's mistakes.

use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
use serde_json::{Value, json};
use shakmaty::Color;

use crate::board::{Arrow, BoardEvent, BoardState, ChessBoard};
use crate::game::puzzle::OnBoard;
use crate::game::{STANDARD_FEN, view};
use crate::settings::SettingsHandle;
use crate::theme::{Palette, radii, tint};
use crate::voice::moves::{Access, Action, BoardBar, Turn, VoiceMoves, board_bar};
use crate::widgets;

/// Before the opponent replies, and the solution's pace when it plays itself.
const REPLY_DELAY: Duration = Duration::from_millis(350);
const SOLUTION_STEP: Duration = Duration::from_millis(650);
const SOLUTION_REPLY: Duration = Duration::from_millis(450);
const FLASH: Duration = Duration::from_millis(900);

/// What the board tells its page.
pub enum PuzzleBoardEvent {
    /// Every attempted move, right or wrong, with its number among the player's moves of the
    /// solution, and the move itself (UCI).
    Move {
        index: usize,
        correct: bool,
        uci: String,
    },
    /// The first verdict on the puzzle: solved cleanly or not, and how far it got. Sent once.
    Outcome { win: bool, detail: Value },
    /// The puzzle is over: solved, or its solution played out.
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Feedback {
    None,
    Good,
    Wrong,
}

pub struct PuzzleBoard {
    on_board: Option<OnBoard>,
    board: Entity<ChessBoard>,
    move_input: Entity<InputState>,
    move_message: Option<SharedString>,
    /// Each side's captured material above and below the board (the Train tab's look).
    material: bool,
    waiting: bool,
    mistakes: u32,
    outcome: Option<bool>,
    revealed: bool,
    hinted: bool,
    /// The player's last wrong move here (UCI), until the next try.
    last_wrong: Option<String>,
    /// Explanations of this board's puzzle.
    explainer: Entity<crate::ai::Explainer>,
    /// A hint was shown at some point: `hinted` itself clears after each move.
    hint_used: bool,
    /// A coach was asked mid-puzzle: a heavier help than a hint, so a solve no longer counts.
    nudged: bool,
    feedback: Feedback,
    /// The board's red outline: a brief flash on a wrong move, while the panel keeps the message.
    flash: bool,
    flipped: bool,
    /// A wrong move may be tried again (Train); without it the puzzle fails at once (runs).
    retry: bool,
    /// No moves for now (a run between puzzles, or over).
    frozen: bool,
    reply_delay: Duration,
    /// Spoken moves: kept with the board, which outlives each puzzle, so
    /// the speech model is not reloaded for every one.
    voice: VoiceMoves,
    /// Each position the puzzle reached and the move into it, to step back through once it is
    /// over; and the one shown then, when not the last.
    seen: Vec<(String, Option<(shakmaty::Square, shakmaty::Square)>)>,
    viewing: Option<usize>,
    /// Bumped when the puzzle changes, so a late timer from the old one does nothing.
    generation: u64,
    timers: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PuzzleBoardEvent> for PuzzleBoard {}

/// Whether a finished puzzle was the player's own work. A hint still counts; a coach does not.
fn counts_as_solved(mistakes: u32, revealed: bool, nudged: bool) -> bool {
    mistakes == 0 && !revealed && !nudged
}

impl PuzzleBoard {
    /// What the board's column holds besides the board: the material row above and the move box
    /// below, or the move box alone.
    pub fn around(&self) -> Pixels {
        if self.material {
            px(18.) + px(4.) * 2. + px(36.)
        } else {
            widgets::STACK_GAP + px(36.)
        }
    }

    pub fn new(material: bool, window: &mut Window, cx: &mut Context<Self>) -> PuzzleBoard {
        let board = cx.new(|_| {
            ChessBoard::new(BoardState::of(
                &kchess_domain::rules::setup_start("standard", STANDARD_FEN).unwrap_or_default(),
                Color::White,
                None,
                None,
            ))
        });
        let move_input = cx.new(|cx| InputState::new(window, cx).placeholder("Type a move"));
        let voice = crate::voice::handle(cx);
        let subscriptions = vec![
            cx.subscribe(&voice, |this, _, event: &crate::voice::VoiceEvent, cx| {
                this.voice_heard(event, cx)
            }),
            cx.observe(&voice, |_, _, cx| cx.notify()),
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Move(uci) = event {
                    this.on_move(uci, cx)
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
        ];
        PuzzleBoard {
            on_board: None,
            board,
            move_input,
            move_message: None,
            material,
            waiting: false,
            mistakes: 0,
            outcome: None,
            revealed: false,
            hinted: false,
            last_wrong: None,
            explainer: cx.new(|_| crate::ai::Explainer::new()),
            hint_used: false,
            nudged: false,
            feedback: Feedback::None,
            flash: false,
            flipped: false,
            retry: true,
            frozen: false,
            reply_delay: REPLY_DELAY,
            voice: VoiceMoves::new("puzzles", cx),
            generation: 0,
            timers: Vec::new(),
            seen: Vec::new(),
            viewing: None,
            _subscriptions: subscriptions,
        }
    }

    /// A puzzle, from its start (`begin`); `outcome` is a verdict its page kept from before.
    pub fn start(&mut self, puzzle: &Value, outcome: Option<bool>, cx: &mut Context<Self>) {
        self.generation += 1;
        self.timers.clear();
        self.on_board = Some(OnBoard::start(puzzle));
        self.seen.clear();
        self.viewing = None;
        self.waiting = false;
        self.outcome = outcome;
        self.mistakes = u32::from(outcome == Some(false));
        self.revealed = false;
        self.last_wrong = None;
        self.hinted = false;
        self.hint_used = false;
        self.nudged = false;
        self.feedback = Feedback::None;
        self.flash = false;
        self.move_message = None;
        self.redraw(cx);
    }

    /// No puzzle on the board.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.timers.clear();
        self.on_board = None;
        cx.notify();
    }

    pub fn puzzle(&self) -> Option<&OnBoard> {
        self.on_board.as_ref()
    }

    /// `playing`, `solved`, `failed`, or `revealed` once the solution was shown.
    pub fn status(&self) -> &str {
        match &self.on_board {
            Some(_) if self.revealed => "revealed",
            Some(board) => board.status(),
            None => "playing",
        }
    }

    pub fn outcome(&self) -> Option<bool> {
        self.outcome
    }

    pub fn feedback(&self) -> Feedback {
        self.feedback
    }

    pub fn waiting(&self) -> bool {
        self.waiting
    }

    pub fn hinted(&self) -> bool {
        self.hinted
    }

    /// A hint was shown at some point in this attempt, even if it is hidden again.
    pub fn hint_used(&self) -> bool {
        self.hint_used
    }

    pub fn explainer(&self) -> &Entity<crate::ai::Explainer> {
        &self.explainer
    }

    pub fn last_wrong(&self) -> Option<&str> {
        self.last_wrong.as_deref()
    }

    /// Asking a coach for a nudge costs more than a hint: the solve is not counted.
    pub fn nudged(&mut self) {
        self.hint_used = true;
        self.nudged = true;
    }

    pub fn was_nudged(&self) -> bool {
        self.nudged
    }

    /// The player may move: not while the opponent replies or the solution plays itself out.
    pub fn playing(&self) -> bool {
        self.on_board
            .as_ref()
            .is_some_and(|board| board.status() == "playing")
            && !self.waiting
            && !self.revealed
            && !self.frozen
    }

    /// Whether the keys are with the typed-move field.
    pub fn move_field_focused(&self, window: &Window, cx: &App) -> bool {
        self.move_input.read(cx).focus_handle(cx).is_focused(window)
    }

    /// Solved, failed or its solution shown: what is left is the next puzzle.
    pub fn over(&self) -> bool {
        self.on_board.is_some() && matches!(self.status(), "solved" | "failed" | "revealed")
    }

    /// Once the puzzle is over, step through its moves as on any board: `left`, `right`, `home` or
    /// `end`. Whether the key was taken.
    pub fn step(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if !self.over() || self.seen.len() < 2 {
            return false;
        }
        let last = self.seen.len() - 1;
        let current = self.viewing.unwrap_or(last);
        let next = match key {
            "left" => current.saturating_sub(1),
            "right" => (current + 1).min(last),
            "home" => 0,
            "end" => last,
            _ => return false,
        };
        self.viewing = (next < last).then_some(next);
        self.redraw(cx);
        true
    }

    /// The position shown while stepping back, counted from the puzzle's start.
    #[cfg(test)]
    pub(crate) fn viewing(&self) -> Option<usize> {
        self.viewing
    }

    /// Runs (`retry: false`, `reply-delay`): a wrong move fails the puzzle, replies come faster.
    pub fn for_runs(&mut self, reply_delay: Duration) {
        self.retry = false;
        self.reply_delay = reply_delay;
    }

    /// Stop or allow moves (`frozen`).
    pub fn set_frozen(&mut self, frozen: bool, cx: &mut Context<Self>) {
        if self.frozen != frozen {
            self.frozen = frozen;
            self.redraw(cx);
        }
    }

    fn orientation(&self) -> Color {
        let color = self
            .on_board
            .as_ref()
            .map_or(Color::White, OnBoard::player_color);
        if self.flipped { !color } else { color }
    }

    #[cfg(test)]
    pub(crate) fn flipped(&self) -> bool {
        self.flipped
    }

    pub fn flip(&mut self, cx: &mut Context<Self>) {
        self.flipped = !self.flipped;
        self.redraw(cx);
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        self.sync_voice(cx);
        let Some(on_board) = &self.on_board else {
            cx.notify();
            return;
        };
        if self
            .seen
            .last()
            .is_none_or(|(fen, _)| fen != on_board.fen())
        {
            self.seen
                .push((on_board.fen().to_string(), on_board.last_move()));
        }
        // A position stepped back to, once the puzzle is over.
        let (fen, last_move) = match self.viewing.filter(|_| self.over()) {
            Some(index) => self.seen[index].clone(),
            None => (on_board.fen().to_string(), on_board.last_move()),
        };
        let position =
            kchess_domain::rules::setup_start(on_board.variant(), &fen).unwrap_or_default();
        let movable = self.playing().then(|| on_board.player_color());
        let mut state = BoardState::of(&position, self.orientation(), movable, last_move);
        if on_board.status() == "playing"
            && let Some((from, to)) = on_board.next_squares()
        {
            if self.revealed {
                state.arrows = vec![Arrow::solution(from, to)];
            } else if self.hinted {
                state.hint = Some(from);
            }
        }
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }

    /// The first verdict on the puzzle, sent once.
    fn decide(&mut self, win: bool, cx: &mut Context<Self>) {
        if self.outcome.is_some() {
            return;
        }
        self.outcome = Some(win);
        let detail = json!({
            "revealed": self.revealed,
            "hinted": self.hint_used,
            "total": self.on_board.as_ref().map_or(0, OnBoard::player_moves),
        });
        cx.emit(PuzzleBoardEvent::Outcome { win, detail });
    }

    fn on_move(&mut self, uci: &str, cx: &mut Context<Self>) {
        if !self.playing() {
            self.redraw(cx);
            return;
        }
        let Some(board) = &mut self.on_board else {
            return;
        };
        let index = board.index() / 2;
        let (correct, reply) = board.player_move(uci);
        self.last_wrong = (!correct).then(|| uci.to_string());
        if !correct {
            self.mistakes += 1;
            self.feedback = Feedback::Wrong;
            self.flash = true;
            self.hinted = false;
            let generation = self.generation;
            self.timers.push(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(FLASH).await;
                let _ = this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.flash = false;
                        cx.notify();
                    }
                });
            }));
            cx.emit(PuzzleBoardEvent::Move {
                index,
                correct: false,
                uci: uci.to_string(),
            });
            self.decide(false, cx);
            if !self.retry {
                if let Some(board) = &mut self.on_board {
                    board.state["status"] = json!("failed");
                }
                cx.emit(PuzzleBoardEvent::Done);
            }
            self.redraw(cx);
            return;
        }
        self.feedback = Feedback::Good;
        self.hinted = false;
        crate::sound::move_sound(&board.last_san(), cx);
        let solved = board.status() == "solved";
        cx.emit(PuzzleBoardEvent::Move {
            index,
            correct: true,
            uci: uci.to_string(),
        });
        if solved {
            let clean = counts_as_solved(self.mistakes, self.revealed, self.nudged);
            self.decide(clean, cx);
            crate::sound::play(crate::sound::Sound::GenericNotify, cx);
            cx.emit(PuzzleBoardEvent::Done);
        } else if let Some(reply) = reply {
            self.reply(reply, cx);
        }
        self.redraw(cx);
    }

    fn reply(&mut self, uci: String, cx: &mut Context<Self>) {
        self.waiting = true;
        let delay = self.reply_delay;
        let generation = self.generation;
        self.timers.push(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                if let Some(board) = &mut this.on_board {
                    board.opponent_reply(&uci);
                    crate::sound::move_sound(&board.last_san(), cx);
                }
                this.waiting = false;
                this.redraw(cx);
            });
        }));
    }

    /// Light up the piece to move, or hide it again.
    pub fn hint(&mut self, cx: &mut Context<Self>) {
        if self.status() != "playing" {
            return;
        }
        self.hinted = !self.hinted;
        if self.hinted {
            self.hint_used = true;
        }
        self.redraw(cx);
    }

    /// Play out the rest of the solution; the puzzle then counts as failed.
    pub fn show_solution(&mut self, cx: &mut Context<Self>) {
        if self.status() != "playing" {
            return;
        }
        self.revealed = true;
        self.decide(false, cx);
        self.redraw(cx);
        let generation = self.generation;
        self.timers.push(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SOLUTION_STEP).await;
                let reply = this
                    .update(cx, |this, cx| {
                        if this.generation != generation {
                            return Err(());
                        }
                        let Some(board) = &mut this.on_board else {
                            return Err(());
                        };
                        if board.status() != "playing" {
                            return Err(());
                        }
                        let Some(uci) = board.solution(board.index()) else {
                            return Err(());
                        };
                        let (correct, reply) = board.player_move(&uci);
                        if !correct {
                            return Err(());
                        }
                        crate::sound::move_sound(&board.last_san(), cx);
                        this.redraw(cx);
                        Ok(reply)
                    })
                    .unwrap_or(Err(()));
                let Ok(reply) = reply else { break };
                if let Some(reply) = reply {
                    cx.background_executor().timer(SOLUTION_REPLY).await;
                    let _ = this.update(cx, |this, cx| {
                        if this.generation != generation {
                            return;
                        }
                        if let Some(board) = &mut this.on_board {
                            board.opponent_reply(&reply);
                            crate::sound::move_sound(&board.last_san(), cx);
                        }
                        this.redraw(cx);
                    });
                }
            }
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    cx.emit(PuzzleBoardEvent::Done);
                    this.redraw(cx);
                }
            });
        }));
    }

    /// A spoken move belongs to this puzzle at this point of it; another drops a choice waiting.
    fn sync_voice(&mut self, cx: &mut Context<Self>) {
        let index = self.on_board.as_ref().map_or(0, OnBoard::index);
        let context = format!("{}:{index}", self.generation);
        let playing = self.playing();
        self.voice.sync(
            &context,
            None,
            crate::voice::grammar("MOVE_GRAMMAR"),
            playing,
            cx,
        );
    }

    fn voice_heard(&mut self, event: &crate::voice::VoiceEvent, cx: &mut Context<Self>) {
        if !self.voice.owns(event) {
            return;
        }
        let (fen, variant) = self
            .on_board
            .as_ref()
            .map(|board| (board.fen().to_string(), board.variant().to_string()))
            .unwrap_or_default();
        let heard = match event {
            crate::voice::VoiceEvent::Missed { heard, .. } => {
                self.voice.missed(heard, &fen, self.playing(), cx);
                return;
            }
            crate::voice::VoiceEvent::Heard { heard, .. } => heard.clone(),
        };
        let setup = json!({ "variant": variant, "fen": fen });
        let turn = Turn {
            taking: self.playing(),
            idle: None,
            immediate: false,
        };
        if let Action::Play(uci) = self.voice.hear(&heard, &setup, turn, |_| None, cx) {
            self.on_move(&uci, cx);
        }
        cx.notify();
    }

    fn typed_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.move_input.read(cx).value().trim().to_string();
        let Some(board) = &self.on_board else { return };
        if text.is_empty() || !self.playing() {
            return;
        }
        let setup = json!({ "variant": board.variant(), "fen": board.fen() });
        let typed = kchess_domain::rules::setup_start(board.variant(), board.fen())
            .and_then(|position| view::typed_uci(&setup, &position, &text));
        match typed {
            Some(uci) => {
                self.move_message = None;
                self.move_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.on_move(&uci, cx);
            }
            None => {
                self.move_message = Some("Enter a legal move, such as Nf3 or g1f3".into());
                cx.notify();
            }
        }
    }
}

impl Render for PuzzleBoard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let fen = self
            .on_board
            .as_ref()
            .map(|board| board.fen().to_string())
            .unwrap_or_default();
        let set = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .piece_set
            .clone();
        let orientation = self.orientation();
        let entry = widgets::move_entry(&self.move_input, true, &palette);
        let playing = self.playing();
        let (voice, voice_line) = board_bar(
            &self.voice,
            cx.entity().downgrade(),
            Access {
                write: |this: &mut Self| &mut this.voice,
                play: |this: &mut Self, uci, _, cx| this.on_move(uci, cx),
            },
            BoardBar {
                id: "puzzle-voice",
                hint: "Say “E four” or “Knight F three”",
                active: playing,
                listening: playing,
                grammar: crate::voice::grammar("MOVE_GRAMMAR"),
            },
            cx,
        );
        // The wrong-move frame hugs the board, its corners concentric with the board's.
        let board = div()
            .rounded(radii::BOARD + px(3.))
            .border_3()
            .border_color(if self.flash {
                tint(palette.loss, 55.)
            } else {
                gpui_kit::transparent_black()
            })
            .m(px(-3.))
            .child(crate::board::cached(&self.board));
        // Crazyhouse drills (a mistake from a Crazyhouse game) show the pockets to drop from.
        let [top_pocket, bottom_pocket] =
            [!orientation, orientation].map(|color| crate::board::pocket(&self.board, color, cx));
        let board = v_flex()
            .gap(px(4.))
            .children(top_pocket)
            .child(board)
            .children(bottom_pocket);
        let material = |white: bool| widgets::material(&fen, white, &set, &palette);
        let column = if self.material {
            v_flex()
                .gap(px(4.))
                .child(
                    h_flex()
                        .h(px(18.))
                        .children(material(orientation == Color::Black)),
                )
                .child(board)
                .child(
                    h_flex()
                        .h(px(36.))
                        .gap(px(12.))
                        .children(material(orientation == Color::White))
                        .child(widgets::move_row(entry, voice)),
                )
        } else {
            v_flex()
                .gap(widgets::STACK_GAP)
                .child(board)
                .child(widgets::input_row(entry, voice))
        };
        column
            .w_full()
            .children(voice_line)
            .children(self.move_message.clone().map(|message| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(message)
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::counts_as_solved;

    #[test]
    fn a_nudge_costs_the_solve() {
        assert!(counts_as_solved(0, false, false));
        assert!(!counts_as_solved(0, false, true));
        assert!(!counts_as_solved(1, false, false));
        assert!(!counts_as_solved(0, true, false));
    }
}
