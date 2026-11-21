//! The board drawn: its squares as a screen reader reads them, coordinates, the promotion
//! picker, and `Render`.

use super::*;

impl ChessBoard {
    /// The squares for a screen reader, in ranks from the viewer's top: each named for what
    /// stands on it, the selected one marked, the cursor's the one read while the board has the
    /// keys (and ringed then). A screen reader's press on one presses it as Enter does.
    fn render_squares(&self, focused: bool, ring: Hsla, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let targets = self.targets();
        let cursor = self.cursor();
        let interactive = self.interactive();
        let entity = cx.entity().downgrade();
        (0..8u32)
            .map(|row| {
                let cells = (0..8u32).map(|column| {
                    let (file, rank) = match self.state.orientation {
                        Color::White => (column, 7 - row),
                        Color::Black => (7 - column, row),
                    };
                    let square = Square::from_coords(File::new(file), Rank::new(rank));
                    let name = keyboard::square_name(
                        square,
                        self.pieces.piece_at(square),
                        self.state.blindfold,
                        targets.contains(&square),
                    );
                    let at_cursor = square == cursor;
                    div()
                        .id(("square", usize::from(square)))
                        .test_support()
                        .role(gpui_kit::Role::GridCell)
                        .aria_label(name)
                        .aria_column_index(column as usize)
                        .aria_selected(self.selected == Some(square))
                        .when(at_cursor, |cell| cell.aria_active_descendant())
                        .when(interactive, |cell| {
                            let entity = entity.clone();
                            cell.on_a11y_action(
                                gpui_kit::AccessibleAction::Click,
                                move |_, _, cx| {
                                    let _ = entity.update(cx, |this, cx| {
                                        this.cursor = Some(square);
                                        this.activate(square, cx);
                                    });
                                },
                            )
                        })
                        .absolute()
                        .top_0()
                        .left(relative(column as f32 / 8.))
                        .w(relative(0.125))
                        .h_full()
                        .when(focused && at_cursor, |cell| {
                            cell.border(px(3.)).border_color(ring)
                        })
                        .into_any_element()
                });
                div()
                    .id(("rank", row as usize))
                    .role(gpui_kit::Role::Row)
                    .aria_row_index(row as usize)
                    .absolute()
                    .left_0()
                    .top(relative(row as f32 / 8.))
                    .w_full()
                    .h(relative(0.125))
                    .children(cells)
                    .into_any_element()
            })
            .collect()
    }

    /// `coords.squares`: each square's name in its top corner, in the other square color.
    fn render_square_names(&self, look: &looks::BoardLook) -> Vec<AnyElement> {
        Square::ALL
            .into_iter()
            .map(|square| {
                let (column, row) = self.cell(square);
                div()
                    .absolute()
                    .left(relative(column as f32 / 8.))
                    .top(relative(row as f32 / 8.))
                    .size(relative(0.125))
                    .flex()
                    .justify_end()
                    .pt(relative(0.06 / 8.))
                    .pr(relative(0.04 / 8.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if square.is_light() {
                        crate::theme::hex(&look.coord_black)
                    } else {
                        crate::theme::hex(&look.coord_white)
                    })
                    .opacity(0.8)
                    .child(square.to_string())
                    .into_any_element()
            })
            .collect()
    }

    /// Lichess's coordinate size: 8px on a small board, 12px from a large one up.
    fn coordinate_size(&self) -> Pixels {
        px((f32::from(self.square_size()) * 8. * 0.0192).clamp(8., 12.))
    }

    fn render_coordinates(
        &self,
        mode: CoordinateMode,
        look: &looks::BoardLook,
        cx: &App,
    ) -> Vec<AnyElement> {
        if mode == CoordinateMode::None {
            return Vec::new();
        }
        let inside = mode == CoordinateMode::Inside;
        let muted = cx.theme().muted_foreground;
        // A label on a light square is drawn in the dark square's color, and the other way round.
        let color = |square: Square| {
            if !inside {
                muted
            } else if square.is_light() {
                crate::theme::hex(&look.coord_black)
            } else {
                crate::theme::hex(&look.coord_white)
            }
        };
        let (edge_file, edge_rank) = match self.state.orientation {
            Color::White => (File::H, Rank::First),
            Color::Black => (File::A, Rank::Eighth),
        };
        let size = self.coordinate_size();
        let mut labels = Vec::new();
        // As on Lichess: ranks in the top-right corner of the edge file, files in the bottom-left
        // corner of the edge rank; outside, each centred on its square in the board's gutter.
        for rank in Rank::ALL {
            let square = Square::from_coords(edge_file, rank);
            let (_, row) = self.cell(square);
            labels.push(
                div()
                    .absolute()
                    .top(relative(row as f32 / 8.))
                    .when(inside, |label| label.right_0().w(size * 0.8).mt(px(1.)))
                    .when(!inside, |label| {
                        label
                            .right(-OUTSIDE_GUTTER)
                            .w(OUTSIDE_GUTTER)
                            .h(relative(0.125))
                            .flex()
                            .items_center()
                            .justify_center()
                    })
                    .text_size(size)
                    .font_weight(FontWeight::BOLD)
                    .text_color(color(square))
                    .child(rank.char().to_string())
                    .into_any_element(),
            );
        }
        for file in File::ALL {
            let square = Square::from_coords(file, edge_rank);
            let (column, _) = self.cell(square);
            labels.push(
                div()
                    .absolute()
                    .left(relative(column as f32 / 8.))
                    .when(inside, |label| {
                        label
                            .bottom_0()
                            .h(size * 1.4)
                            .flex()
                            .items_center()
                            .pl(px(4.))
                    })
                    .when(!inside, |label| {
                        label
                            .bottom(-OUTSIDE_GUTTER)
                            .h(OUTSIDE_GUTTER)
                            .w(relative(0.125))
                            .flex()
                            .items_center()
                            .justify_center()
                    })
                    .text_size(size)
                    .font_weight(FontWeight::BOLD)
                    .text_color(color(square))
                    .child(file.char().to_string())
                    .into_any_element(),
            );
        }
        labels
    }

    /// The promotion picker; while the board has the keys, the piece they would take is ringed
    /// and is the one a screen reader reads.
    fn render_promotion(
        &self,
        set: &str,
        focused: bool,
        ring: Hsla,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (_, to) = self.promotion?;
        let color = if to.rank() == Rank::Eighth {
            Color::White
        } else {
            Color::Black
        };
        let theme = cx.theme();
        let (elevated, border, background) = (theme.muted, theme.border, theme.background);
        let palette = *cx.global::<crate::theme::Palette>();
        let marked = self.promotion_choice % keyboard::PROMOTION.len();
        Some(
            div()
                .id("promotion")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(radii::XL)
                .bg(hsla(0., 0., 0., 0.5))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.cancel_promotion(cx)),
                )
                .child(
                    div()
                        .id("promotion-choices")
                        .test_support()
                        .role(gpui_kit::Role::Dialog)
                        .aria_label("Choose promotion piece")
                        .flex()
                        .gap(px(8.))
                        .p(px(8.))
                        .rounded(radii::XL)
                        .bg(elevated)
                        .border_1()
                        .border_color(border)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .children(keyboard::PROMOTION.into_iter().enumerate().map(
                            |(index, (role, label))| {
                                div()
                                    .id(label)
                                    .test_support()
                                    .role(gpui_kit::Role::Button)
                                    .aria_label(label)
                                    .aria_selected(index == marked)
                                    .when(index == marked, |choice| choice.aria_active_descendant())
                                    .size(px(60.))
                                    .p(px(4.))
                                    // Concentric with the picker's corners, 8px further out.
                                    .rounded(radii::SM)
                                    .bg(background)
                                    .border_1()
                                    .border_color(border)
                                    .cursor_pointer()
                                    .pressable_fill(elevated, &palette)
                                    .when(focused && index == marked, |choice| {
                                        choice.shadow(vec![BoxShadow {
                                            color: ring,
                                            offset: point(px(0.), px(0.)),
                                            blur_radius: px(0.),
                                            spread_radius: px(2.),
                                            inset: false,
                                        }])
                                    })
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(label).build(window, cx)
                                    })
                                    .on_click(
                                        cx.listener(move |this, _, _, cx| this.promote(role, cx)),
                                    )
                                    .child(
                                        img(SharedString::from(looks::piece_path(
                                            set,
                                            Piece { color, role },
                                        )))
                                        .size_full(),
                                    )
                            },
                        )),
                )
                .into_any_element(),
        )
    }
}

/// What a screen reader says of the board: the last move (its piece, and a check), the side to
/// move, and a hint or solution shown on it.
fn describe(state: &BoardState) -> String {
    let to_move = if kchess_domain::rules::fen_turn(&state.fen).is_black() {
        "Black to move"
    } else {
        "White to move"
    };
    let mut parts = Vec::new();
    // The move as a player sees it: the piece that went (kept to itself in blindfold play, as the
    // squares keep theirs) and the check it gives, not only two squares.
    if let Some((from, to)) = state.last_move {
        let piece = (!state.blindfold)
            .then(|| board_of(&state.fen).piece_at(to))
            .flatten()
            .map(|piece| format!("{} ", kchess_domain::material::name(piece.role)))
            .unwrap_or_default();
        let check = match (state.check, &state.badge) {
            (Some(king), Some(badge)) if badge.square == king && badge.glyph == "#" => {
                ", checkmate"
            }
            (Some(_), _) => ", check",
            (None, _) => "",
        };
        parts.push(format!("Last move {piece}{from}–{to}{check}"));
    }
    parts.push(to_move.to_string());
    // What the board shows the player to play, which is otherwise only seen: a hint's piece, and
    // the moves its arrows tell ("Hint g1–f3"), each kind once.
    let mut told: Vec<(&str, Vec<String>)> = Vec::new();
    let shown = state
        .hint
        .map(|square| ("Hint", square.to_string()))
        .into_iter()
        .chain(state.arrows.iter().filter_map(|arrow| {
            arrow
                .spoken
                .map(|kind| (kind, format!("{}–{}", arrow.from, arrow.to)))
        }));
    for (kind, said) in shown {
        match told.iter_mut().find(|(told, _)| *told == kind) {
            Some((_, moves)) => moves.push(said),
            None => told.push((kind, vec![said])),
        }
    }
    parts.extend(
        told.into_iter()
            .map(|(kind, moves)| format!("{kind} {}", moves.join(", "))),
    );
    parts.join(" · ")
}

impl Render for ChessBoard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A small picture takes no keys; any other board, even one only looked at, can be read
        // square by square.
        let focus = (!self.state.preview).then(|| self.focus_handle(cx));
        let focused = self.has_keys(window);
        let ring = cx.global::<crate::theme::Palette>().primary;
        let frontend = crate::settings::frontend(cx).clone();
        let look = looks::board(&frontend.board_theme);
        let set = looks::piece_set(&frontend.piece_set).to_string();
        let lifted = self
            .drag
            .as_ref()
            .filter(|drag| drag.moved)
            .map(|drag| drag.from);
        let hovered = self
            .drag
            .as_ref()
            .filter(|drag| drag.moved)
            .and_then(|drag| self.square_at(drag.position));
        let mut shown = self.state.clone();
        let preview = self
            .drawing
            .and_then(|(from, brush, to)| to.map(|to| Drawn { from, to, brush }));
        for shape in self.drawn.iter().chain(preview.iter()) {
            if shape.from == shape.to {
                shown.circles.push(Circle::brush(shape.brush, shape.from));
            } else {
                shown
                    .arrows
                    .push(Arrow::brush(shape.brush, shape.from, shape.to));
            }
        }
        let premove_dests = self
            .selected
            .filter(|&square| {
                self.state.movable.is_none()
                    && self
                        .pieces
                        .piece_at(square)
                        .is_some_and(|piece| Some(piece.color) == self.state.premove)
            })
            .map(|square| premove_targets(&self.pieces, square))
            .unwrap_or_default();
        let sliding: Vec<Square> = self
            .animation
            .as_ref()
            .map(|animation| animation.moves.iter().map(|(_, _, to)| *to).collect())
            .unwrap_or_default();
        let frame = Frame {
            state: shown,
            pieces: self.pieces.clone(),
            selected: self.selected,
            lifted,
            hovered,
            show_dests: frontend.show_legal_moves,
            set: set.clone(),
            queued: self.queued,
            premove_dests,
            drop_dests: self
                .selected_drop
                .and_then(|role| self.state.drops.get(&role).cloned())
                .unwrap_or_default(),
            sliding,
        };
        let bounds = Rc::clone(&self.bounds);
        let side = self.square_size();
        let origin = self.bounds.get().origin;
        let entity = cx.entity().downgrade();
        let dragging = self.drag.is_some();

        // The king in check sits in Chessground's radial gradient, clipped to its square: solid
        // red to a quarter of the way out, fading to nothing at 89% of the square's half-diagonal.
        let check = self.state.check.map(|square| {
            let (column, row) = self.cell(square);
            div()
                .absolute()
                .left(relative(column as f32 / 8.))
                .top(relative(row as f32 / 8.))
                .size(relative(0.125))
                .overflow_hidden()
                .children(check_rings(side).into_iter().map(|(radius, color)| {
                    div()
                        .absolute()
                        .left(side / 2. - radius)
                        .top(side / 2. - radius)
                        .size(radius * 2.)
                        .rounded_full()
                        .bg(color)
                }))
        });
        // The piece's own square keeps a faint ghost while it is dragged (Chessground's `ghost`).
        let ghost = lifted.and_then(|square| {
            let piece = self.pieces.piece_at(square)?;
            let (column, row) = self.cell(square);
            Some(
                img(SharedString::from(looks::piece_path(&set, piece)))
                    .absolute()
                    .left(relative(column as f32 / 8.))
                    .top(relative(row as f32 / 8.))
                    .size(relative(0.125))
                    .opacity(0.3),
            )
        });
        // Sliding pieces, between where they were and where they land.
        let sliding: Vec<AnyElement> = if self.state.blindfold {
            Vec::new()
        } else {
            self.animation
                .as_ref()
                .map(|animation| {
                    let now = cx.background_executor().now();
                    if !animation.ended(now) {
                        window.request_animation_frame();
                    }
                    let t = animation.progress(now);
                    animation
                        .moves
                        .iter()
                        .enumerate()
                        .map(|(index, (piece, from, to))| {
                            let (c1, r1) = self.cell(*from);
                            let (c2, r2) = self.cell(*to);
                            let column = c1 as f32 + (c2 as f32 - c1 as f32) * t;
                            let row = r1 as f32 + (r2 as f32 - r1 as f32) * t;
                            img(SharedString::from(looks::piece_path(&set, *piece)))
                                .id(("board-slide", index))
                                .absolute()
                                .left(relative(column / 8.))
                                .top(relative(row / 8.))
                                .size(relative(0.125))
                                .test_support()
                                .into_any_element()
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let held = self.drag.as_ref().filter(|drag| drag.moved).map(|drag| {
            deferred(
                img(SharedString::from(looks::piece_path(&set, drag.piece)))
                    .absolute()
                    .left(drag.position.x - origin.x - side / 2.)
                    .top(drag.position.y - origin.y - side / 2.)
                    .size(side)
                    .cursor_grabbing(),
            )
            .with_priority(1)
        });
        let coordinate_mode = self.state.coordinates.unwrap_or(frontend.coordinates);
        let gutter = (!self.state.preview && coordinate_mode == CoordinateMode::Outside)
            .then_some(OUTSIDE_GUTTER);
        let mut coordinates = if self.state.preview {
            Vec::new()
        } else {
            self.render_coordinates(coordinate_mode, look, cx)
        };
        if self.state.square_names {
            coordinates.extend(self.render_square_names(look));
        }
        let promotion = self.render_promotion(&set, focused, ring, cx);
        let squares = if self.state.preview {
            Vec::new()
        } else {
            self.render_squares(focused, ring, cx)
        };
        // The badge overhangs the square's top-right corner, except where that would leave the
        // board.
        let badge = self
            .state
            .badge
            .as_ref()
            .filter(|_| !self.state.preview)
            .map(|badge| {
                let (column, row) = self.cell(badge.square);
                let diameter = (side * 0.42).max(px(16.));
                let x = if column == 7 {
                    side - diameter
                } else {
                    side - diameter * 0.68
                };
                let y = if row == 0 { px(0.) } else { -diameter * 0.32 };
                div()
                    .absolute()
                    .left(side * column as f32 + x)
                    .top(side * row as f32 + y)
                    .size(diameter)
                    .rounded_full()
                    .bg(badge.color)
                    .border_1()
                    .border_color(rgb(0xffffff))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(diameter * 0.55)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(0xffffff))
                    .child(badge.glyph)
            });

        let description = (!self.state.preview).then(|| describe(&self.state));
        div()
            .id("chess-board")
            .test_support()
            // Tab gives a board its keys (a press leaves them with the page); it takes the
            // arrows, Enter, Space and Escape, and hands the rest on.
            .when_some(focus, |board, focus| {
                board
                    .track_focus(&focus)
                    .key_context(keyboard::CONTEXT)
                    .on_key_down(cx.listener(Self::key))
            })
            // Pictures in a list are named by what holds them; a board to play on is a grid of
            // squares, and says each move as it is played (a live region).
            .when_some(description, |board, description| {
                board
                    .role(gpui_kit::Role::Grid)
                    .aria_label("Chess board")
                    .aria_description(description.clone())
                    .aria_row_count(8)
                    .aria_column_count(8)
                    .child(crate::widgets::live_region("board-live", description))
            })
            .w_full()
            .aspect_ratio(1.)
            // Coordinates outside the board take a strip of the board's own square.
            .when_some(gutter, |board, gutter| board.pr(gutter).pb(gutter))
            .child(
                // The board itself: every square, piece and label, which the canvas measures.
                div()
                    .relative()
                    .size_full()
                    .child(
                        img(SharedString::from(format!("boards/{}", look.image)))
                            .absolute()
                            .size_full()
                            .object_fit(ObjectFit::Fill)
                            .rounded(radii::BOARD),
                    )
                    .children(check)
                    .child(
                        canvas(
                            move |canvas_bounds, _, _| bounds.set(canvas_bounds),
                            move |canvas_bounds, (), window, cx| {
                                paint_frame(frame, canvas_bounds, window, cx);
                                if dragging {
                                    // A drag follows the pointer anywhere in the window, not only over the board.
                                    let moving = entity.clone();
                                    window.on_mouse_event(
                                        move |event: &MouseMoveEvent, phase, _, cx| {
                                            if phase == DispatchPhase::Bubble {
                                                let _ = moving.update(cx, |this, cx| {
                                                    // A release this listener missed (it arrived before
                                                    // this frame) ends the drag at the next move.
                                                    if event.pressed_button
                                                        == Some(MouseButton::Left)
                                                    {
                                                        this.pointer_moved(event.position, cx)
                                                    } else {
                                                        this.release(event.position, cx)
                                                    }
                                                });
                                            }
                                        },
                                    );
                                    let releasing = entity.clone();
                                    window.on_mouse_event(
                                        move |event: &MouseUpEvent, phase, _, cx| {
                                            if phase == DispatchPhase::Bubble
                                                && event.button == MouseButton::Left
                                            {
                                                let _ = releasing.update(cx, |this, cx| {
                                                    this.release(event.position, cx)
                                                });
                                            }
                                        },
                                    );
                                }
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                    .children(squares)
                    .children(ghost)
                    .children(sliding)
                    .children(coordinates)
                    .children(badge)
                    .children(held)
                    .children(promotion),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event, window, cx| {
                    // The keys stay where they were, as a button's press leaves them, so the page
                    // keeps its arrows after a click on the board.
                    window.prevent_default();
                    this.press(event, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event, _, cx| {
                    cx.stop_propagation();
                    this.start_drawing(event, cx)
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.drawing.is_some() {
                    this.draw_to(event.position, cx)
                }
            }))
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.finish_drawing(event.position, cx)
                }),
            )
            .on_mouse_up_out(
                MouseButton::Right,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.finish_drawing(event.position, cx)
                }),
            )
            // The press's own frame has no window-wide listeners yet: a quick release lands here.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| this.release(event.position, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| this.release(event.position, cx)),
            )
            .on_drop(cx.listener(|this, spare: &SparePiece, window, cx| {
                this.drop_spare(spare.0, window.mouse_position(), cx)
            }))
            .on_drop(cx.listener(|this, held: &pocket::PocketPiece, window, cx| {
                this.drop_pocket(held.0, window.mouse_position(), cx)
            }))
            .cursor(match self.state.edit {
                Some(EditTool::Place(_) | EditTool::Trash) => CursorStyle::Crosshair,
                Some(EditTool::Pointer) => CursorStyle::PointingHand,
                None if self.state.movable.is_some() => CursorStyle::PointingHand,
                None => CursorStyle::Arrow,
            })
    }
}

#[cfg(test)]
mod description_tests {
    use super::{BoardState, describe};
    use shakmaty::Color;

    /// A screen reader hears the last move and whose turn it is.
    #[test]
    fn the_board_says_the_last_move_and_whose_turn_it_is() {
        let start = BoardState::shown(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            kchess_domain::variant::Variant::Standard,
            Color::White,
            None,
        );
        assert_eq!(describe(&start), "White to move");
        let after = BoardState::shown(
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
            kchess_domain::variant::Variant::Standard,
            Color::White,
            Some("e2e4"),
        );
        assert_eq!(describe(&after), "Last move pawn e2–e4 · Black to move");
    }

    /// The last move says its piece and the check it gives, as the board shows them; blindfold
    /// play keeps the piece to itself.
    #[test]
    fn the_last_move_says_its_piece_and_a_check() {
        let shown = |fen: &str, last: &str| {
            BoardState::shown(
                fen,
                kchess_domain::variant::Variant::Standard,
                Color::White,
                Some(last),
            )
        };
        let check = shown(
            "rnbqkbnr/ppp2ppp/3p4/1B2p3/4P3/8/PPPP1PPP/RNBQK1NR b KQkq - 1 3",
            "f1b5",
        );
        assert_eq!(
            describe(&check),
            "Last move bishop f1–b5, check · Black to move"
        );
        let mate = shown(
            "r1bqkb1r/pppp1Qpp/2n2n2/4p3/2B1P3/8/PPPP1PPP/RNB1K1NR b KQkq - 0 4",
            "h5f7",
        );
        assert_eq!(
            describe(&mate),
            "Last move queen h5–f7, checkmate · Black to move"
        );
        let mut blindfold = mate;
        blindfold.blindfold = true;
        assert_eq!(
            describe(&blindfold),
            "Last move h5–f7, checkmate · Black to move"
        );
    }

    /// A hint or a solution shown on the board is said too, not only drawn; the engine's lines
    /// and a lesson's drawings are not.
    #[test]
    fn the_board_says_a_hint_or_a_solution_it_shows() {
        use super::Arrow;
        use shakmaty::Square;
        let mut state = BoardState::shown(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            kchess_domain::variant::Variant::Standard,
            Color::White,
            None,
        );
        state.arrows = vec![
            Arrow::engine(Square::D2, Square::D4, 0),
            Arrow::brush("green", Square::E2, Square::E4),
        ];
        assert_eq!(describe(&state), "White to move");
        state.hint = Some(Square::G1);
        assert_eq!(describe(&state), "White to move · Hint g1");
        state.hint = None;
        state.arrows.push(Arrow::hint(Square::G1, Square::F3));
        assert_eq!(describe(&state), "White to move · Hint g1–f3");
        state.arrows = vec![
            Arrow::solution(Square::E2, Square::E4),
            Arrow::solution(Square::D2, Square::D4),
        ];
        assert_eq!(describe(&state), "White to move · Solution e2–e4, d2–d4");
    }
}
