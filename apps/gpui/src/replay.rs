//! A game gone through move by move: the board as large as the page allows,
//! the players above and below it, and the move panel. It opens at the last move; the arrow keys
//! step through the moves.

use gpui_kit::component::h_flex;
use gpui_kit::component::v_flex;
use gpui_kit::*;
use serde_json::Value;
use shakmaty::{Color, Position as _};

use crate::assets::icon;
use crate::board::ChessBoard;
use crate::game::view;
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::settings::SettingsHandle;
use crate::theme::Palette;
use crate::widgets::{self, Side};

pub struct GameReplay {
    setup: Value,
    moves: Vec<String>,
    sans: Vec<String>,
    ply: usize,
    /// The side shown at the bottom unless flipped.
    orientation: Color,
    flipped: bool,
    /// The player at the bottom and their opponent, as the board first shows them.
    player: SharedString,
    opponent: SharedString,
    board: Entity<ChessBoard>,
    layout: PlayLayout,
    focus: FocusHandle,
}

impl Focusable for GameReplay {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl GameReplay {
    pub fn new(
        setup: Value,
        moves: Vec<String>,
        orientation: Color,
        player: impl Into<SharedString>,
        opponent: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> GameReplay {
        let sans = view::sans(&setup, &moves);
        let ply = moves.len();
        let board =
            cx.new(|_| ChessBoard::new(view::board_state(&setup, &moves, ply, orientation, None)));
        GameReplay {
            setup,
            moves,
            sans,
            ply,
            orientation,
            flipped: false,
            player: player.into(),
            opponent: opponent.into(),
            board,
            layout: PlayLayout::default(),
            focus: cx.focus_handle(),
        }
    }

    fn shown_orientation(&self) -> Color {
        if self.flipped {
            !self.orientation
        } else {
            self.orientation
        }
    }

    /// The move shown.
    pub fn ply(&self) -> usize {
        self.ply
    }

    fn show(&mut self, ply: usize, cx: &mut Context<Self>) {
        self.ply = ply.min(self.moves.len());
        self.redraw(cx);
    }

    fn flip(&mut self, cx: &mut Context<Self>) {
        self.flipped = !self.flipped;
        self.redraw(cx);
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        let state = view::board_state(
            &self.setup,
            &self.moves,
            self.ply,
            self.shown_orientation(),
            None,
        );
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) || event.keystroke.modifiers.modified() {
            return;
        }
        if let Some(ply) = view::navigate(&event.keystroke.key, self.ply, self.moves.len()) {
            self.show(ply, cx);
            cx.stop_propagation();
        }
    }
}

impl Render for GameReplay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let set = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .piece_set
            .clone();
        let position = view::position_at(&self.setup, &self.moves, self.ply);
        let fen = position
            .as_ref()
            .map(kchess_domain::rules::make_fen)
            .unwrap_or_default();
        let turn = view::turn(position.as_ref());
        let over = position
            .as_ref()
            .is_some_and(|position| position.is_game_over());
        let orientation = self.shown_orientation();
        // The names stay with their sides when the board turns.
        let name = |color: Color| {
            if color == self.orientation {
                self.player.clone()
            } else {
                self.opponent.clone()
            }
        };
        let line = |color: Color| {
            widgets::player_line(
                h_flex()
                    .gap(px(10.))
                    .child(widgets::avatar(
                        icon("user"),
                        if color == Color::White {
                            Side::White
                        } else {
                            Side::Black
                        },
                        turn == color && !over,
                        &palette,
                    ))
                    .child(name(color))
                    .children(widgets::material(
                        &fen,
                        color == Color::White,
                        &set,
                        &palette,
                    )),
                None,
                None,
            )
        };
        let board_size = self.layout.board_size(window);
        let board_column = v_flex()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(line(!orientation))
            .child(crate::board::cached(&self.board))
            .child(line(orientation));

        let page = cx.entity().downgrade();
        let select = move |ply: usize, _: &mut Window, cx: &mut App| {
            let _ = page.update(cx, |page, cx| page.show(ply, cx));
        };
        let flip_page = cx.entity().downgrade();
        let flip: widgets::ClickHandler = Box::new(move |_, _, cx| {
            let _ = flip_page.update(cx, |page, cx| page.flip(cx));
        });
        let count = self.moves.len().div_ceil(2);
        let panel = widgets::side_panel(&palette)
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .child(widgets::section_title(
                "Moves",
                (!self.moves.is_empty())
                    .then(|| format!("{count} {}", if count == 1 { "move" } else { "moves" })),
                &palette,
            ))
            .child(div().flex_1().min_h(px(120.)).child(widgets::moves_list(
                "replay-moves",
                &self.sans,
                self.ply,
                view::black_first(&self.setup),
                "No moves to review.".into(),
                select.clone(),
                &palette,
            )))
            .child(widgets::move_controls(
                self.ply,
                self.moves.len(),
                select,
                ("Flip board".into(), flip),
            ));
        div()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)),
            )
            .child(
                self.layout
                    .render("replay", board_column, panel, window, cx),
            )
    }
}
