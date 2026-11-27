//! A game gone through move by move: the board as large as the page allows,
//! the players above and below it, and the move panel. It opens at the last move; the arrow keys,
//! the mouse wheel over the board and the move list step through the moves. A page shows it under
//! its own toolbar (History's games, a database's) and keeps what goes around it: closing, the
//! previous and next game.

use gpui_kit::component::h_flex;
use gpui_kit::component::v_flex;
use gpui_kit::*;
use kchess_domain::variant::Setup;
use shakmaty::{Color, Position as _};

use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::widgets::{self, Side};
use kchess_app::settings;
use kchess_board::board::{self, ChessBoard};
use kchess_board::game::surface::{self, Steps};
use kchess_board::game::view;
use kchess_board::{FlipBoard, notation};
use kchess_ui::icon;
use kchess_ui::theme::Palette;

/// A game stepped through, as a view (`GameReplay::new`).
pub struct GameReplay {
    setup: Setup,
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
    /// The mouse wheel over the board, which steps through the moves.
    wheel: view::Wheel,
}

impl Focusable for GameReplay {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl GameReplay {
    /// `moves` from `setup`, `orientation`'s side at the bottom, named `player`, against
    /// `opponent`.
    pub fn new(
        setup: Setup,
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
            wheel: Default::default(),
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
}

impl Steps for GameReplay {
    fn steps(&self, _: &App) -> Option<(usize, usize)> {
        Some((self.ply, self.moves.len()))
    }

    fn step_to(&mut self, ply: usize, _: &mut Window, cx: &mut Context<Self>) {
        self.show(ply, cx);
    }

    fn wheel(&mut self) -> &mut view::Wheel {
        &mut self.wheel
    }
}

impl Render for GameReplay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let set = settings::frontend(cx).piece_set.clone();
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
        // A Crazyhouse game shows each side's pocket where the material difference would be.
        let mut pockets = [
            board::pocket(&self.board, Color::White, cx),
            board::pocket(&self.board, Color::Black, cx),
        ];
        let mut line =
            |color: Color| {
                widgets::player_line(
                    h_flex()
                        .gap(px(8.))
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
                        .children(pockets[usize::from(color == Color::Black)].take().or_else(
                            || widgets::material(&fen, color == Color::White, &set, &palette),
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
            .child(board::cached(&self.board))
            .child(line(orientation));
        let board_column = surface::wheel(board_column, cx);

        let page = cx.entity().downgrade();
        let select = move |ply: usize, _: &mut Window, cx: &mut App| {
            page.update(cx, |page, cx| page.show(ply, cx)).ok();
        };
        let flip_page = cx.entity().downgrade();
        let flip: widgets::ClickHandler = Box::new(move |_, _, cx| {
            flip_page.update(cx, |page, cx| page.flip(cx)).ok();
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
                widgets::MoveSheet {
                    sans: &self.sans,
                    shown: self.ply,
                    first: view::first_move(&self.setup),
                },
                "No moves to review".into(),
                select.clone(),
                notation::figurine(cx),
                &palette,
            )))
            .child(widgets::move_controls(
                self.ply,
                self.moves.len(),
                select,
                widgets::TurnButton::Flip(flip),
                cx,
            ));
        surface::keys(div().size_full(), &self.focus, cx)
            .on_action(cx.listener(|this, _: &FlipBoard, _, cx| this.flip(cx)))
            .child(
                self.layout
                    .render("replay", board_column, panel, window, cx),
            )
    }
}
