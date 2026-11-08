//! Opening dialogs and alert dialogs the way every page does.

use super::*;

/// How a dialog appears: at once when a key opened it (a shortcut, Enter on a focused button, a
/// command from the palette), as a command repeated many times a day should; after a click, a
/// fade settling from just above its place (the theme's 180 ms ease-out). The kit's default
/// slides the dialog down from the window's top edge, a long way for something opened this often.
pub fn dialog_entrance(window: &Window) -> gpui_kit::component::dialog::DialogEntrance {
    use gpui_kit::component::dialog::DialogEntrance;
    if window.last_input_was_keyboard() {
        DialogEntrance::None
    } else {
        DialogEntrance::FadeSlide
    }
}

/// `open_dialog`, entering as `dialog_entrance` says for the input that opened it.
pub fn open_dialog(
    window: &mut Window,
    cx: &mut App,
    build: impl Fn(
        gpui_kit::component::dialog::Dialog,
        &mut Window,
        &mut App,
    ) -> gpui_kit::component::dialog::Dialog
    + 'static,
) {
    use gpui_kit::component::WindowExt as _;
    let entrance = dialog_entrance(window);
    // In the middle of the window, over the page dimmed (`theme::OVERLAY`), so the dialog
    // stands apart from a board or list under it.
    window.open_dialog(cx, move |dialog, window, cx| {
        build(dialog.entrance(entrance).centered(), window, cx)
    });
}

/// `open_alert_dialog`, entering as `dialog_entrance` says for the input that opened it.
pub fn open_alert_dialog(
    window: &mut Window,
    cx: &mut App,
    build: impl Fn(
        gpui_kit::component::dialog::AlertDialog,
        &mut Window,
        &mut App,
    ) -> gpui_kit::component::dialog::AlertDialog
    + 'static,
) {
    use gpui_kit::component::WindowExt as _;
    let entrance = dialog_entrance(window);
    window.open_alert_dialog(cx, move |dialog, window, cx| {
        build(dialog.entrance(entrance).centered(), window, cx)
    });
}
