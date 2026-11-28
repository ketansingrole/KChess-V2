//! Visualise: the next moves of a famous game, read and not played, then one question on where
//! they lead, answered from a choice, yes or no, or a square on the board. The moves are then
//! played out on the board and the next round starts; a timed run keeps a best per level and
//! length, untimed practice none.

use std::sync::OnceLock;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, IndexPath, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::{CoordinateMode, RunKind};
use kchess_domain::training::visualise::{self, Answer, LEVEL_PLIES, Question, Round};
use serde_json::json;
use shakmaty::uci::UciMove;
use shakmaty::variant::VariantPosition;
use shakmaty::{CastlingMode, Chess, Color, Move, Position as _, Square};

use super::run::{self, Drill, DrillRun, Stage, Tally};
use super::under_board::{self, SquareBox};
use crate::board::{BoardEvent, BoardState, ChessBoard, Circle};
use crate::theme::Palette;
use crate::widgets::{self, Tone};
use kchess_games::random_unit;
use kchess_nav::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout, tab_bar_height};
use kchess_ui::icon;

const DEFAULT_SECONDS: u32 = 120;
const LENGTH: &str = "kchess:visualise-length";
/// Each move as it is played out on the board, once answered.
const STEP: Duration = Duration::from_millis(500);
/// The real position, held before the next round.
const HOLD: Duration = Duration::from_millis(1500);

/// `LEVELS`: the record name and its label, one for each of `LEVEL_PLIES`.
const LEVELS: [(&str, &str); 5] = [
    ("level-1", "Level 1 · 2 moves"),
    ("level-2", "Level 2 · 3 moves"),
    ("level-3", "Level 3 · 4 moves"),
    ("level-4", "Level 4 · 6 moves"),
    ("level-5", "Level 5 · 8 moves"),
];

/// The famous games' moves, from the starting position: the places rounds start.
fn games() -> &'static [Vec<Move>] {
    static GAMES: OnceLock<Vec<Vec<Move>>> = OnceLock::new();
    GAMES.get_or_init(|| {
        super::classics::CLASSICS
            .iter()
            .filter_map(|classic| super::guess::loaded(classic.pgn).ok())
            .map(|(ucis, _)| {
                let mut position = Chess::default();
                ucis.iter()
                    .map_while(|uci| {
                        let m = uci.parse::<UciMove>().ok()?.to_move(&position).ok()?;
                        position.play_unchecked(m);
                        Some(m)
                    })
                    .collect()
            })
            .collect()
    })
}

/// A round of `plies` moves from a famous game, at a place drawn at random.
fn new_round(plies: usize) -> Option<Round> {
    let mut draw = || crate::game::drawn(random_unit()).unwrap_or(0.);
    visualise::round(games(), plies, &mut draw)
}

/// The board's squares of a move, to mark it as the last one played.
fn squares_of(m: Move) -> Option<(Square, Square)> {
    crate::board::last_squares(&UciMove::from_move(m, CastlingMode::Standard).to_string())
}

/// What a test answers with, read from the round on screen.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, PartialEq)]
pub enum Expected {
    /// The right one of the choices, by index, and how many there are.
    Choice { right: usize, count: usize },
    /// The square, clicked on the board or typed.
    Square(String),
}

pub struct Visualise {
    level: usize,
    run: DrillRun,
    tally: Tally,
    round: Option<Round>,
    /// The answer given to the round on screen, once it is answered.
    given: Option<Answer>,
    /// How many of the moves the board has played out since.
    shown: usize,
    /// The rounds asked, which tells one round from the next.
    asked: u32,
    level_select: Entity<SelectState<Vec<SharedString>>>,
    board: Entity<ChessBoard>,
    layout: PlayLayout,
    /// Plays the moves out once answered, then starts the next round.
    playback: Option<Task<()>>,
    /// The square, typed under the board.
    square_box: SquareBox,
    _subscriptions: Vec<Subscription>,
}

impl Visualise {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Visualise {
        let board = cx.new(|_| ChessBoard::new(BoardState::start()));
        let level_select = cx.new(|cx| {
            SelectState::new(
                LEVELS
                    .iter()
                    .map(|(_, label)| SharedString::new_static(label))
                    .collect(),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let (square_box, typed) = SquareBox::new(Self::typed, window, cx);
        let subscriptions = vec![
            typed,
            cx.subscribe(&board, |this, _, event: &BoardEvent, cx| {
                if let BoardEvent::Select(square) = event {
                    this.square(*square, cx);
                }
            }),
            cx.subscribe(
                &level_select,
                |this, state, _: &SelectEvent<Vec<SharedString>>, cx| {
                    this.level = state
                        .read(cx)
                        .selected_index(cx)
                        .map_or(0, |index| index.row.min(LEVELS.len() - 1));
                    cx.notify();
                },
            ),
        ];
        DrillRun::load(RunKind::Visualise, cx);
        let mut drill = Visualise {
            level: 0,
            run: DrillRun::default(),
            tally: Tally::default(),
            round: None,
            given: None,
            shown: 0,
            asked: 0,
            level_select,
            board,
            layout: PlayLayout::under(tab_bar_height::<crate::Tab>),
            playback: None,
            square_box,
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

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.run.running() {
            return;
        }
        self.tally = Tally::default();
        self.next_round(cx);
        self.run.begin(Self::length(cx), cx);
        self.redraw(cx);
    }

    fn next_round(&mut self, cx: &mut Context<Self>) {
        self.playback = None;
        self.given = None;
        self.shown = 0;
        self.asked += 1;
        self.round = new_round(LEVEL_PLIES[self.level]);
        self.redraw(cx);
    }

    /// Go on to the next round once this one is answered, without waiting for the moves.
    fn next(&mut self, cx: &mut Context<Self>) {
        if self.run.running() && self.given.is_some() {
            self.next_round(cx);
        }
    }

    fn answer(&mut self, answer: Answer, cx: &mut Context<Self>) {
        let Some(round) = &self.round else {
            return;
        };
        if !self.run.running() || self.given.is_some() {
            return;
        }
        let right = round.is_right(answer);
        self.tally.record(right);
        self.given = Some(answer);
        if !right {
            crate::sound::play_at(crate::sound::Sound::LowTime, 0.5, cx);
        }
        // The moves, one at a time, then the real position for a moment.
        self.playback = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(STEP).await;
                let more = this.update(cx, |this, cx| this.step(cx)).unwrap_or(false);
                if !more {
                    break;
                }
            }
            cx.background_executor().timer(HOLD).await;
            this.update(cx, |this, cx| this.next(cx)).ok();
        }));
        self.redraw(cx);
    }

    /// Play the next move out on the board; whether more are left.
    fn step(&mut self, cx: &mut Context<Self>) -> bool {
        let count = self.round.as_ref().map_or(0, |round| round.moves.len());
        if self.shown < count {
            self.shown += 1;
            crate::sound::play(crate::sound::Sound::Move, cx);
            self.redraw(cx);
        }
        self.shown < count
    }

    /// The choice at `index`, picked by its button or its number key.
    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(choice) = self
            .round
            .as_ref()
            .and_then(|round| round.choices.get(index).copied())
        {
            self.answer(choice, cx);
        }
    }

    /// A square clicked on the board or typed: the answer to where a piece is.
    fn square(&mut self, square: Square, cx: &mut Context<Self>) {
        if matches!(
            self.round.as_ref().map(|round| round.question),
            Some(Question::Where(_))
        ) {
            self.answer(Answer::Square(square), cx);
        }
    }

    fn typed(&mut self, square: &str, cx: &mut Context<Self>) {
        if let Ok(square) = square.parse::<Square>() {
            self.square(square, cx);
        }
    }

    /// Whether the round on screen asks for a square.
    fn asks_square(&self) -> bool {
        self.run.running()
            && self.given.is_none()
            && matches!(
                self.round.as_ref().map(|round| round.question),
                Some(Question::Where(_))
            )
    }

    pub fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // A key typed in the box is the box's.
        if event.keystroke.modifiers.modified() || self.square_box.focused(window, cx) {
            return;
        }
        match event.keystroke.key.as_str() {
            key @ ("1" | "2" | "3" | "4") => {
                let index = usize::from(key.as_bytes()[0] - b'1');
                self.choose(index, cx);
            }
            "y" => self.yes_no(true, cx),
            "n" => self.yes_no(false, cx),
            _ => {}
        }
    }

    fn yes_no(&mut self, yes: bool, cx: &mut Context<Self>) {
        if matches!(
            self.round.as_ref().map(|round| round.question),
            Some(Question::Attacks(..))
        ) {
            self.answer(Answer::Yes(yes), cx);
        }
    }

    /// The round on screen's answer, as a test gives it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn expected_for_test(&self) -> Option<Expected> {
        let round = self.round.as_ref()?;
        Some(match round.answer {
            Answer::Square(square) => Expected::Square(square.to_string()),
            answer => Expected::Choice {
                right: round.choices.iter().position(|choice| *choice == answer)?,
                count: round.choices.len(),
            },
        })
    }

    /// The rounds asked so far.
    #[cfg(any(test, feature = "test-support"))]
    pub fn asked_for_test(&self) -> u32 {
        self.asked
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        let mut state = match &self.round {
            Some(round) if self.run.stage != Stage::Idle => {
                let position = round.position_after(self.shown);
                let last = self
                    .shown
                    .checked_sub(1)
                    .and_then(|ply| round.moves.get(ply))
                    .and_then(|m| squares_of(*m));
                let mut state =
                    BoardState::of(&VariantPosition::Chess(position), Color::White, None, last);
                match round.question {
                    Question::On(square) | Question::Attacks(_, square) => {
                        state.circles.push(Circle::yellow(square));
                    }
                    Question::Where(_) => {
                        if let (Some(given), Answer::Square(right)) = (self.given, round.answer) {
                            state.circles.push(Circle::green(right));
                            if let Answer::Square(given) = given
                                && given != right
                            {
                                state.circles.push(Circle::red(given));
                            }
                        }
                    }
                }
                state
            }
            _ => BoardState::of(
                &VariantPosition::Chess(Chess::default()),
                Color::White,
                None,
                None,
            ),
        };
        let frontend = crate::settings::frontend(cx).clone();
        // Squares are named in the questions, so the board names them too.
        state.coordinates = Some(if frontend.coordinates == CoordinateMode::None {
            CoordinateMode::Inside
        } else {
            frontend.coordinates
        });
        state.blindfold = frontend.blindfold;
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }

    /// The moves, numbered, the ones played out so far marked.
    fn moves_line(&self, round: &Round, palette: &Palette, cx: &App) -> impl IntoElement {
        let figurine = crate::notation::figurine(cx);
        let text = round.moves_text();
        h_flex()
            .id("visualise-moves")
            .test_support()
            .aria_label(text)
            .flex_wrap()
            .gap_x(px(6.))
            .gap_y(px(2.))
            .text_size(px(20.))
            .font_weight(FontWeight::MEDIUM)
            .children(
                round
                    .numbered()
                    .into_iter()
                    .enumerate()
                    .map(|(index, (number, san))| {
                        let played = index < self.shown;
                        h_flex()
                            .gap(px(4.))
                            .children(
                                number.map(|number| {
                                    div().text_color(palette.text_muted).child(number)
                                }),
                            )
                            .child(
                                div()
                                    .when(played, |cell| cell.text_color(palette.primary))
                                    .child(crate::notation::shown(san, &figurine)),
                            )
                    }),
            )
    }

    /// The answers to the round on screen: its choices, or nothing for a square found on the
    /// board.
    fn answers(&self, round: &Round, cx: &mut Context<Self>) -> impl IntoElement {
        let answered = self.given;
        let yes_no = matches!(round.question, Question::Attacks(..));
        let buttons = round.choices.iter().enumerate().map(|(index, choice)| {
            let choice = *choice;
            let shows_right = answered.is_some() && choice == round.answer;
            let key = if yes_no {
                if choice == Answer::Yes(true) {
                    "Press Y"
                } else {
                    "Press N"
                }
            } else {
                ["Press 1", "Press 2", "Press 3", "Press 4"][index.min(3)]
            };
            Button::new(("visualise-choice", index))
                .when(shows_right, |button| button.primary())
                .when(!shows_right, |button| button.outline())
                .when(!yes_no, |button| button.w_full())
                .when(yes_no, |button| button.flex_1())
                .label(choice.label())
                .tooltip(SharedString::new_static(key))
                .disabled(answered.is_some())
                .on_click(cx.listener(move |this, _, _, cx| this.answer(choice, cx)))
        });
        if yes_no {
            h_flex().gap(px(8.)).children(buttons).into_any_element()
        } else {
            v_flex().gap(px(6.)).children(buttons).into_any_element()
        }
    }
}

impl Drill for Visualise {
    fn run(&mut self) -> &mut DrillRun {
        &mut self.run
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        if !self.run.end() {
            return;
        }
        self.playback = None;
        // The real position of the round on screen, answered or not.
        if let Some(round) = &self.round {
            self.shown = round.moves.len();
        }
        self.redraw(cx);
        // An untimed score only grows with time, so it is never a record.
        if self.run.clock.untimed {
            return;
        }
        let run = run::input(
            RunKind::Visualise,
            self.record_variant(cx),
            f64::from(self.tally.score),
            json!({ "mistakes": self.tally.mistakes, "accuracy": self.tally.accuracy() }),
        );
        DrillRun::save(run, cx);
    }
}

impl Render for Visualise {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let running = self.run.running();
        let (board_size, panel_height) = self.layout.board_and_panel(under_board::height(), window);
        let length = Self::length(cx);
        let mut panel = widgets::side_panel(&palette)
            .id("visualise-panel")
            .test_support()
            .overflow_y_scroll()
            .h(panel_height)
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(14.));
        if running && let Some(round) = &self.round {
            let prompt = round.question.prompt();
            panel = panel
                .child(run::hud(
                    &self.run.clock,
                    f64::from(self.tally.score),
                    "correct",
                    &palette,
                ))
                .child(div().h(px(1.)).bg(palette.border))
                .child(self.moves_line(round, &palette, cx))
                .child(
                    div()
                        .id("visualise-question")
                        .test_support()
                        .aria_label(prompt.clone())
                        .text_size(px(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.highlighted)
                        .child(prompt),
                )
                .child(self.answers(round, cx));
            if let Some(given) = self.given {
                let right = round.is_right(given);
                panel = panel
                    .child(
                        widgets::StatusBanner::new(
                            if right { Tone::Win } else { Tone::Loss },
                            if right { icon("check") } else { icon("x") },
                            SharedString::new_static(if right { "Correct" } else { "Wrong" }),
                        )
                        .announce()
                        .detail(round.answer.label().into()),
                    )
                    .child(
                        Button::new("visualise-next")
                            .primary()
                            .icon(icon("arrow-right"))
                            .label("Next")
                            .on_click(cx.listener(|this, _, _, cx| this.next(cx))),
                    );
            }
            panel = panel.child(
                Button::new("visualise-end")
                    .ghost()
                    .icon(icon("flag"))
                    .label("End run")
                    .on_click(cx.listener(|this, _, _, cx| this.finish(cx))),
            );
        } else {
            panel = panel
                .child(h_flex().justify_end().child(widgets::view_toggles(cx)))
                .child(run::field(
                    "Level",
                    Select::new(&self.level_select)
                        .small()
                        .accessibility_label("Level"),
                    &palette,
                ))
                .child(run::length_picker(
                    "visualise-length",
                    length,
                    false,
                    move |seconds, _, cx| crate::prefs::set(cx, LENGTH, seconds),
                    &palette,
                ))
                .child(div().h(px(1.)).bg(palette.border));
            let best = self.run.best(&self.record_variant(cx));
            if self.run.stage == Stage::Over {
                let is_best = !self.run.clock.untimed && self.run.is_best;
                panel = panel.child(
                    widgets::StatusBanner::new(
                        if is_best { Tone::Win } else { Tone::Plain },
                        if is_best {
                            icon("trophy")
                        } else {
                            icon("timer")
                        },
                        format!(
                            "{} correct · {}% accuracy",
                            self.tally.score,
                            self.tally.accuracy()
                        )
                        .into(),
                    )
                    .announce()
                    .detail(
                        if self.run.clock.untimed {
                            format!("in {}", self.run.clock.text())
                        } else if is_best {
                            "New personal best".to_string()
                        } else {
                            format!(
                                "Best so far: {}",
                                best.unwrap_or(f64::from(self.tally.score))
                            )
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
                            LEVELS[self.level].1,
                            run::length_label(length),
                            best.map_or_else(|| "—".to_string(), |best| best.to_string())
                        )),
                );
            }
            panel = panel.child(
                Button::new("visualise-start")
                    .primary()
                    .large()
                    .w_full()
                    .icon(icon("play"))
                    .label(if self.run.stage == Stage::Over {
                        "Play again"
                    } else {
                        "Start"
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.square_box.clear(window, cx);
                        this.start(cx)
                    })),
            );
        }
        let row = under_board::row(Some((&self.square_box, self.asks_square())), None, &palette);
        let board_column = under_board::column(
            "visualise-column",
            board_size,
            crate::board::resizable(&self.board),
            row,
            None,
        );
        div().size_full().child(
            self.layout
                .render("visualise", board_column, panel, window, cx),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{LEVEL_PLIES, LEVELS, games};
    use kchess_domain::training::visualise::{Answer, Question, round};
    use kchess_testkit::random::{self, Rng};
    use shakmaty::Position as _;

    /// Every famous game is read whole, so every place in it can start a round.
    #[test]
    fn the_famous_games_are_read_whole() {
        let plies: Vec<usize> = games().iter().map(Vec::len).collect();
        assert_eq!(
            plies,
            [
                74, 45, 47, 33, 36, 75, 50, 46, 50, 44, 27, 81, 66, 82, 57, 81, 61, 148, 73, 37,
                87, 123
            ]
        );
        assert_eq!(LEVELS.len(), LEVEL_PLIES.len());
    }

    /// At every level, rounds drawn at random from the famous games replay legally from their
    /// position, read as long as the level says, and answer by the rules.
    #[test]
    fn every_level_asks_rounds_from_the_famous_games() {
        random::check(32, |rng| {
            let mut draws = Rng::new(rng.next_u64());
            let mut draw = move || draws.below(1_000_000) as f64 / 1_000_000.;
            for plies in LEVEL_PLIES {
                let round = round(games(), plies, &mut draw).expect("a round");
                assert_eq!(round.moves.len(), plies);
                let mut position = round.start.clone();
                for m in &round.moves {
                    assert!(position.is_legal(*m), "{m:?}");
                    position.play_unchecked(*m);
                }
                assert_eq!(round.answer, round.question.answer(&position));
                if let Question::Where(piece) = round.question {
                    assert_eq!(position.board().by_piece(piece).count(), 1);
                } else {
                    assert!(round.choices.contains(&round.answer));
                }
                assert!(!matches!(round.answer, Answer::Square(_)) || round.choices.is_empty());
            }
        });
    }
}
