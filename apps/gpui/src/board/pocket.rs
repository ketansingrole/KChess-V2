//! Crazyhouse pockets: the pieces a side has captured and may drop back onto the board, shown
//! beside that side's name as Lichess shows them. Click one to pick it (the board then marks
//! where it can land) or drag it onto a square.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use shakmaty::{Color, Piece, Role};

use super::{ChessBoard, looks};
use crate::settings::SettingsHandle;
use crate::theme::{Palette, radii, tint};
use crate::widgets::KeyboardFocus as _;

/// The droppable roles, in Lichess's pocket order.
const ROLES: [Role; 5] = [
    Role::Pawn,
    Role::Knight,
    Role::Bishop,
    Role::Rook,
    Role::Queen,
];

/// A pocket piece dragged towards the board.
pub struct PocketPiece(pub Role);

/// The piece a dragged spare or pocket piece shows under the pointer.
pub struct PieceGhost {
    pub image: SharedString,
}

impl Render for PieceGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        img(self.image.clone()).size(px(56.))
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Pawn => "pawn",
        Role::Knight => "knight",
        Role::Bishop => "bishop",
        Role::Rook => "rook",
        Role::Queen => "queen",
        Role::King => "king",
    }
}

/// The pocket of `color` for the position on `board`; `None` outside Crazyhouse.
pub fn pocket(board: &Entity<ChessBoard>, color: Color, cx: &App) -> Option<AnyElement> {
    let shown = board.read(cx);
    let held = *shown.state.pockets.as_ref()?.get(color);
    let movable = shown.state.movable == Some(color);
    let picked = shown.selected_drop.filter(|_| movable);
    let drops: Vec<Role> = ROLES
        .into_iter()
        .filter(|role| movable && shown.state.drops.contains_key(role))
        .collect();
    let palette = *cx.global::<Palette>();
    let set = looks::piece_set(
        &cx.global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .piece_set,
    )
    .to_string();
    let side = if color == Color::White {
        "White"
    } else {
        "Black"
    };
    let cell = px(28.);
    let slots = ROLES.into_iter().map(|role| {
        let count = *held.get(role);
        let image = SharedString::from(looks::piece_path(&set, Piece { color, role }));
        let can_drop = drops.contains(&role);
        let selected = picked == Some(role);
        let label = format!("{side} {} in hand: {count}", role_name(role));
        let entity = board.downgrade();
        let ghost = image.clone();
        div()
            .id(SharedString::from(format!("pocket-{color:?}-{role:?}")))
            .test_support()
            .relative()
            .size(cell)
            .flex()
            .items_center()
            .justify_center()
            .rounded(radii::SM)
            .aria_label(label)
            .when(count == 0, |slot| slot.opacity(0.25))
            .when(selected, |slot| {
                slot.bg(tint(palette.primary, 30.))
                    .border_1()
                    .border_color(palette.primary)
            })
            .when(can_drop, |slot| {
                slot.role(gpui_kit::Role::Button)
                    .aria_selected(selected)
                    .keyboard_focus(palette.primary)
                    .cursor_pointer()
                    .when(!selected, |slot| {
                        slot.hover(|slot| slot.bg(palette.accented))
                    })
                    .on_click(move |_, _, cx| {
                        let _ = entity.update(cx, |board, cx| board.select_drop(role, cx));
                    })
                    .on_drag(PocketPiece(role), move |_, _, _, cx| {
                        cx.new(|_| PieceGhost {
                            image: ghost.clone(),
                        })
                    })
            })
            .child(img(image).size(cell))
            .when(count > 1, |slot| {
                slot.child(
                    div()
                        .absolute()
                        .right(px(-3.))
                        .bottom(px(-3.))
                        .px(px(3.))
                        .rounded(radii::SM)
                        .bg(palette.inverted)
                        .text_color(palette.bg)
                        .text_size(px(10.))
                        .font_weight(FontWeight::BOLD)
                        .line_height(px(12.))
                        .child(count.to_string()),
                )
            })
    });
    Some(
        h_flex()
            .id(SharedString::from(format!("pocket-{color:?}")))
            .flex_none()
            .gap(px(2.))
            .children(slots)
            .into_any_element(),
    )
}
