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

/// Icons scaled to a size they were not drawn for put lines between pixels on a 1x display.
/// An icon is drawn with the drawing made for its size, or one scaled a whole number of times.
#[test]
fn an_icon_uses_the_drawing_made_for_its_size() {
    let fluent = [12, 16, 20, 24, 28, 32, 48];
    // 1x: a 14 px box takes the 16 px drawing, whose margin it fits in.
    assert_eq!(gpui_kit::icon_drawing(14., &fluent), Some((16, 16)));
    assert_eq!(gpui_kit::icon_drawing(16., &fluent), Some((16, 16)));
    assert_eq!(gpui_kit::icon_drawing(18., &fluent), Some((20, 20)));
    assert_eq!(gpui_kit::icon_drawing(12., &fluent), Some((12, 12)));
    // 2x: the same boxes take the 28 and 32 px drawings.
    assert_eq!(gpui_kit::icon_drawing(28., &fluent), Some((32, 32)));
    assert_eq!(gpui_kit::icon_drawing(32., &fluent), Some((32, 32)));
    // KChess's own icons are drawn at 16 px only: doubled on a Retina display.
    assert_eq!(gpui_kit::icon_drawing(28., &[16]), Some((16, 32)));
    assert_eq!(gpui_kit::icon_drawing(14., &[16]), Some((16, 16)));
    // Too small for any drawing: the icon's own file is scaled.
    assert_eq!(gpui_kit::icon_drawing(12., &[16, 20]), None);
}

/// Looking for the sizes an icon is drawn at asked the app's assets for `monitor@12.svg`, which
/// does not exist; GPUI Kit's assets answer that with an error, and every icon without a 12 px
/// drawing (most of the sidebar's) was not drawn at all.
#[test]
fn an_icon_without_every_size_still_finds_its_drawing() {
    let renderer = gpui_kit::SvgRenderer::new(std::sync::Arc::new(crate::assets::Assets));
    let params = |name: &str, device: i32| gpui_kit::RenderSvgParams {
        path: format!("icons/{name}.svg").into(),
        size: gpui_kit::size(
            gpui_kit::DevicePixels(device * 2),
            gpui_kit::DevicePixels(device * 2),
        ),
    };
    let side = |name: &str, device: i32| {
        renderer
            .icon_drawing_for(&params(name, device))
            .map(|(_, side)| side)
    };
    assert_eq!(side("monitor", 14), Some(16));
    assert_eq!(side("monitor", 28), Some(32));
    // KChess's own drawings exist at 16 px only.
    assert_eq!(side("swords", 14), Some(16));
    assert_eq!(side("swords", 28), Some(32));
}
