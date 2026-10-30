//! Regressions for the KChess patches to GPUI (`vendor/gpui-pre/KCHESS-PATCH.md`) that the
//! vendored crate's own tests cannot check: they do not build without its example assets.

use std::sync::Arc;

use gpui_kit::{
    AppContext as _, Context, IntoElement, Render, TestAppContext, TextRun, Window, div, font, px,
};

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// A page opened again showed text shaped for it before, and CoreText shaped it all again: most
/// of the time a page took to open.
#[gpui_kit::test]
fn text_not_shown_for_a_while_is_not_shaped_again(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, _| Blank);
    let run = TextRun {
        len: "Puzzles".len(),
        font: font("Inter"),
        ..Default::default()
    };
    cx.update_window(window.into(), |_, window, cx| {
        let shaped =
            window
                .text_system()
                .layout_line("Puzzles", px(14.), std::slice::from_ref(&run), None);
        for _ in 0..4 {
            window.refresh();
            window.draw(cx).clear(cx);
        }
        let again =
            window
                .text_system()
                .layout_line("Puzzles", px(14.), std::slice::from_ref(&run), None);
        assert!(Arc::ptr_eq(&shaped, &again), "shaped again");
    })
    .unwrap();
}
