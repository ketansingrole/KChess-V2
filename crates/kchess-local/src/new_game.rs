//! The new-game form (`New game over the board`): the variant, its start, the clock and odds.

use super::*;

/// Starts the game the form chose, from its setup and clock.
pub(super) type StartGame = Box<dyn Fn(Setup, Value, &mut Window, &mut App)>;

/// The new-game form (`New game over the board`): the variant, its start, the clock and odds.
pub(super) struct NewGame {
    pub(super) variant: Entity<SelectState<Vec<SharedString>>>,
    pub(super) number: Entity<InputState>,
    pub(super) fen: Entity<InputState>,
    pub(super) clock: Entity<SelectState<Vec<SharedString>>>,
    pub(super) odds: Entity<InputState>,
    pub(super) on_start: StartGame,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl NewGame {
    pub(super) fn new(
        current: &Setup,
        on_start: impl Fn(Setup, Value, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> NewGame {
        // The form opens on the game being played: its variant, its Chess960 number, or the
        // position it started from.
        let current_variant = current.variant;
        let current_fen = current.fen.clone();
        let row = Variant::ALL
            .iter()
            .position(|variant| *variant == current_variant)
            .unwrap_or(0);
        let variant = cx.new(|cx| {
            SelectState::new(
                Variant::ALL
                    .iter()
                    .map(|variant| SharedString::from(variant.label()))
                    .collect(),
                Some(IndexPath::new(row)),
                window,
                cx,
            )
        });
        let current_number = (current_variant == Variant::Chess960)
            .then(|| (0..960).find(|&n| chess960(n) == current_fen))
            .flatten();
        let number = cx.new(|cx| {
            let mut input = InputState::new(window, cx);
            let n = current_number
                .or_else(|| crate::game::drawn(kchess_games::random_chess960()))
                .unwrap_or(518);
            input.set_value(n.to_string(), window, cx);
            input
        });
        let usual = match current_variant {
            Variant::Chess960 => None,
            variant => Some(variant.default_fen()),
        };
        let fen = cx.new(|cx| {
            let label = format!("{} start", current_variant.label());
            let mut input = InputState::new(window, cx).placeholder(label);
            if usual.as_ref().is_some_and(|usual| *usual != current_fen) {
                input.set_value(current_fen.clone(), window, cx);
            }
            input
        });
        let clock = cx.new(|cx| {
            SelectState::new(
                crate::game::CLOCKS
                    .iter()
                    .map(|(label, _)| SharedString::from(*label))
                    .collect(),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let odds = cx.new(|cx| InputState::new(window, cx));
        let refresh = |_: &mut NewGame,
                       _: Entity<InputState>,
                       _: &InputEvent,
                       cx: &mut Context<NewGame>| cx.notify();
        let subscriptions = vec![
            cx.subscribe(&number, refresh),
            cx.subscribe(&fen, refresh),
            cx.subscribe(&odds, refresh),
            // The FEN's placeholder names the start it replaces: "Crazyhouse start".
            cx.subscribe_in(
                &variant,
                window,
                |this, _, _: &SelectEvent<Vec<SharedString>>, window, cx| {
                    let label = format!("{} start", this.chosen_variant(cx).label());
                    this.fen
                        .update(cx, |input, cx| input.set_placeholder(label, window, cx));
                    cx.notify()
                },
            ),
            cx.subscribe(&clock, |_, _, _: &SelectEvent<Vec<SharedString>>, cx| {
                cx.notify()
            }),
        ];
        NewGame {
            variant,
            number,
            fen,
            clock,
            odds,
            on_start: Box::new(on_start),
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn chosen_variant(&self, cx: &App) -> Variant {
        let row = self
            .variant
            .read(cx)
            .selected_index(cx)
            .map_or(0, |index| index.row);
        Variant::ALL.get(row).copied().unwrap_or_default()
    }

    pub(super) fn number960(&self, cx: &App) -> Option<i64> {
        self.number
            .read(cx)
            .value()
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|n| (0..=959).contains(n))
    }

    pub(super) fn fen_problem(&self, cx: &App) -> Option<SharedString> {
        let text = self.fen.read(cx).value().trim().to_string();
        let variant = self.chosen_variant(cx);
        if variant == Variant::Chess960 || text.is_empty() {
            return None;
        }
        crate::game::setup_problem(variant, &text).map(SharedString::from)
    }

    /// The chosen start, or `None` while the form has a problem.
    pub(super) fn setup(&self, cx: &App) -> Option<Setup> {
        let variant = self.chosen_variant(cx);
        if variant == Variant::Chess960 {
            return self.number960(cx).map(|n| Setup {
                variant,
                fen: chess960(n),
            });
        }
        let text = self.fen.read(cx).value().trim().to_string();
        if variant == Variant::Standard && !text.is_empty() {
            return self
                .fen_problem(cx)
                .is_none()
                .then(|| crate::game::setup_from_fen(&text));
        }
        if !text.is_empty() {
            return self
                .fen_problem(cx)
                .is_none()
                .then_some(Setup { variant, fen: text });
        }
        Some(Setup::of(variant))
    }

    /// The chosen clock: equal times unless Black's minutes give odds.
    pub(super) fn clock(&self, cx: &App) -> Value {
        let row = self
            .clock
            .read(cx)
            .selected_index(cx)
            .map_or(0, |index| index.row);
        let Some((minutes, increment)) = crate::game::CLOCKS.get(row).and_then(|(_, clock)| *clock)
        else {
            return Value::Null;
        };
        let odds = self
            .odds
            .read(cx)
            .value()
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|m| *m >= 1.);
        json!({
            "white": { "minutes": minutes, "increment": increment },
            "black": { "minutes": odds.unwrap_or(minutes), "increment": increment },
        })
    }
}

impl Render for NewGame {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let variant = self.chosen_variant(cx);
        let hint = Some(variant.hint().to_string());
        let setup = self.setup(cx);
        let no_clock = self
            .clock
            .read(cx)
            .selected_index(cx)
            .is_none_or(|index| index.row == 0);
        v_flex()
            .gap(px(16.))
            .child(
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
            .when(variant == Variant::Chess960, |form| {
                form.child(
                    h_flex()
                        .items_end()
                        .gap(px(8.))
                        .child(
                            widgets::field(
                                "Position number (0–959)",
                                Input::new(&self.number).aria_label("Position number (0–959)"),
                                self.number960(cx)
                                    .is_none()
                                    .then(|| "Enter a number from 0 to 959".into()),
                                &palette,
                            )
                            .flex_1(),
                        )
                        .child(
                            Button::new("random-960")
                                .outline()
                                .icon(icon("shuffle"))
                                .label("Random")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let Some(n) =
                                        crate::game::drawn(kchess_games::random_chess960())
                                    else {
                                        return;
                                    };
                                    this.number.update(cx, |input, cx| {
                                        input.set_value(n.to_string(), window, cx)
                                    });
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when(variant != Variant::Chess960, |form| {
                form.child(widgets::field(
                    "Start from a FEN (optional)",
                    Input::new(&self.fen).aria_label("Start from a FEN (optional)"),
                    self.fen_problem(cx),
                    &palette,
                ))
            })
            .child(
                h_flex()
                    .gap(px(12.))
                    .items_start()
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
                            "Black's minutes (odds)",
                            Input::new(&self.odds)
                                .aria_label("Black's minutes (odds)")
                                .disabled(no_clock),
                            None,
                            &palette,
                        )
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(palette.text_muted)
                                .child("Leave empty for equal time"),
                        )
                        .flex_1(),
                    ),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("cancel-local")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("start-local")
                            .primary()
                            .label("Start")
                            .disabled(setup.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                let Some(setup) = this.setup(cx) else {
                                    return;
                                };
                                let clock = this.clock(cx);
                                window.close_dialog(cx);
                                (this.on_start)(setup, clock, window, cx);
                            })),
                    ),
            )
    }
}
