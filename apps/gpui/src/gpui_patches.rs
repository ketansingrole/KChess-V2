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
    // 1x: a 14 pt box takes the 16 px drawing, whose margin it fits in.
    assert_eq!(gpui_kit::icon_drawing(14., 1., &fluent), Some((16, 16)));
    assert_eq!(gpui_kit::icon_drawing(16., 1., &fluent), Some((16, 16)));
    assert_eq!(gpui_kit::icon_drawing(12., 1., &fluent), Some((12, 12)));
    // 2x: the same boxes take the 32 and 24 px drawings.
    assert_eq!(gpui_kit::icon_drawing(14., 2., &fluent), Some((32, 32)));
    assert_eq!(gpui_kit::icon_drawing(16., 2., &fluent), Some((32, 32)));
    assert_eq!(gpui_kit::icon_drawing(12., 2., &fluent), Some((24, 24)));
    // KChess's own icons are drawn at 16 px only: doubled on a Retina display.
    assert_eq!(gpui_kit::icon_drawing(14., 2., &[16]), Some((16, 32)));
    assert_eq!(gpui_kit::icon_drawing(14., 1., &[16]), Some((16, 16)));
    // No drawing for the size on screen: the next larger one, scaled down to it.
    assert_eq!(gpui_kit::icon_drawing(12., 1., &[16, 20]), Some((16, 10)));
    assert_eq!(gpui_kit::icon_drawing(14., 1.25, &fluent), Some((20, 20)));
    assert_eq!(gpui_kit::icon_drawing(20., 1.25, &fluent), Some((28, 25)));
    assert_eq!(gpui_kit::icon_drawing(18., 1.5, &[16]), Some((16, 24)));
    assert_eq!(gpui_kit::icon_drawing(12., 1., &[]), None);
}

/// Windows showed the sidebar's 18 pt icons at 20 px and the 14 pt Settings buttons beside them
/// at 16, where a Retina Mac shows both at 16 pt; at 150% a KChess drawing made at 16 px only was
/// drawn at 16 px, 10.7 pt in an 18 pt box. The size each display chose was the drawing that fit
/// its own pixels, so it moved with the scale factor. An icon is now as large in points on every
/// display as on a Retina one.
#[test]
fn an_icon_is_the_same_size_on_every_display() {
    let sets: [&[u32]; 4] = [
        &[12, 16, 20, 24, 28, 32, 48],
        &[16, 20, 24, 28, 32, 48],
        &[16, 20, 24, 28],
        &[16],
    ];
    for drawn in sets {
        for box_points in 10..=40 {
            let points = box_points as f32;
            let (_, retina) = gpui_kit::icon_drawing(points, 2., drawn).expect("a drawing");
            let size = retina as f32 / 2.;
            for scale in [1., 1.25, 1.5, 1.75, 2., 2.25, 2.5, 3.] {
                let (_, side) = gpui_kit::icon_drawing(points, scale, drawn).expect("a drawing");
                let on_screen = side as f32 / scale;
                assert!(
                    (on_screen - size).abs() <= 0.5 / scale + 1e-3,
                    "a {points} pt box with drawings {drawn:?} is {on_screen} pt at {scale}x                      and {size} pt at 2x"
                );
            }
        }
    }
    let fluent = [12, 16, 20, 24, 28, 32, 48];
    // The sidebar's icons and a small button's beside them.
    assert_eq!(gpui_kit::icon_drawing(18., 1., &fluent), Some((16, 16)));
    assert_eq!(gpui_kit::icon_drawing(18., 2., &fluent), Some((32, 32)));
    assert_eq!(gpui_kit::icon_drawing(18., 1.5, &fluent), Some((24, 24)));
}

/// Looking for the sizes an icon is drawn at asked the app's assets for `monitor@12.svg`, which
/// does not exist; GPUI Kit's assets answer that with an error, and every icon without a 12 px
/// drawing (most of the sidebar's) was not drawn at all.
#[test]
fn an_icon_without_every_size_still_finds_its_drawing() {
    let renderer = gpui_kit::SvgRenderer::new(std::sync::Arc::new(crate::assets::Assets));
    let side = |name: &str, points: i32, scale: f32| {
        let device = (points as f32 * scale * 2.).ceil() as i32;
        let params = gpui_kit::RenderSvgParams {
            path: format!("icons/{name}.svg").into(),
            size: gpui_kit::size(
                gpui_kit::DevicePixels(device),
                gpui_kit::DevicePixels(device),
            ),
            scale_milli: (scale * 1000.) as u32,
        };
        renderer.icon_drawing_for(&params).map(|(_, side)| side)
    };
    assert_eq!(side("monitor", 14, 1.), Some(16));
    assert_eq!(side("monitor", 14, 2.), Some(32));
    assert_eq!(side("monitor", 18, 1.), Some(16));
    assert_eq!(side("monitor", 18, 1.25), Some(20));
    // KChess's own drawings exist at 16 px only.
    assert_eq!(side("swords", 14, 1.), Some(16));
    assert_eq!(side("swords", 14, 2.), Some(32));
    assert_eq!(side("swords", 18, 1.5), Some(24));
}
