//! Opening dialogs and alert dialogs the way every page does, and their row of buttons.

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

/// Asks before something that cannot be undone: `title`, what it does, and the danger button
/// `ok` beside Cancel, which runs `act`.
pub fn confirm_danger(
    window: &mut Window,
    cx: &mut App,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    ok: impl Into<SharedString>,
    act: impl Fn(&mut Window, &mut App) + 'static,
) {
    use gpui_kit::component::button::ButtonVariant;
    let (title, description, ok) = (title.into(), description.into(), ok.into());
    let act = std::rc::Rc::new(act);
    open_alert_dialog(window, cx, move |dialog, _, _| {
        let act = act.clone();
        dialog
            .title(title.clone())
            .description(description.clone())
            .ok_text(ok.clone())
            .ok_variant(ButtonVariant::Danger)
            .cancel_text("Cancel")
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                act(window, cx);
                true
            })
    });
}

/// A dialog's last row: its buttons on the right, anything `leading` on the left, and what went
/// wrong between them, spoken as it appears. A line of its own above the buttons grew the dialog,
/// which the kit keeps centred, so the button just pressed moved under the pointer as the error
/// came; beside them, an error of one line moves nothing.
pub fn dialog_footer(
    error: Option<(&'static str, SharedString)>,
    leading: Option<AnyElement>,
    buttons: impl IntoIterator<Item = AnyElement>,
    palette: &Palette,
) -> Div {
    h_flex()
        .w_full()
        .items_center()
        .gap(px(8.))
        .children(leading)
        .child(div().flex_1().min_w_0().children(error.map(|(id, error)| {
            div()
                .id(id)
                .test_support()
                .aria_label(error.clone())
                .relative()
                .text_size(px(13.))
                .text_color(palette.error)
                .child(live_region(
                    SharedString::from(format!("{id}-spoken")),
                    error.clone(),
                ))
                .child(error)
        })))
        .children(buttons)
}
