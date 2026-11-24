//! The component gallery (`kchess_ui::gallery`) in a window of its own, from Quick Pick in a
//! development build.

use gpui_kit::*;
use kchess_contracts::Level;

pub fn open(_: &mut Window, cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(None, size(px(760.), px(820.)), cx)),
        titlebar: Some(TitlebarOptions {
            title: Some(SharedString::new_static("Component gallery")),
            ..TitlebarOptions::default()
        }),
        ..WindowOptions::default()
    };
    let opened = gpui_kit::open_window(options, cx, |_, cx| {
        cx.new(|_| kchess_ui::gallery::Gallery::new(kchess_ui::gallery::samples()))
    });
    if let Err(cause) = opened {
        crate::log::write(
            Level::Error,
            "app",
            &format!("The gallery could not open: {cause}"),
        );
    }
}
