//! What the page draws: the mode tabs, the shared board with its panel, and the chess clock's
//! faces.

use super::*;

/// The row above the page that holds the mode tabs, and its gap to the board: the board is
/// sized to leave room for it.
const MODE_BAR: Pixels = px(48.);
/// The gap between the two clock faces.
pub(super) const FACE_GAP: Pixels = px(16.);

/// "Shared board" and "Chess clock", above the page in both modes, so switching moves nothing but
/// what is in the board's place.
pub(super) fn mode_tabs(mode: Mode, palette: &Palette, cx: &mut Context<LocalPage>) -> Div {
    let page = cx.entity().downgrade();
    div().h(MODE_BAR).child(widgets::header_pills(
        "local-mode",
        [("users-round", "Shared board"), ("timer", "Chess clock")]
            .into_iter()
            .map(|(glyph, label)| widgets::Pill {
                icon: Some(icon(glyph)),
                label: label.into(),
                badge: None,
                disabled: false,
            })
            .collect(),
        usize::from(mode == Mode::Clock),
        move |index, _, cx| {
            let mode = if index == 1 { Mode::Clock } else { Mode::Board };
            crate::prefs::set(cx, MODE_KEY, mode);
            page.update(cx, |page, cx| page.after_change(cx)).ok();
        },
        palette,
    ))
}

impl Render for LocalPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        if self.mode(cx) == Mode::Clock {
            let clock = self.render_clock(&palette, window, cx);
            return surface::keys(div().size_full(), &self.focus, cx)
                .on_key_down(cx.listener(Self::clock_key))
                .child(clock)
                .into_any_element();
        }
        let board_size = self.board_size(window);
        let moves = self.moves();
        let shown = self.shown_ply();
        let live = self.game.result().is_none();
        let orientation = self.orientation(cx);
        let display = view::position_at(self.setup(), &moves, shown);
        let display_turn = view::turn(display.as_ref());
        let frontend = crate::settings::frontend(cx).clone();
        let fen = display
            .as_ref()
            .map(kchess_domain::rules::make_fen)
            .unwrap_or_default();
        let (timed, paused) = (self.game.timed(), self.game.paused());
        // gpui-lints: allow(assign_in_render) a tick redraws only the clock faces, which read this
        self.faces_active = [Color::White, Color::Black]
            .map(|color| live && display_turn == color && timed && !paused);
        let mut pockets = [
            crate::board::pocket(&self.board, Color::White, cx),
            crate::board::pocket(&self.board, Color::Black, cx),
        ];
        let mut line =
            |color: Color, middle: Option<AnyElement>| {
                let name = if color == Color::White {
                    "White"
                } else {
                    "Black"
                };
                let active = live && display_turn == color;
                widgets::player_line(
                    h_flex()
                        .gap(px(8.))
                        .child(widgets::avatar(
                            icon("user"),
                            if color == Color::White {
                                PieceSide::White
                            } else {
                                PieceSide::Black
                            },
                            active,
                            &palette,
                        ))
                        .child(name)
                        .children(pockets[usize::from(color == Color::Black)].take().or_else(
                            || {
                                widgets::material(
                                    &fen,
                                    color == Color::White,
                                    &frontend.piece_set,
                                    &palette,
                                )
                            },
                        )),
                    middle,
                    timed.then(|| self.faces[face(color)].clone().into_any_element()),
                )
            };
        let interactive = self.interactive();
        let entry = widgets::move_entry(&self.moves.input, interactive, &palette);
        let (voice, voice_line) = crate::voice::moves::board_bar(
            &self.voice,
            cx.entity().downgrade(),
            crate::voice::moves::Access {
                write: |this: &mut Self| &mut this.voice,
                play: |this: &mut Self, uci, _, cx| this.play(uci, cx),
            },
            crate::voice::moves::BoardBar {
                id: "local-voice",
                hint: "Either player can say “E four” or “Knight F three”",
                active: self.voice_taking(),
                listening: self.voice_taking(),
                grammar: kchess_domain::voice::grammar(kchess_domain::voice::Grammar::Move)
                    .to_vec(),
            },
            cx,
        );
        let board_column = v_flex()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(line(!orientation, None))
            .child(crate::board::resizable(&self.board))
            .child(line(orientation, Some(widgets::move_row(entry, voice))))
            .children(voice_line)
            .children(self.moves.message(&palette))
            .children(self.saver.problems(cx).into_iter().map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.error)
                    .child(error)
            }));
        let board_column = surface::wheel(board_column, cx);

        let panel = if self.layout.is_stacked() {
            self.render_panel(window, cx)
        } else {
            let max_width = if frontend.zen_mode {
                PANEL_ZEN
            } else {
                PANEL_MAX
            };
            self.panel
                .clone()
                .cached(panel_style(
                    self.layout.panel_height_px(board_size),
                    max_width,
                ))
                .into_any_element()
        };
        let tabs = mode_tabs(Mode::Board, &palette, cx).into_any_element();
        let page = self
            .layout
            .render_under("local-page", tabs, board_column, panel, window, cx)
            // ⌘Z outside the move box takes back, as the button does.
            .on_action(
                cx.listener(|this, _: &Undo, window, cx| this.act(Action::TakeBack, window, cx)),
            );
        surface::keys(page, &self.focus, cx).into_any_element()
    }
}

impl LocalPage {
    /// The board's side, under the mode tabs.
    pub(super) fn board_size(&self, window: &Window) -> Pixels {
        self.layout.board_size_in(px(0.), MODE_BAR, window)
    }

    /// The panel beside (or under) the board: the game's setup and status, its moves and the
    /// actions. Beside the board it is the page's `Part`, kept while only a clock face redraws.
    pub(super) fn render_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = *cx.global::<Palette>();
        let board_size = self.board_size(window);
        let max_width = if crate::settings::frontend(cx).zen_mode {
            PANEL_ZEN
        } else {
            PANEL_MAX
        };
        let narrow = widgets::is_narrow(self.layout.panel_width(board_size, max_width, window));
        let moves = self.moves();
        let sans = view::sans(self.setup(), &moves);
        let shown = self.shown_ply();
        let over = self.game.over();
        let result = self.game.result();
        let display = view::position_at(self.setup(), &moves, shown);
        let frontend = crate::settings::frontend(cx).clone();
        let (timed, paused) = (self.game.timed(), self.game.paused());
        let variant_label = crate::game::setup_label(self.setup());
        let clock_label = self.game.clock().map(|clock| {
            let mut label = widgets::time_control(clock.white.minutes, clock.white.increment);
            if clock.black.minutes != clock.white.minutes {
                label.push_str(&format!(
                    " (Black {} min)",
                    widgets::clock_minutes(clock.black.minutes)
                ));
            }
            label
        });
        let banner = match &result {
            Some(result) => {
                let winner = result.winner;
                Some(
                    widgets::StatusBanner::new(
                        if winner.is_some() {
                            Tone::Win
                        } else {
                            Tone::Plain
                        },
                        if winner.is_some() {
                            icon("trophy")
                        } else {
                            icon("handshake")
                        },
                        match winner {
                            Some(Color::White) => "White wins".into(),
                            Some(Color::Black) => "Black wins".into(),
                            None => "Draw".into(),
                        },
                    )
                    .announce()
                    .detail(SharedString::from(result.reason.clone())),
                )
            }
            None if !self.at_live() => Some(widgets::StatusBanner::reviewing(cx.listener(
                |this, _, _, cx| {
                    let last = this.moves().len();
                    this.show_ply(last, cx)
                },
            ))),
            // The board says whose turn it is (the last move, the clocks, and its description
            // for a screen reader): the line speaks only for a check or a paused clock.
            None => {
                let check = display.as_ref().is_some_and(shakmaty::Position::is_check);
                let detail = if timed && paused && !moves.is_empty() {
                    Some("Clock paused")
                } else if check {
                    Some("Check")
                } else {
                    None
                };
                detail.map(|detail| {
                    widgets::StatusBanner::turn(
                        format!(
                            "{} to move",
                            if self.turn() == Color::White {
                                "White"
                            } else {
                                "Black"
                            }
                        )
                        .into(),
                    )
                    .detail(SharedString::new_static(detail))
                })
            }
        };
        let opening = (frontend.show_opening_name && self.setup().variant == Variant::Standard)
            .then(|| {
                let opening = crate::game::opening_at(self.setup(), &moves, shown)?;
                Some((opening.eco, opening.name))
            });
        let page = cx.entity().downgrade();
        let select = move |ply: usize, _: &mut Window, cx: &mut App| {
            page.update(cx, |page, cx| page.show_ply(ply, cx)).ok();
        };
        let flip_page = cx.entity().downgrade();
        let flip: widgets::ClickHandler = Box::new(move |_, _, cx| {
            flip_page.update(cx, |page, cx| page.flip(cx)).ok();
        });
        let auto_flip = self.auto_flip(cx);
        let in_progress = !over && !moves.is_empty();
        widgets::side_panel(&palette)
            .relative()
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(max_width)
            .child(
                h_flex()
                    .gap(px(8.))
                    .flex_wrap()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .children(
                        variant_label
                            .map(|label| div().font_weight(FontWeight::MEDIUM).child(label)),
                    )
                    .children(clock_label)
                    .child(div().flex_1())
                    .child({
                        let page = cx.entity().downgrade();
                        widgets::labelled_switch(
                            "auto-flip",
                            "Flip after each move",
                            auto_flip,
                            false,
                            move |on, _, cx| {
                                page.update(cx, |this, cx| this.set_auto_flip(*on, cx)).ok();
                            },
                        )
                        .text_size(px(12.))
                    })
                    .child(widgets::view_toggles(cx)),
            )
            // An empty line where it has nothing to say: Earlier position or a check takes it
            // without pushing the moves under the pointer down.
            .child(match banner {
                Some(banner) => banner.into_any_element(),
                None => div().h(px(24.)).flex_none().into_any_element(),
            })
            .child(widgets::section_title(
                "Moves",
                opening.flatten().map(|(eco, name)| format!("{eco} {name}")),
                &palette,
            ))
            .child(div().flex_1().min_h(px(120.)).child(widgets::moves_list(
                "local-moves",
                widgets::MoveSheet {
                    sans: &sans,
                    shown,
                    first: view::first_move(self.setup()),
                },
                if !timed {
                    "No moves yet".into()
                } else {
                    "The first move starts the clock".into()
                },
                select.clone(),
                crate::notation::figurine(cx),
                &palette,
            )))
            .child(widgets::move_controls(
                shown,
                moves.len(),
                select,
                widgets::TurnButton::Flip(flip),
                cx,
            ))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .pt(px(14.))
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        Button::new("local-takeback")
                            .outline()
                            .flex_1()
                            .when(narrow, |button| button.min_w(relative(0.45)))
                            .icon(icon("undo-2"))
                            .label("Take back")
                            .disabled(moves.is_empty() || self.timed_out())
                            .on_click(cx.listener(|this, _, _, cx| this.takeback(cx))),
                    )
                    .when(timed && !over, |row| {
                        row.child(
                            Button::new("local-pause")
                                .outline()
                                .flex_1()
                                .when(narrow, |button| button.min_w(relative(0.45)))
                                .icon(if paused { icon("play") } else { icon("pause") })
                                .label(if paused { "Resume" } else { "Pause" })
                                .disabled(moves.is_empty())
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
                        )
                    })
                    .when(in_progress, |row| {
                        row.child(
                            Button::new("local-draw")
                                .outline()
                                .flex_1()
                                .when(narrow, |button| button.min_w(relative(0.45)))
                                .icon(icon("handshake"))
                                .label("Draw")
                                .on_click(cx.listener(|this, _, _, cx| this.agree_draw(cx))),
                        )
                        .child(
                            Button::new("local-resign")
                                .outline()
                                .danger()
                                .flex_1()
                                .when(narrow, |button| button.min_w(relative(0.45)))
                                .icon(icon("flag"))
                                .label("Resign")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.ask_resign(window, cx)),
                                ),
                        )
                    })
                    .child(
                        Button::new("local-new")
                            .primary()
                            .flex_1()
                            .when(narrow, |button| button.min_w(relative(0.45)))
                            .icon(icon("plus"))
                            .label("New game")
                            .tooltip_with_action("New game", &kchess_nav::page::New, None)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_new_game(window, cx)),
                            ),
                    ),
            )
            .when(!frontend.zen_mode, |panel| {
                // Copy PGN, Export and Analyse share the row.
                panel.child(widgets::panel_actions(
                    narrow,
                    [
                        Button::new("copy-pgn")
                            .ghost()
                            .small()
                            .w_full()
                            .icon(icon("copy"))
                            .label("Copy PGN")
                            .disabled(moves.is_empty())
                            .on_click(cx.listener(|this, _, window, cx| this.copy_pgn(window, cx)))
                            .into_any_element()
                            .into(),
                        crate::export::export_menu(
                            "local-export",
                            &self.exporter,
                            self.played_game(cx),
                            Anchor::BottomCenter,
                            false,
                            cx,
                        )
                        .into_any_element()
                        .into(),
                    ]
                    .into_iter()
                    .chain([Button::new("local-analyse")
                        .ghost()
                        .small()
                        .w_full()
                        .icon(icon("microscope"))
                        .label("Analyse")
                        .disabled(moves.is_empty())
                        .on_click(cx.listener(|this, _, window, cx| this.analyse(window, cx)))
                        .into_any_element()
                        .into()]),
                ))
            })
            .into_any_element()
    }

    /// A player's clock beside their name: its own `Part`, which a tick redraws.
    pub(super) fn render_clock_face(&mut self, color: Color, cx: &mut Context<Self>) -> AnyElement {
        let palette = *cx.global::<Palette>();
        let text = clock_face(self.game.remaining(color, self.now()));
        let label = format!(
            "{} clock, {text}",
            if color == Color::White {
                "White"
            } else {
                "Black"
            }
        );
        widgets::clock(text.into(), self.faces_active[face(color)], &palette)
            .id(("local-clock", face(color)))
            .test_support()
            .aria_label(label)
            .into_any_element()
    }
}
