//! A list of a game database's games, as the Database page shows them and Guess the move picks
//! from: White, Black and the result, then the event, date, opening and length where there is
//! room, each row a button with a slot at its end for what the page puts there.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::GameDbRow;

use crate::theme::{Palette, radii};
use crate::widgets::{KeyboardFocus as _, Pressable as _};

/// The width of the slot at a row's end.
const END: f32 = 24.;

fn column(width: f32) -> Div {
    div().w(px(width)).flex_none().truncate()
}

/// The list's column names. `narrow` keeps White, Black and the result.
pub fn head(narrow: bool, palette: &Palette) -> Div {
    h_flex()
        .px(px(12.))
        .pb(px(8.))
        .gap(px(12.))
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(palette.text_muted)
        .child(div().flex_1().child("WHITE"))
        .child(div().flex_1().child("BLACK"))
        .child(column(64.).child("RESULT"))
        .when(!narrow, |head| {
            head.child(div().flex_1().child("EVENT"))
                .child(column(84.).child("DATE"))
                .child(column(44.).child("ECO"))
                .child(column(52.).text_right().child("MOVES"))
        })
        .child(div().w(px(END)).flex_none())
}

/// Game `game`'s row, named "White against Black, result" for a screen reader; `end` goes in the
/// slot at its end (an actions menu, a spinner while it opens).
pub fn row(
    id: impl Into<ElementId>,
    game: &GameDbRow,
    narrow: bool,
    end: Option<AnyElement>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    palette: &Palette,
) -> AnyElement {
    let muted = palette.text_muted;
    let player = |name: &str, elo: Option<u32>| {
        h_flex()
            .flex_1()
            .min_w_0()
            .gap(px(6.))
            .items_baseline()
            .child(div().truncate().child(name.to_string()))
            .children(elo.map(|elo| {
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(muted)
                    .child(elo.to_string())
            }))
    };
    h_flex()
        .id(id)
        .test_support()
        .role(Role::Button)
        .keyboard_focus(palette.primary)
        .aria_label(label(game))
        .px(px(12.))
        .py(px(8.))
        .gap(px(12.))
        .rounded(radii::BOX)
        .text_size(px(13.))
        .cursor_pointer()
        .pressable(palette)
        .on_click(on_click)
        .child(player(&game.white, game.white_elo))
        .child(player(&game.black, game.black_elo))
        .child(
            column(64.)
                .font_weight(FontWeight::MEDIUM)
                .child(game.result.clone()),
        )
        .when(!narrow, |row| {
            row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(muted)
                    .child(game.event.clone().unwrap_or_default()),
            )
            .child(
                column(84.)
                    .text_color(muted)
                    .child(game.date.clone().unwrap_or_default()),
            )
            .child(
                column(44.)
                    .text_color(muted)
                    .child(game.eco.clone().unwrap_or_default()),
            )
            .child(
                column(52.)
                    .text_right()
                    .text_color(muted)
                    .child(game.plies.div_ceil(2).to_string()),
            )
        })
        .child(
            h_flex()
                .w(px(END))
                .flex_none()
                .justify_center()
                .children(end),
        )
        .into_any_element()
}

/// What a screen reader calls game `game`: "White against Black, result".
pub fn label(game: &GameDbRow) -> String {
    format!("{} against {}, {}", game.white, game.black, game.result)
}
