//! New game against Stockfish: where the game starts (the standard
//! position, a Chess960 start, another variant, which Fairy-Stockfish plays, or a FEN), the clock,
//! the side the player takes and the engine's strength. It opens on the current game's options.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, IndexPath, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::assets::icon;
use crate::board::{BoardState, ChessBoard};
use crate::game::{chess960, position_problem, variants};
use crate::theme::Palette;
use crate::widgets;
use kchess_domain::rules::STANDARD_FEN;

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
    Variant,
    Fen,
}

/// The variants Fairy-Stockfish plays, in the variant list's order.
fn fairy_variants() -> impl Iterator<Item = &'static crate::game::VariantText> {
    variants()
        .iter()
        .filter(|text| !crate::game::standard_rules(&text.key))
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
    variant: Entity<SelectState<Vec<SharedString>>>,
    /// The variant game's position, when not its usual start.
    variant_fen: Entity<InputState>,
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
            .unwrap_or(STANDARD_FEN)
            .to_string();
        let start = if variant == "chess960" {
            Start::Chess960
        } else if variant != "standard" {
            Start::Variant
        } else if fen == STANDARD_FEN {
            Start::Standard
        } else {
            Start::Fen
        };
        let variant_row = fairy_variants()
            .position(|text| text.key == variant)
            .unwrap_or(0);
        let variant_select = cx.new(|cx| {
            SelectState::new(
                fairy_variants()
                    .map(|text| SharedString::from(text.label.clone()))
                    .collect(),
                Some(IndexPath::new(variant_row)),
                window,
                cx,
            )
        });
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
        // A variant game from another position (the analysis board's "Play from here") opens
        // with that position.
        let default_fen = crate::game::rules("defaultFen", &[json!(variant)])
            .ok()
            .and_then(|fen| fen.as_str().map(str::to_string));
        let variant_fen = cx.new(|cx| {
            let label = fairy_variants()
                .nth(variant_row)
                .map_or("Variant", |text| text.label.as_str());
            let mut input = InputState::new(window, cx).placeholder(format!("{label} start"));
            if start == Start::Variant && default_fen.as_deref() != Some(fen.as_str()) {
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
        let subscriptions = vec![
            cx.subscribe(&number, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refresh_preview(cx);
                }
            }),
            // The FEN's placeholder names the start it replaces: "Crazyhouse start".
            cx.subscribe_in(
                &variant_select,
                window,
                |this, _, _: &SelectEvent<Vec<SharedString>>, window, cx| {
                    let label = this
                        .variant_text(cx)
                        .map_or("Variant start".to_string(), |text| {
                            format!("{} start", text.label)
                        });
                    // A position the new variant refuses is the old variant's: it goes.
                    let refused = match (this.variant_text(cx), this.variant_fen(cx)) {
                        (Some(text), Some(fen)) => {
                            kchess_domain::rules::setup_start(&text.key, &fen).is_none()
                        }
                        _ => false,
                    };
                    this.variant_fen.update(cx, |input, cx| {
                        input.set_placeholder(label, window, cx);
                        if refused {
                            input.set_value("", window, cx);
                        }
                    });
                    cx.notify()
                },
            ),
            cx.subscribe(&variant_fen, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        let mut options = GameOptions {
            start,
            variant: variant_select,
            variant_fen,
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

    /// The variant chosen under _Variant_.
    fn variant_text(&self, cx: &App) -> Option<&'static crate::game::VariantText> {
        let row = self
            .variant
            .read(cx)
            .selected_index(cx)
            .map_or(0, |index| index.row);
        fairy_variants().nth(row)
    }

    fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        let fen = chess960(self.number960(cx).unwrap_or(518));
        let mut state = preview_state(&fen);
        state.preview = true;
        self.preview
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }

    /// The variant's FEN when one is typed: None while it is empty.
    fn variant_fen(&self, cx: &App) -> Option<String> {
        let text = self.variant_fen.read(cx).value().trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    fn fen_problem(&self, cx: &App) -> Option<SharedString> {
        if self.start == Start::Variant {
            let variant = self.variant_text(cx)?;
            let fen = self.variant_fen(cx)?;
            return kchess_domain::rules::setup_start(&variant.key, &fen)
                .is_none()
                .then(|| "That FEN is not a legal position".into());
        }
        if self.start != Start::Fen {
            return None;
        }
        let text = self.fen.read(cx).value().trim().to_string();
        if text.is_empty() {
            return Some("Paste a FEN, or set up the position in the board editor".into());
        }
        if let Some(problem) = position_problem(&text) {
            return Some(problem.into());
        }
        kchess_domain::rules::setup_start("standard", &text)
            .is_none()
            .then(|| "That FEN is not a legal position".into())
    }

    fn number_problem(&self, cx: &App) -> Option<SharedString> {
        (self.start == Start::Chess960 && self.number960(cx).is_none())
            .then(|| "Enter a number from 0 to 959".into())
    }

    fn chosen(&self, cx: &App) -> Chosen {
        let setup = match self.start {
            Start::Standard => json!({ "variant": "standard", "fen": STANDARD_FEN }),
            Start::Chess960 => json!({
                "variant": "chess960",
                "fen": chess960(self.number960(cx).unwrap_or(518)),
            }),
            Start::Variant => {
                let variant = self
                    .variant_text(cx)
                    .map_or("crazyhouse", |text| text.key.as_str());
                let fen = self.variant_fen(cx).unwrap_or_else(|| {
                    crate::game::rules("defaultFen", &[json!(variant)])
                        .ok()
                        .and_then(|fen| fen.as_str().map(str::to_string))
                        .unwrap_or_default()
                });
                json!({ "variant": variant, "fen": fen })
            }
            // A Chess960 position pasted here castles as Chess960 (`setup_from_fen`).
            Start::Fen => crate::game::setup_from_fen(self.fen.read(cx).value().trim()),
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
        let n = crate::game::random_chess960();
        self.number
            .update(cx, |input, cx| input.set_value(n.to_string(), window, cx));
        self.refresh_preview(cx);
    }
}

fn preview_state(fen: &str) -> BoardState {
    let position = kchess_domain::rules::setup_start("chess960", fen)
        .or_else(|| kchess_domain::rules::setup_start("standard", STANDARD_FEN));
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
            Start::Variant => 2,
            Start::Fen => 3,
        };
        let hint = self.variant_text(cx).map(|text| text.hint.clone());
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
                            (None, "Variant".into()),
                            (None, "A position".into()),
                        ],
                        selected,
                        move |index, _, cx| {
                            let _ = options.update(cx, |this, cx| {
                                this.start = match index {
                                    1 => Start::Chess960,
                                    2 => Start::Variant,
                                    3 => Start::Fen,
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
                                Input::new(&self.number).aria_label("Position number (0–959)"),
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
                        .child(div().w(px(180.)).child(crate::board::cached(&self.preview))),
                )
            })
            .when(self.start == Start::Variant, |form| {
                form.child(
                    widgets::field(
                        "Variant",
                        Select::new(&self.variant).accessibility_label("Choose a variant"),
                        None,
                        &palette,
                    )
                    .children(hint.map(|hint| {
                        div()
                            .text_size(px(13.))
                            .text_color(palette.text_muted)
                            .child(hint)
                    })),
                )
                .child(widgets::field(
                    "Start from a FEN (optional)",
                    Input::new(&self.variant_fen).aria_label("Start from a FEN (optional)"),
                    fen_problem.clone(),
                    &palette,
                ))
            })
            .when(self.start == Start::Fen, |form| {
                form.child(widgets::field(
                    "FEN",
                    Input::new(&self.fen).aria_label("FEN"),
                    fen_problem.clone(),
                    &palette,
                ))
            })
            .child(
                h_flex()
                    .gap(px(12.))
                    .child(
                        widgets::field(
                            "Clock",
                            Select::new(&self.clock).accessibility_label("Clock"),
                            None,
                            &palette,
                        )
                        .flex_1(),
                    )
                    .child(
                        widgets::field(
                            "You play",
                            Select::new(&self.color).accessibility_label("You play"),
                            None,
                            &palette,
                        )
                        .flex_1(),
                    ),
            )
            .child(widgets::field(
                "Stockfish strength",
                Select::new(&self.level).accessibility_label("Stockfish strength"),
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
    crate::widgets::open_dialog(window, cx, move |dialog, _, _| {
        dialog
            .title("New game against Stockfish")
            .w(px(512.))
            .child(options.clone())
    });
}
