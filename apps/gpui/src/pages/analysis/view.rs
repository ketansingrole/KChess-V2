//! The analysis board drawn: the board with its bars, the move list, the engine and tools
//! beside it, and the controls under it.

use super::*;

impl AnalysisPage {
    /// The tool under the engine, one at a time.
    fn render_tabs(&self, tool: Tool, palette: &Palette) -> impl IntoElement {
        let tabs = [
            (Tool::Moves, "Moves", icon("list-tree")),
            (Tool::Review, "Review", icon("chart-line")),
            (Tool::Explorer, "Explorer", icon("book-open-text")),
        ];
        h_flex()
            .flex_none()
            .gap(px(4.))
            .p(px(2.))
            .rounded(radii::MD)
            .border_1()
            .border_color(palette.border)
            .bg(palette.bg)
            .children(tabs.into_iter().map(|(choice, label, glyph)| {
                let chosen = choice == tool;
                h_flex()
                    .id(label)
                    .test_support()
                    .role(Role::Tab)
                    .keyboard_focus(palette.primary)
                    .aria_label(label)
                    .aria_selected(chosen)
                    .flex_1()
                    .justify_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(radii::SM)
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .when(chosen, |tab| {
                        tab.bg(palette.accented).text_color(palette.highlighted)
                    })
                    .when(!chosen, |tab| {
                        tab.text_color(palette.text_muted)
                            .pressable_text(palette.highlighted)
                    })
                    .on_click(move |_, window, cx| {
                        crate::prefs::set(cx, TOOL, choice.key());
                        window.refresh();
                    })
                    .child(glyph.size(px(16.)))
                    .child(label)
            }))
    }

    fn render_moves(&self, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        if self.root.children.is_empty() {
            return div()
                .p(px(16.))
                .text_size(px(13.))
                .text_center()
                .text_color(palette.text_muted)
                .child("Make a move on the board, or import a game")
                .into_any_element();
        }
        let view = TreeView {
            current: &self.path,
            page: cx.entity().downgrade(),
            palette: *palette,
            marks: self.review_marks(cx),
            mainline: {
                let moves = self.mainline();
                (1..=moves.len())
                    .map(|count| path_of(&moves[..count]))
                    .collect()
            },
            reveal: self.reveal_move.clone(),
            scroll: self.moves_scroll.clone(),
            figurine: crate::notation::figurine(cx),
        };
        let mut pieces = Vec::new();
        view.line(&self.root, "", None, 0, &mut pieces);
        v_flex()
            .children(self.opening(cx).map(|opening| {
                div()
                    .id("analysis-opening")
                    .test_support()
                    .px(px(4.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.text_muted)
                    .aria_label(opening.clone())
                    .child(opening)
            }))
            .child(flow(pieces).text_size(px(13.)).line_height(relative(1.9)))
            .into_any_element()
    }

    /// The opening the moves up to the one shown reached, as the game pages name it:
    /// only in standard chess, and while Settings show opening names.
    fn opening(&self, cx: &App) -> Option<String> {
        let shown = crate::settings::frontend(cx).show_opening_name;
        if !shown || self.root.variant() != Variant::Standard {
            return None;
        }
        let moves = moves_of(&self.path);
        let opening = kchess_domain::training::openings::opening_at(
            "standard",
            &self.root.fen,
            &moves,
            moves.len() as f64,
        )?;
        Some(format!("{} {}", opening.eco, opening.name))
    }

    /// First, previous, next and last move, flip, import, and the rest in a menu.
    fn render_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let at_start = self.path.is_empty();
        let (can_undo, can_redo) = (self.can_undo(), self.can_redo());
        let at_end = self.node().children.is_empty();
        let on_variation = !self.path.is_empty() && !self.root.on_mainline(&self.path);
        // A finished game cannot be played on from its last position.
        let over = game_over(self.position().as_ref()).is_some();
        let page = cx.entity().downgrade();
        let step = |id: &'static str, glyph: &'static str, label: &'static str, disabled: bool| {
            Button::new(id)
                .ghost()
                .small()
                .icon(match glyph {
                    "skip-back" => icon("skip-back"),
                    "chevron-left" => icon("chevron-left"),
                    "chevron-right" => icon("chevron-right"),
                    _ => icon("skip-forward"),
                })
                .tooltip(label)
                .accessibility_label(label)
                .disabled(disabled)
        };
        h_flex()
            .flex_none()
            .justify_center()
            .gap(px(4.))
            .child(
                step("analysis-start", "skip-back", "Go to start", at_start)
                    .on_click(cx.listener(|this, _, _, cx| this.go_to_start(cx))),
            )
            .child(
                step("analysis-back", "chevron-left", "Previous move", at_start)
                    .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
            )
            .child(
                step("analysis-forward", "chevron-right", "Next move", at_end)
                    .on_click(cx.listener(|this, _, _, cx| this.forward(cx))),
            )
            .child(
                step("analysis-end", "skip-forward", "Go to end of line", at_end)
                    .on_click(cx.listener(|this, _, _, cx| this.go_to_end(cx))),
            )
            .child(
                Button::new("analysis-flip")
                    .ghost()
                    .small()
                    .icon(icon("arrow-up-down"))
                    .tooltip(crate::menu::flip_label(cx))
                    .accessibility_label("Flip board")
                    .on_click(cx.listener(|this, _, _, cx| this.flip(cx))),
            )
            .child(crate::export::export_menu(
                "analysis-export",
                &self.exporter,
                self.game_export(),
                Anchor::BottomRight,
                false,
                cx,
            ))
            .child(
                Button::new("analysis-import")
                    .ghost()
                    .small()
                    .icon(icon("import"))
                    .tooltip("Import a PGN or FEN")
                    .accessibility_label("Import")
                    .on_click(cx.listener(|this, _, window, cx| this.open_import(window, cx))),
            )
            .child(
                Button::new("analysis-more")
                    .ghost()
                    .small()
                    .icon(icon("ellipsis"))
                    .accessibility_label("More analysis actions")
                    .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, _, _| {
                        let action = |run: fn(
                            &mut AnalysisPage,
                            &mut Window,
                            &mut Context<AnalysisPage>,
                        )| {
                            let page = page.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                let _ = page.update(cx, |page, cx| run(page, window, cx));
                            }
                        };
                        // Leaving for another page takes the position and orientation first: the
                        // switch stops this page's engine, which must not happen inside an update
                        // of this page.
                        let leave = |run: fn(Setup, Color, &mut Window, &mut App)| {
                            let page = page.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                let Some(page) = page.upgrade() else { return };
                                let (setup, orientation) = {
                                    let page = page.read(cx);
                                    (page.play_setup(), page.orientation)
                                };
                                run(setup, orientation, window, cx);
                            }
                        };
                        // The same, with the position's setup: its variant and FEN.
                        let play = |run: fn(Setup, &mut Window, &mut App)| {
                            let page = page.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                let Some(page) = page.upgrade() else { return };
                                let setup = page.read(cx).play_setup();
                                run(setup, window, cx);
                            }
                        };
                        menu.item(
                            PopupMenuItem::new("Undo")
                                .icon(icon("undo-2"))
                                .disabled(!can_undo)
                                .on_click(action(|page, window, cx| page.undo(window, cx))),
                        )
                        .item(
                            PopupMenuItem::new("Redo")
                                .icon(icon("redo-2"))
                                .disabled(!can_redo)
                                .on_click(action(|page, window, cx| page.redo(window, cx))),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new("Copy FEN")
                                .icon(icon("copy"))
                                .on_click(action(|page, window, cx| page.copy_fen(window, cx))),
                        )
                        .item(
                            PopupMenuItem::new("Copy PGN")
                                .icon(icon("file-text"))
                                .on_click(action(|page, window, cx| page.copy_pgn(window, cx))),
                        )
                        .item(
                            PopupMenuItem::new("Copy line")
                                .icon(icon("list-tree"))
                                .disabled(at_start)
                                .on_click(action(|page, window, cx| {
                                    let path = page.path.clone();
                                    page.copy_line(&path, window, cx)
                                })),
                        )
                        .item(
                            PopupMenuItem::new("Import a position or game…")
                                .icon(icon("import"))
                                .on_click(action(|page, window, cx| page.open_import(window, cx))),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new("Make main line")
                                .icon(icon("arrow-up-to-line"))
                                .disabled(!on_variation)
                                .on_click(action(|page, _, cx| page.promote(cx))),
                        )
                        .item(
                            PopupMenuItem::new("Delete from here")
                                .icon(icon("scissors"))
                                .disabled(at_start)
                                .on_click(action(|page, window, cx| {
                                    page.delete_from_here(window, cx)
                                })),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new("Edit this position")
                                .icon(icon("pencil-ruler"))
                                .on_click(leave(|setup, orientation, window, cx| {
                                    crate::pages::editor::edit_position(
                                        setup,
                                        orientation,
                                        window,
                                        cx,
                                    )
                                })),
                        )
                        .item(
                            PopupMenuItem::new("Play Stockfish from here")
                                .icon(icon("cpu"))
                                .disabled(over)
                                .on_click(play(crate::pages::computer::play_from)),
                        )
                        .item(
                            PopupMenuItem::new("Play a friend from here")
                                .icon(icon("users-round"))
                                .disabled(over)
                                .on_click(play(crate::pages::local::play_from)),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new("New analysis")
                                .icon(icon("rotate-ccw"))
                                .on_click(action(|page, _, cx| page.load(None, cx))),
                        )
                    }),
            )
    }
}

impl AnalysisPage {
    /// The tools beside (or under) the board: the engine or practice, the tabs and the tool
    /// shown, and the controls. Beside the board it is the page's `Part`, so a board animation
    /// or the move box redrawing reuses it.
    pub(super) fn render_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = *cx.global::<Palette>();
        let tool = Tool::of(&crate::prefs::get::<String>(cx, TOOL, "moves".into()));
        let pause = Self::pause(cx);
        let board_size = self.board_size(window);
        let tool_body: AnyElement = match tool {
            // Never shorter than its list's minimum (`.moves-tool { min-height: auto }`).
            Tool::Moves => v_flex()
                .flex_1()
                .gap(px(8.))
                .child(
                    div()
                        .id("analysis-moves")
                        .flex_1()
                        // Under the board the list has a height of its own.
                        .min_h(if self.layout.is_stacked() {
                            px(240.)
                        } else {
                            px(96.)
                        })
                        .overflow_y_scroll()
                        .track_scroll(&self.moves_scroll)
                        .p(px(8.))
                        .border_1()
                        .border_color(palette.border)
                        .rounded(radii::LG)
                        .bg(palette.bg)
                        // Away from any move, the list's menu copies the game.
                        .context_menu({
                            let page = cx.entity().downgrade();
                            move |menu, _, _| {
                                let page = page.clone();
                                menu.item(
                                    PopupMenuItem::new("Copy PGN")
                                        .icon(icon("file-text"))
                                        .on_click(move |_, window, cx| {
                                            let _ = page.update(cx, |page, cx| {
                                                let pgn = page.root.pgn_text();
                                                page.copy(pgn, "PGN", window, cx)
                                            });
                                        }),
                                )
                            }
                        })
                        .child(self.render_moves(cx, &palette)),
                )
                .child(self.render_notes(cx, &palette))
                .into_any_element(),
            Tool::Review => {
                let note = |text: &'static str| {
                    div()
                        .px(px(8.))
                        .py(px(24.))
                        .text_size(px(13.))
                        .text_center()
                        .text_color(palette.text_muted)
                        .child(text)
                        .into_any_element()
                };
                let body = if let Some(pause) = pause {
                    paused_note(pause, &palette)
                } else if self.root.children.is_empty() {
                    note("Make some moves or import a game to review it")
                } else {
                    self.render_review(cx, &palette)
                };
                div()
                    .id("analysis-review")
                    .flex_1()
                    .min_h(px(96.))
                    .overflow_y_scroll()
                    .child(body)
                    .into_any_element()
            }
            Tool::Explorer => {
                let note = if self.hides_engine() {
                    Some(
                        div()
                            .flex_1()
                            .px(px(8.))
                            .py(px(24.))
                            .text_size(px(13.))
                            .text_center()
                            .text_color(palette.text_muted)
                            .child("Shown once you find the move")
                            .into_any_element(),
                    )
                } else {
                    pause.map(|pause| paused_note(pause, &palette))
                };
                match note {
                    Some(note) => note,
                    None => {
                        let (fen, opened) = (self.node().fen.clone(), !self.explorer_open);
                        let variant = self.root.variant();
                        self.explorer.update(cx, |explorer, cx| {
                            explorer.show(variant, &fen, None, opened, cx)
                        });
                        div()
                            .id("analysis-explorer")
                            .flex_1()
                            .min_h(px(96.))
                            .overflow_y_scroll()
                            .child(self.explorer.clone())
                            .into_any_element()
                    }
                }
            }
        };
        self.explorer_open = tool == Tool::Explorer;
        // A click in the panel gives the keys back to the board, before a text field in it
        // (a comment, the explorer's player) takes them for itself.
        widgets::side_panel(&palette)
            // A short window scrolls the panel rather than let the move list cover the controls.
            .id("analysis-panel")
            .overflow_y_scroll()
            .takes_keys(&self.focus)
            .h(self.layout.fixed(self.panel_height(board_size)))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(12.))
            .when(self.practice.is_some(), |panel| {
                panel.child(self.render_practice(cx, &palette))
            })
            .when(self.practice.is_none(), |panel| {
                panel.child(self.render_engine(cx, &palette))
            })
            .child(self.render_tabs(tool, &palette))
            .child(tool_body)
            .child(self.render_controls(cx))
            .into_any_element()
    }

    /// The panel's height beside a board of `board_size`: the board and the move box under it.
    /// The panel beside the board runs from the board's top to the move box's bottom: the row
    /// of material above the board is the board's alone.
    fn panel_height(&self, board_size: Pixels) -> Pixels {
        board_size + px(36.) + widgets::STACK_GAP
    }

    /// The board's size in the page, the eval bar beside it and the study bar above.
    fn board_size(&self, window: &Window) -> Pixels {
        self.layout.board_size_around(
            EVAL_BAR + EVAL_GAP,
            STUDY_BAR,
            MATERIAL_ROW + widgets::STACK_GAP * 2. + px(36.),
            window,
        )
    }
}

impl Render for AnalysisPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_notes(window, cx);
        let palette = *cx.global::<Palette>();
        let allowed = Self::assistance_allowed(cx);
        let using_cloud = self.using_cloud(cx);
        let show_bar = allowed && (Self::engine_on(cx) || using_cloud) && !self.hides_engine();
        let bar_line = self.bar_line(cx);
        let position = self.position();
        let result = position
            .as_ref()
            .filter(|position| game_over(Some(position)).is_some())
            .map(result_of);
        // The column always keeps the eval bar's width beside the board.
        // The column holds the board and the move box under it.
        let board_size = self.board_size(window);
        let entry = widgets::move_entry(&self.move_box.input, true, &palette);
        let (voice, voice_line) = board_bar(
            &self.voice,
            cx.entity().downgrade(),
            Access {
                write: |this: &mut Self| &mut this.voice,
                play: |this: &mut Self, uci, _, cx| {
                    this.play(uci, cx);
                },
            },
            BoardBar {
                id: "analysis-voice",
                hint: "Say a move like “Knight F three”, or “back”, “next”, “best move”",
                active: true,
                listening: true,
                grammar: kchess_domain::voice::grammar(kchess_domain::voice::Grammar::Analysis)
                    .to_vec(),
            },
            cx,
        );
        // A Crazyhouse game's pockets: the far side's above the board, the near side's below.
        let pocket = |color: Color| {
            crate::board::pocket(&self.board, color, cx)
                .map(|pocket| div().pl(EVAL_BAR + EVAL_GAP).child(pocket))
        };
        let (top_pocket, bottom_pocket) = (pocket(!self.orientation), pocket(self.orientation));
        // What each side has won, as in a game: the far side's above the board, the near
        // side's beside the move box. The row above keeps its place, so the board holds still.
        let fen = self.node().fen.clone();
        let piece_set = crate::settings::frontend(cx).piece_set.clone();
        let material = |color: Color, id: &'static str| {
            widgets::material(&fen, color == Color::White, &piece_set, &palette)
                .map(|won| div().id(id).test_support().child(won))
        };
        let board_column = v_flex()
            .w(board_size + EVAL_BAR + EVAL_GAP)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(
                h_flex()
                    .h(MATERIAL_ROW)
                    .pl(EVAL_BAR + EVAL_GAP)
                    .children(material(!self.orientation, "analysis-material-far")),
            )
            .children(top_pocket)
            .child(
                h_flex()
                    .items_stretch()
                    .gap(EVAL_GAP)
                    .when(show_bar, |row| {
                        // The bar ends where the squares end, above any outside coordinates.
                        row.child(div().pb(crate::board::outside_gutter(cx)).child(
                            crate::engine_view::eval_bar(
                                bar_line.as_ref(),
                                self.orientation,
                                result,
                            ),
                        ))
                    })
                    .when(!show_bar, |row| row.pl(EVAL_BAR))
                    .child(
                        crate::game::surface::wheel(div().id("analysis-board"), cx)
                            .w(board_size)
                            // The board takes the keys back from the move box, as a click
                            // outside a text field does in the browser.
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)),
                            )
                            .child(crate::board::cached(&self.board)),
                    ),
            )
            .children(bottom_pocket)
            .child(
                v_flex()
                    .pl(EVAL_BAR + EVAL_GAP)
                    .gap(px(4.))
                    .child(
                        h_flex()
                            .h(px(36.))
                            .gap(px(12.))
                            .children(material(self.orientation, "analysis-material-near"))
                            .child(widgets::move_row(entry, voice)),
                    )
                    .children(voice_line)
                    .children(self.move_box.message(&palette))
                    .children(self.saver.problems(cx).into_iter().enumerate().map(
                        |(ix, error)| {
                            div()
                                .id(("analysis-problem", ix))
                                .test_support()
                                .aria_label(error.clone())
                                .text_size(px(12.))
                                .text_color(palette.error)
                                .child(error)
                        },
                    )),
            );
        let panel = if self.layout.is_stacked() {
            self.render_panel(window, cx)
        } else {
            let mut style = panel_style(self.panel_height(board_size), PANEL_MAX);
            style.margin.top = Some((MATERIAL_ROW + widgets::STACK_GAP).into());
            self.panel.clone().cached(style).into_any_element()
        };
        div()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            // A text field takes these for its own text first.
            .on_action(cx.listener(|this, _: &Undo, window, cx| this.undo(window, cx)))
            .on_action(cx.listener(|this, _: &Redo, window, cx| this.redo(window, cx)))
            .child(self.layout.render_under(
                "analysis",
                self.study_bar.clone(),
                board_column,
                panel,
                window,
                cx,
            ))
    }
}
