//! New game against Stockfish (`ComputerGameOptions.vue`): where the game starts (the standard
//! position, a Chess960 start or a FEN), the clock, the side the player takes and Stockfish's
//! strength. It opens on the current game's options.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::{Disableable as _, IndexPath, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::assets::icon;
use crate::board::{BoardState, ChessBoard};
use crate::game::{chess960, position_problem};
use crate::theme::Palette;
use crate::widgets;

const INITIAL_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// The clocks offered (`CLOCKS`), as minutes and increment.
const CLOCKS: &[(&str, Option<(f64, f64)>)] = &[
    ("No clock", None),
    ("3+2", Some((3., 2.))),
    ("5+3", Some((5., 3.))),
    ("10+5", Some((10., 5.))),
    ("15+10", Some((15., 10.))),
    ("30+0", Some((30., 0.))),
    ("60+30", Some((60., 30.))),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Start {
    Standard,
    Chess960,
    Fen,
}

/// What the dialog starts: the setup, the clock, the player's side and Stockfish's level.
pub struct Chosen {
    pub setup: Value,
    pub clock: Value,
    pub color: &'static str,
    pub level: String,
}

pub struct GameOptions {
    start: Start,
    number: Entity<InputState>,
    fen: Entity<InputState>,
    clock: Entity<SelectState<Vec<SharedString>>>,
    color: Entity<SelectState<Vec<SharedString>>>,
    level: Entity<SelectState<Vec<SharedString>>>,
    levels: Vec<String>,
    preview: Entity<ChessBoard>,
    on_start: OnStart,
    _subscriptions: Vec<Subscription>,
}

/// What starts the chosen game.
type OnStart = Box<dyn Fn(Chosen, &mut Window, &mut App)>;

/// The current game's options, to open on.
pub struct Current {
    pub setup: Value,
    pub clock: Value,
    pub color: String,
    pub level: String,
}

impl GameOptions {
    pub fn new(
        current: Current,
        levels: (Vec<String>, Vec<SharedString>),
        on_start: impl Fn(Chosen, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> GameOptions {
        let variant = current.setup["variant"].as_str().unwrap_or("standard");
        let fen = current.setup["fen"]
            .as_str()
            .unwrap_or(INITIAL_FEN)
            .to_string();
        let start = if variant == "chess960" {
            Start::Chess960
        } else if fen == INITIAL_FEN {
            Start::Standard
        } else {
            Start::Fen
        };
        let number = cx.new(|cx| {
            let mut input = InputState::new(window, cx);
            input.set_value("518", window, cx);
            input
        });
        let fen_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx)
                .placeholder("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1");
            if start == Start::Fen {
                input.set_value(fen.clone(), window, cx);
            }
            input
        });
        let clock_text = current.clock.as_object().map(|clock| {
            format!(
                "{}+{}",
                clock
                    .get("minutes")
                    .and_then(Value::as_f64)
                    .unwrap_or_default(),
                clock
                    .get("increment")
                    .and_then(Value::as_f64)
                    .unwrap_or_default()
            )
        });
        let clock_row = CLOCKS
            .iter()
            .position(|(label, _)| Some(*label) == clock_text.as_deref())
            .unwrap_or(0);
        let clock = cx.new(|cx| {
            SelectState::new(
                CLOCKS
                    .iter()
                    .map(|(label, _)| SharedString::from(*label))
                    .collect(),
                Some(IndexPath::new(clock_row)),
                window,
                cx,
            )
        });
        let color = cx.new(|cx| {
            SelectState::new(
                vec!["White".into(), "Black".into()],
                Some(IndexPath::new(usize::from(current.color == "black"))),
                window,
                cx,
            )
        });
        let (ids, labels) = levels;
        let level_row = ids.iter().position(|id| *id == current.level);
        let level =
            cx.new(|cx| SelectState::new(labels, level_row.map(IndexPath::new), window, cx));
        let preview = cx.new(|_| {
            let mut state = preview_state(&chess960(518));
            state.preview = true;
            ChessBoard::new(state)
        });
        let subscriptions = vec![cx.subscribe(&number, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refresh_preview(cx);
            }
        })];
        let mut options = GameOptions {
            start,
            number,
            fen: fen_input,
            clock,
            color,
            level,
            levels: ids,
            preview,
            on_start: Box::new(on_start),
            _subscriptions: subscriptions,
        };
        if start == Start::Chess960 {
            // Open on the position number of the current Chess960 game when it is one.
            if let Some(n) = (0..960).find(|&n| chess960(n) == fen) {
                options
                    .number
                    .update(cx, |input, cx| input.set_value(n.to_string(), window, cx));
            }
        }
        options.refresh_preview(cx);
        options
    }

    /// The Chess960 number typed, when it is a whole number from 0 to 959 (`valid960`).
    fn number960(&self, cx: &App) -> Option<i64> {
        let text = self.number.read(cx).value();
        text.trim()
            .parse::<i64>()
            .ok()
            .filter(|n| (0..=959).contains(n))
    }

    fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        let fen = chess960(self.number960(cx).unwrap_or(518));
        let mut state = preview_state(&fen);
        state.preview = true;
        self.preview
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }

    fn fen_problem(&self, cx: &App) -> Option<SharedString> {
        if self.start != Start::Fen {
            return None;
        }
        let text = self.fen.read(cx).value().trim().to_string();
        if text.is_empty() {
            return Some("Paste a FEN, or set up the position in the board editor.".into());
        }
        if let Some(problem) = position_problem(&text) {
            return Some(problem.into());
        }
        kchess_domain::rules::setup_start("standard", &text)
            .is_none()
            .then(|| "That FEN is not a legal position.".into())
    }

    fn number_problem(&self, cx: &App) -> Option<SharedString> {
        (self.start == Start::Chess960 && self.number960(cx).is_none())
            .then(|| "Enter a number from 0 to 959.".into())
    }

    fn chosen(&self, cx: &App) -> Chosen {
        let setup = match self.start {
            Start::Standard => json!({ "variant": "standard", "fen": INITIAL_FEN }),
            Start::Chess960 => json!({
                "variant": "chess960",
                "fen": chess960(self.number960(cx).unwrap_or(518)),
            }),
            Start::Fen => json!({ "variant": "standard", "fen": self.fen.read(cx).value().trim() }),
        };
        let row = |select: &Entity<SelectState<Vec<SharedString>>>| {
            select
                .read(cx)
                .selected_index(cx)
                .map_or(0, |index| index.row)
        };
        let clock = CLOCKS
            .get(row(&self.clock))
            .and_then(|(_, clock)| *clock)
            .map_or(
                Value::Null,
                |(minutes, increment)| json!({ "minutes": minutes, "increment": increment }),
            );
        Chosen {
            setup,
            clock,
            color: if row(&self.color) == 1 {
                "black"
            } else {
                "white"
            },
            level: self
                .levels
                .get(row(&self.level))
                .cloned()
                .unwrap_or_else(|| "club".into()),
        }
    }

    fn begin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fen_problem(cx).is_some() || self.number_problem(cx).is_some() {
            return;
        }
        let chosen = self.chosen(cx);
        window.close_dialog(cx);
        (self.on_start)(chosen, window, cx);
    }

    fn randomize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut bytes = [0u8; 2];
        let n = if getrandom::fill(&mut bytes).is_ok() {
            i64::from(u16::from_le_bytes(bytes) % 960)
        } else {
            518
        };
        self.number
            .update(cx, |input, cx| input.set_value(n.to_string(), window, cx));
        self.refresh_preview(cx);
    }
}

fn preview_state(fen: &str) -> BoardState {
    let position = kchess_domain::rules::setup_start("chess960", fen)
        .or_else(|| kchess_domain::rules::setup_start("standard", INITIAL_FEN));
    match position {
        Some(position) => BoardState::of(&position, shakmaty::Color::White, None, None),
        None => BoardState::of(
            &shakmaty::variant::VariantPosition::Chess(shakmaty::Chess::default()),
            shakmaty::Color::White,
            None,
            None,
        ),
    }
}

impl Render for GameOptions {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let fen_problem = self.fen_problem(cx);
        let number_problem = self.number_problem(cx);
        let blocked = fen_problem.is_some() || number_problem.is_some();
        let selected = match self.start {
            Start::Standard => 0,
            Start::Chess960 => 1,
            Start::Fen => 2,
        };
        v_flex()
            .gap(px(16.))
            .child(widgets::field(
                "Start from",
                {
                    let options = cx.entity().downgrade();
                    widgets::pill_tabs(
                        "start",
                        vec![
                            (None, "Standard".into()),
                            (None, "Chess960".into()),
                            (None, "A position".into()),
                        ],
                        selected,
                        move |index, _, cx| {
                            let _ = options.update(cx, |this, cx| {
                                this.start = match index {
                                    1 => Start::Chess960,
                                    2 => Start::Fen,
                                    _ => Start::Standard,
                                };
                                cx.notify();
                            });
                        },
                        &palette,
                    )
                },
                None,
                &palette,
            ))
            .when(self.start == Start::Chess960, |form| {
                form.child(
                    h_flex()
                        .items_end()
                        .gap(px(8.))
                        .child(
                            widgets::field(
                                "Position number (0–959)",
                                Input::new(&self.number),
                                number_problem.clone(),
                                &palette,
                            )
                            .flex_1(),
                        )
                        .child(
                            Button::new("random-960")
                                .outline()
                                .icon(icon("shuffle"))
                                .label("Random")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.randomize(window, cx)),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .justify_center()
                        .child(div().w(px(180.)).child(self.preview.clone())),
                )
            })
            .when(self.start == Start::Fen, |form| {
                form.child(widgets::field(
                    "FEN",
                    Input::new(&self.fen),
                    fen_problem.clone(),
                    &palette,
                ))
            })
            .child(
                h_flex()
                    .gap(px(12.))
                    .child(
                        widgets::field("Clock", Select::new(&self.clock), None, &palette).flex_1(),
                    )
                    .child(
                        widgets::field("You play", Select::new(&self.color), None, &palette)
                            .flex_1(),
                    ),
            )
            .child(widgets::field(
                "Stockfish strength",
                Select::new(&self.level),
                None,
                &palette,
            ))
            .child(
                h_flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("cancel-options")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("start-game")
                            .primary()
                            .label("Start game")
                            .disabled(blocked)
                            .on_click(cx.listener(|this, _, window, cx| this.begin(window, cx))),
                    ),
            )
    }
}

/// Open the dialog over the window.
pub fn open(options: Entity<GameOptions>, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("New game against Stockfish")
            .w(px(512.))
            .child(options.clone())
    });
}
