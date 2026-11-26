//! What the page draws: the board with its players and clocks, the banner above it, and the
//! panel beside it.

use super::*;

impl ComputerPage {
    /// The opening the moves on the board reached, as `(eco, name)`.
    fn opening(&self) -> Option<(SharedString, SharedString)> {
        let opening = crate::game::opening_at(self.game.setup(), &self.moves(), self.ply())?;
        Some((opening.eco.into(), opening.name.into()))
    }

    pub(super) fn clock_text(&self, color: Color) -> Option<SharedString> {
        if !self.game.timed() {
            return None;
        }
        let remaining = self.game.remaining(color, self.host().now);
        Some(format_clock(remaining).into())
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.engine == Engine::Missing {
            let checking = self.engine == Engine::Checking;
            let standard = self.setup_variant().standard_rules();
            let banner = widgets::StatusBanner::new(
                Tone::Alert,
                icon("triangle-alert"),
                if standard {
                    "Stockfish not found"
                } else {
                    "Fairy-Stockfish not found"
                }
                .into(),
            );
            // Fairy-Stockfish comes with KChess; Settings cannot install it.
            let banner = if standard {
                banner.detail("Install or choose an engine in Settings to play".into())
            } else {
                banner
            };
            return banner
                .extra(
                    h_flex()
                        .gap(px(12.))
                        .child(
                            Button::new("check-engine")
                                .link()
                                .small()
                                .icon(icon("rotate-cw"))
                                .label("Check again")
                                .loading(checking)
                                .on_click(cx.listener(|this, _, _, cx| this.check_engine(cx))),
                        )
                        .into_any_element(),
                )
                .into_any_element();
        }
        // The computer waits for a check for a Lichess game that failed: say so, not nothing.
        let pause = cx
            .global::<crate::online::OnlineHandle>()
            .0
            .read(cx)
            .pause();
        if pause == Some(crate::online::Pause::Unverified) && !self.game.over() {
            return widgets::StatusBanner::new(
                Tone::Alert,
                icon("triangle-alert"),
                crate::online::Pause::Unverified.text().into(),
            )
            .extra(crate::online::check_again_button("assistance-check-again").into_any_element())
            .into_any_element();
        }
        if let Some(result) = self.game.result() {
            let (tone, glyph) = match result.kind {
                computer::Kind::Win => (Tone::Win, icon("trophy")),
                computer::Kind::Loss => (Tone::Loss, icon("flag")),
                computer::Kind::Draw => (Tone::Plain, icon("handshake")),
            };
            return widgets::StatusBanner::new(tone, glyph, result.title.into())
                .announce()
                .detail(SharedString::from(result.detail))
                .when_some(
                    self.can_review().then(|| {
                        Button::new("review-game")
                            .outline()
                            .small()
                            .label("Review game")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.review_game(window, cx)),
                            )
                            .into_any_element()
                    }),
                    |banner, value| banner.extra(value),
                )
                .into_any_element();
        }
        if self.ply() != self.moves().len() {
            return widgets::StatusBanner::reviewing(cx.listener(|this, _, _, cx| {
                let last = this.moves().len();
                this.show_ply(last, cx)
            }))
            .into_any_element();
        }
        if self.thinking() {
            let thinking = match self.opponent() {
                Opponent::Engine => SharedString::new_static("Stockfish is thinking…"),
                Opponent::Player(sparring) => format!("{} is thinking…", sparring.player).into(),
                Opponent::People(band) => format!("{} are thinking…", human::name(band)).into(),
            };
            return widgets::StatusBanner::turn(thinking).into_any_element();
        }
        let position = self.position_at(self.ply());
        let turn = position
            .as_ref()
            .map_or(Color::White, shakmaty::Position::turn);
        let check = position.as_ref().is_some_and(shakmaty::Position::is_check);
        widgets::StatusBanner::turn(if turn == self.user_color() {
            "Your move".into()
        } else {
            match self.opponent() {
                Opponent::Engine => SharedString::new_static("Stockfish's move"),
                Opponent::Player(sparring) => format!("{}'s move", sparring.player).into(),
                Opponent::People(_) => SharedString::new_static("Their move"),
            }
        })
        .when(check, |banner| banner.detail("Check".into()))
        .into_any_element()
    }
}

impl Render for ComputerPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let board_size = self.layout.board_size(window);
        let max_width = if crate::settings::frontend(cx).zen_mode {
            PANEL_ZEN
        } else {
            PANEL_MAX
        };
        let narrow = widgets::is_narrow(self.layout.panel_width(board_size, max_width, window));
        let user = self.user_color();
        let position = self.position_at(self.ply());
        let turn = position
            .as_ref()
            .map_or(Color::White, shakmaty::Position::turn);
        let live = self.game.result().is_none();
        let sans = self.sans();
        let moves = self.moves();
        let ply = self.ply();
        let interactive = self.interactive();
        let first = view::first_move(self.game.setup());
        let in_progress = self.in_progress();
        let result = self.game.result();
        let side_of = |color: Color| {
            if color == Color::White {
                Side::White
            } else {
                Side::Black
            }
        };
        let fen = position
            .as_ref()
            .map(kchess_domain::rules::make_fen)
            .unwrap_or_default();
        let piece_set = crate::settings::frontend(cx).piece_set.clone();
        // A running clock says whose turn it is; only an untimed game needs the outline.
        let untimed = !self.game.timed();
        let computer_line = widgets::player_line(
            h_flex()
                .min_w_0()
                .gap(px(8.))
                .child(widgets::avatar(
                    icon("cpu"),
                    side_of(!user),
                    live && turn != user && untimed,
                    &palette,
                ))
                .child(self.opponent_name())
                // Whether the moves still come from their games, or Stockfish took over.
                .children(self.out_of_their_games().map(|out| {
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .font_weight(FontWeight::NORMAL)
                        .child(if out {
                            SharedString::new_static("Out of their games")
                        } else {
                            SharedString::new_static("Their openings")
                        })
                }))
                // People of a band play at their own strength: no level to choose.
                .when(self.opponent().people().is_none(), |line| {
                    line.child(
                        h_flex()
                            .w(px(196.))
                            .min_w_0()
                            .pl(px(8.))
                            .gap(px(4.))
                            .rounded(radii::BOX)
                            .bg(palette.elevated)
                            .text_color(palette.highlighted)
                            .font_weight(FontWeight::NORMAL)
                            .child(icon("gauge").size(px(14.)).text_color(palette.text_muted))
                            .child(
                                div().flex_1().min_w_0().child(
                                    Select::new(&self.level)
                                        .small()
                                        .appearance(false)
                                        // Wider than its trigger: every level's name and rating
                                        // in full ("International Master · ~2400").
                                        .menu_width(px(260.))
                                        .accessibility_label("Difficulty"),
                                ),
                            ),
                    )
                })
                .children(widgets::material(
                    &fen,
                    user != Color::White,
                    &piece_set,
                    &palette,
                )),
            None,
            self.clock_text(!user).map(|text| {
                widgets::clock(text, live && turn != user && !untimed, &palette).into_any_element()
            }),
        );
        let entry = widgets::move_entry(&self.moves.input, interactive, &palette);
        let (voice, voice_line) = crate::voice::moves::board_bar(
            &self.voice,
            cx.entity().downgrade(),
            crate::voice::moves::Access {
                write: |this: &mut Self| &mut this.voice,
                play: |this: &mut Self, uci, _, cx| this.play(uci, cx),
            },
            crate::voice::moves::BoardBar {
                id: "computer-voice",
                hint: "Say “E four”, “Knight F three”, or “take back”",
                active: self.voice_taking(),
                listening: self.voice_listening(),
                grammar: kchess_domain::voice::grammar(kchess_domain::voice::Grammar::Game)
                    .to_vec(),
            },
            cx,
        );
        let you_line = widgets::player_line(
            h_flex()
                .gap(px(8.))
                .child(widgets::avatar(
                    icon("user"),
                    side_of(user),
                    live && turn == user && untimed,
                    &palette,
                ))
                .child("You")
                .children(widgets::material(
                    &fen,
                    user == Color::White,
                    &piece_set,
                    &palette,
                )),
            Some(widgets::move_row(entry, voice)),
            self.clock_text(user).map(|text| {
                widgets::clock(text, live && turn == user && !untimed, &palette).into_any_element()
            }),
        );
        // The lines stay with their sides when the board turns.
        let (top, bottom) = if self.flipped {
            (you_line, computer_line)
        } else {
            (computer_line, you_line)
        };
        let board_column = v_flex()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(top)
            .child(surface::wheel(
                div().child(crate::board::cached(&self.board)),
                cx,
            ))
            .child(bottom)
            .children(voice_line)
            .children(self.moves.message(&palette));

        let page = cx.entity().downgrade();
        let select = move |ply: usize, _: &mut Window, cx: &mut App| {
            let _ = page.update(cx, |page, cx| page.show_ply(ply, cx));
        };
        let other = if user == Color::White {
            "black"
        } else {
            "white"
        };
        let flip_page = cx.entity().downgrade();
        let flip: widgets::ClickHandler = Box::new(move |_, window, cx| {
            let _ = flip_page.update(cx, |page, cx| page.ask(Question::SwitchSides, window, cx));
        });
        let clock_label = self
            .game
            .clock()
            .map(|clock| widgets::time_control(clock.minutes, clock.increment));
        let frontend = crate::settings::frontend(cx);
        let zen = frontend.zen_mode;
        let show_opening = frontend.show_opening_name && self.setup_variant() == Variant::Standard;
        let setup_label = crate::game::setup_label(self.game.setup());
        let opening = show_opening.then(|| self.opening()).flatten();
        let panel = widgets::side_panel(&palette)
            .relative()
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(max_width)
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .children(
                        setup_label.map(|label| div().font_weight(FontWeight::MEDIUM).child(label)),
                    )
                    .children(clock_label)
                    .child(div().flex_1())
                    .child(
                        Button::new("game-options")
                            .ghost()
                            .small()
                            .icon(icon("settings-2"))
                            .label("Game options")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_options(window, cx)),
                            ),
                    )
                    .when(!zen, |row| {
                        row.child(crate::export::export_menu(
                            "computer-export",
                            &self.exporter,
                            self.game_export(),
                            Anchor::TopRight,
                            true,
                            cx,
                        ))
                    })
                    .child(widgets::view_toggles(cx)),
            )
            .child(self.render_banner(cx))
            .children(self.errors(cx).into_iter().map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .child(widgets::section_title(
                "Moves",
                opening.map(|(eco, name)| format!("{eco} {name}")),
                &palette,
            ))
            .child(div().flex_1().min_h(px(120.)).child(widgets::moves_list(
                "computer-moves",
                widgets::MoveSheet {
                    sans: &sans,
                    shown: ply,
                    first,
                },
                "No moves yet".into(),
                select.clone(),
                crate::notation::figurine(cx),
                &palette,
            )))
            .child(widgets::move_controls(
                ply,
                moves.len(),
                select,
                widgets::TurnButton::Named(format!("Switch sides (play {other})").into(), flip),
                cx,
            ))
            .child(
                div()
                    .pt(px(12.))
                    .border_t_1()
                    .border_color(palette.border)
                    .child(widgets::panel_actions(
                        narrow,
                        [
                            Some(
                                Button::new("takeback")
                                    .outline()
                                    .w_full()
                                    .icon(icon("undo-2"))
                                    .label("Take back")
                                    .disabled(!self.can_takeback())
                                    .on_click(cx.listener(|this, _, _, cx| this.takeback(cx))),
                            ),
                            in_progress.then(|| {
                                Button::new("resign")
                                    .outline()
                                    .w_full()
                                    .icon(icon("flag"))
                                    .label("Resign")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.ask(Question::Resign, window, cx)
                                    }))
                            }),
                            Some(
                                Button::new("new-game")
                                    .when(result.is_some(), |button| button.primary())
                                    .when(result.is_none(), |button| button.outline())
                                    .w_full()
                                    .icon(icon("plus"))
                                    .label("New game")
                                    .tooltip_with_action("New game", &crate::menu::New, None)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.ask(Question::NewGame, window, cx)
                                    })),
                            ),
                        ]
                        .into_iter()
                        .flatten()
                        .map(|button| button.into_any_element().into()),
                    )),
            );

        let page = self
            .layout
            .render("computer-page", board_column, panel, window, cx)
            // ⌘Z outside the move box takes back, as the button does.
            .on_action(
                cx.listener(|this, _: &Undo, window, cx| this.act(Action::TakeBack, window, cx)),
            );
        surface::keys(page, &self.focus, cx)
    }
}

impl ComputerPage {
    /// What went wrong that the player should know: the engine's failure, the saved game that
    /// could not be read, a save that failed.
    fn errors(&self, cx: &App) -> Vec<SharedString> {
        self.error
            .clone()
            .into_iter()
            .chain(self.saver.problems(cx))
            .collect()
    }
}
